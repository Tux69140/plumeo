//! Détection des titres en cascade (signets PDF → gras + majuscules → taille)
//! et des pages de sommaire d'origine.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::blocs::{Bloc, Paragraphe, Segment};
use crate::markdown::AncresTitres;
use crate::modele::{Ancre, DemiPage, Moitie, Signet, demi_page_suivante};
use crate::paragraphes::POINTS_DE_CONDUITE_ET_NUMERO;
use crate::points::{GenrePoint, PointAVerifier};
use crate::reglages::SourceTitres;
use crate::zones::{Retrait, Role};

const MAX_MOTS_TITRE: usize = 12;
const MAX_LIGNES_TITRE: usize = 3;
const ECART_TAILLE_TITRE: f32 = 1.5;
const MIN_PAGES_TITRE_COURANT: usize = 5;
/// Part minimale des pages du livre où un titre courant doit se trouver en bordure.
const PART_MIN_TITRE_COURANT: f32 = 0.10;
/// Écart de taille (pt) en deçà duquel deux titres voisins sont du même niveau.
const TOLERANCE_TAILLE: f32 = 0.5;
/// Étendue maximale (pt) des tailles d'un même niveau.
const ETENDUE_NIVEAU: f32 = 1.0;
/// Nombre de titres à partir duquel une taille (ou le mot « chapitre ») fait un niveau.
const MIN_TITRES_NIVEAU: usize = 3;
/// Pages du début du livre où chercher le titre du livre.
const PAGES_TITRE_LIVRE: u32 = 10;

fn arrondi_demi(taille: f32) -> f32 {
    (taille * 2.0).round() / 2.0
}

fn nb_caracteres(p: &Paragraphe) -> usize {
    p.texte_brut().chars().count()
}

/// Taille dominante des paragraphes, pondérée par le nombre de caractères
/// (tailles arrondies à 0,5 pt). 0 s'il n'y a aucun paragraphe.
pub fn taille_corps(blocs: &[Bloc]) -> f32 {
    let mut poids: BTreeMap<i32, usize> = BTreeMap::new();
    for bloc in blocs {
        if let Bloc::Paragraphe(p) = bloc {
            let cle = (arrondi_demi(p.taille) * 2.0) as i32;
            *poids.entry(cle).or_default() += nb_caracteres(p);
        }
    }
    let mut meilleur: Option<(i32, usize)> = None;
    for (cle, poids) in poids {
        if meilleur.is_none_or(|(_, m)| poids > m) {
            meilleur = Some((cle, poids));
        }
    }
    meilleur.map_or(0.0, |(cle, _)| cle as f32 / 2.0)
}

/// Minuscules, sans diacritiques, tout non-alphanumérique devient espace, espaces réduits.
pub fn normaliser(s: &str) -> String {
    let nettoye: String = s
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    nettoye.split_whitespace().collect::<Vec<_>>().join(" ")
}

const NOMBRES: [&str; 21] = [
    "zero", "un", "deux", "trois", "quatre", "cinq", "six", "sept", "huit", "neuf", "dix", "onze", "douze",
    "treize", "quatorze", "quinze", "seize", "dix sept", "dix huit", "dix neuf", "vingt",
];

/// `normaliser`, et les nombres de 0 à 20 écrits en lettres (« LES 7 COULEURS » =
/// « LES SEPT COULEURS ») : sert à retrouver un titre de signet ou de résumé.
fn normaliser_titre(s: &str) -> String {
    normaliser(s)
        .split(' ')
        .map(|m| m.parse::<usize>().ok().and_then(|n| NOMBRES.get(n).copied()).unwrap_or(m))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Les signets servent-ils de plan : assez nombreux, pas un par page, pas des « Page N ».
pub fn signets_utilisables(signets: &[Signet], nb_pages: u32) -> bool {
    if signets.len() < 3 {
        return false;
    }
    let pages: BTreeSet<u32> = signets.iter().map(|s| s.page).collect();
    if pages.len() < 3 {
        return false;
    }
    if signets.len() as f32 >= 0.9 * nb_pages as f32 {
        return false;
    }
    let factices = Regex::new(r"(?i)^(page|p\.?)\s*\d+$|^\d+$|\.(jpe?g|png|tiff?)$").unwrap();
    let nb_factices = signets.iter().filter(|s| factices.is_match(s.titre.trim())).count();
    (nb_factices as f32) < 0.5 * signets.len() as f32
}

fn nb_mots(texte: &str) -> usize {
    texte.split_whitespace().count()
}

fn taux_majuscules(texte: &str) -> f32 {
    let lettres = texte.chars().filter(|c| c.is_alphabetic()).count();
    if lettres == 0 {
        return 0.0;
    }
    texte.chars().filter(|c| c.is_uppercase()).count() as f32 / lettres as f32
}

fn court(p: &Paragraphe, texte: &str) -> bool {
    nb_mots(texte) <= MAX_MOTS_TITRE && p.nb_lignes <= MAX_LIGNES_TITRE
}

fn candidat_gras_majuscules(p: &Paragraphe) -> bool {
    let texte = p.texte_brut();
    p.tout_gras && taux_majuscules(&texte) >= 0.8 && court(p, &texte)
}

fn candidat_taille(p: &Paragraphe, corps: f32) -> bool {
    p.taille >= corps + ECART_TAILLE_TITRE && court(p, &p.texte_brut())
}

/// Part minimale de lettres parmi les caractères non blancs d'un titre.
const PART_MIN_LETTRES_TITRE: f32 = 0.7;
/// Caractères qui trahissent une ligne issue d'un OCR défaillant ou d'un schéma.
const CARACTERES_ILLISIBLES: &str = "~\\|{}[]<>_^@";

/// Le texte d'un candidat à titre est-il lisible ? Un OCR défaillant (symboles du
/// zodiaque, texte de schémas) donne des lignes qui n'ont rien d'un titre. Il faut :
/// au moins 3 lettres ; au moins 70 % de lettres parmi les caractères non blancs ;
/// un premier caractère qui est une lettre, un chiffre, `(`, `«` ou `*` (les titres
/// encadrés d'astérisques, « * SEPHIRA KÉTHER * », sont de vrais titres) ; aucun des
/// caractères `~ \ | { } [ ] < > _ ^ @` ; aucun `--` ; aucun mot qui mêle lettres et
/// chiffres (`LI11î`, `SPIRITUEI.1LE`). Les ordinaux (`2e`, `1er`, `3ème`, `1ère`,
/// `4es`), qui se rencontrent dans de vrais titres, ne comptent pas comme mêlés ;
/// la ponctuation autour du mot est ignorée pour ce test. Une ligne de date seule
/// (`est_date`) est toujours plausible : ses chiffres feraient chuter la part de lettres.
fn titre_plausible(texte: &str) -> bool {
    static ORDINAL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^\d+(?:e|er|re|ère|ème|es)$").expect("motif valide"));
    let texte = texte.trim();
    if est_date(texte) {
        return true;
    }
    let non_blancs = texte.chars().filter(|c| !c.is_whitespace()).count();
    let lettres = texte.chars().filter(|c| c.is_alphabetic()).count();
    if lettres < 3 || (lettres as f32) < PART_MIN_LETTRES_TITRE * non_blancs as f32 {
        return false;
    }
    if !texte.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '(' || c == '«' || c == '*') {
        return false;
    }
    if texte.chars().any(|c| CARACTERES_ILLISIBLES.contains(c)) || texte.contains("--") {
        return false;
    }
    !texte.split_whitespace().any(|mot| {
        let mot = mot.trim_matches(|c: char| !c.is_alphanumeric());
        mot.chars().any(char::is_alphabetic) && mot.chars().any(|c| c.is_ascii_digit()) && !ORDINAL.is_match(mot)
    })
}

/// Un paragraphe est-il un texte plausible de titre ? Seuls les segments de texte comptent :
/// les exposants (appels de note, « e » d'un ordinal) sont ignorés, sans quoi
/// « Introduction » + exposant « 1 » ferait un mot mêlant lettres et chiffres.
fn paragraphe_titre_plausible(p: &Paragraphe) -> bool {
    let texte: String = p
        .segments
        .iter()
        .filter_map(|s| match s {
            Segment::Texte { texte, .. } => Some(texte.as_str()),
            _ => None,
        })
        .collect();
    titre_plausible(&texte)
}

/// Numéro de page en fin de paragraphe, après une espace ou des points de conduite
/// (`POINTS_DE_CONDUITE_ET_NUMERO`). Après des points de conduite, « l 07 » ou « I07 »
/// se lit 107 (OCR).
static NUMERO_DE_PAGE_FINAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"{POINTS_DE_CONDUITE_ET_NUMERO}$|[\s.\u{{2026}}](?P<nombre>\d{{1,4}})$"))
        .expect("motif valide")
});

/// Numéro de page qui finit le texte d'une ligne de sommaire, s'il y en a un.
fn numero_de_page_final(texte: &str) -> Option<u32> {
    let c = NUMERO_DE_PAGE_FINAL.captures(texte)?;
    match (c.name("ocr"), c.name("page").or_else(|| c.name("nombre"))) {
        (Some(ocr), _) => format!("1{}", ocr.as_str()).parse().ok(),
        (None, Some(n)) => n.as_str().parse().ok(),
        _ => None,
    }
}

/// Une (demi-)page, faite des paragraphes donnés dans l'ordre de lecture (numéro de page
/// final, paragraphe court), est-elle un sommaire ? Il faut au moins 4 paragraphes
/// numérotés, et soit qu'ils fassent au moins la moitié des paragraphes, soit que tous
/// les paragraphes soient courts (entrées suivies de lignes de titre, en-têtes de partie)
/// et que leurs numéros soient croissants (égalités admises : deux entrées sur la même
/// page) et plausibles comme numéros de page : au plus 2 × `derniere_page` + 20 (une
/// chronologie, « 1903 », « 1908 », n'est pas un sommaire).
fn moitie_de_sommaire(paragraphes: &[(Option<u32>, bool)], derniere_page: u32) -> bool {
    let numeros: Vec<u32> = paragraphes.iter().filter_map(|(n, _)| *n).collect();
    let plausible = |n: &u32| *n <= 2 * derniere_page + 20;
    numeros.len() >= 4
        && (numeros.len() * 2 >= paragraphes.len()
            || (numeros.windows(2).all(|w| w[1] >= w[0])
                && numeros.iter().all(plausible)
                && paragraphes.iter().all(|(_, court)| *court)))
}

/// (Demi-)pages qui ressemblent à un sommaire (`moitie_de_sommaire`) : leurs paragraphes
/// finissent par un numéro de page, après une espace ou des points de conduite. Des
/// points de suspension au milieu du texte ne suffisent pas. Comme dans
/// `sommaire_cliquable`, la (demi-)page qui précède directement une (demi-)page de
/// sommaire (dans l'ordre de lecture, parmi celles qui portent des paragraphes : p3g
/// avant p3d) en est aussi une si elle porte au moins un paragraphe numéroté et si tous
/// ses paragraphes sont courts (le titre « Table des matières » ou « Sommaire » l'est) ;
/// la remontée va de proche en proche, mais jamais au-delà d'une (demi-)page qui porte
/// le titre du sommaire (`est_titre_de_sommaire`).
fn moities_sommaire(blocs: &[Bloc]) -> BTreeSet<DemiPage> {
    // Par (demi-)page, dans l'ordre de lecture : (numéro de page final, court) par
    // paragraphe.
    let mut par_moitie: BTreeMap<DemiPage, Vec<(Option<u32>, bool)>> = BTreeMap::new();
    let mut titrees: HashSet<DemiPage> = HashSet::new();
    for bloc in blocs {
        if let Bloc::Paragraphe(p) = bloc {
            let texte = p.texte_brut();
            let texte = texte.trim();
            let lieu = p.ancre.demi_page();
            if est_titre_de_sommaire(texte) {
                titrees.insert(lieu);
            }
            par_moitie.entry(lieu).or_default().push((numero_de_page_final(texte), court(p, texte)));
        }
    }
    let derniere_page = blocs.iter().map(|b| b.ancre().page).max().unwrap_or(0);
    let lieux: Vec<DemiPage> = par_moitie.keys().copied().collect();
    let mut sommaire: Vec<bool> =
        lieux.iter().map(|lieu| moitie_de_sommaire(&par_moitie[lieu], derniere_page)).collect();
    for r in (0..lieux.len().saturating_sub(1)).rev() {
        let (lieu, suivante) = (lieux[r], lieux[r + 1]);
        let paragraphes = &par_moitie[&lieu];
        if !sommaire[r]
            && sommaire[r + 1]
            && contigues(lieu, suivante)
            && !titrees.contains(&suivante)
            && paragraphes.iter().any(|(numero, _)| numero.is_some())
            && paragraphes.iter().all(|(_, court)| *court)
        {
            sommaire[r] = true;
        }
    }
    lieux.into_iter().zip(sommaire).filter(|(_, sommaire)| *sommaire).map(|(lieu, _)| lieu).collect()
}

/// La (demi-)page `b`, qui suit `a` parmi les (demi-)pages qui portent des paragraphes,
/// la suit-elle directement : même page ou page suivante ?
fn contigues(a: DemiPage, b: DemiPage) -> bool {
    b.0 <= a.0 + 1
}

/// Le bloc est-il un paragraphe d'une (demi-)page de `sommaires` ?
fn est_sommaire(bloc: &Bloc, sommaires: &BTreeSet<DemiPage>) -> bool {
    matches!(bloc, Bloc::Paragraphe(_)) && sommaires.contains(&bloc.ancre().demi_page())
}

/// Une moitié au moins de `page` est-elle dans `demi_pages` ?
fn page_touchee(demi_pages: &BTreeSet<DemiPage>, page: u32) -> bool {
    demi_pages.range((page, Moitie::Entiere)..=(page, Moitie::Droite)).next().is_some()
}

/// Un point `Sommaire` par page de `sommaires`, ancré au premier bloc de sa première
/// (demi-)page de sommaire dans l'ordre de lecture ; `message` reçoit le numéro de page.
fn points_de_sommaire(
    blocs: &[Bloc],
    sommaires: &BTreeSet<DemiPage>,
    message: impl Fn(u32) -> String,
) -> Vec<PointAVerifier> {
    let mut points: Vec<PointAVerifier> = Vec::new();
    for &demi_page in sommaires {
        if points.last().is_some_and(|p| p.ancre.page == demi_page.0) {
            continue;
        }
        if let Some(bloc) = blocs.iter().find(|b| b.ancre().demi_page() == demi_page) {
            let (ancre, genre) = (bloc.ancre(), GenrePoint::Sommaire);
            points.push(PointAVerifier { ancre, genre, message: message(demi_page.0) });
        }
    }
    points
}

/// Signet à chercher dans le corps. `texte_impose` : le titre garde le texte du signet
/// (numéro et titre fusionnés) au lieu de celui des paragraphes retrouvés, si ceux-ci
/// le disent exactement ; s'ils disent plus, leur texte est gardé (« I. » en tête).
/// `numero_avant` (entrées d'un sommaire imprimé) : le paragraphe fait seulement d'un
/// numéro (`numero_seul_avant`), juste avant les paragraphes retrouvés et sur la même
/// (demi-)page, est absorbé dans le titre (« I » + « LES BASES » → « I. LES BASES »),
/// qui est alors ancré au numéro. `trouve` : paragraphes `debut..fin` déjà retrouvés
/// (entrées d'un sommaire imprimé, `entrees_du_sommaire`), dans les mêmes blocs ; le
/// signet n'est alors pas cherché une seconde fois.
struct SignetCherche {
    signet: Signet,
    texte_impose: bool,
    numero_avant: bool,
    trouve: Option<(usize, usize)>,
}

impl From<Signet> for SignetCherche {
    fn from(signet: Signet) -> Self {
        SignetCherche { signet, texte_impose: false, numero_avant: false, trouve: None }
    }
}

/// Type d'une numérotation de titre, ou genre d'un numéro seul.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Numerotation {
    Arabe,
    Romaine,
}

/// Numéro romain bien formé (« IX », « XIV. », pas « IIX » ni « VX ») ; peut être vide.
static ROMAIN_BIEN_FORME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^M{0,3}(CM|CD|D?C{0,3})(XC|XL|L?X{0,3})(IX|IV|V?I{0,3})\.?$").expect("motif valide")
});
/// Nombre arabe de 1 à 999 (« 12 », « 3. ») : ni « 0 », ni « 07 », ni « 1984 ».
static ARABE_COURT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[1-9]\d{0,2}\.?$").expect("motif valide"));
static NUMEROTE_ARABE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+\.?(?:\s|$)").unwrap());
static NUMEROTE_ROMAIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[IVXLCDM]+\.?(?:\s|$)").unwrap());
/// Numéro en tête d'un texte de titre, suivi ou non d'un point (« I La pierre »).
static NUMERO_EN_TETE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([IVXLCDM]+|\d+)\.?\s+").unwrap());
/// Numéro en tête d'un titre visé par un sommaire, suivi d'un point (« I. », « IV. »,
/// « 3. ») : « IL ÉTAIT… » ou « DIX CONFÉRENCES » ne commencent pas par un numéro. Voir
/// `sans_numero_pointe`.
static NUMERO_POINTE_EN_TETE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([IVXLCDM]+|\d+)\.\s+").expect("motif valide"));
/// Initiale en tête : lettre seule suivie d'un point (« G. JUNG »).
static INITIALE_EN_TETE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\p{L}\.(?:\s|$)").expect("motif valide"));

/// Texte d'un titre sans son numéro en tête suivi d'un point (`NUMERO_POINTE_EN_TETE`),
/// ou `None` s'il n'en a pas. Un numéro romain doit être bien formé (`numero_seul` :
/// pas « IIII. ») ; d'une seule lettre, il ne peut être que I, V ou X (« M. DUPONT » :
/// pas un numéro), et pas suivi d'une autre initiale (« X. Y. Z. »).
fn sans_numero_pointe(texte: &str) -> Option<&str> {
    let tete = NUMERO_POINTE_EN_TETE.captures(texte)?;
    let numero = tete.get(1)?.as_str();
    let reste = &texte[tete.get(0)?.end()..];
    let romain = !numero.starts_with(|c: char| c.is_ascii_digit());
    let lettre_refusee = numero.len() == 1 && (!matches!(numero, "I" | "V" | "X") || INITIALE_EN_TETE.is_match(reste));
    if romain && (numero_seul(numero).is_none() || lettre_refusee) {
        return None;
    }
    Some(reste)
}

/// Genre et valeur d'un texte fait seulement d'un numéro bien formé, suivi ou non d'un
/// point : un romain non vide (`ROMAIN_BIEN_FORME`) ou un nombre arabe de 1 à 999.
fn numero_seul(texte: &str) -> Option<(Numerotation, u32)> {
    if ARABE_COURT.is_match(texte) {
        return texte.trim_end_matches('.').parse().ok().map(|n| (Numerotation::Arabe, n));
    }
    let romain = texte.trim_end_matches('.');
    if romain.is_empty() || !ROMAIN_BIEN_FORME.is_match(texte) {
        return None;
    }
    let chiffre = |c: char| match c {
        'I' => 1,
        'V' => 5,
        'X' => 10,
        'L' => 50,
        'C' => 100,
        'D' => 500,
        _ => 1000,
    };
    let chiffres: Vec<u32> = romain.chars().map(chiffre).collect();
    // Un chiffre plus petit que le suivant se retranche (« IX » = 10 − 1).
    let valeur = chiffres
        .iter()
        .enumerate()
        .map(|(k, &v)| if chiffres.get(k + 1).is_some_and(|&w| w > v) { -(v as i64) } else { v as i64 })
        .sum::<i64>() as u32;
    Some((Numerotation::Romaine, valeur))
}

fn numerotation(titre: &str) -> Option<Numerotation> {
    let titre = titre.trim();
    if NUMEROTE_ARABE.is_match(titre) {
        Some(Numerotation::Arabe)
    } else if NUMEROTE_ROMAIN.is_match(titre) {
        Some(Numerotation::Romaine)
    } else {
        None
    }
}

/// Signets d'un plan à plat (Calibre) rendus utilisables comme plan :
/// - deux signets consécutifs de même niveau et de même page, le premier n'étant qu'un
///   numéro (« I », « 3. »), n'en font qu'un : « I. La pierre cubique » ;
/// - une numérotation romaine qui repart à « I » après un signet numéroté en chiffres
///   arabes, au même niveau, descend d'un niveau jusqu'au prochain signet arabe de ce
///   niveau. Les signets non numérotés entre les deux gardent leur niveau.
fn preparer_signets(signets: &[Signet]) -> Vec<SignetCherche> {
    let mut fusionnes: Vec<SignetCherche> = Vec::with_capacity(signets.len());
    let mut i = 0;
    while i < signets.len() {
        let s = &signets[i];
        let suivant = signets.get(i + 1).filter(|n| {
            numero_seul(s.titre.trim()).is_some()
                && numero_seul(n.titre.trim()).is_none()
                && n.niveau == s.niveau
                && n.page == s.page
        });
        if let Some(n) = suivant {
            let titre = format!("{}. {}", s.titre.trim().trim_end_matches('.'), n.titre.trim());
            fusionnes.push(SignetCherche {
                signet: Signet { titre, ..s.clone() },
                texte_impose: true,
                numero_avant: false,
                trouve: None,
            });
            i += 2;
        } else {
            fusionnes.push(s.clone().into());
            i += 1;
        }
    }

    // Niveau (avant descente) du dernier signet arabe, et niveau de la série romaine en cours.
    let mut dernier_arabe: Option<u8> = None;
    let mut serie: Option<u8> = None;
    for sc in &mut fusionnes {
        let niveau = sc.signet.niveau;
        match numerotation(&sc.signet.titre) {
            Some(Numerotation::Arabe) => {
                dernier_arabe = Some(niveau);
                if serie == Some(niveau) {
                    serie = None;
                }
            }
            Some(Numerotation::Romaine) => {
                let numero = sc.signet.titre.split_whitespace().next().unwrap_or("").trim_end_matches('.');
                let repart_a_un = numero == "I";
                if serie.is_none() && repart_a_un && dernier_arabe == Some(niveau) {
                    serie = Some(niveau);
                }
                if serie == Some(niveau) {
                    sc.signet.niveau = niveau + 1;
                }
            }
            None => {}
        }
    }
    fusionnes
}

/// Texte, genre et valeur du paragraphe qui précède `debut`, s'il est fait seulement d'un
/// numéro bien formé (`numero_seul`), sur la même (demi-)page, et pas déjà pris dans un
/// titre.
fn numero_seul_avant(
    blocs: &[Bloc],
    debut: usize,
    utilises: &HashSet<usize>,
) -> Option<(String, (Numerotation, u32))> {
    let i = debut.checked_sub(1).filter(|i| !utilises.contains(i))?;
    let Bloc::Paragraphe(p) = &blocs[i] else { return None };
    if p.ancre.demi_page() != blocs[debut].ancre().demi_page() {
        return None;
    }
    let texte = p.texte_brut().trim().to_string();
    numero_seul(&texte).map(|valeur| (texte, valeur))
}

/// Pour chaque numéro (genre et valeur, dans l'ordre des entrées retrouvées), est-il
/// absorbé ? Le premier absorbé vaut 1 ou 2 (« I » manquant). Ensuite, un numéro `n` du
/// genre du précédent absorbé `prec` l'est s'il vaut 1 (nouvelle partie, nouveau tome)
/// ou s'il est entre `prec + 1` et `prec + 2` (un numéro sauté). Un numéro refusé (un
/// folio « 12 » resté seul) n'est pas absorbé et ne change pas `prec` ; une entrée sans
/// numéro non plus.
fn numeros_en_suite(numeros: &[Option<(Numerotation, u32)>]) -> Vec<bool> {
    let mut prec: Option<(Numerotation, u32)> = None;
    numeros
        .iter()
        .map(|numero| {
            let Some((genre, n)) = *numero else { return false };
            let absorbe = match prec {
                None => n == 1 || n == 2,
                Some((genre_prec, p)) => genre == genre_prec && (n == 1 || (p + 1..=p + 2).contains(&n)),
            };
            if absorbe {
                prec = Some((genre, n));
            }
            absorbe
        })
        .collect()
}

