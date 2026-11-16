//! Détection des titres en cascade (signets PDF → gras + majuscules → taille)
//! et des pages de sommaire d'origine.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::blocs::{Bloc, Paragraphe};
use crate::modele::{Ancre, Moitie, Signet};
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

/// Pages dont la plupart des paragraphes ressemblent à des lignes de sommaire : ils
/// finissent par un numéro de page, après une espace ou des points de conduite. Des
/// points de suspension au milieu du texte ne suffisent pas.
fn pages_sommaire(blocs: &[Bloc]) -> BTreeSet<u32> {
    let fin_numero = Regex::new(r"[\s.\u{2026}]\d{1,4}$").unwrap();
    let mut par_page: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
    for bloc in blocs {
        if let Bloc::Paragraphe(p) = bloc {
            let texte = p.texte_brut();
            let texte = texte.trim();
            let compte = par_page.entry(p.ancre.page).or_default();
            compte.0 += 1;
            if fin_numero.is_match(texte) {
                compte.1 += 1;
            }
        }
    }
    par_page
        .into_iter()
        .filter(|(_, (total, numerotes))| *numerotes >= 4 && *numerotes * 2 >= *total)
        .map(|(page, _)| page)
        .collect()
}

fn est_sommaire(bloc: &Bloc, pages: &BTreeSet<u32>) -> bool {
    matches!(bloc, Bloc::Paragraphe(_)) && pages.contains(&bloc.ancre().page)
}

/// Titres issus des signets. Retourne les blocs et les points de signets introuvables.
fn titres_depuis_signets(
    blocs: Vec<Bloc>,
    signets: &[Signet],
    pages_som: &BTreeSet<u32>,
) -> (Vec<Bloc>, Vec<PointAVerifier>) {
    let mut points = Vec::new();
    // Remplacements : indice de début → (nombre de blocs avalés, titre).
    let mut remplacements: HashMap<usize, (usize, Bloc)> = HashMap::new();
    let mut utilises: HashSet<usize> = HashSet::new();
    // Insertions avant l'indice (blocs.len() = à la fin).
    let mut insertions: BTreeMap<usize, Vec<Bloc>> = BTreeMap::new();
    let index = IndexTitres::new(&blocs);

    for signet in signets {
        let niveau = signet.niveau.clamp(1, 3);
        let cible = normaliser_titre(&signet.titre);
        let trouve = if cible.is_empty() {
            None
        } else {
            let candidats = index.candidats(&cible);
            [signet.page, signet.page + 1]
                .into_iter()
                .find_map(|page| chercher_signet(&blocs, &index, &candidats, &cible, page, &utilises, pages_som))
        };
        match trouve {
            Some((debut, fin, texte)) => {
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
    let mut a_sauter = 0;
    for (i, bloc) in blocs.iter().enumerate() {
        if let Some(titres) = insertions.remove(&i) {
            resultat.extend(titres);
        }
        if a_sauter > 0 {
            a_sauter -= 1;
            continue;
        }
        if let Some((n, titre)) = remplacements.remove(&i) {
            resultat.push(titre);
            a_sauter = n - 1;
        } else {
            resultat.push(bloc.clone());
        }
    }
    if let Some(titres) = insertions.remove(&blocs.len()) {
        resultat.extend(titres);
    }
    (resultat, points)
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
}

impl IndexTitres {
    fn new(blocs: &[Bloc]) -> Self {
        let mut index = IndexTitres {
            paragraphes: Vec::with_capacity(blocs.len()),
            par_texte: HashMap::new(),
            par_debut: HashMap::new(),
        };
        for (i, bloc) in blocs.iter().enumerate() {
            let Bloc::Paragraphe(p) = bloc else {
                index.paragraphes.push(None);
                continue;
            };
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
    pages_som: &BTreeSet<u32>,
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
fn titres_par_candidats(
    blocs: Vec<Bloc>,
    pages_som: &BTreeSet<u32>,
    candidat: impl Fn(&Paragraphe) -> bool,
    niveau_1_pris: bool,
) -> Vec<Bloc> {
    let est_candidat = |p: &Paragraphe| !pages_som.contains(&p.ancre.page) && candidat(p);
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

    blocs
        .into_iter()
        .map(|b| match b {
            Bloc::Paragraphe(p) if est_candidat(&p) => Bloc::Titre {
                niveau: if chapitre(&p) { 1 } else { (niveaux.niveau(p.taille) + decalage).min(3) },
                texte: p.texte_brut().trim().to_string(),
                ancre: p.ancre,
            },
            autre => autre,
        })
        .collect()
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
/// date n'est jamais collée à un autre titre.
fn fusionner_titres_consecutifs(blocs: Vec<Bloc>) -> Vec<Bloc> {
    let mut resultat: Vec<Bloc> = Vec::with_capacity(blocs.len());
    for bloc in blocs {
        if let (
            Bloc::Titre { niveau, texte, ancre },
            Some(Bloc::Titre { niveau: n0, texte: t0, ancre: a0 }),
        ) = (&bloc, resultat.last_mut())
        {
            if niveau == n0 && ancre.page == a0.page && ancre.moitie == a0.moitie && !est_date(texte) && !est_date(t0) {
                t0.push(' ');
                t0.push_str(texte);
                continue;
            }
        }
        resultat.push(bloc);
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
    pages_som: &BTreeSet<u32>,
    candidat: impl Fn(&Paragraphe) -> bool,
    nb_pages_livre: u32,
) -> (Vec<Bloc>, Vec<Retrait>, Vec<PointAVerifier>) {
    let paragraphes: Vec<(usize, &Paragraphe, String)> = blocs
        .iter()
        .enumerate()
        .filter_map(|(i, b)| match b {
            Bloc::Paragraphe(p) if !pages_som.contains(&p.ancre.page) => Some((i, p, normaliser(&p.texte_brut()))),
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
/// Renvoie ses pages et, s'il est fait d'entrées « CHAPITRE n », un pseudo-signet de
/// niveau 1 par entrée retrouvée, à la page où elle revient.
fn resume_sans_numeros(blocs: &[Bloc], pages_som: &BTreeSet<u32>) -> (BTreeSet<u32>, Vec<Signet>) {
    let mut par_page: BTreeMap<u32, Vec<&Paragraphe>> = BTreeMap::new();
    for bloc in blocs {
        if let Bloc::Paragraphe(p) = bloc {
            par_page.entry(p.ancre.page).or_default().push(p);
        }
    }
    let courte = |page: &u32| !pages_som.contains(page) && par_page[page].iter().all(|p| court(p, &p.texte_brut()));
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
            // Seules les pages qui portent un candidat (ou dont la suivante en porte
            // un) peuvent aboutir : elles sont essayées dans l'ordre, comme le livre.
            let candidats = index.candidats(&cible);
            let pages: BTreeSet<u32> = candidats
                .iter()
                .map(|&i| blocs[i].ancre().page)
                .flat_map(|page| [page.saturating_sub(1), page])
                .filter(|page| *page >= depart && par_page.contains_key(page))
                .collect();
            let trouve = pages.into_iter().find_map(|page| {
                chercher_signet(blocs, &index, &candidats, &cible, page, &utilises, pages_som)
            });
            if let Some((debut, fin_bloc, _)) = trouve {
                let page = blocs[debut].ancre().page;
                utilises.extend(debut..fin_bloc);
                depart = page;
                retrouves.push(Signet { titre: entree.trim().to_string(), niveau: 1, page });
            }
        }
        if retrouves.len() >= MIN_TITRES_NIVEAU && retrouves.len() * 2 >= entrees.len() {
            pages_resume.extend(suite);
            if par_chapitres {
                pseudo.extend(retrouves);
            }
        }
    }
    (pages_resume, pseudo)
}

/// Détecte les titres selon `source` (ou la cascade en mode `Auto`) et les pages de
/// sommaire. Retourne les blocs, les points à vérifier et la source réellement utilisée.
/// Les signets d'appel de note (« [12] ») sont ignorés. Sans signets, un résumé sans
/// numéros de page compte comme sommaire et ses entrées « CHAPITRE n » donnent les
/// titres de niveau 1 du corps. Les titres courants pris dans le texte sont
/// retirés (retraits renvoyés en dernier).
pub fn detecter_titres(
    blocs: Vec<Bloc>,
    signets: &[Signet],
    nb_pages: u32,
    source: SourceTitres,
) -> (Vec<Bloc>, Vec<PointAVerifier>, SourceTitres, Vec<Retrait>) {
    // Les signets « [12] » mènent aux notes de fin (Calibre) : ni titres, ni plan.
    let appel_de_note = Regex::new(r"^\[\d+\]$").unwrap();
    let signets: Vec<Signet> = signets.iter().filter(|s| !appel_de_note.is_match(s.titre.trim())).cloned().collect();
    let signets = &signets[..];
    let mut pages_som = pages_sommaire(&blocs);
    let (pages_resume, pseudo_signets) = resume_sans_numeros(&blocs, &pages_som);
    let sans_resume = pages_som.clone();
    pages_som.extend(pages_resume);

    let corps = taille_corps(&blocs);
    let effective = match source {
        SourceTitres::Auto => {
            let nb_gras_maj = blocs
                .iter()
                .filter(|b| match b {
                    Bloc::Paragraphe(p) => !pages_som.contains(&p.ancre.page) && candidat_gras_majuscules(p),
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
        pages_som = sans_resume;
    }
    let mut points: Vec<PointAVerifier> = pages_som
        .iter()
        .filter_map(|page| blocs.iter().find(|b| b.ancre().page == *page))
        .map(|bloc| PointAVerifier {
            ancre: bloc.ancre(),
            genre: GenrePoint::Sommaire,
            message: format!(
                "la page {} semble être un sommaire d'origine : conservée comme texte",
                bloc.ancre().page
            ),
        })
        .collect();

    let mut retraits = Vec::new();
    let blocs = match effective {
        SourceTitres::Signets => {
            let (blocs, manquants) = titres_depuis_signets(blocs, signets, &pages_som);
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
            let niveau_1_pris = !pseudo_signets.is_empty();
            let blocs = if niveau_1_pris {
                let (blocs, manquants) = titres_depuis_signets(blocs, &pseudo_signets, &pages_som);
                points.extend(manquants);
                blocs
            } else {
                blocs
            };
            fusionner_titres_consecutifs(titres_par_candidats(blocs, &pages_som, candidat, niveau_1_pris))
        }
    };
    points.extend(titres_courants_suspects(&blocs));
    (blocs, points, effective, retraits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocs::Segment;
    use crate::zones::Role;

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
        let (blocs, _, source, _) = detecter_titres(blocs, &signets, 100, SourceTitres::Signets);
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
        let (blocs, points, _, _) = detecter_titres(blocs, &signets, 100, SourceTitres::Signets);
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
        let (blocs, points, _, _) = detecter_titres(blocs, &signets, 100, SourceTitres::Signets);
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
        let (blocs, points, _, _) = detecter_titres(blocs, &signets, 100, SourceTitres::Signets);
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
        let (blocs, points, source, _) = detecter_titres(blocs, &signets, 100, SourceTitres::Auto);
        assert_eq!(source, SourceTitres::Signets);
        let t: Vec<String> = titres(&blocs).into_iter().map(|(_, x)| x).collect();
        assert_eq!(t, ["1. VISITE À PATMOS", "2. INTRODUCTION", "3. MELKHITSÉDEK"]);
        assert!(points.iter().all(|p| !p.message.contains("[1]")));
    }

    #[test]
    fn signet_introuvable() {
        let blocs = vec![corps(6, &texte_long())];
        let signets = [signet("Épilogue", 1, 6)];
        let (blocs, points, _, _) = detecter_titres(blocs, &signets, 100, SourceTitres::Signets);
        assert!(matches!(&blocs[0], Bloc::Titre { texte, niveau: 1, .. } if texte == "Épilogue"));
        assert!(matches!(blocs[1], Bloc::Paragraphe(_)));
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].genre, GenrePoint::TitreSuspect);
        assert!(points[0].message.contains("Épilogue") && points[0].message.contains('6'));
    }

    #[test]
    fn signet_court_ne_avale_pas_le_corps() {
        let blocs = vec![corps(3, "Il était une fois un livre très long."), corps(3, &texte_long())];
        let (blocs, points, _, _) = detecter_titres(blocs, &[signet("I", 1, 3)], 100, SourceTitres::Signets);
        assert!(matches!(&blocs[0], Bloc::Titre { texte, .. } if texte == "I"));
        assert!(matches!(blocs[1], Bloc::Paragraphe(_)));
        assert_eq!(points.len(), 1);
    }

    #[test]
    fn signet_refuse_un_long_paragraphe() {
        let long = format!("Introduction de la méthode {}", vec!["mot"; 20].join(" "));
        let blocs = vec![para(3, &long, 12.0, false, 2)];
        let (blocs, points, _, _) = detecter_titres(blocs, &[signet("Introduction", 1, 3)], 100, SourceTitres::Signets);
        assert!(matches!(&blocs[0], Bloc::Titre { texte, .. } if texte == "Introduction"));
        assert!(matches!(blocs[1], Bloc::Paragraphe(_)));
        assert_eq!(points.len(), 1);
    }

    #[test]
    fn signet_prefere_la_correspondance_exacte() {
        let blocs = vec![para(3, "Introduction générale", 12.0, false, 1), para(3, "Introduction", 12.0, false, 1)];
        let (blocs, points, _, _) = detecter_titres(blocs, &[signet("Introduction", 1, 3)], 100, SourceTitres::Signets);
        assert!(matches!(&blocs[0], Bloc::Paragraphe(_)));
        assert!(matches!(&blocs[1], Bloc::Titre { texte, .. } if texte == "Introduction"));
        assert!(points.is_empty());
    }

    #[test]
    fn signet_page_vide_insere_avant_la_page_suivante() {
        let blocs = vec![corps(3, &texte_long()), corps(5, &texte_long())];
        let (blocs, _, _, _) = detecter_titres(blocs, &[signet("Partie", 1, 4)], 100, SourceTitres::Signets);
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
        let (blocs, _, source, _) = detecter_titres(blocs, &[], 11, SourceTitres::Auto);
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
        let (blocs, _, source, _) = detecter_titres(blocs, &[], 4, SourceTitres::Auto);
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
        let (blocs, _, _, _) = detecter_titres(blocs, &[], 70, SourceTitres::Taille);
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
        let (blocs, _, _, _) = detecter_titres(blocs, &[], 4, SourceTitres::Taille);
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
        let (blocs, _, _, _) = detecter_titres(blocs, &[], 13, SourceTitres::Taille);
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
        let (blocs, _, source, _) = detecter_titres(blocs, &[], 20, SourceTitres::Auto);
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
        let (blocs, _, _, _) = detecter_titres(blocs, &[], 2, SourceTitres::Taille);
        assert_eq!(titres(&blocs), vec![(1, "CHAPITRE 1: LES ANGES".to_string())]);
        assert_eq!(blocs.len(), 3);
    }

    #[test]
    fn sommaire_non_titre() {
        let mut blocs = vec![corps(1, &texte_long())];
        for (i, nom) in ["Visite à Patmos", "Les anges", "Le ciel", "La terre", "La mer", "Le feu"].iter().enumerate() {
            blocs.push(para(2, &format!("{nom} ...... {}", 12 + i), 18.0, true, 1));
        }
        let (blocs, points, _, _) = detecter_titres(blocs, &[], 10, SourceTitres::Taille);
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
        let (blocs, points, _, _) = detecter_titres(blocs, &[], 3, SourceTitres::GrasMajuscules);
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
        let (blocs, points, _, retraits) = detecter_titres(blocs, &[], 10, SourceTitres::GrasMajuscules);
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
        let (blocs, _, _, retraits) = detecter_titres(blocs, &[], 12, SourceTitres::GrasMajuscules);
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
        let (blocs, _, _, retraits) = detecter_titres(blocs, &[], 300, SourceTitres::GrasMajuscules);
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
        let (blocs, points, _, retraits) = detecter_titres(blocs, &[], 100, SourceTitres::GrasMajuscules);
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
        let (blocs, _, _, _) = detecter_titres(blocs, &[], 4, SourceTitres::GrasMajuscules);
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
        let (blocs, points, source, _) = detecter_titres(blocs, &[], 40, SourceTitres::Auto);
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
    fn taille_corps_pondere_par_caracteres() {
        let blocs = vec![para(1, "court", 18.0, false, 1), corps(1, &texte_long()), para(1, "x", 12.2, false, 1)];
        assert_eq!(taille_corps(&blocs), 12.0);
    }
}