/// Titres issus des signets. Retourne les blocs, les points de signets introuvables et
/// les indices, dans les blocs rendus, des titres produits (retrouvés ou insérés).
fn titres_depuis_signets(
    blocs: Vec<Bloc>,
    signets: &[SignetCherche],
    pages_som: &BTreeSet<DemiPage>,
) -> (Vec<Bloc>, Vec<PointAVerifier>, HashSet<usize>) {
    let mut points = Vec::new();
    // Remplacements : indice de début → (nombre de blocs avalés, titre).
    let mut remplacements: HashMap<usize, (usize, Bloc)> = HashMap::new();
    let mut utilises: HashSet<usize> = HashSet::new();
    // Insertions avant l'indice (blocs.len() = à la fin).
    let mut insertions: BTreeMap<usize, Vec<Bloc>> = BTreeMap::new();
    let index = IndexTitres::new(&blocs);

    for SignetCherche { signet, texte_impose, numero_avant, trouve } in signets {
        let niveau = signet.niveau.clamp(1, 3);
        let cible = normaliser_titre(&signet.titre);
        // Correspondance exacte trouvée dans ces mêmes blocs (`entrees_du_sommaire`). Elle
        // n'est reprise que si elle désigne encore des paragraphes libres qui disent le
        // titre ; sinon (blocs changés depuis), le signet est cherché normalement.
        let transmise = trouve.and_then(|(debut, fin)| {
            let paragraphes = blocs.get(debut..fin).filter(|_| debut < fin)?;
            let textes: Vec<String> = paragraphes
                .iter()
                .map(|b| match b {
                    Bloc::Paragraphe(p) => Some(p.texte_brut().trim().to_string()),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            let texte = textes.join(" ");
            (normaliser_titre(&texte) == cible && !(debut..fin).any(|i| utilises.contains(&i)))
                .then_some((debut, fin, texte))
        });
        let trouve = if transmise.is_some() {
            transmise
        } else if cible.is_empty() {
            None
        } else {
            let candidats = index.candidats(&cible);
            [signet.page, signet.page + 1]
                .into_iter()
                .find_map(|page| chercher_signet(&blocs, &index, &candidats, &cible, page, &utilises, pages_som))
        };
        match trouve {
            Some((mut debut, fin, texte)) => {
                // Le texte du signet ne remplace que des paragraphes qui le disent
                // exactement : sur une correspondance de début, les mots en trop
                // seraient perdus. Le texte des paragraphes garde alors le numéro en tête.
                let mut texte = if !*texte_impose {
                    texte
                } else if normaliser_titre(&texte) == cible {
                    signet.titre.trim().to_string()
                } else {
                    NUMERO_EN_TETE.replace(&texte, "$1. ").into_owned()
                };
                if *numero_avant
                    && let Some((numero, _)) = numero_seul_avant(&blocs, debut, &utilises)
                {
                    texte = format!("{}. {texte}", numero.trim_end_matches('.'));
                    debut -= 1;
                }
                utilises.extend(debut..fin);
                let ancre = blocs[debut].ancre();
                remplacements.insert(debut, (fin - debut, Bloc::Titre { niveau, texte, ancre }));
            }
            None => {
                let existant = blocs.iter().position(|b| b.ancre().page == signet.page);
                let (position, ancre) = match existant {
                    Some(i) => (i, blocs[i].ancre()),
                    None => (
                        blocs.iter().position(|b| b.ancre().page > signet.page).unwrap_or(blocs.len()),
                        Ancre { page: signet.page, moitie: Moitie::Entiere, rect: [0, 0, 1000, 1000] },
                    ),
                };
                insertions.entry(position).or_default().push(Bloc::Titre {
                    niveau,
                    texte: signet.titre.trim().to_string(),
                    ancre,
                });
                points.push(PointAVerifier {
                    ancre,
                    genre: GenrePoint::TitreSuspect,
                    message: format!(
                        "signet « {} » introuvable page {}",
                        signet.titre.trim(),
                        signet.page
                    ),
                });
            }
        }
    }

    let mut resultat = Vec::with_capacity(blocs.len());
    let mut produits = HashSet::new();
    let mut inserer = |resultat: &mut Vec<Bloc>, titres: Vec<Bloc>| {
        for titre in titres {
            produits.insert(resultat.len());
            resultat.push(titre);
        }
    };
    let mut a_sauter = 0;
    for (i, bloc) in blocs.iter().enumerate() {
        if let Some(titres) = insertions.remove(&i) {
            inserer(&mut resultat, titres);
        }
        if a_sauter > 0 {
            a_sauter -= 1;
            continue;
        }
        if let Some((n, titre)) = remplacements.remove(&i) {
            inserer(&mut resultat, vec![titre]);
            a_sauter = n - 1;
        } else {
            resultat.push(bloc.clone());
        }
    }
    if let Some(titres) = insertions.remove(&blocs.len()) {
        inserer(&mut resultat, titres);
    }
    (resultat, points, produits)
}

/// Paragraphes du livre préparés pour retrouver un titre (signet ou entrée de
/// résumé) sans parcourir tout le livre à chaque recherche.
struct IndexTitres {
    /// Par bloc : texte brut (sans espaces de bord), texte normalisé (`normaliser_titre`)
    /// et nombre de mots du texte brut ; `None` hors paragraphe.
    paragraphes: Vec<Option<(String, String, usize)>>,
    /// Texte normalisé → paragraphes qui le portent.
    par_texte: HashMap<String, Vec<usize>>,
    /// Début (en mots entiers, strictement plus court) du texte normalisé d'un
    /// paragraphe court → paragraphes.
    par_debut: HashMap<String, Vec<usize>>,
    /// Pages qui portent au moins un paragraphe.
    pages: HashSet<u32>,
}

impl IndexTitres {
    fn new(blocs: &[Bloc]) -> Self {
        let mut index = IndexTitres {
            paragraphes: Vec::with_capacity(blocs.len()),
            par_texte: HashMap::new(),
            par_debut: HashMap::new(),
            pages: HashSet::new(),
        };
        for (i, bloc) in blocs.iter().enumerate() {
            let Bloc::Paragraphe(p) = bloc else {
                index.paragraphes.push(None);
                continue;
            };
            index.pages.insert(p.ancre.page);
            let texte = p.texte_brut().trim().to_string();
            let norm = normaliser_titre(&texte);
            let mots = nb_mots(&texte);
            if !norm.is_empty() {
                index.par_texte.entry(norm.clone()).or_default().push(i);
                if mots <= MAX_MOTS_TITRE && p.nb_lignes <= MAX_LIGNES_TITRE {
                    for (k, _) in norm.match_indices(' ') {
                        index.par_debut.entry(norm[..k].to_string()).or_default().push(i);
                    }
                }
            }
            index.paragraphes.push(Some((texte, norm, mots)));
        }
        index
    }

    /// Paragraphes où une recherche de `cible` peut aboutir, dans l'ordre du livre :
    /// ceux dont le texte normalisé est `cible` ou un début de `cible` en mots
    /// entiers (titre sur plusieurs paragraphes), et les paragraphes courts qui
    /// commencent par `cible`. Les autres ne peuvent jamais correspondre.
    fn candidats(&self, cible: &str) -> Vec<usize> {
        let debuts = cible.match_indices(' ').map(|(k, _)| &cible[..k]).chain([cible]);
        let mut candidats: Vec<usize> =
            debuts.filter_map(|d| self.par_texte.get(d)).flatten().copied().collect();
        candidats.extend(self.par_debut.get(cible).into_iter().flatten().copied());
        candidats.sort_unstable();
        candidats.dedup();
        candidats
    }
}

/// Cherche le paragraphe (ou la suite de paragraphes) qui porte le titre du signet sur
/// `page` ou la suivante. Les correspondances s'arrêtent à une frontière de mot ; une
/// correspondance exacte est préférée à une simple correspondance de début. Une
/// correspondance de début qui n'est pas courte (plus de 12 mots ou 3 lignes) est
/// refusée ; une correspondance exacte ne peut pas avaler le corps du texte et n'a pas
/// de limite. Seuls les `candidats` (voir `IndexTitres::candidats`) sont examinés.
fn chercher_signet(
    blocs: &[Bloc],
    index: &IndexTitres,
    candidats: &[usize],
    cible: &str,
    page: u32,
    utilises: &HashSet<usize>,
    pages_som: &BTreeSet<DemiPage>,
) -> Option<(usize, usize, String)> {
    let libre = |i: usize| {
        !utilises.contains(&i) && matches!(blocs[i], Bloc::Paragraphe(_)) && !est_sommaire(&blocs[i], pages_som)
    };
    let mut premier_prefixe = None;
    for page_cherchee in [page, page + 1] {
        for &debut in candidats {
            if blocs[debut].ancre().page != page_cherchee || !libre(debut) {
                continue;
            }
            let mut textes: Vec<&str> = Vec::new();
            let mut norm = String::new();
            let mut mots = 0;
            let mut lignes = 0;
            let mut fin = debut;
            loop {
                let Bloc::Paragraphe(p) = &blocs[fin] else { break };
                let (texte, norm_bloc, mots_bloc) = index.paragraphes[fin].as_ref().expect("paragraphe indexé");
                textes.push(texte);
                // Le texte normalisé d'une suite de paragraphes est la suite de leurs
                // textes normalisés non vides.
                if !norm_bloc.is_empty() {
                    if !norm.is_empty() {
                        norm.push(' ');
                    }
                    norm.push_str(norm_bloc);
                }
                mots += mots_bloc;
                lignes += p.nb_lignes;
                fin += 1;
                if norm.is_empty() {
                    break;
                }
                let court = mots <= MAX_MOTS_TITRE && lignes <= MAX_LIGNES_TITRE;
                if norm == cible {
                    return Some((debut, fin, textes.join(" ")));
                }
                if norm.strip_prefix(cible).is_some_and(|reste| reste.starts_with(' ')) {
                    if court && premier_prefixe.is_none() {
                        premier_prefixe = Some((debut, fin, textes.join(" ")));
                    }
                    break;
                }
                let debut_du_titre = cible.strip_prefix(norm.as_str()).is_some_and(|r| r.starts_with(' '));
                if !debut_du_titre || fin >= blocs.len() || !libre(fin) {
                    break;
                }
            }
        }
    }
    premier_prefixe
}

/// Cherche dans le corps une entrée de résumé ou de sommaire (`cible`, texte normalisé)
/// à partir de la (demi-)page `depart`, hors paragraphes `utilises` et (demi-)pages
/// `pages_som`. Seuls les candidats (`IndexTitres::candidats`) à partir de `depart`
/// peuvent aboutir ; les pages qui en portent, et la précédente de chacune, sont essayées
/// dans l'ordre du livre (`chercher_signet`). `exacte` : seule une correspondance exacte
/// est retenue.
///
/// Ce filtre, appliqué après `chercher_signet`, ne fait pas perdre de correspondance
/// exacte : `chercher_signet` préfère l'exacte à une correspondance de début sur la page
/// qu'il examine et la suivante, et ne rend une correspondance de début que s'il n'y a
/// trouvé aucune exacte. Une correspondance de début écartée laisse donc la recherche
/// continuer à la page suivante, où une exacte peut encore se trouver.
fn chercher_entree(
    blocs: &[Bloc],
    index: &IndexTitres,
    cible: &str,
    depart: DemiPage,
    utilises: &HashSet<usize>,
    pages_som: &BTreeSet<DemiPage>,
    exacte: bool,
) -> Option<(usize, usize, String)> {
    let candidats: Vec<usize> =
        index.candidats(cible).into_iter().filter(|&i| blocs[i].ancre().demi_page() >= depart).collect();
    let pages: BTreeSet<u32> = candidats
        .iter()
        .map(|&i| blocs[i].ancre().page)
        .flat_map(|page| [page.saturating_sub(1), page])
        .filter(|page| *page >= depart.0 && index.pages.contains(page))
        .collect();
    pages.into_iter().find_map(|page| {
        chercher_signet(blocs, index, &candidats, cible, page, utilises, pages_som)
            .filter(|(_, _, texte)| !exacte || normaliser_titre(texte) == cible)
    })
}

/// Niveaux des titres d'après leur taille. Les tailles voisines (écart ≤ 0,5 pt, de
/// proche en proche) forment une chaîne ; une chaîne qui s'étend sur plus d'1 pt est
/// coupée à son plus grand écart (bruit d'OCR : 16,1 à 16,9 pt restent ensemble). Les
/// groupes obtenus sont classés du plus grand au plus petit. Le premier groupe d'au
/// moins 3 titres donne le niveau 1, et les groupes plus grands mais plus rares
/// (couverture, schéma) le rejoignent au lieu de le repousser.
struct NiveauxParTaille {
    /// (taille minimale, taille maximale, niveau) de chaque groupe.
    groupes: Vec<(f32, f32, u8)>,
}

/// Ajoute `chaine` (tailles décroissantes) à `groupes`, coupée à son plus grand écart
/// tant qu'elle s'étend sur plus de `ETENDUE_NIVEAU`.
fn couper_au_plus_grand_ecart(chaine: Vec<f32>, groupes: &mut Vec<Vec<f32>>) {
    if chaine[0] - chaine[chaine.len() - 1] <= ETENDUE_NIVEAU {
        groupes.push(chaine);
        return;
    }
    let coupe = (1..chaine.len())
        .max_by(|&a, &b| (chaine[a - 1] - chaine[a]).total_cmp(&(chaine[b - 1] - chaine[b])))
        .expect("chaîne d'au moins deux tailles");
    let mut haut = chaine;
    let bas = haut.split_off(coupe);
    couper_au_plus_grand_ecart(haut, groupes);
    couper_au_plus_grand_ecart(bas, groupes);
}

impl NiveauxParTaille {
    fn mesurer(mut tailles: Vec<f32>) -> Self {
        tailles.sort_by(|a, b| b.total_cmp(a));
        let mut chaines: Vec<Vec<f32>> = Vec::new();
        for t in tailles {
            match chaines.last_mut() {
                Some(g) if g.last().is_some_and(|d| d - t <= TOLERANCE_TAILLE) => g.push(t),
                _ => chaines.push(vec![t]),
            }
        }
        let mut groupes: Vec<Vec<f32>> = Vec::new();
        for chaine in chaines {
            couper_au_plus_grand_ecart(chaine, &mut groupes);
        }
        let premier_frequent = groupes.iter().position(|g| g.len() >= MIN_TITRES_NIVEAU).unwrap_or(0);
        let groupes = groupes
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let niveau = (1 + i.saturating_sub(premier_frequent)).min(3) as u8;
                (*g.last().expect("groupe non vide"), g[0], niveau)
            })
            .collect();
        NiveauxParTaille { groupes }
    }

    fn niveau(&self, taille: f32) -> u8 {
        self.groupes
            .iter()
            .find(|(min, max, _)| taille >= *min && taille <= *max)
            .map_or(1, |(_, _, n)| *n)
    }
}

/// Titre qui nomme un chapitre : « CHAPITRE III », « PREMIER CHAPITRE ».
fn nomme_un_chapitre(p: &Paragraphe) -> bool {
    static CHAPITRE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(?:\p{L}+ )?chapitre\b").expect("motif valide"));
    CHAPITRE.is_match(&normaliser(&p.texte_brut()))
}

/// Transforme en titres les paragraphes retenus par `candidat`. Quand au moins
/// 3 titres nomment un chapitre, ils forment le niveau 1 et les autres descendent
/// d'un niveau ; sinon les niveaux suivent la taille. `niveau_1_pris` : le niveau 1
/// est déjà donné (titres du résumé), les candidats commencent au niveau 2.
/// Un candidat dont le texte n'est pas plausible (`titre_plausible`) reste un
/// paragraphe et donne un point à vérifier ; son ancre est aussi renvoyée.
fn titres_par_candidats(
    blocs: Vec<Bloc>,
    pages_som: &BTreeSet<DemiPage>,
    candidat: impl Fn(&Paragraphe) -> bool,
    niveau_1_pris: bool,
) -> (Vec<Bloc>, Vec<PointAVerifier>, HashSet<Ancre>) {
    let est_candidat_brut = |p: &Paragraphe| !pages_som.contains(&p.ancre.demi_page()) && candidat(p);
    let est_candidat = |p: &Paragraphe| est_candidat_brut(p) && paragraphe_titre_plausible(p);
    let refuses: Vec<&Paragraphe> = blocs
        .iter()
        .filter_map(|b| match b {
            Bloc::Paragraphe(p) if est_candidat_brut(p) && !est_candidat(p) => Some(p),
            _ => None,
        })
        .collect();
    let points = refuses
        .iter()
        .map(|p| PointAVerifier {
            ancre: p.ancre,
            genre: GenrePoint::TitreSuspect,
            message: "texte peu plausible pour un titre (OCR, schéma, code) : non retenu".to_string(),
        })
        .collect();
    let illisibles: HashSet<Ancre> = refuses.iter().map(|p| p.ancre).collect();
    let candidats: Vec<&Paragraphe> = blocs
        .iter()
        .filter_map(|b| match b {
            Bloc::Paragraphe(p) if est_candidat(p) => Some(p),
            _ => None,
        })
        .collect();
    let par_chapitres = candidats.iter().filter(|p| nomme_un_chapitre(p)).count() >= MIN_TITRES_NIVEAU;
    let chapitre = |p: &Paragraphe| par_chapitres && nomme_un_chapitre(p);
    let niveaux = NiveauxParTaille::mesurer(candidats.iter().filter(|p| !chapitre(p)).map(|p| p.taille).collect());
    let decalage = u8::from(par_chapitres || niveau_1_pris);

    let blocs = blocs
        .into_iter()
        .map(|b| match b {
            Bloc::Paragraphe(p) if est_candidat(&p) => Bloc::Titre {
                niveau: if chapitre(&p) { 1 } else { (niveaux.niveau(p.taille) + decalage).min(3) },
                texte: p.texte_brut().trim().to_string(),
                ancre: p.ancre,
            },
            autre => autre,
        })
        .collect();
    (blocs, points, illisibles)
}

/// Ligne de date seule : « (2 FÉVRIER 1995) », « 1er mars 1914. ».
fn est_date(texte: &str) -> bool {
    static DATE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:\d{1,2}|1er) (?:janvier|fevrier|mars|avril|mai|juin|juillet|aout|septembre|octobre|novembre|decembre) \d{4}$")
            .expect("motif valide")
    });
    DATE.is_match(&normaliser(texte))
}

/// Fusionne les titres adjacents de même niveau, même page et même moitié ; une
/// date n'est jamais collée à un autre titre, ni un titre dont l'indice dans `blocs` est
/// dans `proteges` (titres tirés d'un sommaire ou d'un résumé, déjà entiers) à son
/// voisin. La position, et non l'ancre, désigne le titre protégé : un titre inséré pour
/// un signet introuvable partage l'ancre du premier bloc de sa page.
fn fusionner_titres_consecutifs(blocs: Vec<Bloc>, proteges: &HashSet<usize>) -> Vec<Bloc> {
    let mut resultat: Vec<Bloc> = Vec::with_capacity(blocs.len());
    // Le dernier bloc de `resultat` est-il protégé ? (Un titre fusionné ne l'est jamais.)
    let mut dernier_protege = false;
    for (i, bloc) in blocs.into_iter().enumerate() {
        let protege = proteges.contains(&i);
        if let (
            Bloc::Titre { niveau, texte, ancre },
            Some(Bloc::Titre { niveau: n0, texte: t0, ancre: a0 }),
        ) = (&bloc, resultat.last_mut())
        {
            if niveau == n0
                && ancre.page == a0.page
                && ancre.moitie == a0.moitie
                && !est_date(texte)
                && !est_date(t0)
                && !protege
                && !dernier_protege
            {
                t0.push(' ');
                t0.push_str(texte);
                continue;
            }
        }
        resultat.push(bloc);
        dernier_protege = protege;
    }
    resultat
}

/// Un même titre sur au moins 5 pages distinctes est probablement un titre courant.
fn titres_courants_suspects(blocs: &[Bloc]) -> Vec<PointAVerifier> {
    let mut pages: HashMap<String, BTreeSet<u32>> = HashMap::new();
    for bloc in blocs {
        if let Bloc::Titre { texte, ancre, .. } = bloc {
            pages.entry(normaliser(texte)).or_default().insert(ancre.page);
        }
    }
    blocs
        .iter()
        .filter_map(|bloc| match bloc {
            Bloc::Titre { texte, ancre, .. }
                if pages.get(&normaliser(texte)).is_some_and(|p| p.len() >= MIN_PAGES_TITRE_COURANT) =>
            {
                Some(PointAVerifier {
                    ancre: *ancre,
                    genre: GenrePoint::TitreSuspect,
                    message: format!(
                        "le titre « {} » se répète sur {} pages : titre courant possible",
                        texte,
                        pages[&normaliser(texte)].len()
                    ),
                })
            }
            _ => None,
        })
        .collect()
}

/// Retire les titres courants pris dans le fil du texte. Un titre candidat est un
/// titre courant si, sur au moins 5 pages et au moins 10 % des pages du livre, il se
/// trouve en bordure : premier ou dernier paragraphe de sa page, ou voisin d'un
/// paragraphe lui-même répété sur 5 pages (pied de page d'origine resté dans le texte
/// d'un PDF recomposé). « Notes » n'est jamais un titre courant. Chaque occurrence
/// retirée devient un retrait et un point à vérifier.
fn retirer_titres_courants(
    blocs: Vec<Bloc>,
    pages_som: &BTreeSet<DemiPage>,
    candidat: impl Fn(&Paragraphe) -> bool,
    nb_pages_livre: u32,
) -> (Vec<Bloc>, Vec<Retrait>, Vec<PointAVerifier>) {
    let paragraphes: Vec<(usize, &Paragraphe, String)> = blocs
        .iter()
        .enumerate()
        .filter_map(|(i, b)| match b {
            Bloc::Paragraphe(p) if !pages_som.contains(&p.ancre.demi_page()) => {
                Some((i, p, normaliser(&p.texte_brut())))
            }
            _ => None,
        })
        .collect();
    let mut pages_du_texte: HashMap<&str, BTreeSet<u32>> = HashMap::new();
    for (_, p, norm) in &paragraphes {
        pages_du_texte.entry(norm.as_str()).or_default().insert(p.ancre.page);
    }
    let repete = |norm: &str| pages_du_texte.get(norm).is_some_and(|p| p.len() >= MIN_PAGES_TITRE_COURANT);
    let mut bordures: HashMap<&str, BTreeSet<u32>> = HashMap::new();
    for (k, (_, p, norm)) in paragraphes.iter().enumerate() {
        if !candidat(p) || norm == "notes" {
            continue;
        }
        let voisin = |j: Option<usize>| {
            j.and_then(|j| paragraphes.get(j)).filter(|(_, q, _)| q.ancre.page == p.ancre.page)
        };
        let (avant, apres) = (voisin(k.checked_sub(1)), voisin(Some(k + 1)));
        let en_bordure = avant.is_none()
            || apres.is_none()
            || [avant, apres].into_iter().flatten().any(|(_, _, n)| n != norm && repete(n));
        if en_bordure {
            bordures.entry(norm.as_str()).or_default().insert(p.ancre.page);
        }
    }
    let seuil = MIN_PAGES_TITRE_COURANT.max((PART_MIN_TITRE_COURANT * nb_pages_livre as f32).ceil() as usize);
    let courants: HashMap<String, usize> = bordures
        .into_iter()
        .filter(|(_, pages)| pages.len() >= seuil)
        .map(|(norm, pages)| (norm.to_string(), pages.len()))
        .collect();
    let a_retirer: HashSet<usize> = paragraphes
        .iter()
        .filter(|(_, p, norm)| candidat(p) && courants.contains_key(norm))
        .map(|(i, _, _)| *i)
        .collect();

    let (mut retraits, mut points, mut sortie) = (Vec::new(), Vec::new(), Vec::new());
    for (i, bloc) in blocs.into_iter().enumerate() {
        match bloc {
            Bloc::Paragraphe(p) if a_retirer.contains(&i) => {
                let texte = p.texte_brut().trim().to_string();
                let nb = courants[&normaliser(&texte)];
                points.push(PointAVerifier {
                    ancre: p.ancre,
                    genre: GenrePoint::TitreSuspect,
                    message: format!("titre courant « {texte} » retiré (en bordure de {nb} pages)"),
                });
                retraits.push(Retrait { ancre: p.ancre, role: Role::TitreCourant, texte });
            }
            autre => sortie.push(autre),
        }
    }
    (sortie, retraits, points)
}

/// Ligne « CHAPITRE n » d'un résumé, et ce qui la suit (« CHAPITRE 2: LA GENÈSE »).
static ENTREE_CHAPITRE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*chapitre\s+[\p{L}\d]+\s*[:.)\-–—]*\s*").expect("motif valide"));

/// Résumé sans numéros de page : suite de pages faites seulement de lignes courtes,
/// dont au moins 3 titres (et au moins la moitié) reviennent plus loin dans le livre.
/// Le résumé compte par page : une page dont une moitié est dans `pages_som` n'en fait
/// pas partie, et une page du résumé le met tout entier (toutes ses moitiés) dans les
/// (demi-)pages renvoyées. Renvoie ces (demi-)pages et, s'il est fait d'entrées
/// « CHAPITRE n », un pseudo-signet de niveau 1 par entrée retrouvée, à la page où elle
/// revient.
fn resume_sans_numeros(blocs: &[Bloc], pages_som: &BTreeSet<DemiPage>) -> (BTreeSet<DemiPage>, Vec<Signet>) {
    let mut par_page: BTreeMap<u32, Vec<&Paragraphe>> = BTreeMap::new();
    for bloc in blocs {
        if let Bloc::Paragraphe(p) = bloc {
            par_page.entry(p.ancre.page).or_default().push(p);
        }
    }
    let courte =
        |page: &u32| !page_touchee(pages_som, *page) && par_page[page].iter().all(|p| court(p, &p.texte_brut()));
    let mut suites: Vec<Vec<u32>> = Vec::new();
    let mut precedente: Option<u32> = None;
    for page in par_page.keys().copied() {
        if courte(&page) {
            match suites.last_mut() {
                Some(suite) if precedente.is_some_and(|p| suite.last() == Some(&p)) => suite.push(page),
                _ => suites.push(vec![page]),
            }
        }
        precedente = Some(page);
    }

    let mut pages_resume = BTreeSet::new();
    let mut pseudo = Vec::new();
    let index = IndexTitres::new(blocs);
    for suite in suites {
        let textes: Vec<String> = suite
            .iter()
            .flat_map(|page| par_page[page].iter().map(|p| p.texte_brut().trim().to_string()))
            .collect();
        let par_chapitres = textes.iter().any(|t| ENTREE_CHAPITRE.is_match(t));
        let mut entrees: Vec<String> = Vec::new();
        for texte in textes {
            if par_chapitres {
                if let Some(m) = ENTREE_CHAPITRE.find(&texte) {
                    entrees.push(texte[m.end()..].to_string());
                } else if let Some(entree) = entrees.last_mut() {
                    entree.push(' ');
                    entree.push_str(&texte);
                }
            } else {
                entrees.push(texte);
            }
        }
        let fin = *suite.last().expect("suite non vide");
        let mut utilises: HashSet<usize> = HashSet::new();
        let mut depart = fin + 1;
        let mut retrouves = Vec::new();
        for entree in &entrees {
            let cible = normaliser_titre(entree);
            if cible.is_empty() {
                continue;
            }
            // Critère par page : `depart` ouvre sa page entière.
            let trouve = chercher_entree(blocs, &index, &cible, (depart, Moitie::Entiere), &utilises, pages_som, false);
            if let Some((debut, fin_bloc, _)) = trouve {
                let page = blocs[debut].ancre().page;
                utilises.extend(debut..fin_bloc);
                depart = page;
                retrouves.push(Signet { titre: entree.trim().to_string(), niveau: 1, page });
            }
        }
        if retrouves.len() >= MIN_TITRES_NIVEAU && retrouves.len() * 2 >= entrees.len() {
            pages_resume.extend(suite.iter().flat_map(|page| par_page[page].iter().map(|p| p.ancre.demi_page())));
            if par_chapitres {
                pseudo.extend(retrouves);
            }
        }
    }
    (pages_resume, pseudo)
}

/// Nombre maximal de paragraphes d'une entrée de sommaire imprimé.
const MAX_PARAGRAPHES_ENTREE: usize = 3;

/// Nombre de paragraphes, parmi `textes` (paragraphes consécutifs d'un sommaire), que
/// peut couvrir une entrée qui part du premier : au plus 3, et jamais au-delà d'un
/// paragraphe numéroté (qui finit par un numéro de page, `numero_de_page_final`).
fn etendue_entree(textes: &[&str]) -> usize {
    let mut n = 0;
    for texte in textes.iter().take(MAX_PARAGRAPHES_ENTREE) {
        n += 1;
        if numero_de_page_final(texte.trim()).is_some() {
            break;
        }
    }
    n
}

/// Textes des paragraphes que peut couvrir une entrée de sommaire qui part du bloc
/// `debut` : paragraphes consécutifs dans les blocs, de la même (demi-)page, sans appel
/// de note (un lien ne peut pas en contenir), dans la limite de `etendue_entree`. Vide si
/// `debut` n'est pas un tel paragraphe. Même découpage pour les titres tirés du sommaire
/// (`entrees_du_sommaire`) et pour le sommaire cliquable (`sommaire_cliquable`).
fn paragraphes_d_entree(blocs: &[Bloc], debut: usize) -> Vec<String> {
    let lieu = |b: &Bloc| b.ancre().demi_page();
    let textes: Vec<String> = blocs[debut..]
        .iter()
        .take(MAX_PARAGRAPHES_ENTREE)
        .map_while(|b| match b {
            Bloc::Paragraphe(p)
                if lieu(b) == lieu(&blocs[debut])
                    && !p.segments.iter().any(|s| matches!(s, Segment::AppelNote { .. })) =>
            {
                Some(p.texte_brut().trim().to_string())
            }
            _ => None,
        })
        .collect();
    let n = etendue_entree(&textes.iter().map(String::as_str).collect::<Vec<_>>());
    textes.into_iter().take(n).collect()
}

/// `avant` finit-il par une césure que `suite` complète : lettre + « - », la suite
/// commençant par une minuscule (« caté- » + « gories ») ?
fn fin_de_cesure(avant: &str, suite: &str) -> bool {
    let mut fin = avant.trim_end().chars().rev();
    fin.next() == Some('-')
        && fin.next().is_some_and(char::is_alphabetic)
        && suite.trim_start().chars().next().is_some_and(char::is_lowercase)
}

/// Texte d'une entrée de sommaire faite des paragraphes `textes` : leurs textes séparés
/// par une espace, une césure de fin de paragraphe (lettre + « - », la suite commençant
/// par une minuscule) recollée sans tiret (« caté- » + « gories » → « catégories »), et
/// sans le numéro de page final (`NUMERO_FINAL`).
fn texte_entree(textes: &[&str]) -> String {
    let mut joint = String::new();
    for texte in textes.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        if fin_de_cesure(&joint, texte) {
            joint.pop();
        } else if !joint.is_empty() {
            joint.push(' ');
        }
        joint.push_str(texte);
    }
    NUMERO_FINAL.replace(&joint, "").trim().to_string()
}

/// Entrées des sommaires imprimés à numéros de page (moitiés `moities_som`, voir
/// `moities_sommaire`) retrouvées dans le corps. Les paragraphes de pages consécutives
/// forment un même sommaire, lu dans l'ordre : une entrée part d'un paragraphe et peut
/// s'étendre aux suivants de la même (demi-)page (`paragraphes_d_entree`, texte
/// `texte_entree`) ; la plus longue est essayée d'abord, et seule une correspondance
/// exacte (`chercher_entree`, hors `pages_som`) est retenue. Les paragraphes du sommaire
/// d'une entrée retrouvée sont sautés ; une entrée introuvable est ignorée. Un paragraphe
/// fait seulement d'un numéro bien formé (`numero_seul` : « IX » seul sur sa ligne), sans
/// numéro de page, ou suivi d'un numéro de page si le numéro vaut moins de 100
/// (« IX 15 », mais « DIX 15 » est une entrée), n'ouvre pas d'entrée et n'est pas
/// cherché : l'entrée suivante absorbera le numéro du corps. Un sommaire de
/// la première moitié du livre est cherché à partir de la (demi-)page qui suit sa
/// dernière (demi-)page dans l'ordre de lecture (p3d après un sommaire fini en p3g), un
/// sommaire de fin à partir de la page 1 ; ensuite, chaque entrée à partir de la
/// (demi-)page de la précédente retrouvée (p5d après une entrée retrouvée en p5d : la p5g
/// n'est plus cherchée). Un paragraphe du corps ne sert qu'une fois.
/// Renvoie, par entrée retrouvée, un pseudo-signet de niveau 1 à sa page, avec sa
/// correspondance (`trouve`) dans `blocs`. Le numéro seul qui précède une correspondance
/// (`numero_seul_avant`) est absorbé (`numero_avant`) s'il suit les numéros absorbés
/// avant lui, dans l'ordre des entrées retrouvées (`numeros_en_suite`).
fn entrees_du_sommaire(
    blocs: &[Bloc],
    moities_som: &BTreeSet<DemiPage>,
    pages_som: &BTreeSet<DemiPage>,
) -> Vec<SignetCherche> {
    // Sommaires : (première page, dernière (demi-)page, indices des paragraphes).
    let mut sommaires: Vec<(u32, DemiPage, Vec<usize>)> = Vec::new();
    for (i, bloc) in blocs.iter().enumerate() {
        let Bloc::Paragraphe(p) = bloc else { continue };
        let demi_page = p.ancre.demi_page();
        if !moities_som.contains(&demi_page) {
            continue;
        }
        let page = demi_page.0;
        match sommaires.last_mut() {
            Some((_, fin, paragraphes)) if page == fin.0 || page == fin.0 + 1 => {
                *fin = (*fin).max(demi_page);
                paragraphes.push(i);
            }
            _ => sommaires.push((page, demi_page, vec![i])),
        }
    }
    let derniere_page = blocs.iter().map(|b| b.ancre().page).max().unwrap_or(0);
    let index = IndexTitres::new(blocs);
    let mut utilises: HashSet<usize> = HashSet::new();
    let mut retrouves = Vec::new();
    for (premiere, fin, paragraphes) in sommaires {
        let mut depart: DemiPage =
            if premiere <= derniere_page / 2 { demi_page_suivante(fin) } else { (1, Moitie::Entiere) };
        let mut k = 0;
        while k < paragraphes.len() {
            let suite = paragraphes_d_entree(blocs, paragraphes[k]);
            let textes: Vec<&str> = suite.iter().map(String::as_str).collect();
            // Ligne d'un numéro seul : sautée sans numéro de page (« IX »), ou avec un numéro
            // de page si le numéro vaut moins de 100 (« IX 15 ») ; « DIX 15 » est une entrée.
            let numero_seul_sans_titre = |t: &&str| {
                numero_seul(&texte_entree(&[t]))
                    .is_some_and(|(_, n)| n < 100 || numero_de_page_final(t.trim()).is_none())
            };
            if textes.first().is_some_and(numero_seul_sans_titre) {
                k += 1;
                continue;
            }
            let trouve = (1..=textes.len()).rev().find_map(|n| {
                let entree = texte_entree(&textes[..n]);
                let cible = normaliser_titre(&entree);
                if cible.is_empty() {
                    return None;
                }
                chercher_entree(blocs, &index, &cible, depart, &utilises, pages_som, true)
                    .map(|(debut, fin_bloc, _)| (n, entree, debut, fin_bloc))
            });
            match trouve {
                Some((n, entree, debut, fin_bloc)) => {
                    let page = blocs[debut].ancre().page;
                    utilises.extend(debut..fin_bloc);
                    depart = blocs[debut].ancre().demi_page();
                    retrouves.push((Signet { titre: entree, niveau: 1, page }, debut, fin_bloc));
                    k += n;
                }
                None => k += 1,
            }
        }
    }
    // Numéros seuls avant les correspondances, hors paragraphes déjà retrouvés.
    let numeros: Vec<Option<(Numerotation, u32)>> = retrouves
        .iter()
        .map(|(_, debut, _)| numero_seul_avant(blocs, *debut, &utilises).map(|(_, valeur)| valeur))
        .collect();
    retrouves
        .into_iter()
        .zip(numeros_en_suite(&numeros))
        .map(|((signet, debut, fin), numero_avant)| SignetCherche {
            signet,
            texte_impose: false,
            numero_avant,
            trouve: Some((debut, fin)),
        })
        .collect()
}

/// Résultat de `detecter_titres`.
pub struct TitresDetectes {
    /// Blocs, avec leurs titres.
    pub blocs: Vec<Bloc>,
    /// Points à vérifier : un point `Sommaire` par page, ancré à sa première moitié de
    /// sommaire, et ceux de la détection des titres.
    pub points: Vec<PointAVerifier>,
    /// Source des titres réellement utilisée.
    pub source: SourceTitres,
    /// Titres courants retirés.
    pub retraits: Vec<Retrait>,
    /// Ancres des candidats refusés comme illisibles : paragraphes qui ne doivent pas être
    /// raccordés d'une page à l'autre.
    pub illisibles: HashSet<Ancre>,
    /// (Demi-)pages de sommaire.
    pub sommaires: BTreeSet<DemiPage>,
}

/// Détecte les titres selon `source` (ou la cascade en mode `Auto`) et les (demi-)pages
/// de sommaire (voir `TitresDetectes`). Une (demi-)page de sommaire n'apporte ni titre ni
/// cible de recherche ; l'autre moitié de sa page double reste une page ordinaire.
/// Les signets d'appel de note (« [12] ») sont ignorés. Sans signets, un résumé sans
/// numéros de page compte comme sommaire et ses entrées « CHAPITRE n » donnent les
/// titres de niveau 1 du corps ; à défaut, les entrées d'un sommaire imprimé à numéros
/// de page retrouvées dans le corps (`entrees_du_sommaire`) les donnent, si au moins 3
/// d'entre elles, et au moins la moitié, ne sont pas déjà des titres candidats. Ces titres
/// tirés du résumé ou du sommaire ne sont jamais fusionnés avec un titre voisin. Les
/// titres courants pris dans le texte sont retirés avant ces recherches.
pub fn detecter_titres(
    blocs: Vec<Bloc>,
    signets: &[Signet],
    nb_pages: u32,
    source: SourceTitres,
) -> TitresDetectes {
    // Les signets « [12] » mènent aux notes de fin (Calibre) : ni titres, ni plan.
    let appel_de_note = Regex::new(r"^\[\d+\]$").unwrap();
    let signets: Vec<Signet> = signets.iter().filter(|s| !appel_de_note.is_match(s.titre.trim())).cloned().collect();
    let signets = &signets[..];
    let moities_som = moities_sommaire(&blocs);
    let (pages_resume, pseudo_signets) = resume_sans_numeros(&blocs, &moities_som);
    let mut pages_som: BTreeSet<DemiPage> = moities_som.union(&pages_resume).copied().collect();

    let corps = taille_corps(&blocs);
    let effective = match source {
        SourceTitres::Auto => {
            let nb_gras_maj = blocs
                .iter()
                .filter(|b| match b {
                    Bloc::Paragraphe(p) => {
                        !pages_som.contains(&p.ancre.demi_page())
                            && candidat_gras_majuscules(p)
                            && paragraphe_titre_plausible(p)
                    }
                    _ => false,
                })
                .count();
            if signets_utilisables(signets, nb_pages) {
                SourceTitres::Signets
            } else if nb_gras_maj >= 3 {
                SourceTitres::GrasMajuscules
            } else {
                SourceTitres::Taille
            }
        }
        autre => autre,
    };
    // Avec des signets, le résumé n'est pas utile et ses pages restent cherchables
    // (une page de titre visée par un signet peut s'y trouver).
    if effective == SourceTitres::Signets {
        pages_som = moities_som.clone();
    }
    let mut points = points_de_sommaire(&blocs, &pages_som, |page| {
        format!("la page {page} semble être un sommaire d'origine : conservée comme texte")
    });

    let mut retraits = Vec::new();
    let mut illisibles = HashSet::new();
    let blocs = match effective {
        SourceTitres::Signets => {
            let (blocs, manquants, _) = titres_depuis_signets(blocs, &preparer_signets(signets), &pages_som);
            points.extend(manquants);
            blocs
        }
        _ => {
            let candidat = |p: &Paragraphe| match effective {
                SourceTitres::GrasMajuscules => candidat_gras_majuscules(p),
                _ => candidat_taille(p, corps),
            };
            let (blocs, courants, pts) = retirer_titres_courants(blocs, &pages_som, candidat, nb_pages);
            retraits.extend(courants);
            points.extend(pts);
            let pseudo: Vec<SignetCherche> = if !pseudo_signets.is_empty() {
                pseudo_signets.iter().cloned().map(Into::into).collect()
            } else {
                // Sommaire imprimé : ses entrées font le niveau 1 si au moins 3 d'entre
                // elles, et au moins la moitié, ne sont pas déjà des titres candidats.
                // Les correspondances (`trouve`) visent ces `blocs`, déjà privés de leurs
                // titres courants : ils ne changent plus jusqu'à `titres_depuis_signets`.
                let entrees = entrees_du_sommaire(&blocs, &moities_som, &pages_som);
                let deja_candidat = |i: usize| match &blocs[i] {
                    Bloc::Paragraphe(p) => {
                        !pages_som.contains(&p.ancre.demi_page()) && candidat(p) && paragraphe_titre_plausible(p)
                    }
                    _ => false,
                };
                let nouvelles =
                    entrees.iter().filter(|e| matches!(e.trouve, Some((debut, _)) if !deja_candidat(debut))).count();
                if nouvelles >= MIN_TITRES_NIVEAU && nouvelles * 2 >= entrees.len() { entrees } else { Vec::new() }
            };
            let niveau_1_pris = !pseudo.is_empty();
            // Titres tirés du sommaire ou du résumé : chacun est déjà entier, aucun n'est
            // fusionné avec son voisin (une partie et le chapitre qui ouvrent la même page).
            // Ils sont repérés par leur position dans les blocs, que `titres_par_candidats`
            // garde (un bloc pour un bloc, dans le même ordre).
            let mut proteges: HashSet<usize> = HashSet::new();
            let blocs = if niveau_1_pris {
                let (blocs, manquants, produits) = titres_depuis_signets(blocs, &pseudo, &pages_som);
                points.extend(manquants);
                proteges = produits;
                blocs
            } else {
                blocs
            };
            let nb_blocs = blocs.len();
            let (blocs, pts, refuses) = titres_par_candidats(blocs, &pages_som, candidat, niveau_1_pris);
            // Contrôle de la longueur seulement : l'ordre est garanti par construction
            // (`titres_par_candidats` est un `map` sur les blocs).
            debug_assert_eq!(blocs.len(), nb_blocs, "titres_par_candidats doit garder le nombre de blocs");
            points.extend(pts);
            illisibles = refuses;
            fusionner_titres_consecutifs(blocs, &proteges)
        }
    };
    points.extend(titres_courants_suspects(&blocs));
    TitresDetectes { blocs, points, source: effective, retraits, illisibles, sommaires: pages_som }
}

/// Titre du livre en tête du Markdown (titre de niveau 1). Il est cherché, comme un
/// signet, dans les blocs qui précèdent le premier titre de niveau 1 et au plus dans
/// les `PAGES_TITRE_LIVRE` premières pages : la suite de paragraphes qui le porte
/// exactement devient ce titre, placé avant les autres blocs de sa (demi-)page (le nom
/// d'auteur au-dessus passe après). Introuvable, il est inséré au début, ancré à la
/// page 1 entière. Le texte du titre est `titre_livre` tel quel. Si un titre de tout
/// niveau, avant le premier titre de niveau 1 ou celui-ci, porte déjà le titre du livre,
/// le premier d'entre eux est le titre trouvé : dans les `PAGES_TITRE_LIVRE` premières
/// pages, il passe au niveau 1 et remonte de même en tête de sa (demi-)page en gardant
/// son texte (un titre de niveau 2 quand les entrées d'un sommaire imprimé ont pris le
/// niveau 1) ; plus loin, rien ne bouge s'il est le premier titre de niveau 1. Quand le
/// titre déplacé séparait deux paragraphes de (demi-)pages différentes, un saut le
/// remplace : ils ne se raccordent pas. Rien à faire si le titre est vide.
pub fn placer_titre_livre(mut blocs: Vec<Bloc>, titre_livre: &str) -> Vec<Bloc> {
    let titre = titre_livre.trim();
    let cible = normaliser_titre(titre);
    if cible.is_empty() {
        return blocs;
    }
    let premier_n1 = blocs.iter().position(|b| matches!(b, Bloc::Titre { niveau: 1, .. }));
    // Premier titre, de tout niveau, qui porte le titre du livre, jusqu'au premier titre
    // de niveau 1 compris.
    let deja = blocs[..premier_n1.map_or(blocs.len(), |i| i + 1)].iter().enumerate().find_map(|(i, b)| match b {
        Bloc::Titre { texte, ancre, .. } if normaliser_titre(texte) == cible => Some((i, texte.clone(), ancre.page)),
        _ => None,
    });
    let trouve = match deja {
        Some((i, _, page)) if page > PAGES_TITRE_LIVRE && Some(i) == premier_n1 => return blocs,
        Some((i, texte, page)) if page <= PAGES_TITRE_LIVRE => Some((i, i + 1, texte)),
        _ => {
            let premier_n1 = premier_n1.unwrap_or(blocs.len());
            let fenetre = blocs[..premier_n1]
                .iter()
                .position(|b| b.ancre().page > PAGES_TITRE_LIVRE)
                .unwrap_or(premier_n1);
            let fenetre = &blocs[..fenetre];
            let index = IndexTitres::new(fenetre);
            let candidats = index.candidats(&cible);
            let pages: BTreeSet<u32> = fenetre.iter().map(|b| b.ancre().page).collect();
            // Seule une correspondance exacte est retenue : un paragraphe qui dit plus
            // que le titre garde son texte.
            pages.into_iter().find_map(|page| {
                chercher_signet(fenetre, &index, &candidats, &cible, page, &HashSet::new(), &BTreeSet::new())
                    .filter(|(_, _, texte)| normaliser_titre(texte) == cible)
                    .map(|(debut, fin, _)| (debut, fin, titre.to_string()))
            })
        }
    };
    let (position, texte, ancre) = match trouve {
        Some((debut, fin, texte)) => {
            let ancre = blocs[debut].ancre();
            let meme_page = |b: &Bloc| b.ancre().demi_page() == ancre.demi_page();
            let tete = blocs[..debut].iter().rposition(|b| !meme_page(b)).map_or(0, |i| i + 1);
            blocs.drain(debut..fin);
            // Le titre déplacé séparait deux (demi-)pages : un saut prend sa place, sans quoi
            // le raccord des pages collerait le paragraphe d'avant à celui d'après.
            let separe = |b: &Bloc| matches!(b, Bloc::Paragraphe(_));
            if tete < debut
                && let (Some(avant), Some(apres)) = (debut.checked_sub(1).map(|i| &blocs[i]), blocs.get(debut))
                && separe(avant)
                && separe(apres)
                && avant.ancre().demi_page() != apres.ancre().demi_page()
            {
                blocs.insert(debut, Bloc::Saut { ancre: apres.ancre() });
            }
            (tete, texte, ancre)
        }
        None => (0, titre.to_string(), Ancre { page: 1, moitie: Moitie::Entiere, rect: [0, 0, 1000, 1000] }),
    };
    blocs.insert(position, Bloc::Titre { niveau: 1, texte, ancre });
    blocs
}

/// Numéro de page en fin d'entrée de sommaire, après une espace ou des points de conduite ;
/// après des points de conduite, aussi « l 07 », « I07 »… (OCR de « 107 »,
/// `POINTS_DE_CONDUITE_ET_NUMERO`).
static NUMERO_FINAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(?:[\s.\u{{2026}}]+\d{{1,4}}|\s*{POINTS_DE_CONDUITE_ET_NUMERO})\s*$")).expect("motif valide")
});

/// Titres d'un sommaire imprimé (paragraphe placé juste avant la première entrée).
const TITRES_DE_SOMMAIRE: [&str; 2] = ["table des matieres", "sommaire"];

/// Le texte est-il « Table des matières » ou « Sommaire » (casse, accents et
/// ponctuation ignorés) ? Même décision que `plumeo_epub::est_titre_de_sommaire`,
/// vérifiée par un test.
fn est_titre_de_sommaire(texte: &str) -> bool {
    TITRES_DE_SOMMAIRE.contains(&normaliser(texte).as_str())
}

/// Le bloc est-il un paragraphe « Table des matières » ou « Sommaire » ?
fn titre_de_sommaire(bloc: &Bloc) -> bool {
    matches!(bloc, Bloc::Paragraphe(p) if est_titre_de_sommaire(&p.texte_brut()))
}

/// Titre qu'une entrée de sommaire peut viser : indice de son bloc et sa (demi-)page.
#[derive(Clone, Copy)]
struct TitreCible {
    indice: usize,
    lieu: DemiPage,
}

/// Titres que peuvent viser les entrées d'un sommaire imprimé (tous, sauf le titre du
/// livre), par texte normalisé (`normaliser_titre`) et dans l'ordre du livre ;
/// `sans_numero` : de même pour le texte des titres numérotés, sans le numéro en tête
/// (`sans_numero_pointe`).
struct TitresCibles {
    par_texte: HashMap<String, Vec<TitreCible>>,
    sans_numero: HashMap<String, Vec<TitreCible>>,
}

impl TitresCibles {
    /// Index des titres de `blocs`. Le titre du livre est le premier titre de niveau 1,
    /// s'il porte `titre_livre`.
    fn new(blocs: &[Bloc], titre_livre: &str) -> Self {
        let titre_livre = normaliser_titre(titre_livre.trim());
        let livre = blocs.iter().position(|b| matches!(b, Bloc::Titre { niveau: 1, .. })).filter(|&i| match &blocs[i] {
            Bloc::Titre { texte, .. } => !titre_livre.is_empty() && normaliser_titre(texte) == titre_livre,
            _ => false,
        });
        let mut cibles = TitresCibles { par_texte: HashMap::new(), sans_numero: HashMap::new() };
        for (indice, bloc) in blocs.iter().enumerate() {
            if let Bloc::Titre { texte, ancre, .. } = bloc
                && Some(indice) != livre
            {
                let cible = TitreCible { indice, lieu: ancre.demi_page() };
                let texte = texte.trim();
                let norm = normaliser_titre(texte);
                if !norm.is_empty() {
                    cibles.par_texte.entry(norm).or_default().push(cible);
                }
                if let Some(court) = sans_numero_pointe(texte)
                    && let norm = normaliser_titre(court)
                    && !norm.is_empty()
                {
                    cibles.sans_numero.entry(norm).or_default().push(cible);
                }
            }
        }
        cibles
    }

    /// Titres (hors de la (demi-)page `lieu`) que vise une entrée faite des paragraphes
    /// `textes` : son texte brut (entrée d'un seul paragraphe), puis son texte joint
    /// (`texte_entree`) ; le texte entier des titres d'abord, puis leur texte sans numéro
    /// en tête. Vide si aucun.
    fn vises(&self, textes: &[&str], lieu: DemiPage) -> Vec<TitreCible> {
        let joint = texte_entree(textes);
        let brut = if textes.len() == 1 { Some(textes[0].trim()) } else { None };
        for t in brut.into_iter().chain([joint.as_str()]) {
            let norm = normaliser_titre(t);
            for table in [&self.par_texte, &self.sans_numero] {
                let autres: Vec<TitreCible> =
                    table.get(&norm).into_iter().flatten().copied().filter(|c| c.lieu != lieu).collect();
                if !autres.is_empty() {
                    return autres;
                }
            }
        }
        Vec::new()
    }
}

/// Entrée reconnue sur une (demi-)page : paragraphes consécutifs `couverts` (indices dans
/// les blocs, jamais à cheval sur deux (demi-)pages) et titres qu'elle peut viser, dans
/// l'ordre du livre.
struct EntreeReconnue {
    couverts: Range<usize>,
    cibles: Vec<TitreCible>,
}

/// (Demi-)page du livre lue par `reconnaitre_sommaires` : ses paragraphes (indices dans
/// les blocs), ses entrées reconnues, et si c'est une (demi-)page de sommaire.
struct PageLue {
    lieu: DemiPage,
    paragraphes: Vec<usize>,
    entrees: Vec<EntreeReconnue>,
    sommaire: bool,
}

impl PageLue {
    /// Nombre de paragraphes couverts par des entrées.
    fn nb_couverts(&self) -> usize {
        self.entrees.iter().map(|e| e.couverts.len()).sum()
    }

    /// La (demi-)page s'ouvre-t-elle sur le titre du sommaire (« Table des matières » ou
    /// « Sommaire » avant sa première entrée) ?
    fn ouvre_le_sommaire(&self, blocs: &[Bloc]) -> bool {
        self.entrees.first().is_some_and(|premiere| {
            self.paragraphes.iter().any(|&j| j < premiere.couverts.start && titre_de_sommaire(&blocs[j]))
        })
    }
}

/// Entrée d'un sommaire imprimé liée à son titre : paragraphes couverts et indice du
/// titre visé.
struct EntreeLiee {
    couverts: Range<usize>,
    titre: usize,
}

/// Sommaires imprimés reconnus par `reconnaitre_sommaires` : entrées liées (dans l'ordre
/// des blocs), paragraphes « Table des matières » ou « Sommaire » qui deviennent des
/// titres, et (demi-)pages de sommaire.
struct SommairesReconnus {
    entrees: Vec<EntreeLiee>,
    titres_de_sommaire: Vec<usize>,
    demi_pages: BTreeSet<DemiPage>,
}

/// Entrées reconnues sur la (demi-)page `lieu`, faite des `paragraphes`. Une entrée part
/// d'un paragraphe et couvre ceux que permet `paragraphes_d_entree` ; la plus longue qui
/// vise un titre après la (demi-)page l'emporte, à défaut la plus longue qui en vise un.
/// Les paragraphes qu'elle couvre ne sont pas essayés comme début d'une autre.
fn entrees_de_la_page(
    blocs: &[Bloc],
    cibles: &TitresCibles,
    lieu: DemiPage,
    paragraphes: &[usize],
) -> Vec<EntreeReconnue> {
    let mut entrees = Vec::new();
    let mut k = 0;
    while k < paragraphes.len() {
        let suite = paragraphes_d_entree(blocs, paragraphes[k]);
        let textes: Vec<&str> = suite.iter().map(String::as_str).collect();
        let essais: Vec<(usize, Vec<TitreCible>)> =
            (1..=textes.len()).rev().map(|n| (n, cibles.vises(&textes[..n], lieu))).collect();
        let trouve = essais
            .iter()
            .find(|(_, vises)| vises.iter().any(|c| c.lieu > lieu))
            .or_else(|| essais.iter().find(|(_, vises)| !vises.is_empty()));
        match trouve {
            Some((n, vises)) => {
                let debut = paragraphes[k];
                entrees.push(EntreeReconnue { couverts: debut..debut + n, cibles: vises.clone() });
                k += n;
            }
            None => k += 1,
        }
    }
    entrees
}

/// (Demi-)pages du livre qui portent au moins un paragraphe, dans l'ordre du livre, avec
/// leurs entrées reconnues (pas encore marquées comme pages de sommaire).
fn lire_pages(blocs: &[Bloc], cibles: &TitresCibles) -> Vec<PageLue> {
    let mut rang: HashMap<DemiPage, usize> = HashMap::new();
    let mut pages: Vec<PageLue> = Vec::new();
    for (i, bloc) in blocs.iter().enumerate() {
        if let Bloc::Paragraphe(p) = bloc {
            let lieu = p.ancre.demi_page();
            let r = *rang.entry(lieu).or_insert_with(|| {
                pages.push(PageLue { lieu, paragraphes: Vec::new(), entrees: Vec::new(), sommaire: false });
                pages.len() - 1
            });
            pages[r].paragraphes.push(i);
        }
    }
    for page in &mut pages {
        page.entrees = entrees_de_la_page(blocs, cibles, page.lieu, &page.paragraphes);
    }
    pages
}

/// Marque les (demi-)pages de sommaire. Critère principal : au moins 3 entrées et au moins
/// la moitié des paragraphes couverts. Suite : la (demi-)page qui suit directement une
/// (demi-)page de sommaire en est une si tous ses paragraphes sont couverts. Précédente :
/// celle qui précède directement une (demi-)page de sommaire en est une si elle porte au
/// moins une entrée et si tous ses paragraphes sont couverts, sauf un éventuel titre
/// « Table des matières » ou « Sommaire » avant la première ; la chaîne remonte de proche
/// en proche, mais jamais au-delà d'une (demi-)page qui s'ouvre sur le titre du sommaire.
fn marquer_sommaires(pages: &mut [PageLue], blocs: &[Bloc]) {
    for r in 0..pages.len() {
        let suite = r.checked_sub(1).is_some_and(|a| pages[a].sommaire && contigues(pages[a].lieu, pages[r].lieu));
        let page = &pages[r];
        let couverts = page.nb_couverts();
        pages[r].sommaire = (page.entrees.len() >= MIN_TITRES_NIVEAU && couverts * 2 >= page.paragraphes.len())
            || (suite && couverts == page.paragraphes.len());
    }
    for r in (0..pages.len().saturating_sub(1)).rev() {
        let page = &pages[r];
        if page.sommaire
            || !pages[r + 1].sommaire
            || !contigues(page.lieu, pages[r + 1].lieu)
            || page.entrees.is_empty()
            || pages[r + 1].ouvre_le_sommaire(blocs)
        {
            continue;
        }
        let couverts: HashSet<usize> = page.entrees.iter().flat_map(|e| e.couverts.clone()).collect();
        let hors: Vec<usize> = page.paragraphes.iter().copied().filter(|i| !couverts.contains(i)).collect();
        pages[r].sommaire = match hors[..] {
            [] => true,
            [j] => j < page.entrees[0].couverts.start && titre_de_sommaire(&blocs[j]),
            _ => false,
        };
    }
}

/// Suites maximales de (demi-)pages de sommaire (rangs dans `pages`) de rangs consécutifs
/// et `voisines` deux à deux (`voisines(a, b)` : rangs `a` puis `b = a + 1`). Seules les
/// (demi-)pages qui portent des paragraphes ont un rang. Deux usages :
/// - `contigues` (même page ou page suivante) : sommaires où l'on cherche le titre de
///   chacun (avant la première entrée de la suite) ;
/// - toujours voisines : sommaires pour le suivi des homonymes ; une page vide ne coupe
///   pas un sommaire, mais le corps du livre sépare deux sommaires.
fn suites_de_sommaire(pages: &[PageLue], voisines: impl Fn(usize, usize) -> bool) -> Vec<Range<usize>> {
    let mut suites: Vec<Range<usize>> = Vec::new();
    for r in (0..pages.len()).filter(|&r| pages[r].sommaire) {
        match suites.last_mut() {
            Some(suite) if suite.end == r && voisines(r - 1, r) => suite.end = r + 1,
            _ => suites.push(r..r + 1),
        }
    }
    suites
}

/// Reconnaissance des sommaires imprimés de `blocs` (voir `sommaire_cliquable`) : leurs
/// (demi-)pages, leurs entrées et le titre visé par chacune, et leurs titres.
fn reconnaitre_sommaires(blocs: &[Bloc], titre_livre: &str) -> SommairesReconnus {
    let cibles = TitresCibles::new(blocs, titre_livre);
    let mut pages = lire_pages(blocs, &cibles);
    marquer_sommaires(&mut pages, blocs);
    let suites = suites_de_sommaire(&pages, |a, b| contigues(pages[a].lieu, pages[b].lieu));

    // Entrées des (demi-)pages de sommaire, dans l'ordre des blocs, avec leur groupe
    // (rang parmi les suites de rangs consécutifs, `suites_de_sommaire`) et leur
    // (demi-)page.
    let mut a_lier: Vec<(usize, &EntreeReconnue, DemiPage)> = suites_de_sommaire(&pages, |_, _| true)
        .into_iter()
        .enumerate()
        .flat_map(|(groupe, rangs)| pages[rangs].iter().map(move |page| (groupe, page)))
        .flat_map(|(groupe, page)| page.entrees.iter().map(move |entree| (groupe, entree, page.lieu)))
        .collect();
    a_lier.sort_by_key(|(_, entree, _)| entree.couverts.start);
    // Titres déjà visés, par groupe : un second sommaire qui liste les mêmes homonymes
    // les vise lui aussi dans l'ordre.
    let mut deja_vises: HashMap<usize, HashSet<usize>> = HashMap::new();
    let entrees: Vec<EntreeLiee> = a_lier
        .into_iter()
        .map(|(groupe, entree, lieu)| {
            let pris = deja_vises.entry(groupe).or_default();
            let libres: Vec<&TitreCible> = entree.cibles.iter().filter(|c| !pris.contains(&c.indice)).collect();
            let parmi = if libres.is_empty() { entree.cibles.iter().collect() } else { libres };
            let titre = parmi.iter().find(|c| c.lieu > lieu).or_else(|| parmi.first()).expect("titre visé").indice;
            pris.insert(titre);
            EntreeLiee { couverts: entree.couverts.clone(), titre }
        })
        .collect();

    // Titre de chaque sommaire : paragraphe placé juste avant sa première entrée (notes
    // et sauts passés : titre seul sur une page courte), s'il n'est pas lui-même dans une
    // entrée (suite de sommaire après une page vide).
    let couverts: HashSet<usize> = entrees.iter().flat_map(|e| e.couverts.clone()).collect();
    let titres_de_sommaire: Vec<usize> = suites
        .iter()
        .filter_map(|rangs| {
            let premiere = pages[rangs.start].entrees.first().expect("page avec entrées").couverts.start;
            blocs[..premiere]
                .iter()
                .rposition(|b| !matches!(b, Bloc::Note { .. } | Bloc::Saut { .. }))
                .filter(|j| !couverts.contains(j) && titre_de_sommaire(&blocs[*j]))
        })
        .collect();

    let demi_pages = pages.iter().filter(|p| p.sommaire).map(|p| p.lieu).collect();
    SommairesReconnus { entrees, titres_de_sommaire, demi_pages }
}

/// Ajoute le paragraphe `suite` (même (demi-)page) à l'entrée `entree`. Les espaces de
/// bord sont d'abord retirés : en fin de l'entrée (espaces de fin du dernier segment,
/// segments faits seulement d'espaces) et en tête de la suite. Puis une espace entre les
/// deux, ou une césure recollée sans tiret (« caté- » + « gories » → « catégories ») ; le
/// rectangle s'étend au paragraphe.
fn joindre_a_l_entree(entree: &mut Paragraphe, mut suite: Paragraphe) {
    while let Some(Segment::Texte { texte, .. }) = entree.segments.last_mut() {
        texte.truncate(texte.trim_end().len());
        if !texte.is_empty() {
            break;
        }
        entree.segments.pop();
    }
    while let Some(Segment::Texte { texte, .. }) = suite.segments.first_mut() {
        *texte = texte.trim_start().to_string();
        if !texte.is_empty() {
            break;
        }
        suite.segments.remove(0);
    }
    let cesure = fin_de_cesure(&entree.texte_brut(), &suite.texte_brut());
    match entree.segments.last_mut() {
        Some(Segment::Texte { texte, .. }) if cesure => {
            texte.pop();
        }
        _ => entree.segments.push(Segment::Texte { texte: " ".into(), gras: false, italique: false }),
    }
    let [x0, y0, x1, y1] = entree.ancre.rect;
    let [a0, b0, a1, b1] = suite.ancre.rect;
    entree.ancre.rect = [x0.min(a0), y0.min(b0), x1.max(a1), y1.max(b1)];
    entree.segments.extend(suite.segments);
}

/// Transformation des blocs selon les sommaires `reconnus` : leurs titres deviennent des
/// titres de niveau 1, et chaque entrée une seule `EntreeSommaire` (cible : identifiant du
/// titre visé, voir `markdown::ancre_titre`), ancrée à son premier paragraphe.
fn lier_les_entrees(mut blocs: Vec<Bloc>, reconnus: &SommairesReconnus) -> Vec<Bloc> {
    for &j in &reconnus.titres_de_sommaire {
        if let Bloc::Paragraphe(p) = &blocs[j] {
            blocs[j] = Bloc::Titre { niveau: 1, texte: p.texte_brut().trim().to_string(), ancre: p.ancre };
        }
    }
    // Identifiants de tous les titres, dans l'ordre du Markdown.
    let mut ancres = AncresTitres::default();
    let ids: HashMap<usize, String> = blocs
        .iter()
        .enumerate()
        .filter_map(|(i, b)| match b {
            Bloc::Titre { texte, .. } => Some((i, ancres.attribuer(texte))),
            _ => None,
        })
        .collect();
    let par_debut: HashMap<usize, &EntreeLiee> = reconnus.entrees.iter().map(|e| (e.couverts.start, e)).collect();
    let mut sortie = Vec::with_capacity(blocs.len());
    let mut restants = blocs.into_iter().enumerate();
    while let Some((i, bloc)) = restants.next() {
        let Some(liee) = par_debut.get(&i) else {
            sortie.push(bloc);
            continue;
        };
        let Bloc::Paragraphe(mut entree) = bloc else { unreachable!("entrée faite de paragraphes") };
        for (_, autre) in restants.by_ref().take(liee.couverts.len() - 1) {
            let Bloc::Paragraphe(p) = autre else { unreachable!("entrée faite de paragraphes") };
            joindre_a_l_entree(&mut entree, p);
        }
        let cible = ids[&liee.titre].clone();
        sortie.push(Bloc::EntreeSommaire { segments: entree.segments, cible, ancre: entree.ancre });
    }
    sortie
}

/// Sommaire imprimé rendu cliquable, en deux temps : reconnaissance des sommaires
/// (`reconnaitre_sommaires`), puis transformation des blocs (`lier_les_entrees`).
///
/// Une entrée est faite de 1 à 3 paragraphes consécutifs d'une même (demi-)page
/// (`paragraphes_d_entree`, texte joint `texte_entree` : césure recollée, sans le numéro
/// de page final ni les points de conduite) qui portent le texte d'un titre situé sur une
/// autre (demi-)page (`normaliser_titre`), ou ce texte sans son numéro en tête
/// suivi d'un point (`sans_numero_pointe` : « Les bases » vise « I. LES BASES », mais
/// « Était une fois » ne vise pas « IL ÉTAIT UNE FOIS ») ; l'entrée la plus longue qui
/// vise un titre après sa (demi-)page l'emporte, à défaut la plus longue qui en vise un.
///
/// Une (demi-)page de sommaire porte au moins 3 entrées, et au moins la moitié de ses
/// paragraphes sont couverts par des entrées (un faux-titre de 3 lignes, une seule entrée,
/// n'en est pas une) ; la (demi-)page qui suit directement une (demi-)page de sommaire en
/// est aussi une si tous ses paragraphes sont couverts, et celle qui la précède
/// directement (dans l'ordre de lecture : p3g avant p3d) aussi, si elle porte au moins
/// une entrée et si tous ses paragraphes sont couverts, sauf un éventuel titre « Table
/// des matières » ou « Sommaire » placé avant la première (voir `marquer_sommaires`).
///
/// Le titre visé est le premier de ce texte après la (demi-)page de l'entrée, à défaut le
/// premier du livre, en sautant les titres déjà visés par une entrée précédente du même
/// sommaire (`suites_de_sommaire` : une page vide ne le coupe pas) tant qu'il en reste
/// un libre (titres homonymes visés dans l'ordre, dans chaque sommaire) ; le titre du
/// livre (premier titre de niveau 1, s'il porte `titre_livre`) n'est jamais visé. Chaque
/// entrée devient une seule `EntreeSommaire` qui garde tout le texte de ses paragraphes,
/// joints par une espace ou recollés sur une césure (`joindre_a_l_entree`), ancrée au
/// premier, rectangle étendu aux autres ; le paragraphe « Table des matières » ou « Sommaire » placé juste avant la
/// première entrée d'une suite de (demi-)pages de sommaire (notes et sauts passés), s'il
/// n'est pas lui-même dans une entrée, devient un titre de niveau 1.
///
/// Renvoie aussi un point `Sommaire` par page qui contient un sommaire, ancré à sa
/// première (demi-)page de sommaire, et les (demi-)pages de sommaire.
pub fn sommaire_cliquable(
    blocs: Vec<Bloc>,
    titre_livre: &str,
) -> (Vec<Bloc>, Vec<PointAVerifier>, BTreeSet<DemiPage>) {
    let reconnus = reconnaitre_sommaires(&blocs, titre_livre);
    let blocs = lier_les_entrees(blocs, &reconnus);
    let points = points_de_sommaire(&blocs, &reconnus.demi_pages, |page| {
        format!("la page {page} est un sommaire imprimé : ses entrées renvoient aux titres")
    });
    (blocs, points, reconnus.demi_pages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocs::Segment;
    use crate::zones::Role;

    /// Pages dont une moitié (ou la page entière) ressemble à un sommaire.
    fn pages_sommaire(blocs: &[Bloc]) -> BTreeSet<u32> {
        moities_sommaire(blocs).into_iter().map(|(page, _)| page).collect()
    }

    fn ancre(page: u32) -> Ancre {
        Ancre { page, moitie: Moitie::Entiere, rect: [100, 100, 900, 200] }
    }

    fn para(page: u32, texte: &str, taille: f32, gras: bool, lignes: usize) -> Bloc {
        Bloc::Paragraphe(Paragraphe {
            segments: vec![Segment::Texte { texte: texte.into(), gras, italique: false }],
            ancre: ancre(page),
            taille,
            tout_gras: gras,
            nb_lignes: lignes,
        })
    }

    fn corps(page: u32, texte: &str) -> Bloc {
        para(page, texte, 12.0, false, 5)
    }

    fn signet(titre: &str, niveau: u8, page: u32) -> Signet {
        Signet { titre: titre.into(), niveau, page }
    }

    fn titres(blocs: &[Bloc]) -> Vec<(u8, String)> {
        blocs
            .iter()
            .filter_map(|b| match b {
                Bloc::Titre { niveau, texte, .. } => Some((*niveau, texte.clone())),
                _ => None,
            })
            .collect()
    }

    fn texte_long() -> String {
        "Un long paragraphe de texte courant qui occupe plusieurs lignes du livre. ".repeat(4)
    }

    #[test]
    fn signets_rejetes_si_un_par_page() {
        let un_par_page: Vec<Signet> = (1..=207).map(|p| signet(&format!("Chapitre {p}"), 1, p)).collect();
        assert!(!signets_utilisables(&un_par_page, 207));
        let pages: Vec<Signet> = (1..=96).map(|p| signet(&format!("Page {p}"), 1, p)).collect();
        assert!(!signets_utilisables(&pages, 96));
        let bons: Vec<Signet> = (1..=73).map(|i| signet(&format!("Chapitre {i}"), 1, i * 2 - 1)).collect();
        assert!(signets_utilisables(&bons, 136));
    }

    #[test]
    fn titre_depuis_signet() {
        let blocs = vec![corps(2, &texte_long()), para(3, "1. VISITE À PATMOS", 12.0, false, 1), corps(3, &texte_long())];
        let signets = [signet("1. VISITE À PATMOS", 1, 3), signet("B", 1, 5), signet("C", 1, 7)];
        let TitresDetectes { blocs, source, .. } = detecter_titres(blocs, &signets, 100, SourceTitres::Signets);
        assert_eq!(source, SourceTitres::Signets);
        assert_eq!(titres(&blocs)[0], (1, "1. VISITE À PATMOS".to_string()));
        assert_eq!(blocs.len(), 3 + 2); // deux titres insérés pour B et C
    }

    #[test]
    fn titre_multiligne_depuis_signet() {
        let blocs = vec![
            para(4, "LE SENS DE L’AMOUR", 12.0, false, 1),
            para(4, "DANS LE MONDE", 12.0, false, 1),
            corps(4, &texte_long()),
        ];
        let signets = [signet("LE SENS DE L’AMOUR DANS LE MONDE", 1, 4)];
        let TitresDetectes { blocs, points, .. } = detecter_titres(blocs, &signets, 100, SourceTitres::Signets);
        assert_eq!(titres(&blocs), vec![(1, "LE SENS DE L’AMOUR DANS LE MONDE".to_string())]);
        assert_eq!(blocs.len(), 2);
        assert!(points.is_empty());
    }

    #[test]
    fn signet_numero_et_titre_sur_trois_lignes() {
        // Calibre : numéro de chapitre puis titre centré sur trois lignes.
        let blocs = vec![
            para(22, "3", 19.4, true, 1),
            para(22, "MELKHITSÉDEK", 19.4, true, 1),
            para(22, "ET L’ENSEIGNEMENT", 19.4, true, 1),
            para(22, "DES DEUX PRINCIPES", 19.4, true, 1),
            corps(22, &texte_long()),
        ];
        let signets = [signet("3. MELKHITSÉDEK ET L’ENSEIGNEMENT DES DEUX PRINCIPES", 1, 22)];
        let TitresDetectes { blocs, points, .. } = detecter_titres(blocs, &signets, 100, SourceTitres::Signets);
        assert_eq!(titres(&blocs), vec![(1, "3 MELKHITSÉDEK ET L’ENSEIGNEMENT DES DEUX PRINCIPES".to_string())]);
        assert_eq!(blocs.len(), 2);
        assert!(points.is_empty());
    }

    #[test]
    fn signet_long_retrouve_a_l_identique() {
        // Le titre du signet fait 16 mots : la limite de 12 mots ne vaut que pour une
        // correspondance partielle.
        let blocs = vec![
            para(93, "13", 19.4, true, 1),
            para(93, "LA BÊTE QUI MONTE", 19.4, true, 1),
            para(93, "DE LA MER ET LA BÊTE", 19.4, true, 1),
            para(93, "QUI MONTE DE LA TERRE", 19.4, true, 1),
            corps(93, &texte_long()),
        ];
        let signets = [signet("13. LA BÊTE QUI MONTE DE LA MER ET LA BÊTE QUI MONTE DE LA TERRE", 1, 93)];
        let TitresDetectes { blocs, points, .. } = detecter_titres(blocs, &signets, 100, SourceTitres::Signets);
        assert_eq!(blocs.len(), 2);
        assert!(points.is_empty());
    }

    #[test]
    fn signets_d_appel_de_note_ignores() {
        // Calibre : chaque note de fin a son signet « [12] » ; ce ne sont pas des titres.
        let blocs = vec![
            para(3, "1. VISITE À PATMOS", 12.0, false, 1),
            corps(3, &texte_long()),
            corps(9, "[1] Nom donné en grec à saint Jean."),
        ];
        let signets = [
            signet("1. VISITE À PATMOS", 1, 3),
            signet("2. INTRODUCTION", 1, 5),
            signet("3. MELKHITSÉDEK", 1, 7),
            signet("[1]", 1, 9),
            signet("[2]", 1, 9),
        ];
        let TitresDetectes { blocs, points, source, .. } = detecter_titres(blocs, &signets, 100, SourceTitres::Auto);
        assert_eq!(source, SourceTitres::Signets);
        let t: Vec<String> = titres(&blocs).into_iter().map(|(_, x)| x).collect();
        assert_eq!(t, ["1. VISITE À PATMOS", "2. INTRODUCTION", "3. MELKHITSÉDEK"]);
        assert!(points.iter().all(|p| !p.message.contains("[1]")));
    }

    #[test]
    fn signet_introuvable() {
        let blocs = vec![corps(6, &texte_long())];
        let signets = [signet("Épilogue", 1, 6)];
        let TitresDetectes { blocs, points, .. } = detecter_titres(blocs, &signets, 100, SourceTitres::Signets);
        assert!(matches!(&blocs[0], Bloc::Titre { texte, niveau: 1, .. } if texte == "Épilogue"));
        assert!(matches!(blocs[1], Bloc::Paragraphe(_)));
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].genre, GenrePoint::TitreSuspect);
        assert!(points[0].message.contains("Épilogue") && points[0].message.contains('6'));
    }

    #[test]
    fn signet_court_ne_avale_pas_le_corps() {
        let blocs = vec![corps(3, "Il était une fois un livre très long."), corps(3, &texte_long())];
        let TitresDetectes { blocs, points, .. } =
            detecter_titres(blocs, &[signet("I", 1, 3)], 100, SourceTitres::Signets);
        assert!(matches!(&blocs[0], Bloc::Titre { texte, .. } if texte == "I"));
        assert!(matches!(blocs[1], Bloc::Paragraphe(_)));
        assert_eq!(points.len(), 1);
    }

    #[test]
    fn signet_refuse_un_long_paragraphe() {
        let long = format!("Introduction de la méthode {}", vec!["mot"; 20].join(" "));
        let blocs = vec![para(3, &long, 12.0, false, 2)];
        let TitresDetectes { blocs, points, .. } =
            detecter_titres(blocs, &[signet("Introduction", 1, 3)], 100, SourceTitres::Signets);
        assert!(matches!(&blocs[0], Bloc::Titre { texte, .. } if texte == "Introduction"));
        assert!(matches!(blocs[1], Bloc::Paragraphe(_)));
        assert_eq!(points.len(), 1);
    }

    #[test]
    fn signet_prefere_la_correspondance_exacte() {
        let blocs = vec![para(3, "Introduction générale", 12.0, false, 1), para(3, "Introduction", 12.0, false, 1)];
        let TitresDetectes { blocs, points, .. } =
            detecter_titres(blocs, &[signet("Introduction", 1, 3)], 100, SourceTitres::Signets);
        assert!(matches!(&blocs[0], Bloc::Paragraphe(_)));
        assert!(matches!(&blocs[1], Bloc::Titre { texte, .. } if texte == "Introduction"));
        assert!(points.is_empty());
    }

    /// Plan à plat de Calibre : chapitres numérotés en arabe, puis sections « I », titre.
    fn plan_a_plat() -> Vec<Signet> {
        vec![
            signet("1. VISITE À PATMOS", 1, 3),
            signet("2. INTRODUCTION", 1, 5),
            signet("3. LA CITÉ CÉLESTE", 1, 7),
            signet("I", 1, 9),
            signet("La pierre cubique", 1, 9),
            signet("II", 1, 11),
            signet("Les assises", 1, 11),
        ]
    }

    #[test]
    fn signets_numero_et_titre_fusionnes() {
        let blocs = vec![
            para(9, "I", 19.4, true, 1),
            para(9, "La pierre cubique", 19.4, true, 1),
            corps(9, &texte_long()),
        ];
        let signets = [signet("1. A", 1, 3), signet("2. B", 1, 5), signet("I", 1, 9), signet("La pierre cubique", 1, 9)];
        let TitresDetectes { blocs, points, .. } = detecter_titres(blocs, &signets, 100, SourceTitres::Signets);
        let t = titres(&blocs);
        assert!(t.iter().any(|(_, x)| x == "I. La pierre cubique"), "{t:?}");
        assert!(t.iter().all(|(_, x)| x != "I" && x != "La pierre cubique"), "{t:?}");
        // Les deux paragraphes sont consommés : il reste le titre et le corps de la page 9.
        assert_eq!(blocs.iter().filter(|b| b.ancre().page == 9).count(), 2);
        assert!(points.iter().all(|p| p.ancre.page != 9), "{points:?}");
    }

    #[test]
    fn signets_numero_et_titre_dans_un_seul_paragraphe() {
        let blocs = vec![para(9, "I La pierre cubique", 19.4, true, 1), corps(9, &texte_long())];
        let signets = [signet("1. A", 1, 3), signet("2. B", 1, 5), signet("I", 1, 9), signet("La pierre cubique", 1, 9)];
        let TitresDetectes { blocs, points, .. } = detecter_titres(blocs, &signets, 100, SourceTitres::Signets);
        assert!(titres(&blocs).iter().any(|(_, x)| x == "I. La pierre cubique"));
        assert_eq!(blocs.iter().filter(|b| b.ancre().page == 9).count(), 2);
        assert!(points.iter().all(|p| p.ancre.page != 9), "{points:?}");
    }

    #[test]
    fn signets_fusionnes_sur_un_titre_plus_long_gardent_le_texte_des_paragraphes() {
        // Le corps dit plus que le signet : le texte du signet ne doit pas en effacer la fin.
        let blocs = vec![
            para(9, "I", 19.4, true, 1),
            para(9, "La pierre cubique et la mesure", 19.4, true, 1),
            corps(9, &texte_long()),
        ];
        let signets = [signet("1. A", 1, 3), signet("2. B", 1, 5), signet("I", 1, 9), signet("La pierre cubique", 1, 9)];
        let TitresDetectes { blocs, .. } = detecter_titres(blocs, &signets, 100, SourceTitres::Signets);
        let t = titres(&blocs);
        assert!(t.iter().any(|(_, x)| x == "I. La pierre cubique et la mesure"), "{t:?}");
        assert!(t.iter().all(|(_, x)| x != "I. La pierre cubique"), "{t:?}");
        assert_eq!(blocs.iter().filter(|b| b.ancre().page == 9).count(), 2);
    }

    #[test]
    fn signets_romains_apres_arabes_descendent_d_un_niveau() {
        let blocs: Vec<Bloc> = plan_a_plat().iter().map(|s| para(s.page, &s.titre, 19.4, true, 1)).collect();
        let TitresDetectes { blocs, points, .. } = detecter_titres(blocs, &plan_a_plat(), 100, SourceTitres::Signets);
        assert_eq!(
            titres(&blocs),
            vec![
                (1, "1. VISITE À PATMOS".to_string()),
                (1, "2. INTRODUCTION".to_string()),
                (1, "3. LA CITÉ CÉLESTE".to_string()),
                (2, "I. La pierre cubique".to_string()),
                (2, "II. Les assises".to_string()),
            ]
        );
        assert!(points.is_empty(), "{points:?}");
    }

    #[test]
    fn serie_romaine_finie_par_le_prochain_signet_arabe() {
        let signets = vec![
            signet("1. A", 1, 3),
            signet("I", 1, 5),
            signet("Un", 1, 5),
            signet("Épilogue", 1, 6),
            signet("II", 1, 7),
            signet("Deux", 1, 7),
            signet("2. B", 1, 9),
            signet("I", 1, 11),
            signet("Trois", 1, 11),
        ];
        let blocs = vec![corps(1, &texte_long())];
        let TitresDetectes { blocs, .. } = detecter_titres(blocs, &signets, 100, SourceTitres::Signets);
        let niveaux: Vec<(u8, String)> = titres(&blocs);
        let attendu = [
            (1, "1. A"),
            (2, "I. Un"),
            (1, "Épilogue"), // non numéroté : garde son niveau
            (2, "II. Deux"),
            (1, "2. B"),
            (2, "I. Trois"),
        ];
        assert_eq!(niveaux.len(), attendu.len(), "{niveaux:?}");
        for (obtenu, attendu) in niveaux.iter().zip(attendu) {
            assert_eq!((obtenu.0, obtenu.1.as_str()), attendu);
        }
    }

    #[test]
    fn signets_romains_sans_arabe_avant_gardent_leur_niveau() {
        let signets = [signet("I", 1, 3), signet("Un", 1, 3), signet("II", 1, 5), signet("Deux", 1, 5), signet("III", 1, 7)];
        let TitresDetectes { blocs, .. } =
            detecter_titres(vec![corps(1, &texte_long())], &signets, 100, SourceTitres::Signets);
        assert!(titres(&blocs).iter().all(|(n, _)| *n == 1), "{:?}", titres(&blocs));
    }

    #[test]
    fn serie_romaine_qui_ne_repart_pas_a_un_ne_descend_pas() {
        let signets = [signet("1. A", 1, 3), signet("II", 1, 5), signet("Deux", 1, 5), signet("2. B", 1, 7)];
        let TitresDetectes { blocs, .. } =
            detecter_titres(vec![corps(1, &texte_long())], &signets, 100, SourceTitres::Signets);
        assert_eq!(titres(&blocs)[1], (1, "II. Deux".to_string()));
    }

    #[test]
    fn plan_deja_imbrique_inchange() {
        let signets = [
            signet("1. A", 1, 3),
            signet("I", 2, 3),
            signet("2. B", 1, 5),
            signet("II", 2, 5),
            signet("3. C", 1, 7),
        ];
        let TitresDetectes { blocs, .. } =
            detecter_titres(vec![corps(1, &texte_long())], &signets, 100, SourceTitres::Signets);
        let niveaux: Vec<u8> = titres(&blocs).into_iter().map(|(n, _)| n).collect();
        assert_eq!(niveaux, [1, 2, 1, 2, 1]);
    }

    #[test]
    fn deux_numeros_consecutifs_ne_sont_pas_fusionnes() {
        let signets = [signet("1. A", 1, 3), signet("2. B", 1, 5), signet("I", 1, 7), signet("II", 1, 7)];
        let TitresDetectes { blocs, .. } =
            detecter_titres(vec![corps(1, &texte_long())], &signets, 100, SourceTitres::Signets);
        let t: Vec<String> = titres(&blocs).into_iter().map(|(_, x)| x).collect();
        assert_eq!(t, ["1. A", "2. B", "I", "II"]);
    }

    #[test]
    fn signets_fusionnes_introuvables_gardent_le_titre_fusionne() {
        let signets = [signet("1. A", 1, 3), signet("2. B", 1, 5), signet("I", 1, 7), signet("Un", 1, 7)];
        let TitresDetectes { blocs, points, .. } =
            detecter_titres(vec![corps(7, &texte_long())], &signets, 100, SourceTitres::Signets);
        assert!(titres(&blocs).contains(&(2, "I. Un".to_string())), "{:?}", titres(&blocs));
        assert!(points.iter().any(|p| p.message.contains("I. Un")));
    }

    #[test]
    fn signet_page_vide_insere_avant_la_page_suivante() {
        let blocs = vec![corps(3, &texte_long()), corps(5, &texte_long())];
        let TitresDetectes { blocs, .. } =
            detecter_titres(blocs, &[signet("Partie", 1, 4)], 100, SourceTitres::Signets);
        assert!(matches!(blocs[0], Bloc::Paragraphe(_)));
        assert!(matches!(&blocs[1], Bloc::Titre { ancre, .. } if ancre.page == 4 && ancre.rect == [0, 0, 1000, 1000]));
        assert!(matches!(blocs[2], Bloc::Paragraphe(_)));
    }

    #[test]
    fn gras_majuscules() {
        let mut blocs = Vec::new();
        for n in 1..=10 {
            blocs.push(para(n, &format!("CHAPITRE {n}:"), 12.0, true, 1));
            blocs.push(corps(n, &texte_long()));
        }
        blocs.push(para(11, "Un titre en gras minuscule", 12.0, true, 1));
        let TitresDetectes { blocs, source, .. } = detecter_titres(blocs, &[], 11, SourceTitres::Auto);
        assert_eq!(source, SourceTitres::GrasMajuscules);
        let t = titres(&blocs);
        assert_eq!(t.len(), 10);
        assert!(t.iter().all(|(n, _)| *n == 1));
        assert!(matches!(blocs.last(), Some(Bloc::Paragraphe(_))));
    }

    #[test]
    fn par_taille_niveaux() {
        let long: String = vec!["mot"; 20].join(" ");
        let blocs = vec![
            corps(1, &texte_long()),
            para(2, "Grand titre", 18.0, false, 1),
            corps(2, &texte_long()),
            para(3, "Sous-titre", 14.0, false, 1),
            corps(3, &texte_long()),
            para(4, &long, 18.0, false, 2),
        ];
        let TitresDetectes { blocs, source, .. } = detecter_titres(blocs, &[], 4, SourceTitres::Auto);
        assert_eq!(source, SourceTitres::Taille);
        assert_eq!(titres(&blocs), vec![(1, "Grand titre".to_string()), (2, "Sous-titre".to_string())]);
        assert!(matches!(blocs[5], Bloc::Paragraphe(_)));
    }

    #[test]
    fn niveaux_tolerants_aux_tailles_d_ocr_et_aux_grands_titres_isoles() {
        // Scan OCR : les titres de chapitre varient de 16,2 à 16,9 pt ; la couverture et
        // un schéma ont des tailles isolées plus grandes, qui ne prennent pas le niveau 1.
        let mut blocs = vec![
            para(1, "L'ALCHIMIE", 34.0, false, 1),
            para(1, "Omraam", 28.6, false, 1),
            corps(2, &texte_long()),
        ];
        for (page, taille) in [(10, 16.5), (29, 16.9), (47, 16.2), (62, 16.4)] {
            blocs.push(para(page, &format!("Titre de la page {page}"), taille, false, 1));
            blocs.push(para(page, "(sous-titre)", 14.0, false, 1));
            blocs.push(corps(page, &texte_long()));
        }
        blocs.push(para(65, "Corps astral", 34.1, false, 1));
        blocs.push(corps(65, &texte_long()));
        let TitresDetectes { blocs, .. } = detecter_titres(blocs, &[], 70, SourceTitres::Taille);
        let t = titres(&blocs);
        let niveau = |texte: &str| t.iter().find(|(_, x)| x == texte).map(|(n, _)| *n);
        assert_eq!(niveau("Titre de la page 10"), Some(1));
        assert_eq!(niveau("Titre de la page 29"), Some(1));
        assert_eq!(niveau("Titre de la page 47"), Some(1));
        assert_eq!(niveau("L'ALCHIMIE Omraam"), Some(1));
        assert_eq!(niveau("Corps astral"), Some(1));
        assert_eq!(t.iter().filter(|(n, x)| *n == 2 && x == "(sous-titre)").count(), 4);
    }

    #[test]
    fn tailles_espacees_d_un_point_font_trois_niveaux() {
        // Tolérance de taille 0,5 pt : 16, 15 et 14 pt sont trois niveaux distincts.
        let mut blocs = vec![corps(1, &texte_long())];
        for page in 2..=4 {
            for (texte, taille) in [("Partie", 16.0), ("Livre", 15.0), ("Section", 14.0)] {
                blocs.push(para(page, &format!("{texte} {page}"), taille, false, 1));
                blocs.push(corps(page, &texte_long()));
            }
        }
        let TitresDetectes { blocs, .. } = detecter_titres(blocs, &[], 4, SourceTitres::Taille);
        let t = titres(&blocs);
        let niveau = |texte: &str| t.iter().find(|(_, x)| x == texte).map(|(n, _)| *n);
        assert_eq!(niveau("Partie 2"), Some(1));
        assert_eq!(niveau("Livre 3"), Some(2));
        assert_eq!(niveau("Section 4"), Some(3));
    }

    #[test]
    fn groupe_de_tailles_limite_a_un_point() {
        // Une chaîne de tailles voisines de 0,4 pt s'étend sur plus d'un point : elle est
        // coupée à son plus grand écart (titre isolé à 17,3 pt, chapitres de 16,1 à 16,9).
        let mut blocs = vec![corps(1, &texte_long()), para(2, "s", 17.3, false, 1), corps(2, &texte_long())];
        for (page, taille) in [(10, 16.9), (11, 16.5), (12, 16.2), (13, 16.1)] {
            blocs.push(para(page, &format!("Titre {page}"), taille, false, 1));
            blocs.push(corps(page, &texte_long()));
        }
        let TitresDetectes { blocs, .. } = detecter_titres(blocs, &[], 13, SourceTitres::Taille);
        let t = titres(&blocs);
        assert!(t.iter().all(|(n, _)| *n == 1), "{t:?}");
    }

    #[test]
    fn titres_chapitre_au_niveau_1() {
        // Les titres « CHAPITRE … » donnent le niveau 1, même plus petits qu'un autre titre.
        let mut blocs = vec![para(1, "NOTE DE L'ÉDITEUR:", 22.5, true, 1), corps(1, &texte_long())];
        for (n, nom) in ["I", "II", "III", "IV"].iter().enumerate() {
            let page = 10 + n as u32;
            blocs.push(para(page, &format!("CHAPITRE {nom}"), 15.0, true, 1));
            blocs.push(corps(page, &texte_long()));
            blocs.push(para(page, "(2 FÉVRIER 1995)", 15.0, true, 1));
            blocs.push(corps(page, &texte_long()));
        }
        blocs.push(para(20, "PREMIER CHAPITRE", 15.0, true, 1));
        let TitresDetectes { blocs, source, .. } = detecter_titres(blocs, &[], 20, SourceTitres::Auto);
        assert_eq!(source, SourceTitres::GrasMajuscules);
        let t = titres(&blocs);
        let n1: Vec<&str> = t.iter().filter(|(n, _)| *n == 1).map(|(_, x)| x.as_str()).collect();
        assert_eq!(n1, ["CHAPITRE I", "CHAPITRE II", "CHAPITRE III", "CHAPITRE IV", "PREMIER CHAPITRE"]);
        assert!(t.iter().filter(|(n, _)| *n != 1).all(|(n, _)| *n == 2));
    }

    #[test]
    fn titres_consecutifs_fusionnes() {
        let blocs = vec![
            corps(1, &texte_long()),
            para(2, "CHAPITRE 1:", 18.0, true, 1),
            para(2, "LES ANGES", 18.0, true, 1),
            corps(2, &texte_long()),
        ];
        let TitresDetectes { blocs, .. } = detecter_titres(blocs, &[], 2, SourceTitres::Taille);
        assert_eq!(titres(&blocs), vec![(1, "CHAPITRE 1: LES ANGES".to_string())]);
        assert_eq!(blocs.len(), 3);
    }

    #[test]
    fn sommaire_non_titre() {
        let mut blocs = vec![corps(1, &texte_long())];
        for (i, nom) in ["Visite à Patmos", "Les anges", "Le ciel", "La terre", "La mer", "Le feu"].iter().enumerate() {
            blocs.push(para(2, &format!("{nom} ...... {}", 12 + i), 18.0, true, 1));
        }
        let TitresDetectes { blocs, points, .. } = detecter_titres(blocs, &[], 10, SourceTitres::Taille);
        assert!(titres(&blocs).is_empty());
        assert_eq!(blocs.len(), 7);
        let sommaires: Vec<_> = points.iter().filter(|p| p.genre == GenrePoint::Sommaire).collect();
        assert_eq!(sommaires.len(), 1);
        assert_eq!(sommaires[0].ancre.page, 2);
    }

    #[test]
    fn points_de_suspension_ne_font_pas_un_sommaire() {
        let mut blocs = Vec::new();
        for n in 1..=3 {
            blocs.push(para(n, &format!("({n} JANVIER 2000)"), 12.0, true, 1));
            blocs.push(corps(n, &texte_long()));
        }
        for _ in 0..4 {
            blocs.push(corps(2, "Il a dit... puis il est reparti, sans un mot... Voilà."));
        }
        let TitresDetectes { blocs, points, .. } = detecter_titres(blocs, &[], 3, SourceTitres::GrasMajuscules);
        assert!(points.iter().all(|p| p.genre != GenrePoint::Sommaire));
        assert_eq!(titres(&blocs).len(), 3);
    }

    #[test]
    fn titre_courant_retire_et_trace() {
        // Le titre du livre, répété au fil des pages, n'est pas un titre de chapitre.
        // PDF recomposé : l'ancien pied de page (annonce + titre du livre) est au milieu
        // des pages, sur 6 pages d'un livre de 10.
        let mut blocs = Vec::new();
        for page in 1..=6 {
            blocs.push(para(page, &format!("CHAPITRE {page}"), 12.0, true, 1));
            blocs.push(corps(page, &texte_long()));
            blocs.push(corps(page, "S.v.p. n'hésitez pas à me contacter pour de plus amples informations."));
            blocs.push(para(page, "LES ANGES ET L’ABRE DE VIE", 12.0, true, 1));
            blocs.push(corps(page, &texte_long()));
        }
        let TitresDetectes { blocs, points, retraits, .. } =
            detecter_titres(blocs, &[], 10, SourceTitres::GrasMajuscules);
        let t = titres(&blocs);
        assert_eq!(t.len(), 6);
        assert!(t.iter().all(|(_, x)| x.starts_with("CHAPITRE")), "{t:?}");
        assert_eq!(retraits.len(), 6);
        assert!(retraits.iter().all(|r| r.role == Role::TitreCourant && r.texte == "LES ANGES ET L’ABRE DE VIE"));
        assert_eq!(retraits[2].ancre.page, 3);
        let suspects: Vec<_> = points.iter().filter(|p| p.genre == GenrePoint::TitreSuspect).collect();
        assert_eq!(suspects.len(), 6);
    }

    #[test]
    fn titre_notes_repete_n_est_pas_un_titre_courant() {
        // « NOTES » en tête de page à la fin de 6 chapitres, sur un livre de 12 pages.
        let mut blocs = Vec::new();
        for n in 0..6 {
            let page = 2 * n + 1;
            blocs.push(para(page, &format!("CHAPITRE {n}"), 12.0, true, 1));
            blocs.push(Bloc::Paragraphe(Paragraphe {
                segments: vec![Segment::Texte { texte: "Un mot".into(), gras: false, italique: false }, Segment::Exposant("1".into())],
                ..match corps(page, &texte_long()) { Bloc::Paragraphe(p) => p, _ => unreachable!() }
            }));
            blocs.push(para(page + 1, "NOTES", 12.0, true, 1));
            blocs.push(corps(page + 1, "1. La note du chapitre."));
        }
        let TitresDetectes { blocs, retraits, .. } = detecter_titres(blocs, &[], 12, SourceTitres::GrasMajuscules);
        assert!(retraits.is_empty());
        let (blocs, _, _) = crate::notes::notes_regroupees(blocs, &mut crate::notes::Etiqueteuse::new(), &crate::paragraphes::Vocabulaire::default());
        assert_eq!(blocs.iter().filter(|b| matches!(b, Bloc::Note { .. })).count(), 6);
    }

    #[test]
    fn titre_repete_sur_peu_de_pages_garde() {
        // « PREMIÈRE CONFÉRENCE » en tête de 5 pages d'un livre de 300 pages : 1,7 %.
        let mut blocs = Vec::new();
        for page in [10, 70, 130, 190, 250] {
            blocs.push(para(page, "PREMIÈRE CONFÉRENCE", 12.0, true, 1));
            blocs.push(corps(page, &texte_long()));
            blocs.push(para(page + 1, &format!("CHAPITRE {page}"), 12.0, true, 1));
        }
        let TitresDetectes { blocs, retraits, .. } = detecter_titres(blocs, &[], 300, SourceTitres::GrasMajuscules);
        assert!(retraits.is_empty());
        assert_eq!(titres(&blocs).iter().filter(|(_, t)| t == "PREMIÈRE CONFÉRENCE").count(), 5);
    }

    #[test]
    fn titre_courant_en_tete_de_page_retire() {
        // Titre courant en premier bloc de 40 % des pages (40 pages sur 100).
        let mut blocs = Vec::new();
        for page in 1..=100 {
            if page % 5 < 2 {
                blocs.push(para(page, "L'ALCHIMIE SPIRITUELLE", 12.0, true, 1));
            }
            if page % 20 == 3 {
                blocs.push(para(page, &format!("CHAPITRE {page}"), 12.0, true, 1));
            }
            blocs.push(corps(page, &texte_long()));
        }
        let TitresDetectes { blocs, points, retraits, .. } =
            detecter_titres(blocs, &[], 100, SourceTitres::GrasMajuscules);
        assert_eq!(retraits.len(), 40);
        assert!(retraits.iter().all(|r| r.role == Role::TitreCourant));
        assert!(titres(&blocs).iter().all(|(_, t)| t.starts_with("CHAPITRE")));
        assert_eq!(points.iter().filter(|p| p.message.contains("titre courant")).count(), 40);
    }

    #[test]
    fn date_jamais_collee_a_un_titre() {
        let mut blocs = vec![corps(1, &texte_long())];
        for page in 2..=4 {
            blocs.push(para(page, &format!("({page} MARS 1999)"), 12.0, true, 1));
            blocs.push(para(page, "LES ANGES DANS LA TRADITION", 12.0, true, 1));
            blocs.push(corps(page, &texte_long()));
        }
        let TitresDetectes { blocs, .. } = detecter_titres(blocs, &[], 4, SourceTitres::GrasMajuscules);
        let t: Vec<String> = titres(&blocs).into_iter().map(|(_, x)| x).collect();
        assert_eq!(t[0], "(2 MARS 1999)");
        assert_eq!(t[1], "LES ANGES DANS LA TRADITION");
    }

    #[test]
    fn resume_sans_numeros_de_page() {
        // Pages 6-8 : résumé fait de lignes courtes ; les titres reviennent dans le corps,
        // où ils deviennent les titres de niveau 1 (« 4 » s'écrit « QUATRE » dans le corps).
        let g = |page, texte: &str| para(page, texte, 15.0, true, 1);
        let mut blocs = vec![
            corps(5, &texte_long()),
            g(6, "RÉSUMÉ"),
            g(6, "CHAPITRE 1:"),
            g(7, "LES 4 ÉLÉMENTS"),
            g(7, "CHAPITRE 2:"),
            g(7, "LA GENÈSE"),
            g(8, "CHAPITRE 3:"),
            g(8, "LUCIFER ET"),
            g(8, "LE CHRIST"),
            corps(12, &texte_long()),
        ];
        for (page, titre) in [(19, "LES QUATRE ÉLÉMENTS"), (25, "LA GENÈSE"), (30, "LUCIFER ET LE CHRIST")] {
            blocs.push(g(page, titre));
            blocs.push(g(page, &format!("({page} FÉVRIER 1995)")));
            blocs.push(corps(page, &texte_long()));
        }
        let TitresDetectes { blocs, points, source, .. } = detecter_titres(blocs, &[], 40, SourceTitres::Auto);
        assert_eq!(source, SourceTitres::GrasMajuscules);
        let sommaires: Vec<u32> =
            points.iter().filter(|p| p.genre == GenrePoint::Sommaire).map(|p| p.ancre.page).collect();
        assert_eq!(sommaires, [6, 7, 8]);
        let n1: Vec<(String, u32)> = blocs
            .iter()
            .filter_map(|b| match b {
                Bloc::Titre { niveau: 1, texte, ancre } => Some((texte.clone(), ancre.page)),
                _ => None,
            })
            .collect();
        let attendu = [("LES QUATRE ÉLÉMENTS", 19), ("LA GENÈSE", 25), ("LUCIFER ET LE CHRIST", 30)];
        assert_eq!(n1, attendu.map(|(t, p)| (t.to_string(), p)));
        assert!(titres(&blocs).iter().filter(|(_, x)| x.contains("FÉVRIER")).all(|(n, _)| *n == 2));
        assert!(blocs.iter().all(|b| b.ancre().page > 8 || !matches!(b, Bloc::Titre { .. })));
    }

    /// Garde-fou de coût (relecture finale, I2) : des pages de lignes courtes (poème,
    /// liste) sont candidates au résumé sans numéros, et chacune de leurs lignes est
    /// cherchée dans la suite du livre. Ce cas prenait 10 à 108 s (en --release) ;
    /// il doit rester linéaire (environ 0,1 s en --release, 1 s en debug).
    #[test]
    fn resume_sans_numeros_en_temps_lineaire() {
        for (courtes, prose) in [(5, 760), (60, 700), (400, 400)] {
            let mut blocs = Vec::new();
            for page in 1..=courtes {
                for i in 0..20 {
                    blocs.push(para(page, &format!("Vers {i} de la page {page} du poème"), 12.0, false, 1));
                }
            }
            for page in courtes + 1..=courtes + prose {
                for _ in 0..8 {
                    blocs.push(corps(page, &texte_long()));
                }
            }
            let debut = std::time::Instant::now();
            let _ = detecter_titres(blocs, &[], courtes + prose, SourceTitres::Auto);
            let duree = debut.elapsed();
            assert!(duree.as_secs_f32() < 5.0, "{courtes} pages courtes + {prose} : {duree:?}");
        }
    }

    #[test]
    fn normaliser_retire_diacritiques_et_ponctuation() {
        assert_eq!(normaliser("  L’Âme,  d'été! "), "l ame d ete");
    }

    #[test]
    fn titre_de_sommaire_identique_dans_le_moteur_et_l_epub() {
        for (texte, attendu) in [
            ("Table des matières", true),
            ("TABLE DES MATIÈRES", true),
            ("Table des Matieres", true),
            ("Sommaire", true),
            ("SOMMAIRE", true),
            ("Sommaire :", true),
            ("Table", false),
            ("Sommaires et index", false),
        ] {
            assert_eq!(est_titre_de_sommaire(texte), attendu, "moteur : « {texte} »");
            assert_eq!(plumeo_epub::est_titre_de_sommaire(texte), attendu, "EPUB : « {texte} »");
        }
    }

    /// Chaque caractère latin étendu (U+00C0–U+024F, U+1E00–U+1EFF) mis à la place de
    /// chaque lettre de « table des matieres » et de « sommaire » : même décision des deux côtés.
    #[test]
    fn titre_de_sommaire_identique_pour_tout_caractere_latin_substitue() {
        let mut ecarts = Vec::new();
        for base in ["table des matieres", "sommaire"] {
            let lettres: Vec<char> = base.chars().collect();
            let latins = ('\u{c0}'..='\u{24f}').chain('\u{1e00}'..='\u{1eff}');
            for c in latins {
                for i in (0..lettres.len()).filter(|&i| lettres[i] != ' ') {
                    let mut l = lettres.clone();
                    l[i] = c;
                    let texte: String = l.into_iter().collect();
                    if est_titre_de_sommaire(&texte) != plumeo_epub::est_titre_de_sommaire(&texte) {
                        ecarts.push(texte);
                    }
                }
            }
        }
        assert!(ecarts.is_empty(), "{} écarts, dont {:?}", ecarts.len(), &ecarts[..ecarts.len().min(10)]);
    }

    #[test]
    fn taille_corps_pondere_par_caracteres() {
        let blocs = vec![para(1, "court", 18.0, false, 1), corps(1, &texte_long()), para(1, "x", 12.2, false, 1)];
        assert_eq!(taille_corps(&blocs), 12.0);
    }

    #[test]
    fn titre_plausible_refuse_les_lignes_illisibles() {
        for faux in [
            "Omraam Mikhaël AiYanhov SPIRITUEI.1LE~ALCHIM[IE",
            "~uvres complètes - Tome 2 EDITIONS PROS VET A",
            "Omraam Mikhaël Aïvanhov ~ALCHIMIE SPIRITUELLE",
            "? SUPERIEURE",
            "Corps m\\--",
            "Corpsastral -- ~Q .,> .. ~--- -- --- --- Cœur",
            "np / /",
            "s",
            "\\-- ---- ----glandelacrymale",
            "1U LI11î ISBN 2-85566-331·8",
            "Titre avec -- tirets",
            "Mot LI11î mêlé",
            "ab",
        ] {
            assert!(!titre_plausible(faux), "devrait être refusé : {faux}");
        }
    }

    #[test]
    fn titre_plausible_accepte_les_vrais_titres() {
        for vrai in [
            "Douceur et humilité",
            "(Jésus entre les deux larrons)",
            "Si vous ne mourez . pas, vous ne vivrez pas",
            "Echanges vivants et conscients",
            "L’économe infidèle",
            "L'économe infidèle",
            "(personnalité et individualité)",
            "Amassez des trésors",
            "Le miracle des deux poissons et des cinq pains",
            "Les pieds et le plexus solaire",
            "La parabole de l’ivraie et du froment",
            "L’alchimie spirituelle",
            "La galvanoplastie spirituelle",
            "Les pouvoirs de la mère pendant la gestation",
            "EDITIONS PROSVETA",
            "« Le Verbe » et la parole",
            "2e partie",
            "1er mars et XIIe siècle",
            "1ère leçon",
            "3. MELKHITSÉDEK",
            "* SEPHIRA KÉTHER *",
            "(2 FÉVRIER 1995)",
            "1er mars 1914.",
        ] {
            assert!(titre_plausible(vrai), "devrait être accepté : {vrai}");
        }
    }

    #[test]
    fn titre_peu_plausible_reste_paragraphe_avec_un_point() {
        let blocs = vec![
            corps(1, &texte_long()),
            para(2, "Douceur et humilité", 18.0, false, 1),
            corps(2, &texte_long()),
            para(3, "Corps m\\--", 18.0, false, 1),
            corps(3, &texte_long()),
        ];
        let TitresDetectes { blocs, points, illisibles, .. } = detecter_titres(blocs, &[], 3, SourceTitres::Taille);
        assert_eq!(illisibles, HashSet::from([ancre(3)]));
        assert_eq!(titres(&blocs), vec![(1, "Douceur et humilité".to_string())]);
        assert!(blocs.iter().any(|b| matches!(b, Bloc::Paragraphe(p) if p.texte_brut() == "Corps m\\--")));
        let suspects: Vec<_> = points
            .iter()
            .filter(|p| p.genre == GenrePoint::TitreSuspect && p.ancre.page == 3)
            .collect();
        assert_eq!(suspects.len(), 1);
        assert_eq!(suspects[0].message, "texte peu plausible pour un titre (OCR, schéma, code) : non retenu");
        assert!(points.iter().all(|p| p.ancre.page != 2));
    }

    #[test]
    fn titre_avec_appel_de_note_en_exposant_reste_plausible() {
        let avec_exposants = |segments: Vec<Segment>| {
            Bloc::Paragraphe(Paragraphe {
                segments,
                ancre: ancre(2),
                taille: 18.0,
                tout_gras: false,
                nb_lignes: 1,
            })
        };
        let texte = |t: &str| Segment::Texte { texte: t.into(), gras: false, italique: false };
        let blocs = vec![
            corps(1, &texte_long()),
            avec_exposants(vec![texte("Introduction"), Segment::Exposant("1".into())]),
            corps(2, &texte_long()),
            avec_exposants(vec![texte("2"), Segment::Exposant("e".into()), texte(" partie")]),
            corps(3, &texte_long()),
        ];
        let TitresDetectes { blocs, points, .. } = detecter_titres(blocs, &[], 3, SourceTitres::Taille);
        assert_eq!(titres(&blocs).len(), 2);
        assert!(points.iter().all(|p| p.genre != GenrePoint::TitreSuspect));
    }

    fn para_en(page: u32, y: u16, texte: &str, gras: bool) -> Bloc {
        Bloc::Paragraphe(Paragraphe {
            segments: vec![Segment::Texte { texte: texte.into(), gras, italique: false }],
            ancre: Ancre { page, moitie: Moitie::Entiere, rect: [100, y, 900, y + 30] },
            taille: 14.0,
            tout_gras: gras,
            nb_lignes: 1,
        })
    }

    fn texte_de(bloc: &Bloc) -> String {
        match bloc {
            Bloc::Paragraphe(p) => p.texte_brut(),
            Bloc::Titre { niveau, texte, .. } => format!("{} {texte}", "#".repeat(*niveau as usize)),
            Bloc::Note { marqueur, .. } => format!("[{marqueur}]"),
            Bloc::Saut { .. } => "* * *".into(),
            Bloc::EntreeSommaire { cible, .. } => format!("→ {cible}"),
        }
    }

    #[test]
    fn titre_du_livre_retrouve_sur_deux_paragraphes() {
        let blocs = vec![
            para_en(1, 500, "Éditions Prosveta", false),
            Bloc::Saut { ancre: Ancre { page: 2, moitie: Moitie::Entiere, rect: [100, 100, 900, 130] } },
            para_en(2, 100, "Omraam Mikhaël Aïvanhov", true),
            para_en(2, 300, "Approche de", true),
            para_en(2, 340, "la Cité céleste", true),
            para_en(2, 600, "Collection Izvor", false),
            para_en(3, 100, "Approche de la Cité céleste", false),
            Bloc::Titre { niveau: 1, texte: "Chapitre premier".into(), ancre: ancre(5) },
            corps(5, &texte_long()),
        ];
        let blocs = placer_titre_livre(blocs, "Approche de la Cité céleste");
        let textes: Vec<String> = blocs.iter().map(texte_de).collect();
        assert_eq!(
            textes,
            [
                "Éditions Prosveta",
                "# Approche de la Cité céleste",
                "* * *",
                "Omraam Mikhaël Aïvanhov",
                "Collection Izvor",
                "Approche de la Cité céleste",
                "# Chapitre premier",
                texte_long().as_str(),
            ]
        );
        assert_eq!(blocs[1].ancre(), Ancre { page: 2, moitie: Moitie::Entiere, rect: [100, 300, 900, 330] });
    }

    #[test]
    fn titre_du_livre_introuvable_insere_en_tete() {
        let blocs = vec![
            para_en(1, 300, "L’ALCH1MIE SP1R1TUELLE", true),
            Bloc::Titre { niveau: 1, texte: "Chapitre premier".into(), ancre: ancre(3) },
            // Après le premier titre de niveau 1 : hors de la fenêtre de recherche.
            para_en(3, 300, "L’alchimie spirituelle", true),
        ];
        let blocs = placer_titre_livre(blocs, "L’alchimie spirituelle");
        assert_eq!(
            blocs.iter().map(texte_de).collect::<Vec<_>>(),
            ["# L’alchimie spirituelle", "L’ALCH1MIE SP1R1TUELLE", "# Chapitre premier", "L’alchimie spirituelle"]
        );
        assert_eq!(blocs[0].ancre(), Ancre { page: 1, moitie: Moitie::Entiere, rect: [0, 0, 1000, 1000] });
    }

    #[test]
    fn titre_du_livre_cherche_dans_les_dix_premieres_pages() {
        let blocs = vec![corps(1, &texte_long()), para_en(11, 300, "Mon livre", true), corps(11, &texte_long())];
        let blocs = placer_titre_livre(blocs, "Mon livre");
        assert_eq!(texte_de(&blocs[0]), "# Mon livre");
        assert_eq!(blocs[0].ancre().page, 1);
        assert_eq!(texte_de(&blocs[2]), "Mon livre");
    }

    #[test]
    fn titre_du_livre_jamais_pris_dans_un_paragraphe_plus_long() {
        let blocs = vec![para_en(2, 300, "Mon livre et autres textes", true), corps(2, &texte_long())];
        let blocs = placer_titre_livre(blocs, "Mon livre");
        assert_eq!(texte_de(&blocs[0]), "# Mon livre");
        assert_eq!(blocs[0].ancre().page, 1);
        assert_eq!(texte_de(&blocs[1]), "Mon livre et autres textes");
    }

    #[test]
    fn titre_du_livre_vide_ou_deja_premier_titre() {
        let blocs = vec![para_en(1, 300, "Mon livre", true), corps(1, &texte_long())];
        assert_eq!(placer_titre_livre(blocs.clone(), " ").len(), 2);
        let deja = vec![
            Bloc::Titre { niveau: 1, texte: "MON LIVRE".into(), ancre: ancre(1) },
            corps(1, &texte_long()),
        ];
        assert_eq!(placer_titre_livre(deja, "Mon livre").iter().map(texte_de).collect::<Vec<_>>()[0], "# MON LIVRE");
    }

    #[test]
    fn titre_du_livre_deja_detecte_remonte_en_tete_de_sa_demi_page() {
        let en = |moitie: Moitie, y: u16| Ancre { page: 1, moitie, rect: [100, y, 900, y + 30] };
        let para_moitie = |moitie: Moitie, y: u16, texte: &str| {
            let mut p = para_en(1, y, texte, true);
            if let Bloc::Paragraphe(q) = &mut p {
                q.ancre = en(moitie, y);
            }
            p
        };
        let blocs = vec![
            para_moitie(Moitie::Gauche, 100, "Rudolf Steiner"),
            Bloc::Titre { niveau: 1, texte: "MON LIVRE".into(), ancre: en(Moitie::Gauche, 300) },
            Bloc::Saut { ancre: en(Moitie::Droite, 100) },
            para_moitie(Moitie::Droite, 100, "Rudolf STEINER"),
            corps(2, &texte_long()),
        ];
        let blocs = placer_titre_livre(blocs, "Mon livre");
        assert_eq!(
            blocs.iter().map(texte_de).collect::<Vec<_>>(),
            ["# MON LIVRE", "Rudolf Steiner", "* * *", "Rudolf STEINER", texte_long().as_str()]
        );
        assert_eq!(blocs[0].ancre(), en(Moitie::Gauche, 300));
    }

    #[test]
    fn titre_du_livre_remonte_laisse_un_saut_entre_deux_demi_pages() {
        // Sans saut (ligne de titre pleine largeur : page gauche non courte), les
        // paragraphes de part et d'autre du titre retiré ne doivent pas se raccorder.
        let en = |moitie: Moitie, y: u16| Ancre { page: 1, moitie, rect: [100, y, 900, y + 30] };
        let para_moitie = |moitie: Moitie, y: u16, texte: &str| {
            let mut p = para_en(1, y, texte, true);
            if let Bloc::Paragraphe(q) = &mut p {
                q.ancre = en(moitie, y);
            }
            p
        };
        let blocs = vec![
            para_moitie(Moitie::Gauche, 100, "Rudolf Steiner"),
            Bloc::Titre { niveau: 1, texte: "MON LIVRE".into(), ancre: en(Moitie::Gauche, 300) },
            para_moitie(Moitie::Droite, 100, "Rudolf STEINER"),
            para_moitie(Moitie::Droite, 300, "Un texte."),
        ];
        let blocs = placer_titre_livre(blocs, "Mon livre");
        assert_eq!(
            blocs.iter().map(texte_de).collect::<Vec<_>>(),
            ["# MON LIVRE", "Rudolf Steiner", "* * *", "Rudolf STEINER", "Un texte."]
        );
        assert_eq!(blocs[2].ancre(), en(Moitie::Droite, 100));
        // Même (demi-)page de part et d'autre : pas de saut.
        let blocs = vec![
            para_en(2, 100, "Omraam Mikhaël Aïvanhov", true),
            Bloc::Titre { niveau: 1, texte: "Mon livre".into(), ancre: ancre(2) },
            para_en(2, 600, "Collection Izvor", false),
        ];
        let blocs = placer_titre_livre(blocs, "Mon livre");
        assert_eq!(
            blocs.iter().map(texte_de).collect::<Vec<_>>(),
            ["# Mon livre", "Omraam Mikhaël Aïvanhov", "Collection Izvor"]
        );
        // Déjà en tête de sa page : le titre ne bouge pas et sépare toujours les pages.
        let blocs = vec![
            para_en(1, 100, "Jean Dupont", true),
            Bloc::Titre { niveau: 1, texte: "Mon livre".into(), ancre: ancre(2) },
            para_en(2, 600, "Collection Izvor", false),
        ];
        let attendu: Vec<String> = blocs.iter().map(texte_de).collect();
        let blocs = placer_titre_livre(blocs, "Mon livre");
        assert_eq!(blocs.iter().map(texte_de).collect::<Vec<_>>(), attendu);
    }

    #[test]
    fn chapitre_homonyme_du_livre_hors_fenetre_ne_remonte_pas() {
        let blocs = vec![
            corps(1, &texte_long()),
            para_en(12, 100, "Préface", false),
            Bloc::Titre { niveau: 1, texte: "Mon livre".into(), ancre: ancre(12) },
            corps(12, &texte_long()),
        ];
        let attendu: Vec<String> = blocs.iter().map(texte_de).collect();
        let blocs = placer_titre_livre(blocs, "Mon livre");
        assert_eq!(blocs.iter().map(texte_de).collect::<Vec<_>>(), attendu);
    }

    fn titre_en(niveau: u8, page: u32, texte: &str) -> Bloc {
        Bloc::Titre { niveau, texte: texte.into(), ancre: ancre(page) }
    }

    /// Texte et cible des blocs : `[texte](#cible)` pour une entrée de sommaire.
    fn rendu_sommaire(blocs: &[Bloc]) -> Vec<String> {
        blocs
            .iter()
            .map(|b| match b {
                Bloc::EntreeSommaire { segments, cible, .. } => {
                    let texte: String = segments
                        .iter()
                        .map(|s| match s {
                            Segment::Texte { texte, .. } | Segment::Exposant(texte) => texte.as_str(),
                            Segment::AppelNote { marqueur, .. } => marqueur.as_str(),
                        })
                        .collect();
                    format!("[{texte}](#{cible})")
                }
                autre => texte_de(autre),
            })
            .collect()
    }

    fn livre_avec_sommaire() -> Vec<Bloc> {
        vec![
            titre_en(1, 1, "Mon livre"),
            para_en(2, 100, "Table des matières", true),
            para_en(2, 200, "1. Premier chapitre ........ 5", true),
            para_en(2, 240, "2. DEUXIÈME CHAPITRE 9", true),
            para_en(2, 280, "Un mot de l’éditeur", false),
            para_en(2, 320, "3. Le troisième", true),
            para_en(3, 100, "I. La pierre", false),
            titre_en(1, 5, "1 Premier chapitre"),
            corps(5, &texte_long()),
            titre_en(1, 9, "2 Deuxième chapitre"),
            corps(9, &texte_long()),
            titre_en(1, 12, "3 Le troisième"),
            titre_en(2, 12, "I. La pierre"),
            corps(12, &texte_long()),
        ]
    }

    #[test]
    fn sommaire_imprime_rendu_cliquable() {
        let (blocs, points, _) = sommaire_cliquable(livre_avec_sommaire(), "Mon livre");
        assert_eq!(
            rendu_sommaire(&blocs)[..8],
            [
                "# Mon livre",
                "# Table des matières",
                "[1. Premier chapitre ........ 5](#t-1-premier-chapitre)",
                "[2. DEUXIÈME CHAPITRE 9](#t-2-deuxième-chapitre)",
                "Un mot de l’éditeur",
                "[3. Le troisième](#t-3-le-troisième)",
                // Page suivante dont tous les paragraphes sont des entrées.
                "[I. La pierre](#t-i-la-pierre)",
                "# 1 Premier chapitre",
            ]
        );
        assert_eq!(blocs[1].ancre(), Ancre { page: 2, moitie: Moitie::Entiere, rect: [100, 100, 900, 130] });
        let pages: Vec<u32> =
            points.iter().filter(|p| p.genre == GenrePoint::Sommaire).map(|p| p.ancre.page).collect();
        assert_eq!(pages, [2, 3]);
    }

    #[test]
    fn sommaire_imprime_trop_peu_d_entrees() {
        // Deux entrées seulement.
        let mut blocs = livre_avec_sommaire();
        blocs.remove(5);
        let (blocs, points, _) = sommaire_cliquable(blocs, "Mon livre");
        assert!(blocs.iter().all(|b| !matches!(b, Bloc::EntreeSommaire { .. })));
        assert!(rendu_sommaire(&blocs).contains(&"Table des matières".to_string()));
        assert!(points.is_empty());
        // Trois entrées, mais moins de la moitié des paragraphes de la page.
        let mut blocs = livre_avec_sommaire();
        for i in 0..4 {
            blocs.insert(2, para_en(2, 150 + i, "Une ligne qui ne renvoie à aucun titre", false));
        }
        let (blocs, _, _) = sommaire_cliquable(blocs, "Mon livre");
        assert!(blocs.iter().all(|b| !matches!(b, Bloc::EntreeSommaire { .. })));
    }

    #[test]
    fn faux_titre_d_une_seule_entree_n_est_pas_un_sommaire() {
        // Trois paragraphes, tous couverts, mais par une seule entrée.
        let blocs = vec![
            para_en(2, 100, "LES BASES", false),
            para_en(2, 140, "DE LA", false),
            para_en(2, 180, "SUITE", false),
            titre_en(1, 5, "LES BASES DE LA SUITE"),
            corps(5, &texte_long()),
        ];
        let (blocs, points, _) = sommaire_cliquable(blocs, "");
        assert!(blocs.iter().all(|b| !matches!(b, Bloc::EntreeSommaire { .. })));
        assert!(points.is_empty());
    }

    #[test]
    fn page_suivante_du_sommaire_entierement_faite_d_entrees() {
        let mut blocs = livre_avec_sommaire();
        blocs.insert(7, para_en(3, 300, "Achevé d’imprimer", false));
        let (blocs, points, _) = sommaire_cliquable(blocs, "Mon livre");
        let rendu = rendu_sommaire(&blocs);
        assert_eq!(rendu[6..8], ["I. La pierre", "Achevé d’imprimer"]);
        assert_eq!(points.len(), 1);
    }

    #[test]
    fn cible_du_sommaire_titre_d_une_autre_page_jamais_le_titre_du_livre() {
        let blocs = vec![
            titre_en(1, 1, "Mon livre"),
            titre_en(1, 1, "Introduction"),
            corps(1, &texte_long()),
            para_en(2, 100, "Sommaire", false),
            para_en(2, 200, "Introduction", false),
            para_en(2, 240, "Conclusion 12", false),
            para_en(2, 280, "Mon livre", false),
            para_en(2, 320, "Annexe", false),
            titre_en(2, 2, "Annexe"),
            titre_en(1, 4, "Introduction"),
            corps(4, &texte_long()),
            titre_en(1, 8, "Conclusion"),
            titre_en(1, 9, "Annexe"),
            titre_en(1, 12, "Conclusion"),
        ];
        let (blocs, _, _) = sommaire_cliquable(blocs, "Mon livre");
        assert_eq!(
            rendu_sommaire(&blocs)[3..8],
            [
                "# Sommaire",
                // Premier titre après la page du sommaire ; identifiants uniques dans tout le livre.
                "[Introduction](#t-introduction-2)",
                "[Conclusion 12](#t-conclusion)",
                // Le titre du livre n'est jamais une cible.
                "Mon livre",
                // Le titre de la même page n'est pas une cible.
                "[Annexe](#t-annexe-2)",
            ]
        );
    }

    #[test]
    fn cible_du_sommaire_avant_la_page_a_defaut() {
        let blocs = vec![
            titre_en(1, 1, "Un"),
            titre_en(1, 2, "Deux"),
            titre_en(1, 3, "Trois"),
            para_en(9, 100, "Un", false),
            para_en(9, 140, "Deux", false),
            para_en(9, 180, "Trois", false),
        ];
        let (blocs, _, _) = sommaire_cliquable(blocs, "");
        assert_eq!(rendu_sommaire(&blocs)[3..], ["[Un](#t-un)", "[Deux](#t-deux)", "[Trois](#t-trois)"]);
    }

    #[test]
    fn entree_avant_une_suite_de_sommaire_n_en_devient_pas_le_titre() {
        // La dernière entrée de la page 3 s'appelle « Table des matières » ; la page 4
        // est vide, la page 5 reprend une suite d'entrées : rien ne doit paniquer.
        let blocs = vec![
            para_en(3, 100, "Un", false),
            para_en(3, 140, "Deux", false),
            para_en(3, 180, "Trois", false),
            para_en(3, 220, "Table des matières", false),
            para_en(5, 100, "Quatre", false),
            para_en(5, 140, "Cinq", false),
            para_en(5, 180, "Six", false),
            titre_en(1, 6, "Un"),
            titre_en(1, 7, "Deux"),
            titre_en(1, 8, "Trois"),
            titre_en(1, 9, "Quatre"),
            titre_en(1, 10, "Cinq"),
            titre_en(1, 11, "Six"),
            titre_en(1, 20, "Table des matières"),
        ];
        let (blocs, _, _) = sommaire_cliquable(blocs, "");
        assert_eq!(
            rendu_sommaire(&blocs)[..7],
            [
                "[Un](#t-un)",
                "[Deux](#t-deux)",
                "[Trois](#t-trois)",
                "[Table des matières](#t-table-des-matières)",
                "[Quatre](#t-quatre)",
                "[Cinq](#t-cinq)",
                "[Six](#t-six)",
            ]
        );
    }

    #[test]
    fn titre_du_sommaire_seul_sur_sa_page_avant_un_saut() {
        let mut blocs = livre_avec_sommaire();
        // « Table des matières » seule sur la page 1 (page courte), saut, puis les entrées.
        if let Bloc::Paragraphe(p) = &mut blocs[1] {
            p.ancre.page = 1;
        }
        blocs.insert(2, Bloc::Saut { ancre: ancre(2) });
        let (blocs, _, _) = sommaire_cliquable(blocs, "Mon livre");
        assert_eq!(rendu_sommaire(&blocs)[1..4], ["# Table des matières", "* * *", "[1. Premier chapitre ........ 5](#t-1-premier-chapitre)"]);
    }

    /// Titres du corps visés par `SOMMAIRE_DE_CONFERENCES` : (page, texte).
    const CONFERENCES: [(u32, &str); 5] = [
        (5, "II. LES BASES DE LA SUITE (II)"),
        (8, "VII. JUGEMENTS ANALYTIQUES ET JUGEMENTS SYNTHÉTIQUES (Logique II)"),
        (11, "VIII. L'ÉLABORATION DES CONCEPTS ET LE SYSTÈME DES CATÉGORIES CHEZ HEGEL"),
        (14, "IX. LA CULTURE PRATIQUE DE LA PENSEE"),
        (17, "X. LA CULTURE PRATIQUE DE LA PENSÉE"),
    ];

    /// Sommaire de la page 2, à la manière de Connaissance : une ligne de conférence
    /// numérotée, puis une ou deux lignes de titre sans numéro de page.
    const SOMMAIRE_DE_CONFERENCES: [&str; 13] = [
        "Table des matières",
        "Deuxième conférence, Berlin 5",
        "Les bases de la suite (II)",
        "Septième conférence, Berlin 8",
        "Jugements analytiques et jugements synthétiques",
        "(Logique II)",
        "Huitième conférence, Berlin 11",
        "L'élaboration des concepts et le système des caté-",
        "gories chez Hegel.",
        "Neuvième conférence, Dornach 14",
        "La culture pratique de la pensée.",
        "Dixième conférence, Dornach 17",
        "La culture pratique de la pensée.",
    ];

    /// Livre : titre, sommaire de la page 2 (une ligne tous les 40 millièmes), titres
    /// `CONFERENCES` suivis de texte courant.
    fn livre_a_conferences(sommaire: &[&str]) -> Vec<Bloc> {
        let mut blocs = vec![titre_en(1, 1, "Mon livre")];
        for (k, texte) in sommaire.iter().enumerate() {
            blocs.push(para_en(2, 100 + 40 * k as u16, texte, false));
        }
        for (page, texte) in CONFERENCES {
            blocs.push(titre_en(1, page, texte));
            blocs.push(corps(page, &texte_long()));
        }
        blocs
    }

    fn lien(texte: &str, titre: &str) -> String {
        format!("[{texte}](#{})", crate::markdown::ancre_titre(titre))
    }

    #[test]
    fn entree_vise_le_titre_sans_son_numero_en_tete() {
        let (blocs, points, _) = sommaire_cliquable(livre_a_conferences(&SOMMAIRE_DE_CONFERENCES), "Mon livre");
        let rendu = rendu_sommaire(&blocs);
        assert_eq!(rendu[3], lien("Les bases de la suite (II)", CONFERENCES[0].1));
        assert_eq!(lien("Les bases de la suite (II)", CONFERENCES[0].1), "[Les bases de la suite (II)](#t-ii-les-bases-de-la-suite-ii)");
        // Les paragraphes couverts comptent pour reconnaître la page de sommaire.
        assert_eq!(points.iter().map(|p| p.ancre.page).collect::<Vec<_>>(), [2]);
    }

    #[test]
    fn entree_sur_deux_paragraphes_donne_une_seule_entree_liee() {
        let livre = livre_a_conferences(&SOMMAIRE_DE_CONFERENCES);
        let nb_blocs = livre.len();
        let (blocs, _, _) = sommaire_cliquable(livre, "Mon livre");
        let rendu = rendu_sommaire(&blocs);
        assert_eq!(rendu[4..6], [
            "Septième conférence, Berlin 8".to_string(),
            lien("Jugements analytiques et jugements synthétiques (Logique II)", CONFERENCES[1].1),
        ]);
        // Une seule entrée pour les deux paragraphes, ancrée au premier, rectangle étendu au second.
        assert_eq!(rendu[6], "Huitième conférence, Berlin 11");
        assert_eq!(blocs[5].ancre(), Ancre { page: 2, moitie: Moitie::Entiere, rect: [100, 260, 900, 330] });
        // Deux entrées de deux paragraphes : deux blocs de moins, aucun texte perdu.
        assert_eq!(blocs.len(), nb_blocs - 2);
    }

    #[test]
    fn entree_coupee_par_une_cesure_recollee() {
        let (blocs, _, _) = sommaire_cliquable(livre_a_conferences(&SOMMAIRE_DE_CONFERENCES), "Mon livre");
        assert_eq!(
            rendu_sommaire(&blocs)[7],
            lien("L'élaboration des concepts et le système des catégories chez Hegel.", CONFERENCES[2].1)
        );
    }

    #[test]
    fn entrees_homonymes_visent_des_titres_distincts_dans_l_ordre() {
        let (blocs, _, _) = sommaire_cliquable(livre_a_conferences(&SOMMAIRE_DE_CONFERENCES), "Mon livre");
        let rendu = rendu_sommaire(&blocs);
        assert_eq!(rendu[9], lien("La culture pratique de la pensée.", CONFERENCES[3].1));
        assert_eq!(rendu[11], lien("La culture pratique de la pensée.", CONFERENCES[4].1));
        assert_ne!(rendu[9], rendu[11]);
        // Plus d'entrées que de titres : une fois tous visés, la règle habituelle (le
        // premier après la page de l'entrée).
        let mut sommaire = SOMMAIRE_DE_CONFERENCES.to_vec();
        sommaire.push("La culture pratique de la pensée.");
        let (blocs, _, _) = sommaire_cliquable(livre_a_conferences(&sommaire), "Mon livre");
        assert_eq!(rendu_sommaire(&blocs)[12], lien("La culture pratique de la pensée.", CONFERENCES[3].1));
    }

    #[test]
    fn sommaire_compte_par_demi_page() {
        // Page 2 en double page : colophon à gauche (14 paragraphes), sommaire à droite.
        let mut livre = livre_a_conferences(&SOMMAIRE_DE_CONFERENCES);
        for bloc in &mut livre[1..=SOMMAIRE_DE_CONFERENCES.len()] {
            let Bloc::Paragraphe(p) = bloc else { panic!("paragraphe attendu") };
            p.ancre.moitie = Moitie::Droite;
        }
        for k in 0..14 {
            let texte = if k == 0 { "Les bases de la suite (II)".to_string() } else { format!("Ligne {k} du colophon") };
            livre.insert(1 + k, para_moitie(2, Moitie::Gauche, &texte, 1));
        }
        let (blocs, points, _) = sommaire_cliquable(livre, "Mon livre");
        let rendu = rendu_sommaire(&blocs);
        assert_eq!(points.iter().map(|p| p.ancre.page).collect::<Vec<_>>(), [2]);
        // La moitié gauche n'est pas un sommaire : son paragraphe reste un paragraphe.
        assert_eq!(rendu[1], "Les bases de la suite (II)");
        assert_eq!(rendu[15], "Table des matières");
        assert_eq!(rendu[17], lien("Les bases de la suite (II)", CONFERENCES[0].1));
    }

    #[test]
    fn entree_plus_courte_preferee_si_seule_a_viser_un_titre_apres_le_sommaire() {
        // Le titre sur deux lignes du sommaire est aussi celui de la page de faux-titre
        // (avant le sommaire), et ses deux lignes sont deux parties du corps : chacune
        // vise sa partie, comme le veut la règle « après la page, à défaut avant ».
        let blocs = vec![
            titre_en(1, 1, "Mon livre"),
            titre_en(1, 6, "LE SENS DE L’AMOUR DANS LE MONDE MORALE ANTHROPOSOPHIQUE"),
            para_en(7, 100, "SOMMAIRE", false),
            para_en(7, 140, "LE SENS DE L’AMOUR DANS LE MONDE", false),
            para_en(7, 180, "MORALE ANTHROPOSOPHIQUE", false),
            para_en(7, 220, "Première conférence.", false),
            titre_en(1, 10, "LE SENS DE L’AMOUR DANS LE MONDE"),
            titre_en(1, 15, "MORALE ANTHROPOSOPHIQUE"),
            titre_en(2, 15, "PREMIÈRE CONFÉRENCE"),
        ];
        let (blocs, _, _) = sommaire_cliquable(blocs, "Mon livre");
        assert_eq!(
            rendu_sommaire(&blocs)[2..6],
            [
                "# SOMMAIRE",
                "[LE SENS DE L’AMOUR DANS LE MONDE](#t-le-sens-de-l-amour-dans-le-monde)",
                "[MORALE ANTHROPOSOPHIQUE](#t-morale-anthroposophique)",
                "[Première conférence.](#t-première-conférence)",
            ]
        );
        // Sans titre après le sommaire, l'entrée de deux paragraphes, seule à viser un
        // titre, est retenue (étendues en concurrence : test suivant).
        let blocs = vec![
            titre_en(1, 6, "LE SENS DE L’AMOUR DANS LE MONDE MORALE ANTHROPOSOPHIQUE"),
            titre_en(1, 7, "MORALE ANTHROPOSOPHIQUE"),
            titre_en(1, 8, "Première conférence."),
            titre_en(1, 9, "Deuxième conférence."),
            para_en(20, 100, "LE SENS DE L’AMOUR DANS LE MONDE", false),
            para_en(20, 140, "MORALE ANTHROPOSOPHIQUE", false),
            para_en(20, 180, "Première conférence.", false),
            para_en(20, 220, "Deuxième conférence.", false),
        ];
        let (blocs, _, _) = sommaire_cliquable(blocs, "");
        assert_eq!(
            rendu_sommaire(&blocs)[4],
            "[LE SENS DE L’AMOUR DANS LE MONDE MORALE ANTHROPOSOPHIQUE](#t-le-sens-de-l-amour-dans-le-monde-morale-anthroposophique)"
        );
    }

    /// Livre dont la page `page_sommaire` porte les entrées « Un », « Deux », « Trois »,
    /// « Quatre », et dont les titres sont `titres_du_corps` : (page, texte).
    fn sommaire_un_deux_trois(page_sommaire: u32, titres_du_corps: &[(u32, &str)]) -> Vec<String> {
        let mut blocs: Vec<Bloc> = titres_du_corps.iter().map(|&(page, texte)| titre_en(1, page, texte)).collect();
        for (k, texte) in ["Un", "Deux", "Trois", "Quatre"].into_iter().enumerate() {
            blocs.push(para_en(page_sommaire, 100 + 40 * k as u16, texte, false));
        }
        let nb_titres = titres_du_corps.len();
        let (blocs, _, _) = sommaire_cliquable(blocs, "");
        rendu_sommaire(&blocs)[nb_titres..].to_vec()
    }

    #[test]
    fn deux_etendues_visent_un_titre_apres_le_sommaire_la_plus_longue_l_emporte() {
        let rendu = sommaire_un_deux_trois(2, &[(5, "Un"), (6, "Un Deux"), (7, "Trois"), (8, "Quatre")]);
        assert_eq!(rendu, ["[Un Deux](#t-un-deux)", "[Trois](#t-trois)", "[Quatre](#t-quatre)"]);
    }

    #[test]
    fn deux_etendues_visent_un_titre_avant_le_sommaire_la_plus_longue_l_emporte() {
        // Aucun titre après le sommaire : « à défaut, la plus longue qui en vise un ».
        let rendu = sommaire_un_deux_trois(20, &[(5, "Un"), (6, "Un Deux"), (7, "Trois"), (8, "Quatre")]);
        assert_eq!(rendu, ["[Un Deux](#t-un-deux)", "[Trois](#t-trois)", "[Quatre](#t-quatre)"]);
    }

    #[test]
    fn etendue_qui_vise_apres_le_sommaire_l_emporte_sur_une_plus_longue_qui_vise_avant() {
        // Trois étendues : la plus longue vise un titre avant le sommaire (faux-titre), les
        // deux autres un titre après ; la plus longue de ces deux-là l'emporte.
        let rendu = sommaire_un_deux_trois(
            7,
            &[(6, "Un Deux Trois"), (10, "Un Deux"), (11, "Un"), (12, "Trois"), (13, "Quatre")],
        );
        assert_eq!(rendu, ["[Un Deux](#t-un-deux)", "[Trois](#t-trois)", "[Quatre](#t-quatre)"]);
    }

    #[test]
    fn entree_jamais_a_cheval_sur_deux_pages() {
        let mut livre = livre_a_conferences(&SOMMAIRE_DE_CONFERENCES);
        // « (Logique II) » passe en tête de la page 3 : l'entrée ne la rejoint pas.
        let Bloc::Paragraphe(p) = &mut livre[6] else { panic!("paragraphe attendu") };
        assert_eq!(p.texte_brut(), "(Logique II)");
        p.ancre.page = 3;
        let (blocs, _, _) = sommaire_cliquable(livre, "Mon livre");
        let rendu = rendu_sommaire(&blocs);
        assert_eq!(rendu[5..7], ["Jugements analytiques et jugements synthétiques", "(Logique II)"]);
    }

    /// Paragraphe d'une ligne dans la moitié `moitie` de la page.
    fn para_moitie(page: u32, moitie: Moitie, texte: &str, lignes: usize) -> Bloc {
        let Bloc::Paragraphe(mut p) = para(page, texte, 12.0, false, lignes) else { unreachable!() };
        p.ancre.moitie = moitie;
        Bloc::Paragraphe(p)
    }

    /// Page 3 d'un livre de 80 pages : moitié gauche de texte courant, moitié droite de
    /// 10 lignes courtes dont 4 finissent par les `numeros` donnés.
    fn demi_page_de_sommaire(numeros: [u32; 4]) -> Vec<Bloc> {
        let mut blocs: Vec<Bloc> = (0..3).map(|_| para_moitie(3, Moitie::Gauche, &texte_long(), 5)).collect();
        blocs.push(para_moitie(3, Moitie::Droite, "CONNAISSANCE", 1));
        for (i, n) in numeros.iter().enumerate() {
            blocs.push(para_moitie(3, Moitie::Droite, &format!("Conférence {}, Berlin, 1903 {n}", i + 1), 1));
            blocs.push(para_moitie(3, Moitie::Droite, "Les bases d'une théorie de la connaissance", 1));
        }
        blocs.push(para_moitie(3, Moitie::Droite, "LOGIQUE", 1));
        blocs.push(para_moitie(80, Moitie::Gauche, &texte_long(), 5));
        blocs
    }

    #[test]
    fn demi_page_de_lignes_courtes_aux_numeros_croissants() {
        assert_eq!(pages_sommaire(&demi_page_de_sommaire([7, 23, 40, 58])), BTreeSet::from([3]));
        // Égalités admises : deux entrées sur la même page.
        assert_eq!(pages_sommaire(&demi_page_de_sommaire([7, 23, 23, 58])), BTreeSet::from([3]));
        // Numéros décroissants : pas un sommaire.
        assert!(pages_sommaire(&demi_page_de_sommaire([58, 40, 23, 7])).is_empty());
        // Un paragraphe long dans la moitié : pas un sommaire.
        let mut blocs = demi_page_de_sommaire([7, 23, 40, 58]);
        blocs.push(para_moitie(3, Moitie::Droite, &texte_long(), 5));
        assert!(pages_sommaire(&blocs).is_empty());
    }

    #[test]
    fn annees_croissantes_ne_font_pas_un_sommaire() {
        // Chronologie : lignes courtes finies par des années croissantes, dans un livre de
        // 80 pages (numéros plausibles jusqu'à 2 × 80 + 20 = 180).
        assert!(pages_sommaire(&demi_page_de_sommaire([1903, 1905, 1908, 1910])).is_empty());
        // Borne incluse : 180 est plausible, 181 ne l'est plus.
        assert_eq!(pages_sommaire(&demi_page_de_sommaire([7, 23, 40, 180])), BTreeSet::from([3]));
        assert!(pages_sommaire(&demi_page_de_sommaire([7, 23, 40, 181])).is_empty());
    }

    #[test]
    fn numero_l_07_lu_107_apres_des_points_de_conduite() {
        for fin in ["l 07", "I07", "l07", "I 07"] {
            let texte = format!("Le miracle des deux poissons . . . . {fin}");
            assert_eq!(numero_de_page_final(&texte), Some(107), "{texte}");
            let texte = format!("Le miracle des deux poissons \u{2026} {fin}");
            assert_eq!(numero_de_page_final(&texte), Some(107), "{texte}");
        }
        // Sans points de conduite, « l07 » n'est pas un numéro.
        assert_eq!(numero_de_page_final("Le miracle l07"), None);
        assert_eq!(numero_de_page_final("Douceur . . . . 11"), Some(11));
        assert_eq!(numero_de_page_final("Notes 175"), Some(175));
    }

    #[test]
    fn critere_de_la_moitie_compte_par_demi_page() {
        // 4 entrées sur 5 paragraphes à droite, mais 10 paragraphes de texte à gauche.
        let mut blocs: Vec<Bloc> = (0..10).map(|_| para_moitie(3, Moitie::Gauche, "Une ligne de texte courant.", 1)).collect();
        blocs.push(para_moitie(3, Moitie::Droite, &texte_long(), 5));
        for n in [9, 5, 30, 12] {
            blocs.push(para_moitie(3, Moitie::Droite, &format!("{} ...... {n}", texte_long()), 4));
        }
        assert_eq!(pages_sommaire(&blocs), BTreeSet::from([3]));
    }

    #[test]
    fn numero_l_07_retire_de_l_entree() {
        let mut blocs: Vec<Bloc> = ["Douceur", "Les pieds", "Le miracle des deux poissons"]
            .iter()
            .enumerate()
            .map(|(i, t)| titre_en(1, 10 + i as u32, t))
            .collect();
        blocs.push(para_en(2, 100, "Douceur . . . . . 11", false));
        blocs.push(para_en(2, 140, "Le miracle des deux poissons . . . . l 07", false));
        blocs.push(para_en(2, 180, "Les pieds ..... I07", false));
        let (blocs, _, _) = sommaire_cliquable(blocs, "");
        assert_eq!(
            rendu_sommaire(&blocs)[3..],
            [
                "[Douceur . . . . . 11](#t-douceur)",
                "[Le miracle des deux poissons . . . . l 07](#t-le-miracle-des-deux-poissons)",
                "[Les pieds ..... I07](#t-les-pieds)",
            ]
        );
    }

    #[test]
    fn entree_avec_appel_de_note_reste_un_paragraphe() {
        let mut blocs = vec![
            titre_en(1, 3, "Un"),
            titre_en(1, 4, "Deux"),
            titre_en(1, 5, "Trois"),
            titre_en(1, 6, "Quatre"),
            para_en(2, 100, "Un", false),
            para_en(2, 140, "Deux", false),
            para_en(2, 180, "Trois", false),
            para_en(2, 220, "Quatre", false),
        ];
        // Un lien ne peut pas contenir l'appel (lien imbriqué) : le texte reste tel quel.
        if let Bloc::Paragraphe(p) = &mut blocs[7] {
            p.segments.push(Segment::AppelNote { etiquette: "1-*".into(), marqueur: "*".into() });
        }
        let (blocs, _, _) = sommaire_cliquable(blocs, "");
        assert_eq!(rendu_sommaire(&blocs)[4..], ["[Un](#t-un)", "[Deux](#t-deux)", "[Trois](#t-trois)", "Quatre*"]);
    }

    #[test]
    fn entree_arretee_avant_un_paragraphe_a_appel_de_note() {
        // Sans l'appel, « Alpha » + « beta 6 » viserait « ALPHA BETA » (la plus longue) ;
        // avec l'appel sur le 2e paragraphe, l'entrée s'arrête au 1er.
        let mut blocs = vec![
            para_en(2, 100, "Alpha", false),
            para_en(2, 140, "beta 6", false),
            para_en(2, 180, "Gamma 7", false),
            para_en(2, 220, "Delta 8", false),
            titre_en(1, 5, "ALPHA"),
            titre_en(1, 6, "ALPHA BETA"),
            titre_en(1, 7, "Gamma"),
            titre_en(1, 8, "Delta"),
        ];
        if let Bloc::Paragraphe(p) = &mut blocs[1] {
            p.segments.push(Segment::AppelNote { etiquette: "1-*".into(), marqueur: "*".into() });
        }
        let (blocs, _, _) = sommaire_cliquable(blocs, "");
        assert_eq!(
            rendu_sommaire(&blocs)[..4],
            [lien("Alpha", "ALPHA"), "beta 6*".to_string(), lien("Gamma 7", "Gamma"), lien("Delta 8", "Delta")]
        );
        // L'appel reste hors du lien, dans son paragraphe.
        assert!(matches!(
            &blocs[1],
            Bloc::Paragraphe(p) if p.segments.iter().any(|s| matches!(s, Segment::AppelNote { .. }))
        ));
    }

    #[test]
    fn table_des_matieres_titre_quand_la_premiere_entree_couvre_plusieurs_paragraphes() {
        let sommaire = [
            "Table des matières",
            "Jugements analytiques et jugements synthétiques",
            "(Logique II)",
            "Les bases de la suite (II)",
            "La culture pratique de la pensée.",
            "La culture pratique de la pensée.",
        ];
        let (blocs, points, _) = sommaire_cliquable(livre_a_conferences(&sommaire), "Mon livre");
        let rendu = rendu_sommaire(&blocs);
        assert_eq!(
            rendu[1..4],
            [
                "# Table des matières".to_string(),
                lien("Jugements analytiques et jugements synthétiques (Logique II)", CONFERENCES[1].1),
                lien("Les bases de la suite (II)", CONFERENCES[0].1),
            ]
        );
        assert_eq!(points.iter().map(|p| p.ancre.page).collect::<Vec<_>>(), [2]);
    }

    /// Rendu de l'entrée qui vise `titre`, dans `livre_a_conferences`, quand les
    /// paragraphes du sommaire `premier` et `second` (retrouvés par leur texte) reçoivent
    /// les segments `fin` et `debut`.
    fn entree_jointe(premier: &str, fin: Vec<Segment>, second: &str, debut: Vec<Segment>, titre: &str) -> String {
        let mut livre = livre_a_conferences(&SOMMAIRE_DE_CONFERENCES);
        for (texte, segments) in [(premier, fin), (second, debut)] {
            let Some(Bloc::Paragraphe(p)) =
                livre.iter_mut().find(|b| matches!(&**b, Bloc::Paragraphe(p) if p.texte_brut() == texte))
            else {
                panic!("paragraphe « {texte} » attendu")
            };
            p.segments = segments;
        }
        let (blocs, _, _) = sommaire_cliquable(livre, "Mon livre");
        let cible = format!("](#{})", crate::markdown::ancre_titre(titre));
        rendu_sommaire(&blocs).into_iter().find(|r| r.ends_with(&cible)).expect("entrée liée")
    }

    fn segment(texte: &str) -> Segment {
        Segment::Texte { texte: texte.into(), gras: false, italique: false }
    }

    #[test]
    fn cesure_recollee_malgre_les_espaces_de_bord() {
        let attendu = lien("L'élaboration des concepts et le système des catégories chez Hegel.", CONFERENCES[2].1);
        let avant = "L'élaboration des concepts et le système des caté-";
        let apres = "gories chez Hegel.";
        let cas: [(Vec<Segment>, Vec<Segment>); 4] = [
            // Espace finale dans le dernier segment.
            (vec![segment(&format!("{avant} "))], vec![segment(apres)]),
            // Dernier segment fait seulement d'espaces.
            (vec![segment(avant), segment("  ")], vec![segment(apres)]),
            // Espace en tête du paragraphe suivant.
            (vec![segment(avant)], vec![segment(&format!(" {apres}"))]),
            // Premier segment du paragraphe suivant fait seulement d'espaces.
            (vec![segment(avant), segment(" ")], vec![segment(" "), segment(apres)]),
        ];
        let rendus: Vec<String> =
            cas.into_iter().map(|(fin, debut)| entree_jointe(avant, fin, apres, debut, CONFERENCES[2].1)).collect();
        assert_eq!(rendus, vec![attendu; 4]);
    }

    #[test]
    fn jonction_sans_cesure_une_seule_espace_malgre_les_espaces_de_bord() {
        let attendu = lien("Jugements analytiques et jugements synthétiques (Logique II)", CONFERENCES[1].1);
        let avant = "Jugements analytiques et jugements synthétiques";
        let apres = "(Logique II)";
        let cas: [(Vec<Segment>, Vec<Segment>); 2] = [
            // Espaces de bord dans les segments de texte.
            (vec![segment(&format!("{avant}  "))], vec![segment(&format!(" {apres}"))]),
            // Segments faits seulement d'espaces, des deux côtés.
            (vec![segment(avant), segment("  ")], vec![segment(" "), segment(apres)]),
        ];
        let rendus: Vec<String> =
            cas.into_iter().map(|(fin, debut)| entree_jointe(avant, fin, apres, debut, CONFERENCES[1].1)).collect();
        assert_eq!(rendus, vec![attendu; 2]);
    }

    #[test]
    fn homonymes_d_un_sommaire_coupe_par_une_page_vide_visent_dans_l_ordre() {
        // Sommaire en p3 et p5 ; la p4 ne porte aucun paragraphe : c'est un seul sommaire,
        // et ses deux « La pensée » visent IX puis X.
        let mut blocs = vec![titre_en(1, 1, "Mon livre")];
        let sommaires = [(3, ["Alpha 10", "Beta 11", "La pensée 12"]), (5, ["Gamma 13", "Delta 14", "La pensée 20"])];
        for (page, entrees) in sommaires {
            for (k, texte) in entrees.into_iter().enumerate() {
                blocs.push(para_en(page, 100 + 40 * k as u16, texte, false));
            }
        }
        for (page, titre) in
            [(10, "Alpha"), (11, "Beta"), (12, "IX. La pensée"), (13, "Gamma"), (14, "Delta"), (20, "X. La pensée")]
        {
            blocs.push(titre_en(1, page, titre));
            blocs.push(corps(page, &texte_long()));
        }
        let (blocs, _, demi_pages) = sommaire_cliquable(blocs, "Mon livre");
        assert_eq!(demi_pages, BTreeSet::from([(3, Moitie::Entiere), (5, Moitie::Entiere)]));
        let rendu = rendu_sommaire(&blocs);
        assert_eq!(rendu[3], lien("La pensée 12", "IX. La pensée"));
        assert_eq!(rendu[6], lien("La pensée 20", "X. La pensée"));
    }

    #[test]
    fn initiales_et_romains_mal_formes_ne_sont_pas_des_numeros() {
        let blocs = vec![
            para_en(2, 100, "G. Jung 5", false),
            para_en(2, 140, "La suite 6", false),
            para_en(2, 180, "Mal formé 7", false),
            para_en(2, 220, "Alpha 8", false),
            para_en(2, 260, "Beta 9", false),
            para_en(2, 300, "Dupont 10", false),
            titre_en(1, 5, "C. G. JUNG"),
            titre_en(1, 6, "V. LA SUITE"),
            titre_en(1, 7, "IIII. MAL FORMÉ"),
            titre_en(1, 8, "XIV. ALPHA"),
            titre_en(1, 9, "Beta"),
            titre_en(1, 10, "M. DUPONT"),
        ];
        let (blocs, _, _) = sommaire_cliquable(blocs, "");
        assert_eq!(
            rendu_sommaire(&blocs)[..6],
            [
                // « C. » suivi d'une autre initiale : pas un numéro.
                "G. Jung 5".to_string(),
                lien("La suite 6", "V. LA SUITE"),
                // « IIII » n'est pas un romain bien formé.
                "Mal formé 7".to_string(),
                lien("Alpha 8", "XIV. ALPHA"),
                lien("Beta 9", "Beta"),
                // Lettre seule autre que I, V ou X : pas un numéro.
                "Dupont 10".to_string(),
            ]
        );
    }

    #[test]
    fn homonymes_vises_dans_l_ordre_dans_chaque_sommaire() {
        // Le même sommaire en tête (p2) et en fin de livre (p40) : chacun vise IX puis X.
        let sommaire = ["Alpha 5", "Beta 6", "La pensée 10", "La pensée 20"];
        let mut blocs = vec![titre_en(1, 1, "Mon livre")];
        let entrees =
            |page: u32| sommaire.iter().enumerate().map(move |(k, t)| para_en(page, 100 + 40 * k as u16, t, false));
        blocs.extend(entrees(2));
        for (page, titre) in [(5, "Alpha"), (6, "Beta"), (10, "IX. La pensée"), (20, "X. La pensée")] {
            blocs.push(titre_en(1, page, titre));
            blocs.push(corps(page, &texte_long()));
        }
        blocs.extend(entrees(40));
        let (blocs, _, moities) = sommaire_cliquable(blocs, "Mon livre");
        assert_eq!(moities, BTreeSet::from([(2, Moitie::Entiere), (40, Moitie::Entiere)]));
        let liens: Vec<String> = rendu_sommaire(&blocs).into_iter().filter(|r| r.starts_with('[')).collect();
        let attendu = [
            lien("Alpha 5", "Alpha"),
            lien("Beta 6", "Beta"),
            lien("La pensée 10", "IX. La pensée"),
            lien("La pensée 20", "X. La pensée"),
        ];
        assert_eq!(liens[..4], attendu);
        assert_eq!(liens[4..], attendu);
    }

    #[test]
    fn titre_vise_sans_son_numero_seulement_si_le_numero_porte_un_point() {
        let blocs = vec![
            para_en(2, 100, "Était une fois 5", false),
            para_en(2, 140, "Conférences 6", false),
            para_en(2, 180, "Les bases 7", false),
            para_en(2, 220, "La suite 8", false),
            para_en(2, 260, "La fin 9", false),
            para_en(2, 300, "Sans point 10", false),
            titre_en(1, 5, "IL ÉTAIT UNE FOIS"),
            titre_en(1, 6, "DIX CONFÉRENCES"),
            titre_en(1, 7, "I. LES BASES"),
            titre_en(1, 8, "IV. LA SUITE"),
            titre_en(1, 9, "3. LA FIN"),
            titre_en(1, 10, "V SANS POINT"),
        ];
        let (blocs, _, _) = sommaire_cliquable(blocs, "");
        assert_eq!(
            rendu_sommaire(&blocs)[..6],
            [
                // « IL » et « DIX » ne sont pas des numéros suivis d'un point.
                "Était une fois 5".to_string(),
                "Conférences 6".to_string(),
                lien("Les bases 7", "I. LES BASES"),
                lien("La suite 8", "IV. LA SUITE"),
                lien("La fin 9", "3. LA FIN"),
                // Numéro sans point : le titre n'est pas retrouvé sans lui.
                "Sans point 10".to_string(),
            ]
        );
    }

    /// Ligne de la taille du texte (12 pt), non grasse, à la hauteur `y`.
    fn ligne_en(page: u32, y: u16, texte: &str) -> Bloc {
        Bloc::Paragraphe(Paragraphe {
            segments: vec![Segment::Texte { texte: texte.into(), gras: false, italique: false }],
            ancre: Ancre { page, moitie: Moitie::Entiere, rect: [100, y, 900, y + 30] },
            taille: 12.0,
            tout_gras: false,
            nb_lignes: 1,
        })
    }

    /// Livre de 20 pages sans signets : le sommaire imprimé (`sommaire`, une ligne par
    /// paragraphe) en page `page_sommaire` ; sur les autres pages, les lignes de titre
    /// données (numéro puis lignes du titre, de la taille du texte), puis du texte courant.
    fn livre_a_sommaire(page_sommaire: u32, sommaire: &[&str], titres_du_corps: &[(u32, &[&str])]) -> Vec<Bloc> {
        livre_a_sommaires(&[(page_sommaire, sommaire)], titres_du_corps)
    }

    /// `livre_a_sommaire`, avec un sommaire réparti sur plusieurs pages (page, lignes).
    fn livre_a_sommaires(sommaires: &[(u32, &[&str])], titres_du_corps: &[(u32, &[&str])]) -> Vec<Bloc> {
        let mut blocs = Vec::new();
        for page in 1..=20 {
            if let Some((_, sommaire)) = sommaires.iter().find(|(p, _)| *p == page) {
                blocs.extend(sommaire.iter().enumerate().map(|(k, t)| ligne_en(page, 100 + 30 * k as u16, t)));
                continue;
            }
            for (p, lignes) in titres_du_corps {
                if *p == page {
                    blocs.extend(lignes.iter().enumerate().map(|(k, t)| ligne_en(page, 100 + 40 * k as u16, t)));
                }
            }
            blocs.push(corps(page, &texte_long()));
        }
        blocs
    }

    const SOMMAIRE_IMPRIME: [&str; 11] = [
        "TABLE DES MATIÈRES",
        "Première conférence, Berlin 7",
        "Les bases de la suite (I)",
        "Deuxième conférence, Berlin 10",
        "Les bases de la suite (II)",
        "Troisième conférence, Munich 13",
        "Jugements analytiques",
        "(Logique II)",
        "Quatrième conférence, Munich 16",
        "Le système des caté-",
        "gories chez Hegel.",
    ];

    const TITRES_DU_CORPS: [(u32, &[&str]); 4] = [
        (7, &["I", "LES BASES", "DE LA SUITE (I)"]),
        (10, &["II", "LES BASES", "DE LA SUITE (II)"]),
        (13, &["III.", "JUGEMENTS ANALYTIQUES", "(LOGIQUE II)"]),
        (16, &["IV", "LE SYSTÈME DES CATÉGORIES CHEZ HEGEL"]),
    ];

    #[test]
    fn texte_d_une_entree_de_sommaire() {
        assert_eq!(texte_entree(&["Notes 175"]), "Notes");
        assert_eq!(texte_entree(&["Jugements analytiques", "(Logique II)"]), "Jugements analytiques (Logique II)");
        assert_eq!(
            texte_entree(&["L'élaboration des concepts et le système des caté-", "gories chez Hegel."]),
            "L'élaboration des concepts et le système des catégories chez Hegel."
        );
        // La suite commence par une majuscule : un vrai tiret, pas une césure.
        assert_eq!(texte_entree(&["Anti-", "Dühring 12"]), "Anti- Dühring");
        assert_eq!(texte_entree(&["Le miracle des deux poissons . . . . l 07"]), "Le miracle des deux poissons");
        assert_eq!(texte_entree(&["Première conférence, Berlin, 27 novembre 1903 7"]), "Première conférence, Berlin, 27 novembre 1903");
        // Une entrée s'étend sur 3 paragraphes au plus, et s'arrête après un paragraphe numéroté.
        assert_eq!(etendue_entree(&["Les bases (I)", "Deuxième conférence 23", "Les bases (II)"]), 2);
        assert_eq!(etendue_entree(&["Première conférence 7", "Les bases (I)"]), 1);
        assert_eq!(etendue_entree(&["Jugements", "(Logique II)", "Huitième conférence 114", "L'élaboration"]), 3);
        assert_eq!(etendue_entree(&["A", "B", "C", "D"]), 3);
        assert_eq!(etendue_entree(&["Notes"]), 1);
    }

    #[test]
    fn entrees_du_sommaire_imprime_donnent_les_titres_de_niveau_1() {
        let blocs = livre_a_sommaire(2, &SOMMAIRE_IMPRIME, &TITRES_DU_CORPS);
        let numero_i = blocs.iter().find(|b| texte_de(b) == "I").expect("numéro I").ancre();
        let nb_textes = blocs.len();
        let TitresDetectes { blocs, source, .. } = detecter_titres(blocs, &[], 20, SourceTitres::Auto);
        assert_eq!(source, SourceTitres::Taille);
        assert_eq!(
            titres(&blocs),
            [
                (1, "I. LES BASES DE LA SUITE (I)".to_string()),
                (1, "II. LES BASES DE LA SUITE (II)".to_string()),
                (1, "III. JUGEMENTS ANALYTIQUES (LOGIQUE II)".to_string()),
                (1, "IV. LE SYSTÈME DES CATÉGORIES CHEZ HEGEL".to_string()),
            ]
        );
        // Le titre est ancré au paragraphe du numéro, qui n'en reste pas un.
        assert_eq!(blocs.iter().find(|b| matches!(b, Bloc::Titre { .. })).unwrap().ancre(), numero_i);
        assert!(
            blocs.iter().all(|b| !matches!(b, Bloc::Paragraphe(p) if numero_seul(p.texte_brut().trim()).is_some()))
        );
        // Les paragraphes du sommaire restent tels quels ; les 11 lignes de titre du corps
        // font 4 titres.
        assert_eq!(blocs.len(), nb_textes - 11 + 4);
        assert!(blocs.iter().filter(|b| b.ancre().page == 2).all(|b| matches!(b, Bloc::Paragraphe(_))));
    }

    #[test]
    fn entree_sur_deux_paragraphes_retrouvee_sur_deux_paragraphes() {
        let blocs = livre_a_sommaire(2, &SOMMAIRE_IMPRIME, &TITRES_DU_CORPS);
        let TitresDetectes { blocs, .. } = detecter_titres(blocs, &[], 20, SourceTitres::Auto);
        let page_13: Vec<String> = blocs.iter().filter(|b| b.ancre().page == 13).map(texte_de).collect();
        assert_eq!(page_13, ["# III. JUGEMENTS ANALYTIQUES (LOGIQUE II)", texte_long().as_str()]);
    }

    #[test]
    fn cesure_du_sommaire_recollee() {
        let blocs = livre_a_sommaire(2, &SOMMAIRE_IMPRIME, &TITRES_DU_CORPS);
        let TitresDetectes { blocs, .. } = detecter_titres(blocs, &[], 20, SourceTitres::Auto);
        let page_16: Vec<String> = blocs.iter().filter(|b| b.ancre().page == 16).map(texte_de).collect();
        assert_eq!(page_16, ["# IV. LE SYSTÈME DES CATÉGORIES CHEZ HEGEL", texte_long().as_str()]);
    }

    #[test]
    fn deux_entrees_de_meme_texte_visent_deux_titres_dans_l_ordre() {
        let sommaire = [
            "Neuvième conférence, Berlin 7",
            "La culture pratique de la pensée.",
            "Dixième conférence, Nuremberg 10",
            "La culture pratique de la pensée.",
            "Onzième conférence 13",
            "Les bases de la suite (I)",
            "Notes 17",
        ];
        let corps_titres: [(u32, &[&str]); 4] = [
            (7, &["I", "LA CULTURE PRATIQUE DE LA PENSEE"]),
            (10, &["II", "LA CULTURE PRATIQUE DE LA PENSÉE"]),
            (13, &["III", "LES BASES DE LA SUITE (I)"]),
            (17, &["Notes"]),
        ];
        let TitresDetectes { blocs, .. } =
            detecter_titres(livre_a_sommaire(2, &sommaire, &corps_titres), &[], 20, SourceTitres::Auto);
        let n1: Vec<(String, u32)> = blocs
            .iter()
            .filter_map(|b| match b {
                Bloc::Titre { niveau: 1, texte, ancre } => Some((texte.clone(), ancre.page)),
                _ => None,
            })
            .collect();
        let attendu = [
            ("I. LA CULTURE PRATIQUE DE LA PENSEE", 7),
            ("II. LA CULTURE PRATIQUE DE LA PENSÉE", 10),
            ("III. LES BASES DE LA SUITE (I)", 13),
            ("Notes", 17),
        ];
        assert_eq!(n1, attendu.map(|(t, p)| (t.to_string(), p)));
    }

    #[test]
    fn sommaire_de_fin_cherche_depuis_la_page_1() {
        let corps_titres: [(u32, &[&str]); 4] = [
            (3, &["I", "LES BASES", "DE LA SUITE (I)"]),
            (5, &["II", "LES BASES", "DE LA SUITE (II)"]),
            (6, &["III", "JUGEMENTS ANALYTIQUES", "(LOGIQUE II)"]),
            (9, &["IV", "LE SYSTÈME DES CATÉGORIES CHEZ HEGEL"]),
        ];
        let TitresDetectes { blocs, .. } =
            detecter_titres(livre_a_sommaire(20, &SOMMAIRE_IMPRIME, &corps_titres), &[], 20, SourceTitres::Auto);
        let pages: Vec<(u8, u32)> = blocs
            .iter()
            .filter_map(|b| match b {
                Bloc::Titre { niveau, ancre, .. } => Some((*niveau, ancre.page)),
                _ => None,
            })
            .collect();
        assert_eq!(pages, [(1, 3), (1, 5), (1, 6), (1, 9)]);
    }

    #[test]
    fn sommaire_de_tete_cherche_apres_lui() {
        // Sommaire en page 4 : le titre de la page 3 (avant le sommaire) n'est pas visé,
        // celui de la page 8 l'est.
        let corps_titres: [(u32, &[&str]); 5] = [
            (3, &["LES BASES", "DE LA SUITE (I)"]),
            (8, &["I", "LES BASES", "DE LA SUITE (I)"]),
            (10, &["II", "LES BASES", "DE LA SUITE (II)"]),
            (13, &["III", "JUGEMENTS ANALYTIQUES", "(LOGIQUE II)"]),
            (16, &["IV", "LE SYSTÈME DES CATÉGORIES CHEZ HEGEL"]),
        ];
        let TitresDetectes { blocs, .. } =
            detecter_titres(livre_a_sommaire(4, &SOMMAIRE_IMPRIME, &corps_titres), &[], 20, SourceTitres::Auto);
        let premier = blocs.iter().find(|b| matches!(b, Bloc::Titre { .. })).unwrap();
        assert_eq!((texte_de(premier), premier.ancre().page), ("# I. LES BASES DE LA SUITE (I)".to_string(), 8));
        assert_eq!(blocs.iter().filter(|b| b.ancre().page == 3).count(), 3);
    }

    #[test]
    fn moins_de_trois_entrees_nouvelles_rien_ne_change() {
        // Deux entrées sont déjà des titres par la taille : il n'en reste que deux nouvelles.
        let grand = |page, texte: &str| para(page, texte, 18.0, false, 1);
        let mut blocs = livre_a_sommaire(2, &SOMMAIRE_IMPRIME, &TITRES_DU_CORPS);
        for b in &mut blocs {
            if b.ancre().page == 7 || b.ancre().page == 10 {
                let texte = texte_de(b);
                if texte.starts_with("LES BASES") || texte.starts_with("DE LA SUITE") {
                    *b = grand(b.ancre().page, &texte);
                }
            }
        }
        let TitresDetectes { blocs, source, .. } = detecter_titres(blocs, &[], 20, SourceTitres::Auto);
        assert_eq!(source, SourceTitres::Taille);
        assert_eq!(
            titres(&blocs),
            [(1, "LES BASES DE LA SUITE (I)".to_string()), (1, "LES BASES DE LA SUITE (II)".to_string())]
        );
        assert!(blocs.iter().any(|b| texte_de(b) == "I"));
        assert!(blocs.iter().any(|b| texte_de(b) == "JUGEMENTS ANALYTIQUES"));
    }

    #[test]
    fn entrees_nouvelles_moins_de_la_moitie_rien_ne_change() {
        // 7 entrées retrouvées, dont 4 déjà titres par la taille : 3 nouvelles, mais moins
        // de la moitié des entrées retrouvées.
        let sommaire = ["Alpha 3", "Beta 5", "Gamma 7", "Delta 9", "Epsilon 11", "Zeta 13", "Eta 15"];
        let corps_titres: [(u32, &[&str]); 7] = [
            (3, &["ALPHA"]),
            (5, &["BETA"]),
            (7, &["GAMMA"]),
            (9, &["DELTA"]),
            (11, &["EPSILON"]),
            (13, &["ZETA"]),
            (15, &["ETA"]),
        ];
        let mut blocs = livre_a_sommaire(2, &sommaire, &corps_titres);
        for b in &mut blocs {
            let texte = texte_de(b);
            if ["ALPHA", "BETA", "GAMMA", "DELTA"].contains(&texte.as_str()) {
                *b = para(b.ancre().page, &texte, 18.0, false, 1);
            }
        }
        let TitresDetectes { blocs, source, .. } = detecter_titres(blocs, &[], 20, SourceTitres::Auto);
        assert_eq!(source, SourceTitres::Taille);
        let t: Vec<String> = titres(&blocs).into_iter().map(|(_, x)| x).collect();
        assert_eq!(t, ["ALPHA", "BETA", "GAMMA", "DELTA"]);
        assert!(blocs.iter().any(|b| texte_de(b) == "EPSILON"));
    }

    #[test]
    fn partie_et_chapitre_sur_la_meme_page_restent_deux_titres() {
        // La partie « LOGIQUE » et le chapitre III ouvrent la même page : deux titres
        // issus du sommaire, jamais fusionnés.
        let mut sommaire = SOMMAIRE_IMPRIME.to_vec();
        sommaire.insert(5, "LOGIQUE");
        let corps_titres: [(u32, &[&str]); 4] = [
            TITRES_DU_CORPS[0],
            TITRES_DU_CORPS[1],
            (13, &["LOGIQUE", "III.", "JUGEMENTS ANALYTIQUES", "(LOGIQUE II)"]),
            TITRES_DU_CORPS[3],
        ];
        let TitresDetectes { blocs, .. } =
            detecter_titres(livre_a_sommaire(2, &sommaire, &corps_titres), &[], 20, SourceTitres::Auto);
        let page_13: Vec<String> = blocs.iter().filter(|b| b.ancre().page == 13).map(texte_de).collect();
        assert_eq!(page_13, ["# LOGIQUE", "# III. JUGEMENTS ANALYTIQUES (LOGIQUE II)", texte_long().as_str()]);
        // Le sommaire reste cliquable : ses 5 entrées sont liées.
        let (blocs, points, _) = sommaire_cliquable(blocs, "");
        assert_eq!(blocs.iter().filter(|b| matches!(b, Bloc::EntreeSommaire { .. })).count(), 5);
        assert_eq!(points.iter().map(|p| p.ancre.page).collect::<Vec<_>>(), [2]);
        let rendu = rendu_sommaire(&blocs);
        assert!(rendu.contains(&lien("LOGIQUE", "LOGIQUE")), "{rendu:?}");
    }

    #[test]
    fn titres_d_un_resume_par_chapitres_jamais_fusionnes() {
        // Deux entrées « CHAPITRE n » d'un résumé retrouvées sur la même page : deux titres.
        let blocs = vec![
            corps(1, &texte_long()),
            ligne_en(2, 100, "CHAPITRE 1: LES ANGES"),
            ligne_en(2, 140, "CHAPITRE 2: LA GENÈSE"),
            ligne_en(2, 180, "CHAPITRE 3: LE CIEL"),
            ligne_en(4, 100, "LES ANGES"),
            ligne_en(4, 140, "LA GENÈSE"),
            corps(4, &texte_long()),
            ligne_en(6, 100, "LE CIEL"),
            corps(6, &texte_long()),
        ];
        let TitresDetectes { blocs, .. } = detecter_titres(blocs, &[], 6, SourceTitres::Auto);
        let t: Vec<String> = titres(&blocs).into_iter().map(|(_, x)| x).collect();
        assert_eq!(t, ["LES ANGES", "LA GENÈSE", "LE CIEL"]);
    }

    #[test]
    fn numero_seul_du_sommaire_absorbe_par_l_entree_suivante() {
        // La ligne « III » seule n'est pas une entrée : l'entrée suivante retrouve le titre
        // et absorbe le numéro du corps, avec son point.
        let sommaire = [
            "Première conférence, Berlin 7",
            "Les bases de la suite (I)",
            "Deuxième conférence, Berlin 10",
            "Les bases de la suite (II)",
            "Troisième conférence, Dornach 13",
            "III",
            "La culture pratique de la pensée.",
            "Quatrième conférence, Dornach 16",
            "IV.",
            "Le système des caté-",
            "gories chez Hegel.",
        ];
        let corps_titres: [(u32, &[&str]); 4] = [
            TITRES_DU_CORPS[0],
            TITRES_DU_CORPS[1],
            (13, &["III", "LA CULTURE PRATIQUE DE LA PENSÉE"]),
            (16, &["IV", "LE SYSTÈME DES CATÉGORIES CHEZ HEGEL"]),
        ];
        let TitresDetectes { blocs, .. } =
            detecter_titres(livre_a_sommaire(2, &sommaire, &corps_titres), &[], 20, SourceTitres::Auto);
        let t: Vec<String> = titres(&blocs).into_iter().map(|(_, x)| x).collect();
        assert_eq!(
            t,
            [
                "I. LES BASES DE LA SUITE (I)",
                "II. LES BASES DE LA SUITE (II)",
                "III. LA CULTURE PRATIQUE DE LA PENSÉE",
                "IV. LE SYSTÈME DES CATÉGORIES CHEZ HEGEL",
            ]
        );
    }

    #[test]
    fn entree_du_sommaire_decoupee_comme_dans_le_sommaire_cliquable() {
        // Les deux tâches joignent les mêmes paragraphes : consécutifs, de la même
        // (demi-)page, sans appel de note.
        let livre = || {
            livre_a_sommaire(
                2,
                &["Troisième conférence 13", "Jugements analytiques", "(Logique II)"],
                &[(13, &["JUGEMENTS ANALYTIQUES (LOGIQUE II)"])],
            )
        };
        let moities = BTreeSet::from([(2, Moitie::Entiere), (2, Moitie::Droite), (3, Moitie::Entiere)]);
        let pages = moities.clone();
        // Entrées retrouvées (le texte courant de la page 3, présent sur toutes les pages,
        // se retrouve aussi : il est écarté).
        let retrouvees = |blocs: &[Bloc]| -> Vec<String> {
            entrees_du_sommaire(blocs, &moities, &pages)
                .into_iter()
                .map(|e| e.signet.titre)
                .filter(|t| *t != texte_long().trim())
                .collect()
        };
        // Témoin : sur la même page, l'entrée de deux paragraphes est retrouvée.
        assert_eq!(retrouvees(&livre()), ["Jugements analytiques (Logique II)"]);
        fn logique(blocs: &mut [Bloc]) -> &mut Paragraphe {
            let i = blocs.iter().position(|b| texte_de(b) == "(Logique II)").expect("(Logique II)");
            let Bloc::Paragraphe(p) = &mut blocs[i] else { unreachable!() };
            p
        }
        // « (Logique II) » en tête de la page suivante, dans l'autre moitié, ou avec un appel.
        let mut autre_page = livre();
        logique(&mut autre_page).ancre.page = 3;
        let mut autre_moitie = livre();
        logique(&mut autre_moitie).ancre.moitie = Moitie::Droite;
        let mut avec_appel = livre();
        logique(&mut avec_appel).segments.push(Segment::AppelNote { etiquette: "2-1".into(), marqueur: "1".into() });
        for blocs in [autre_page, autre_moitie, avec_appel] {
            assert!(retrouvees(&blocs).is_empty());
            let rendu = rendu_sommaire(&sommaire_cliquable(blocs, "").0);
            assert!(rendu.iter().all(|r| !r.contains("(Logique II)](")), "{rendu:?}");
        }
        // Paragraphes que peut couvrir une entrée, dans les deux tâches.
        assert_eq!(paragraphes_d_entree(&livre(), 1), ["Troisième conférence 13"]);
        assert_eq!(paragraphes_d_entree(&livre(), 2), ["Jugements analytiques", "(Logique II)"]);
    }

    #[test]
    fn titre_du_livre_de_niveau_2_dans_les_premieres_pages_devient_le_titre_du_livre() {
        let blocs = vec![
            para_en(1, 100, "Rudolf Steiner", true),
            titre_en(2, 1, "MON LIVRE"),
            corps(1, &texte_long()),
            titre_en(1, 4, "CONNAISSANCE"),
            corps(4, &texte_long()),
        ];
        let blocs = placer_titre_livre(blocs, "Mon livre");
        assert_eq!(
            blocs.iter().map(texte_de).collect::<Vec<_>>(),
            ["# MON LIVRE", "Rudolf Steiner", texte_long().as_str(), "# CONNAISSANCE", texte_long().as_str()]
        );
        // Après le premier titre de niveau 1, ou hors des premières pages : pas le titre trouvé.
        for blocs in [
            vec![titre_en(1, 4, "CONNAISSANCE"), titre_en(2, 5, "MON LIVRE"), corps(5, &texte_long())],
            vec![corps(1, &texte_long()), titre_en(2, 11, "MON LIVRE"), titre_en(1, 12, "CONNAISSANCE")],
        ] {
            let attendu: Vec<String> =
                ["# Mon livre".to_string()].into_iter().chain(blocs.iter().map(texte_de)).collect();
            let blocs = placer_titre_livre(blocs, "Mon livre");
            assert_eq!(blocs.iter().map(texte_de).collect::<Vec<_>>(), attendu);
        }
    }

    #[test]
    fn titre_du_livre_candidat_et_sommaire_imprime_sans_doublon() {
        let mut blocs = livre_a_sommaire(2, &SOMMAIRE_IMPRIME, &TITRES_DU_CORPS);
        blocs.insert(0, para(1, "MON LIVRE", 20.0, false, 1));
        let TitresDetectes { blocs, .. } = detecter_titres(blocs, &[], 20, SourceTitres::Auto);
        assert_eq!(titres(&blocs)[0], (2, "MON LIVRE".to_string()));
        let blocs = placer_titre_livre(blocs, "Mon livre");
        let t = titres(&blocs);
        assert_eq!(t[0], (1, "MON LIVRE".to_string()));
        assert_eq!(t.iter().filter(|(_, x)| normaliser(x) == "mon livre").count(), 1);
        assert_eq!(t[1], (1, "I. LES BASES DE LA SUITE (I)".to_string()));
    }

    #[test]
    fn moitie_gauche_d_une_page_de_sommaire_garde_ses_titres() {
        // Page 3 double : texte à gauche, sommaire à droite. Le titre candidat de la moitié
        // gauche reste un titre.
        let mut blocs = demi_page_de_sommaire([7, 23, 40, 58]);
        let Bloc::Paragraphe(mut avant_propos) = para(3, "AVANT-PROPOS", 12.0, true, 1) else { unreachable!() };
        avant_propos.ancre.moitie = Moitie::Gauche;
        blocs.insert(0, Bloc::Paragraphe(avant_propos));
        let TitresDetectes { blocs, points, sommaires: sommaires_detectes, .. } =
            detecter_titres(blocs, &[], 80, SourceTitres::GrasMajuscules);
        assert_eq!(titres(&blocs), [(1, "AVANT-PROPOS".to_string())]);
        assert_eq!(sommaires_detectes, BTreeSet::from([(3, Moitie::Droite)]));
        // Un seul point « sommaire », ancré à la moitié de sommaire.
        let sommaires: Vec<Ancre> =
            points.iter().filter(|p| p.genre == GenrePoint::Sommaire).map(|p| p.ancre).collect();
        assert_eq!(sommaires.iter().map(Ancre::demi_page).collect::<Vec<_>>(), [(3, Moitie::Droite)]);
    }

    /// Livre de 20 pages au sommaire imprimé (`SOMMAIRE_IMPRIME`) en p3g ; le premier titre
    /// du corps (« I / LES BASES / DE LA SUITE (I) ») en p3d, les autres plus loin.
    fn livre_au_sommaire_en_p3g() -> Vec<Bloc> {
        let titres_du_corps: Vec<(u32, &[&str])> = TITRES_DU_CORPS[1..].to_vec();
        let mut blocs = livre_a_sommaire(3, &SOMMAIRE_IMPRIME, &titres_du_corps);
        for bloc in blocs.iter_mut() {
            if let Bloc::Paragraphe(p) = bloc
                && p.ancre.page == 3
            {
                p.ancre.moitie = Moitie::Gauche;
            }
        }
        let apres = blocs.iter().rposition(|b| b.ancre().page == 3).expect("page 3") + 1;
        let droite = ["I", "LES BASES", "DE LA SUITE (I)"]
            .iter()
            .enumerate()
            .map(|(k, t)| ligne_en(3, 100 + 40 * k as u16, t))
            .chain([corps(3, &texte_long())])
            .map(|b| {
                let Bloc::Paragraphe(mut p) = b else { unreachable!() };
                p.ancre.moitie = Moitie::Droite;
                Bloc::Paragraphe(p)
            });
        blocs.splice(apres..apres, droite);
        blocs
    }

    #[test]
    fn recherche_apres_un_sommaire_fini_sur_une_moitie_gauche() {
        // Sommaire en p3g, premier titre du corps en p3d : la recherche commence en p3d.
        let TitresDetectes { blocs, .. } = detecter_titres(livre_au_sommaire_en_p3g(), &[], 20, SourceTitres::Auto);
        let premier = blocs.iter().find(|b| matches!(b, Bloc::Titre { .. })).expect("un titre");
        assert_eq!(texte_de(premier), "# I. LES BASES DE LA SUITE (I)");
        assert_eq!(premier.ancre().demi_page(), (3, Moitie::Droite));
        assert_eq!(titres(&blocs).len(), 4);
    }

    #[test]
    fn entree_d_un_sommaire_en_p3g_liee_au_titre_de_p3d() {
        // Même livre, puis le sommaire cliquable : la p3g est un sommaire, et son entrée
        // « Les bases de la suite (I) » vise le titre de la p3d (même page, autre moitié).
        let TitresDetectes { blocs, .. } = detecter_titres(livre_au_sommaire_en_p3g(), &[], 20, SourceTitres::Auto);
        let (blocs, _, moities) = sommaire_cliquable(blocs, "");
        assert_eq!(moities, BTreeSet::from([(3, Moitie::Gauche)]));
        let rendu = rendu_sommaire(&blocs);
        assert!(rendu.contains(&lien("Les bases de la suite (I)", "I. LES BASES DE LA SUITE (I)")), "{rendu:?}");
    }

    /// Chapitres « Un » à « Huit », aux pages 5, 7… 19, que visent les entrées « Un 5 »…
    const CHAPITRES: [&str; 8] = ["Un", "Deux", "Trois", "Quatre", "Cinq", "Six", "Sept", "Huit"];

    /// Livre au sommaire réparti sur les (demi-)pages données, chacune avec ses lignes,
    /// puis les `CHAPITRES` (titre et texte courant).
    fn livre_au_sommaire_reparti(moities: &[(u32, Moitie, &[&str])]) -> Vec<Bloc> {
        let mut blocs = vec![titre_en(1, 1, "Mon livre")];
        for (page, moitie, lignes) in moities {
            for (k, texte) in lignes.iter().enumerate() {
                let Bloc::Paragraphe(mut p) = para_en(*page, 100 + 40 * k as u16, texte, false) else { unreachable!() };
                p.ancre.moitie = *moitie;
                blocs.push(Bloc::Paragraphe(p));
            }
        }
        for (k, chapitre) in CHAPITRES.iter().enumerate() {
            let page = 5 + 2 * k as u32;
            blocs.push(titre_en(1, page, chapitre));
            blocs.push(corps(page, &texte_long()));
        }
        blocs
    }

    /// Livre au sommaire réparti sur la page 3 double : `gauche` en p3g, puis 4 entrées en
    /// p3d (« Trois 9 » à « Six 15 »).
    fn livre_au_sommaire_sur_deux_moities(gauche: &[&str]) -> Vec<Bloc> {
        livre_au_sommaire_reparti(&[
            (3, Moitie::Gauche, gauche),
            (3, Moitie::Droite, &["Trois 9", "Quatre 11", "Cinq 13", "Six 15"]),
        ])
    }

    #[test]
    fn enchainement_a_rebours_arrete_au_titre_du_sommaire() {
        // p2d : entrées seules ; p3g : titre et entrées ; p3d : entrées. La chaîne remonte
        // jusqu'à la p3g, qui porte le titre, et s'y arrête : la p2d n'est pas avalée.
        let livre = livre_au_sommaire_reparti(&[
            (2, Moitie::Droite, &["Un 5", "Deux 7"]),
            (3, Moitie::Gauche, &["Table des matières", "Trois 9", "Quatre 11"]),
            (3, Moitie::Droite, &["Cinq 13", "Six 15", "Sept 17", "Huit 19"]),
        ]);
        let (blocs, _, moities) = sommaire_cliquable(livre, "Mon livre");
        assert_eq!(moities, BTreeSet::from([(3, Moitie::Gauche), (3, Moitie::Droite)]));
        let rendu = rendu_sommaire(&blocs);
        assert_eq!(rendu[1..3], ["Un 5", "Deux 7"]);
        assert_eq!(rendu[3], "# Table des matières");
        assert_eq!(
            rendu[4..10],
            ["Trois 9", "Quatre 11", "Cinq 13", "Six 15", "Sept 17", "Huit 19"]
                .map(|t| lien(t, t.split(' ').next().unwrap()))
        );
        // Sans titre, la chaîne remonte d'entrées en entrées.
        let livre = livre_au_sommaire_reparti(&[
            (2, Moitie::Droite, &["Un 5", "Deux 7"]),
            (3, Moitie::Gauche, &["Trois 9", "Quatre 11"]),
            (3, Moitie::Droite, &["Cinq 13", "Six 15", "Sept 17", "Huit 19"]),
        ]);
        let (_, _, moities) = sommaire_cliquable(livre, "Mon livre");
        assert_eq!(moities, BTreeSet::from([(2, Moitie::Droite), (3, Moitie::Gauche), (3, Moitie::Droite)]));
    }

    #[test]
    fn resume_sans_numeros_sur_une_page_double() {
        // Résumé sur les deux moitiés de la page 6 : il les couvre toutes les deux.
        let g = |moitie, texte: &str| {
            let Bloc::Paragraphe(mut p) = para(6, texte, 15.0, true, 1) else { unreachable!() };
            p.ancre.moitie = moitie;
            Bloc::Paragraphe(p)
        };
        let mut blocs = vec![
            corps(5, &texte_long()),
            g(Moitie::Gauche, "RÉSUMÉ"),
            g(Moitie::Gauche, "CHAPITRE 1:"),
            g(Moitie::Gauche, "LES ÉLÉMENTS"),
            g(Moitie::Droite, "CHAPITRE 2:"),
            g(Moitie::Droite, "LA GENÈSE"),
            g(Moitie::Droite, "CHAPITRE 3:"),
            g(Moitie::Droite, "LUCIFER ET LE CHRIST"),
        ];
        for (page, titre) in [(19, "LES ÉLÉMENTS"), (25, "LA GENÈSE"), (30, "LUCIFER ET LE CHRIST")] {
            blocs.push(para(page, titre, 15.0, true, 1));
            blocs.push(corps(page, &texte_long()));
        }
        let (pages, pseudo) = resume_sans_numeros(&blocs, &BTreeSet::new());
        assert_eq!(pages, BTreeSet::from([(6, Moitie::Gauche), (6, Moitie::Droite)]));
        assert_eq!(pseudo.iter().map(|s| s.page).collect::<Vec<_>>(), [19, 25, 30]);
        // Une moitié de sommaire sur la page l'exclut du résumé (critère par page).
        let (pages, _) = resume_sans_numeros(&blocs, &BTreeSet::from([(6, Moitie::Droite)]));
        assert!(pages.is_empty());
    }

    #[test]
    fn sommaire_commence_sur_la_moitie_qui_precede() {
        let livre = livre_au_sommaire_sur_deux_moities(&["Table des matières", "Un 5", "Deux 7"]);
        let (blocs, points, moities) = sommaire_cliquable(livre, "Mon livre");
        // Les deux moitiés sont des sommaires : l'ensemble les contient toutes les deux.
        assert_eq!(moities, BTreeSet::from([(3, Moitie::Gauche), (3, Moitie::Droite)]));
        let rendu = rendu_sommaire(&blocs);
        assert_eq!(rendu[1], "# Table des matières");
        assert_eq!(
            rendu[2..8],
            ["Un 5", "Deux 7", "Trois 9", "Quatre 11", "Cinq 13", "Six 15"]
                .map(|t| lien(t, t.split(' ').next().unwrap()))
        );
        // Un point par page, ancré à la première moitié de sommaire.
        assert_eq!(points.iter().map(|p| p.ancre.demi_page()).collect::<Vec<_>>(), [(3, Moitie::Gauche)]);
    }

    /// Chapitres du livre sans signets de `livre_sans_signets_au_sommaire_reparti` : titre
    /// (une ligne de la taille du texte) et page.
    const CHAPITRES_REPARTIS: [(&str, u32); 6] = [
        ("LES ORIGINES", 5),
        ("LA LUMIÈRE", 7),
        ("LE FEU", 9),
        ("LA TERRE", 11),
        ("LES EAUX", 13),
        ("LE CIEL", 15),
    ];

    /// Livre de 20 pages sans signets : le sommaire réparti sur les (demi-)pages données,
    /// chacune avec ses lignes, puis le corps (`CHAPITRES_REPARTIS` et texte courant, des
    /// pages 4 à 20).
    fn livre_sans_signets_au_sommaire_reparti(moities: &[(u32, Moitie, &[&str])]) -> Vec<Bloc> {
        let mut blocs = vec![corps(1, &texte_long())];
        for (page, moitie, lignes) in moities {
            blocs.extend(lignes.iter().map(|t| para_moitie(*page, *moitie, t, 1)));
        }
        for page in 4..=20 {
            if let Some((titre, _)) = CHAPITRES_REPARTIS.iter().find(|(_, p)| *p == page) {
                blocs.push(ligne_en(page, 100, titre));
            }
            blocs.push(corps(page, &texte_long()));
        }
        blocs
    }

    /// Les 4 dernières entrées (`CHAPITRES_REPARTIS`), pour la p3d.
    const ENTREES_EN_P3D: [&str; 4] = ["Le feu 9", "La terre 11", "Les eaux 13", "Le ciel 15"];

    #[test]
    fn sommaire_reparti_detecte_sur_la_moitie_qui_precede() {
        let detectees = |gauche: &[&str]| {
            moities_sommaire(&livre_sans_signets_au_sommaire_reparti(&[
                (3, Moitie::Gauche, gauche),
                (3, Moitie::Droite, &ENTREES_EN_P3D),
            ]))
        };
        let p3 = BTreeSet::from([(3, Moitie::Gauche), (3, Moitie::Droite)]);
        let p3d = BTreeSet::from([(3, Moitie::Droite)]);
        // Titre du sommaire et 2 entrées numérotées : la p3g en fait partie.
        assert_eq!(detectees(&["Table des matières", "Les origines 5", "La lumière 7"]), p3);
        // Une entrée numérotée et des lignes courtes suffisent, sans titre.
        assert_eq!(detectees(&["Les origines 5", "La lumière"]), p3);
        // Aucune ligne numérotée : pas de sommaire.
        assert_eq!(detectees(&["Table des matières", "Avant-propos"]), p3d);
        // Un paragraphe long : pas de sommaire.
        let long = texte_long();
        assert_eq!(detectees(&[long.as_str(), "Les origines 5", "La lumière 7"]), p3d);

        // La remontée s'arrête à la (demi-)page qui porte le titre du sommaire.
        let livre = |gauche: &'static [&'static str], droite: &'static [&'static str]| {
            livre_sans_signets_au_sommaire_reparti(&[
                (2, Moitie::Droite, &["Avant 2", "Avant 3"]),
                (3, Moitie::Gauche, gauche),
                (3, Moitie::Droite, droite),
            ])
        };
        let entrees: &[&str] = &ENTREES_EN_P3D;
        assert_eq!(moities_sommaire(&livre(&["Table des matières", "Les origines 5", "La lumière 7"], entrees)), p3);
        assert_eq!(
            moities_sommaire(&livre(&["Les origines 5", "La lumière 7"], entrees)),
            BTreeSet::from([(2, Moitie::Droite), (3, Moitie::Gauche), (3, Moitie::Droite)])
        );
        let titre_en_p3d: &[&str] = &["Table des matières", "Le feu 9", "La terre 11", "Les eaux 13", "Le ciel 15"];
        assert_eq!(moities_sommaire(&livre(&["Les origines 5", "La lumière 7"], titre_en_p3d)), p3d);
    }

    #[test]
    fn sommaire_reparti_sur_deux_moities_donne_tous_les_titres_et_tous_les_liens() {
        let livre = livre_sans_signets_au_sommaire_reparti(&[
            (3, Moitie::Gauche, &["Table des matières", "Les origines 5", "La lumière 7"]),
            (3, Moitie::Droite, &ENTREES_EN_P3D),
        ]);
        let TitresDetectes { blocs, source, .. } = detecter_titres(livre, &[], 20, SourceTitres::Auto);
        assert_eq!(source, SourceTitres::Taille);
        let attendus: Vec<(u8, String)> = CHAPITRES_REPARTIS.iter().map(|(t, _)| (1, t.to_string())).collect();
        assert_eq!(titres(&blocs), attendus);
        let (blocs, _, moities) = sommaire_cliquable(blocs, "");
        assert_eq!(moities, BTreeSet::from([(3, Moitie::Gauche), (3, Moitie::Droite)]));
        let rendu = rendu_sommaire(&blocs);
        let debut = rendu.iter().position(|t| t == "# Table des matières").expect("titre du sommaire");
        let entrees = ["Les origines 5", "La lumière 7"].into_iter().chain(ENTREES_EN_P3D);
        let liens: Vec<String> = entrees.zip(CHAPITRES_REPARTIS).map(|(e, (titre, _))| lien(e, titre)).collect();
        assert_eq!(rendu[debut + 1..debut + 7], liens[..]);
    }

    #[test]
    fn recherche_d_entree_reprend_a_la_demi_page_de_la_precedente() {
        // « Alpha » est retrouvé en p5d. « Beta » figure aussi en p5g, avant lui dans
        // l'ordre de lecture : la recherche de l'entrée suivante part de la p5d et le
        // retrouve en p6.
        let mut blocs = vec![corps(1, &texte_long())];
        blocs.extend(["Alpha 5", "Beta 6", "Gamma 7", "Delta 8"].iter().map(|t| ligne_en(2, 100, t)));
        blocs.extend([
            para_moitie(5, Moitie::Gauche, "Beta", 1),
            para_moitie(5, Moitie::Gauche, &texte_long(), 5),
            para_moitie(5, Moitie::Droite, "Alpha", 1),
            para_moitie(5, Moitie::Droite, &texte_long(), 5),
        ]);
        for (page, titre) in [(6, "Beta"), (7, "Gamma"), (8, "Delta")] {
            blocs.push(ligne_en(page, 100, titre));
            blocs.push(corps(page, &texte_long()));
        }
        blocs.push(corps(20, &texte_long()));
        let TitresDetectes { blocs, .. } = detecter_titres(blocs, &[], 20, SourceTitres::Auto);
        let lieux: Vec<(String, DemiPage)> = blocs
            .iter()
            .filter_map(|b| match b {
                Bloc::Titre { texte, ancre, .. } => Some((texte.clone(), ancre.demi_page())),
                _ => None,
            })
            .collect();
        assert_eq!(
            lieux,
            [
                ("Alpha".to_string(), (5, Moitie::Droite)),
                ("Beta".to_string(), (6, Moitie::Entiere)),
                ("Gamma".to_string(), (7, Moitie::Entiere)),
                ("Delta".to_string(), (8, Moitie::Entiere)),
            ]
        );
    }

    #[test]
    fn moitie_qui_precede_avec_du_texte_n_est_pas_un_sommaire() {
        let livre = livre_au_sommaire_sur_deux_moities(&["Imprimé en Suisse", "Un 5", "Deux 7"]);
        let (blocs, points, moities) = sommaire_cliquable(livre, "Mon livre");
        assert_eq!(moities, BTreeSet::from([(3, Moitie::Droite)]));
        let rendu = rendu_sommaire(&blocs);
        assert_eq!(rendu[1..4], ["Imprimé en Suisse", "Un 5", "Deux 7"]);
        assert_eq!(rendu[4], lien("Trois 9", "Trois"));
        assert_eq!(points.iter().map(|p| p.ancre.demi_page()).collect::<Vec<_>>(), [(3, Moitie::Droite)]);
    }

    #[test]
    fn numero_seul_bien_forme() {
        use Numerotation::{Arabe, Romaine};
        for (texte, valeur) in [
            ("I", (Romaine, 1)),
            ("IV", (Romaine, 4)),
            ("IX.", (Romaine, 9)),
            ("XIV", (Romaine, 14)),
            ("XL", (Romaine, 40)),
            ("MCMXC", (Romaine, 1990)),
            ("DIX", (Romaine, 509)),
            ("1", (Arabe, 1)),
            ("12.", (Arabe, 12)),
            ("999", (Arabe, 999)),
        ] {
            assert_eq!(numero_seul(texte), Some(valeur), "{texte}");
        }
        let mal_formes =
            ["", ".", "0", "0.", "07", "IIII", "IIX", "VX", "IL", "VV", "1984", "1984.", "I.I", "A", "iv", "12 3"];
        for texte in mal_formes {
            assert_eq!(numero_seul(texte), None, "{texte}");
        }
    }

    #[test]
    fn connaissance_de_i_a_x_tous_absorbes() {
        // Parties sans numéro intercalées entre I…III, IV…VIII et IX, X : elles ne cassent
        // pas la suite. Les lignes « IX » et « X » seules du sommaire ne sont pas des entrées.
        let sommaire = [
            "CONNAISSANCE 3",
            "Alpha 4",
            "Beta 5",
            "Gamma 6",
            "LOGIQUE 7",
            "Delta 8",
            "Epsilon 9",
            "Zeta 10",
            "Eta 11",
            "Theta 12",
            "PENSÉE PRATIQUE 13",
            "IX",
            "Iota 14",
            "X",
            "Kappa 15",
        ];
        let corps_titres: [(u32, &[&str]); 13] = [
            (3, &["CONNAISSANCE"]),
            (4, &["I", "ALPHA"]),
            (5, &["II", "BETA"]),
            (6, &["III.", "GAMMA"]),
            (7, &["LOGIQUE"]),
            (8, &["IV", "DELTA"]),
            (9, &["V", "EPSILON"]),
            (10, &["VI", "ZETA"]),
            (11, &["VII", "ETA"]),
            (12, &["VIII", "THETA"]),
            (13, &["PENSÉE PRATIQUE"]),
            (14, &["IX", "IOTA"]),
            (15, &["X", "KAPPA"]),
        ];
        let TitresDetectes { blocs, .. } =
            detecter_titres(livre_a_sommaire(2, &sommaire, &corps_titres), &[], 20, SourceTitres::Auto);
        let t: Vec<String> = titres(&blocs).into_iter().map(|(_, x)| x).collect();
        assert_eq!(
            t,
            [
                "CONNAISSANCE", "I. ALPHA", "II. BETA", "III. GAMMA", "LOGIQUE", "IV. DELTA", "V. EPSILON", "VI. ZETA",
                "VII. ETA", "VIII. THETA", "PENSÉE PRATIQUE", "IX. IOTA", "X. KAPPA",
            ]
        );
    }

    #[test]
    fn folio_isole_hors_suite_pas_absorbe() {
        // Suite I, II, puis un folio « 12 » resté seul avant le 3e titre : il reste un
        // paragraphe, et la suite reprend à III.
        let sommaire = ["Alpha 7", "Beta 9", "Gamma 11", "Delta 13"];
        let corps_titres: [(u32, &[&str]); 4] = [
            (7, &["I", "ALPHA"]),
            (9, &["II", "BETA"]),
            (11, &["12", "GAMMA"]),
            (13, &["III", "DELTA"]),
        ];
        let TitresDetectes { blocs, .. } =
            detecter_titres(livre_a_sommaire(2, &sommaire, &corps_titres), &[], 20, SourceTitres::Auto);
        let t: Vec<String> = titres(&blocs).into_iter().map(|(_, x)| x).collect();
        assert_eq!(t, ["I. ALPHA", "II. BETA", "GAMMA", "III. DELTA"]);
        let page_11: Vec<String> = blocs.iter().filter(|b| b.ancre().page == 11).map(texte_de).collect();
        assert_eq!(page_11, ["12", "# GAMMA", texte_long().as_str()]);
    }

    #[test]
    fn entree_numero_seul_suivie_d_un_numero_de_page_cherchee() {
        // « DIX 15 », « 1984 11 », « MIX 17 » finissent par un numéro de page : ce sont de
        // vraies entrées, cherchées et retrouvées.
        let sommaire = ["Alpha 5", "Beta 9", "1984 11", "Gamma 13", "DIX 15", "MIX 17"];
        let corps_titres: [(u32, &[&str]); 6] = [
            (5, &["ALPHA"]),
            (9, &["BETA"]),
            (11, &["1984"]),
            (13, &["GAMMA"]),
            (15, &["DIX"]),
            (17, &["MIX"]),
        ];
        let TitresDetectes { blocs, .. } =
            detecter_titres(livre_a_sommaire(2, &sommaire, &corps_titres), &[], 20, SourceTitres::Auto);
        let t: Vec<(u8, String)> = titres(&blocs);
        assert_eq!(t, ["ALPHA", "BETA", "1984", "GAMMA", "DIX", "MIX"].map(|x| (1, x.to_string())));
    }

    #[test]
    fn sommaire_sur_deux_pages_consecutives() {
        let titres_du_corps: Vec<(u32, &[&str])> = vec![
            (5, &["ALPHA"]),
            (6, &["BETA"]),
            (7, &["GAMMA"]),
            (8, &["DELTA"]),
            (9, &["EPSILON"]),
            (10, &["ZETA"]),
            (11, &["ETA"]),
            (12, &["THETA"]),
        ];
        let blocs = livre_a_sommaires(
            &[
                (2, &["Alpha 5", "Beta 6", "Gamma 7", "Delta 8"]),
                (3, &["Epsilon 9", "Zeta 10", "Eta 11", "Theta 12"]),
            ],
            &titres_du_corps,
        );
        let TitresDetectes { blocs, sommaires: pages_som, .. } = detecter_titres(blocs, &[], 20, SourceTitres::Auto);
        assert_eq!(pages_som, BTreeSet::from([(2, Moitie::Entiere), (3, Moitie::Entiere)]));
        let t: Vec<(u8, String)> = titres(&blocs);
        assert_eq!(
            t,
            ["ALPHA", "BETA", "GAMMA", "DELTA", "EPSILON", "ZETA", "ETA", "THETA"].map(|x| (1, x.to_string()))
        );
    }

    #[test]
    fn numero_seul_sur_une_autre_demi_page_pas_absorbe() {
        // Page 16 double : « IV » seul en p16g, le titre en p16d. Le numéro n'est pas sur
        // la (demi-)page du titre : il reste un paragraphe.
        let mut blocs = livre_a_sommaire(2, &SOMMAIRE_IMPRIME, &TITRES_DU_CORPS);
        for bloc in blocs.iter_mut() {
            if let Bloc::Paragraphe(p) = bloc
                && p.ancre.page == 16
            {
                p.ancre.moitie = if p.texte_brut() == "IV" { Moitie::Gauche } else { Moitie::Droite };
            }
        }
        let TitresDetectes { blocs, .. } = detecter_titres(blocs, &[], 20, SourceTitres::Auto);
        let t: Vec<String> = titres(&blocs).into_iter().map(|(_, x)| x).collect();
        assert_eq!(
            t,
            [
                "I. LES BASES DE LA SUITE (I)",
                "II. LES BASES DE LA SUITE (II)",
                "III. JUGEMENTS ANALYTIQUES (LOGIQUE II)",
                "LE SYSTÈME DES CATÉGORIES CHEZ HEGEL",
            ]
        );
        let page_16: Vec<String> = blocs.iter().filter(|b| b.ancre().page == 16).map(texte_de).collect();
        assert_eq!(page_16, ["IV", "# LE SYSTÈME DES CATÉGORIES CHEZ HEGEL", texte_long().as_str()]);
    }

    #[test]
    fn sommaire_imprime_ignore_avec_des_signets() {
        // Avec des signets utilisables, les titres viennent des signets : les entrées du
        // sommaire imprimé n'en donnent pas, et aucun numéro seul n'est absorbé.
        let signets = [
            signet("LES BASES DE LA SUITE (I)", 1, 7),
            signet("LES BASES DE LA SUITE (II)", 1, 10),
            signet("JUGEMENTS ANALYTIQUES (LOGIQUE II)", 1, 13),
        ];
        let blocs = livre_a_sommaire(2, &SOMMAIRE_IMPRIME, &TITRES_DU_CORPS);
        let TitresDetectes { blocs, source, .. } = detecter_titres(blocs, &signets, 20, SourceTitres::Auto);
        assert_eq!(source, SourceTitres::Signets);
        let t: Vec<String> = titres(&blocs).into_iter().map(|(_, x)| x).collect();
        assert_eq!(
            t,
            ["LES BASES DE LA SUITE (I)", "LES BASES DE LA SUITE (II)", "JUGEMENTS ANALYTIQUES (LOGIQUE II)"]
        );
        assert!(blocs.iter().any(|b| texte_de(b) == "I"));
        assert!(blocs.iter().any(|b| texte_de(b) == "LE SYSTÈME DES CATÉGORIES CHEZ HEGEL"));
    }

    #[test]
    fn sommaire_imprime_ignore_avec_un_resume_par_chapitres() {
        // Un résumé « CHAPITRE n » (p2) donne le niveau 1 : le sommaire imprimé (p3) n'en
        // donne pas, et son entrée « Delta » ne devient pas un titre.
        let mut blocs = vec![
            corps(1, &texte_long()),
            ligne_en(2, 100, "CHAPITRE 1: ALPHA"),
            ligne_en(2, 140, "CHAPITRE 2: BETA"),
            ligne_en(2, 180, "CHAPITRE 3: GAMMA"),
        ];
        let sommaire = ["Alpha 5", "Beta 7", "Gamma 9", "Delta 11"];
        blocs.extend(sommaire.iter().enumerate().map(|(k, t)| ligne_en(3, 100 + 30 * k as u16, t)));
        for (page, titre) in [(5, "ALPHA"), (7, "BETA"), (9, "GAMMA"), (11, "DELTA")] {
            blocs.push(ligne_en(page, 100, titre));
            blocs.push(corps(page, &texte_long()));
        }
        let TitresDetectes { blocs, .. } = detecter_titres(blocs, &[], 11, SourceTitres::Auto);
        assert_eq!(titres(&blocs), ["ALPHA", "BETA", "GAMMA"].map(|x| (1, x.to_string())));
        assert!(blocs.iter().any(|b| texte_de(b) == "DELTA"));
    }

    #[test]
    fn titre_courant_retire_pas_retrouve_comme_entree() {
        // « LE LIVRE COURANT », en grand en tête des pages 3 à 20, est un titre courant
        // retiré : l'entrée « Le livre courant 13 » du sommaire ne le retrouve pas.
        let sommaire = ["Alpha 5", "Beta 7", "Gamma 9", "Delta 11", "Le livre courant 13"];
        let corps_titres: [(u32, &[&str]); 4] = [(5, &["ALPHA"]), (7, &["BETA"]), (9, &["GAMMA"]), (11, &["DELTA"])];
        let mut blocs = Vec::new();
        for bloc in livre_a_sommaire(2, &sommaire, &corps_titres) {
            let page = bloc.ancre().page;
            if page >= 3 && blocs.last().is_none_or(|b: &Bloc| b.ancre().page != page) {
                blocs.push(para(page, "LE LIVRE COURANT", 18.0, false, 1));
            }
            blocs.push(bloc);
        }
        let TitresDetectes { blocs, retraits, .. } = detecter_titres(blocs, &[], 20, SourceTitres::Auto);
        assert_eq!(retraits.len(), 18);
        assert_eq!(titres(&blocs), ["ALPHA", "BETA", "GAMMA", "DELTA"].map(|x| (1, x.to_string())));
    }

    #[test]
    fn correspondance_transmise_sans_seconde_recherche() {
        // Deux « ALPHA » sur la page 5 : le titre est pris là où la recherche l'a trouvé
        // (le second), sans chercher de nouveau.
        let blocs = vec![
            corps(4, &texte_long()),
            para(5, "ALPHA", 12.0, false, 1),
            corps(5, &texte_long()),
            para(5, "ALPHA", 12.0, false, 1),
            corps(5, &texte_long()),
        ];
        let cherche = SignetCherche {
            signet: signet("Alpha", 1, 5),
            texte_impose: false,
            numero_avant: false,
            trouve: Some((3, 4)),
        };
        let (blocs, points, titres_produits) = titres_depuis_signets(blocs, &[cherche], &BTreeSet::new());
        assert!(points.is_empty());
        assert!(matches!(&blocs[1], Bloc::Paragraphe(_)));
        assert!(matches!(&blocs[3], Bloc::Titre { niveau: 1, texte, .. } if texte == "ALPHA"));
        assert_eq!(titres_produits, HashSet::from([3]));
    }

    #[test]
    fn titres_proteges_par_position_et_non_par_ancre() {
        // Un titre inséré pour un signet introuvable prend l'ancre du premier bloc de sa
        // page : le candidat de cette ancre n'est pas protégé pour autant, et se fusionne
        // avec son voisin.
        let a = ancre(5);
        let blocs = vec![
            Bloc::Titre { niveau: 1, texte: "PARTIE".into(), ancre: a },
            Bloc::Titre { niveau: 2, texte: "LE CHAPITRE".into(), ancre: a },
            Bloc::Titre { niveau: 2, texte: "SANS FIN".into(), ancre: Ancre { rect: [100, 300, 900, 400], ..a } },
        ];
        let blocs = fusionner_titres_consecutifs(blocs, &HashSet::from([0]));
        assert_eq!(titres(&blocs), [(1, "PARTIE".to_string()), (2, "LE CHAPITRE SANS FIN".to_string())]);
        // Un titre protégé n'est jamais fusionné, ni avant ni après.
        let blocs = vec![
            Bloc::Titre { niveau: 2, texte: "UN".into(), ancre: a },
            Bloc::Titre { niveau: 2, texte: "DEUX".into(), ancre: a },
            Bloc::Titre { niveau: 2, texte: "TROIS".into(), ancre: a },
        ];
        let t = titres(&fusionner_titres_consecutifs(blocs, &HashSet::from([1])));
        assert_eq!(t, ["UN", "DEUX", "TROIS"].map(|x| (2, x.to_string())));
    }

    #[test]
    fn numeros_absorbes_tolerants() {
        use Numerotation::{Arabe, Romaine};
        let r = |n| Some((Romaine, n));
        let a = |n| Some((Arabe, n));
        // Un numéro sauté est toléré.
        assert_eq!(numeros_en_suite(&[r(1), r(3), r(4)]), [true, true, true]);
        // « I » manquant : la suite peut commencer à II.
        assert_eq!(numeros_en_suite(&[r(2), r(3)]), [true, true]);
        // Deux parties, chacune numérotée depuis I.
        assert_eq!(numeros_en_suite(&[r(1), r(2), r(1), r(2)]), [true, true, true, true]);
        // Un folio arabe après une suite romaine : refusé (genre et saut).
        assert_eq!(numeros_en_suite(&[r(1), r(2), a(12)]), [true, true, false]);
        // « 3 » arabe après I, II : refusé, le genre diffère.
        assert_eq!(numeros_en_suite(&[r(1), r(2), a(3)]), [true, true, false]);
        // 1, 2, puis « 12 » : refusé, trop loin.
        assert_eq!(numeros_en_suite(&[a(1), a(2), a(12)]), [true, true, false]);
        // Un numéro refusé ne change pas le précédent ; une entrée sans numéro non plus.
        assert_eq!(numeros_en_suite(&[r(1), a(12), None, r(2), r(3)]), [true, false, false, true, true]);
        // Le premier numéro absorbé vaut 1 ou 2.
        assert_eq!(numeros_en_suite(&[r(9), r(10)]), [false, false]);
    }

    #[test]
    fn ligne_numero_et_numero_de_page_du_sommaire_sautee() {
        // « III 9 » : un numéro de moins de 100 suivi du numéro de page n'est pas une
        // entrée ; l'entrée suivante « Gamma » absorbe le « III » du corps.
        let sommaire = ["Alpha 5", "Beta 7", "III 9", "Gamma", "Delta 11"];
        let corps_titres: [(u32, &[&str]); 4] = [
            (5, &["I", "ALPHA"]),
            (7, &["II", "BETA"]),
            (9, &["III", "GAMMA"]),
            (11, &["IV", "DELTA"]),
        ];
        let TitresDetectes { blocs, .. } =
            detecter_titres(livre_a_sommaire(2, &sommaire, &corps_titres), &[], 20, SourceTitres::Auto);
        let t: Vec<String> = titres(&blocs).into_iter().map(|(_, x)| x).collect();
        assert_eq!(t, ["I. ALPHA", "II. BETA", "III. GAMMA", "IV. DELTA"]);
    }

    #[test]
    fn correspondance_transmise_incoherente_cherchee_de_nouveau() {
        // `trouve` vise un paragraphe qui ne dit pas le titre, ou sort des blocs : le
        // signet est cherché normalement, sans panique.
        let blocs = || {
            vec![
                corps(4, &texte_long()),
                para(5, "ALPHA", 12.0, false, 1),
                corps(5, &texte_long()),
            ]
        };
        for trouve in [(2, 3), (1, 1), (5, 7), (2, 1)] {
            let cherche = SignetCherche {
                signet: signet("Alpha", 1, 5),
                texte_impose: false,
                numero_avant: false,
                trouve: Some(trouve),
            };
            let (blocs, points, produits) = titres_depuis_signets(blocs(), &[cherche], &BTreeSet::new());
            assert!(points.is_empty(), "{trouve:?}");
            assert!(matches!(&blocs[1], Bloc::Titre { texte, .. } if texte == "ALPHA"), "{trouve:?}");
            assert!(matches!(&blocs[2], Bloc::Paragraphe(_)), "{trouve:?}");
            assert_eq!(produits, HashSet::from([1]), "{trouve:?}");
        }
    }

    #[test]
    fn titre_insere_pour_un_signet_introuvable_protege() {
        // « OMEGA » est introuvable : son titre est inséré avant le premier bloc de la page
        // 5, et son indice dans les blocs rendus fait partie des titres produits.
        let blocs = vec![corps(4, &texte_long()), para(5, "ALPHA", 12.0, false, 1), corps(5, &texte_long())];
        let signets: Vec<SignetCherche> = vec![signet("Alpha", 1, 5).into(), signet("Omega", 1, 5).into()];
        let (blocs, points, produits) = titres_depuis_signets(blocs, &signets, &BTreeSet::new());
        assert_eq!(points.len(), 1);
        assert_eq!(texte_de(&blocs[1]), "# Omega");
        assert_eq!(texte_de(&blocs[2]), "# ALPHA");
        assert_eq!(produits, HashSet::from([1, 2]));
    }
}
