//! Regroupement des lignes de corps en paragraphes, recollage des césures et
//! raccord des paragraphes coupés par un saut de page.

use std::collections::{BTreeSet, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use crate::blocs::{Bloc, Paragraphe, Segment};
use crate::lignes::Ligne;
use crate::modele::{Moitie, Ancre, Rect};

/// Géométrie du bloc de texte, mesurée sur le livre.
#[derive(Clone, Copy, Debug)]
pub struct BlocTexte {
    pub x0: f32,
    pub x1: f32,
    /// Bas du bloc : bas de la dernière ligne d'une page pleine.
    pub y1: f32,
    pub interligne: f32,
}

fn mediane(mut valeurs: Vec<f32>) -> f32 {
    if valeurs.is_empty() {
        return 0.0;
    }
    valeurs.sort_by(|a, b| a.total_cmp(b));
    let n = valeurs.len();
    if n % 2 == 1 { valeurs[n / 2] } else { (valeurs[n / 2 - 1] + valeurs[n / 2]) / 2.0 }
}

fn bas(lignes: &[Ligne]) -> f32 {
    lignes.iter().map(|l| l.rect.y1).fold(f32::MIN, f32::max)
}

/// Mesure le bloc de texte : médianes des bords (et du bas de la dernière ligne)
/// sur les pages d'au moins 10 lignes (à défaut, sur toutes les pages), médiane
/// de l'interligne.
pub fn mesurer_bloc(pages: &[Vec<Ligne>]) -> BlocTexte {
    let pleines: Vec<&Vec<Ligne>> = pages.iter().filter(|p| p.len() >= 10).collect();
    let retenues: Vec<&Vec<Ligne>> =
        if pleines.is_empty() { pages.iter().filter(|p| !p.is_empty()).collect() } else { pleines };
    let min = |p: &&Vec<Ligne>| p.iter().map(|l| l.rect.x0).fold(f32::MAX, f32::min);
    let max = |p: &&Vec<Ligne>| p.iter().map(|l| l.rect.x1).fold(f32::MIN, f32::max);
    let x0 = mediane(retenues.iter().map(min).collect());
    let x1 = mediane(retenues.iter().map(max).collect());
    let y1 = mediane(retenues.iter().map(|p| bas(p)).collect());
    let ecarts: Vec<f32> = pages
        .iter()
        .flat_map(|p| p.windows(2).map(|w| w[1].rect.y0 - w[0].rect.y0))
        .collect();
    BlocTexte { x0, x1, y1, interligne: mediane(ecarts) }
}

/// La ligne va-t-elle jusqu'au bord droit du bloc (pas de blanc de fin de paragraphe) ?
fn ligne_pleine(l: &Ligne, bloc: BlocTexte) -> bool {
    bloc.x1 - l.rect.x1 <= 0.15 * (bloc.x1 - bloc.x0)
}

/// Écart (en interlignes) entre la dernière ligne et le bas effectif au-delà duquel
/// une page est courte.
const ECART_PAGE_COURTE: f32 = 2.5;

/// Ligne de texte courant coupée par le bas de la page : elle occupe toute la largeur
/// du bloc (ni courte, ni alignée à droite) et ne finit pas par une ponctuation finale.
fn paragraphe_continue(l: &Ligne, bloc: BlocTexte) -> bool {
    let fin = l
        .mots
        .iter()
        .enumerate()
        .rev()
        .find(|(i, _)| !l.exposants.contains(i))
        .and_then(|(_, m)| m.texte.trim_end().chars().last());
    ligne_pleine(l, bloc)
        && l.rect.x0 - bloc.x0 <= 0.15 * (bloc.x1 - bloc.x0)
        && fin.is_some_and(|c| !PONCTUATION_FINALE.contains(&c))
}

/// La page s'arrête-t-elle bien avant le bas du bloc ? Le bas effectif est le haut de
/// la première ligne de notes s'il y en a, sinon le bas du bloc. Une page sans corps
/// n'est pas courte, ni une page dont le dernier paragraphe continue sur la page
/// suivante (figure en bas de page).
pub fn page_courte(corps: &[Ligne], notes: &[Ligne], bloc: BlocTexte) -> bool {
    let Some(derniere) = corps.last() else { return false };
    if paragraphe_continue(derniere, bloc) {
        return false;
    }
    let bas_effectif =
        if notes.is_empty() { bloc.y1 } else { notes.iter().map(|l| l.rect.y0).fold(f32::MAX, f32::min) };
    bas_effectif - bas(corps) > ECART_PAGE_COURTE * bloc.interligne
}

/// Genre d'une énumération en tête de ligne (« 7. », « IV. »).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Enumeration {
    Chiffres,
    Romains,
}

static ENUMERATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:(\d{1,3})|([IVXLC]{1,6}))\.\s").unwrap());

/// Genre de l'énumération par laquelle commence le texte, s'il y en a une.
fn enumeration(texte: &str) -> Option<Enumeration> {
    let c = ENUMERATION.captures(texte)?;
    Some(if c.get(1).is_some() { Enumeration::Chiffres } else { Enumeration::Romains })
}

/// Points de conduite (au moins 3 points, espacés ou non, ou une ellipse) suivis d'un
/// numéro de page en fin de texte. « l 07 », « I07 »… y comptent comme un numéro
/// (OCR de « 107 »).
static ENTREE_A_POINTS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:(?:\.\s*){3,}|\u{2026}\s*)(?:\d{1,4}|[lI]\s?\d{1,3})\s*$").unwrap());

static NUMERO_DE_PAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{1,4}$").unwrap());

/// Écart (part de la largeur du bloc) entre le numéro de page et le mot qui le précède
/// au-delà duquel une ligne est une entrée de sommaire (« Notes      175 »).
const GRAND_BLANC: f32 = 0.10;

/// Le texte finit-il par des points de conduite et un numéro de page ?
pub(crate) fn finit_par_points_de_conduite(texte: &str) -> bool {
    ENTREE_A_POINTS.is_match(texte)
}

/// Un grand blanc dépasse aussi ce multiple du plus grand des autres écarts entre mots de
/// la ligne : une ligne justifiée, aux espaces étirées de façon égale, n'en a pas.
const GRAND_BLANC_RELATIF: f32 = 3.0;

/// Ligne d'entrée de sommaire : elle finit par un numéro de page précédé de points de
/// conduite ou d'un grand blanc. Le grand blanc (écart entre le numéro et le mot
/// précédent) dépasse `GRAND_BLANC` de la largeur du bloc et `GRAND_BLANC_RELATIF` fois
/// le plus grand des autres écarts entre mots séparés par une espace (exposants exclus).
fn ligne_entree(l: &Ligne, bloc: BlocTexte) -> bool {
    if finit_par_points_de_conduite(&l.texte()) {
        return true;
    }
    if l.mots.is_empty() || l.exposants.contains(&(l.mots.len() - 1)) {
        return false;
    }
    let mots: Vec<_> = l.mots.iter().enumerate().filter(|(i, _)| !l.exposants.contains(i)).map(|(_, m)| m).collect();
    let [.., avant, numero] = mots.as_slice() else { return false };
    let blanc = numero.rect.x0 - avant.rect.x1;
    let autres = mots[..mots.len() - 1]
        .windows(2)
        .filter(|w| w[1].espace_avant)
        .map(|w| w[1].rect.x0 - w[0].rect.x1)
        .fold(0.0_f32, f32::max);
    NUMERO_DE_PAGE.is_match(numero.texte.trim())
        && blanc > GRAND_BLANC * (bloc.x1 - bloc.x0)
        && blanc > GRAND_BLANC_RELATIF * autres
}

/// Faut-il couper le paragraphe entre `prec` et `l` ? `premiere` est la première
/// ligne du paragraphe en cours.
fn coupure(premiere: &Ligne, prec: &Ligne, l: &Ligne, bloc: BlocTexte) -> bool {
    let largeur = bloc.x1 - bloc.x0;
    let retrait = |x0: f32| x0 - bloc.x0 > 0.03 * largeur;
    // Chaque entrée numérotée (sommaire, liste) reste un paragraphe à part.
    let entree = enumeration(&l.texte()).is_some_and(|e| enumeration(&premiere.texte()) == Some(e));
    entree
        // Une ligne d'entrée de sommaire finit son paragraphe (jamais de coupure avant).
        || ligne_entree(prec, bloc)
        || !ligne_pleine(prec, bloc)
        || l.rect.y0 - prec.rect.y0 > 1.5 * bloc.interligne
        || (retrait(l.rect.x0) && !retrait(prec.rect.x0))
        || (l.taille() - prec.taille()).abs() > 0.5
        || l.tout_gras() != prec.tout_gras()
}

fn est_tiret(c: char) -> bool {
    c == '-' || c == '\u{2010}'
}

/// Tiret de césure : tiret final précédé d'une lettre.
fn finit_par_cesure(texte: &str) -> bool {
    let mut it = texte.chars().rev();
    matches!((it.next(), it.next()), (Some(t), Some(c)) if est_tiret(t) && c.is_alphabetic())
}

/// Fins de mot composé sans ambiguïté (« dites-vous », « celle-ci », « peut-être ») :
/// aucune césure ne laisse ces fragments en début de ligne.
const SUITES_DE_TRAIT_D_UNION: [&str; 13] =
    ["vous", "nous", "même", "mêmes", "ci", "là", "t", "être", "dessus", "dessous", "moi", "toi", "soi"];

/// Jonction d'un texte qui finit par un tiret de fin de ligne avec la suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cesure {
    /// Césure : le tiret disparaît, les deux morceaux sont collés.
    Recoller,
    /// Trait d'union d'un mot composé : le tiret reste, sans espace après.
    GarderTiret,
}

fn est_trait_d_union(c: char) -> bool {
    est_tiret(c) || c == '\u{2011}'
}

/// Vocabulaire d'un livre : mots et mots composés attestés à l'intérieur des lignes.
/// Il tranche les tirets de fin de ligne (césure ou mot composé).
#[derive(Debug, Default)]
pub struct Vocabulaire {
    mots: HashSet<String>,
    composes: HashSet<String>,
}

impl Vocabulaire {
    pub fn mesurer<'a>(lignes: impl IntoIterator<Item = &'a Ligne>) -> Vocabulaire {
        let mut v = Vocabulaire::default();
        for ligne in lignes {
            for mot in &ligne.mots {
                let texte = mot.texte.replace('\u{AD}', "").to_lowercase();
                for jeton in texte.split(|c: char| !c.is_alphabetic() && !est_trait_d_union(c)) {
                    let parties: Vec<&str> = jeton.split(est_trait_d_union).filter(|p| !p.is_empty()).collect();
                    for paire in parties.windows(2) {
                        v.composes.insert(format!("{}-{}", paire[0], paire[1]));
                    }
                    v.mots.extend(parties.iter().map(|p| p.to_string()));
                }
            }
        }
        v
    }

    /// Jonction de `avant` (qui finit par un tiret de fin de ligne) avec `suite` :
    /// - suite en majuscule : mot composé (« Saint-Esprit ») ;
    /// - mot recollé attesté dans le livre : césure ;
    /// - mot composé attesté dans le livre : trait d'union gardé ;
    /// - suite sans ambiguïté (« vous », « là », « même »…) : trait d'union gardé ;
    /// - sinon : césure.
    ///
    /// `None` si `avant` ne finit pas par une césure ou si la suite ne commence pas par
    /// une lettre.
    pub fn jonction(&self, avant: &str, suite: &str) -> Option<Cesure> {
        if !finit_par_cesure(avant) {
            return None;
        }
        let premier = suite.chars().next()?;
        if premier.is_uppercase() {
            return Some(Cesure::GarderTiret);
        }
        if !premier.is_lowercase() {
            return None;
        }
        let tete = &avant[..avant.len() - avant.chars().last().map_or(0, char::len_utf8)];
        let gauche: String = tete.chars().rev().take_while(|c| c.is_alphabetic()).collect::<Vec<_>>().into_iter().rev().collect();
        let gauche = gauche.to_lowercase();
        let droite: String = suite.chars().take_while(|c| c.is_alphabetic()).collect::<String>().to_lowercase();
        if self.mots.contains(&format!("{gauche}{droite}")) {
            Some(Cesure::Recoller)
        } else if self.composes.contains(&format!("{gauche}-{droite}"))
            || SUITES_DE_TRAIT_D_UNION.contains(&droite.as_str())
        {
            Some(Cesure::GarderTiret)
        } else {
            Some(Cesure::Recoller)
        }
    }
}

/// Ajoute du texte aux segments, en fusionnant avec le segment précédent s'il a le même style.
fn ajouter_texte(segments: &mut Vec<Segment>, texte: &str, gras: bool, italique: bool) {
    if texte.is_empty() {
        return;
    }
    if let Some(Segment::Texte { texte: t, gras: g, italique: i }) = segments.last_mut() {
        if *g == gras && *i == italique {
            t.push_str(texte);
            return;
        }
    }
    segments.push(Segment::Texte { texte: texte.to_string(), gras, italique });
}

pub(crate) fn segments_des_lignes(lignes: &[Ligne], vocab: &Vocabulaire) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    // Séparateur à placer devant le premier mot de la ligne courante.
    let mut espace_en_tete = false;
    for (n, ligne) in lignes.iter().enumerate() {
        for (i, mot) in ligne.mots.iter().enumerate() {
            let propre = mot.texte.replace('\u{AD}', "");
            if propre.is_empty() {
                continue;
            }
            let exposant = ligne.exposants.contains(&i);
            let espace = if i == 0 { n > 0 && espace_en_tete } else { mot.espace_avant && !exposant };
            if exposant {
                segments.push(Segment::Exposant(propre));
            } else {
                let texte = if espace { format!(" {propre}") } else { propre };
                ajouter_texte(&mut segments, &texte, mot.gras, mot.italique);
            }
        }
        // Décide de la jonction avec la ligne suivante.
        espace_en_tete = true;
        if let (Some(suivante), Some(dernier)) = (lignes.get(n + 1), ligne.mots.last()) {
            if dernier.texte.ends_with('\u{AD}') {
                espace_en_tete = false;
            } else if !ligne.exposants.contains(&(ligne.mots.len() - 1)) {
                match vocab.jonction(&dernier.texte, &suivante.texte()) {
                    Some(Cesure::Recoller) => {
                        retirer_dernier_caractere(&mut segments);
                        espace_en_tete = false;
                    }
                    Some(Cesure::GarderTiret) => espace_en_tete = false,
                    None => {}
                }
            }
        }
    }
    segments
}

fn retirer_dernier_caractere(segments: &mut Vec<Segment>) {
    if let Some(Segment::Texte { texte, .. }) = segments.last_mut() {
        texte.pop();
    }
    if matches!(segments.last(), Some(Segment::Texte { texte, .. }) if texte.is_empty()) {
        segments.pop();
    }
}

fn paragraphe_de(lignes: &[Ligne], page: u32, moitie: Moitie, vocab: &Vocabulaire) -> Paragraphe {
    let rect = Rect {
        x0: lignes.iter().map(|l| l.rect.x0).fold(f32::MAX, f32::min),
        y0: lignes.iter().map(|l| l.rect.y0).fold(f32::MAX, f32::min),
        x1: lignes.iter().map(|l| l.rect.x1).fold(f32::MIN, f32::max),
        y1: lignes.iter().map(|l| l.rect.y1).fold(f32::MIN, f32::max),
    };
    Paragraphe {
        segments: segments_des_lignes(lignes, vocab),
        ancre: Ancre::depuis(page, moitie, rect),
        taille: mediane(lignes.iter().map(Ligne::taille).collect()),
        tout_gras: lignes.iter().all(Ligne::tout_gras),
        nb_lignes: lignes.len(),
    }
}

pub fn paragraphes_page(
    lignes: &[Ligne],
    page: u32,
    moitie: Moitie,
    bloc: BlocTexte,
    vocab: &Vocabulaire,
) -> Vec<Paragraphe> {
    let mut resultat = Vec::new();
    let mut debut = 0;
    for i in 1..=lignes.len() {
        if i == lignes.len() || coupure(&lignes[debut], &lignes[i - 1], &lignes[i], bloc) {
            resultat.push(paragraphe_de(&lignes[debut..i], page, moitie, vocab));
            debut = i;
        }
    }
    resultat
}

const PONCTUATION_FINALE: [char; 9] = ['.', '!', '?', ':', '\u{BB}', '\u{2026}', ')', '"', '\u{201D}'];

/// Dernier caractère non blanc des segments `Texte` : appels de note et
/// exposants en fin de paragraphe sont ignorés.
fn dernier_caractere_de_texte(segments: &[Segment]) -> Option<char> {
    segments.iter().rev().find_map(|s| match s {
        Segment::Texte { texte, .. } => texte.trim_end().chars().last(),
        _ => None,
    })
}

/// Le texte finit-il ouvert (sans ponctuation finale), coupé par un saut de page ?
pub(crate) fn fin_ouverte(segments: &[Segment]) -> bool {
    dernier_caractere_de_texte(segments).is_some_and(|c| !PONCTUATION_FINALE.contains(&c))
}

/// Colle les segments `suite` à la fin de `segments` : césure recollée, trait d'union
/// gardé, ou une espace.
pub(crate) fn joindre(segments: &mut Vec<Segment>, suite: Vec<Segment>, vocab: &Vocabulaire) {
    let brut = |s: &[Segment]| -> String {
        s.iter()
            .map(|x| match x {
                Segment::Texte { texte, .. } | Segment::Exposant(texte) => texte.as_str(),
                Segment::AppelNote { marqueur, .. } => marqueur.as_str(),
            })
            .collect()
    };
    let jonction = match segments.last() {
        Some(Segment::Texte { .. }) => vocab.jonction(&brut(segments), brut(&suite).trim_start()),
        _ => None,
    };
    match jonction {
        Some(Cesure::Recoller) => retirer_dernier_caractere(segments),
        Some(Cesure::GarderTiret) => {}
        None => {
            let (gras, italique) = match segments.last() {
                Some(Segment::Texte { gras, italique, .. }) => (*gras, *italique),
                _ => (false, false),
            };
            ajouter_texte(segments, " ", gras, italique);
        }
    }
    let mut debut = jonction.is_some();
    for segment in suite {
        match segment {
            Segment::Texte { texte, gras, italique } => {
                let texte = if debut { texte.trim_start() } else { texte.as_str() };
                debut = false;
                ajouter_texte(segments, texte, gras, italique);
            }
            autre => {
                debut = false;
                segments.push(autre);
            }
        }
    }
}

/// Colle `suite` à `p` (paragraphe coupé par un saut de page).
fn fusionner(p: &mut Paragraphe, suite: Paragraphe, vocab: &Vocabulaire) {
    joindre(&mut p.segments, suite.segments, vocab);
    p.tout_gras = p.tout_gras && suite.tout_gras;
    p.nb_lignes += suite.nb_lignes;
}

/// Raccorde les paragraphes coupés par un saut de page. Les notes situées entre
/// les deux sont sautées (elles gardent leur place) ; un titre bloque le raccord,
/// de même qu'une page de sommaire d'un côté ou de l'autre (ses lignes restent seules),
/// ou deux entrées numérotées de même genre (« IV. » puis « V. »), ou un paragraphe qui
/// finit par des points de conduite et un numéro de page. Un paragraphe dont
/// l'ancre est dans `isoles` (ligne illisible refusée comme titre) ne se raccorde ni
/// à ce qui précède ni à ce qui suit : il reste seul.
pub fn raccorder_pages(
    blocs: Vec<Bloc>,
    vocab: &Vocabulaire,
    pages_sommaire: &BTreeSet<u32>,
    isoles: &HashSet<Ancre>,
) -> Vec<Bloc> {
    let mut sortie: Vec<Bloc> = Vec::new();
    // (Demi-)page où finit le dernier paragraphe : celle de son ancre, ou de la suite qui
    // lui a été collée. Seul le premier paragraphe de la (demi-)page suivante peut s'y raccorder.
    let mut fin: Option<(u32, Moitie)> = None;
    for bloc in blocs {
        let Bloc::Paragraphe(suite) = bloc else {
            sortie.push(bloc);
            continue;
        };
        let debut_suite = (suite.ancre.page, suite.ancre.moitie);
        let precedent = sortie.iter_mut().rev().find(|b| !matches!(b, Bloc::Note { .. }));
        if let Some(Bloc::Paragraphe(p)) = precedent {
            let autre_page = fin.is_some_and(|f| f != debut_suite);
            let ouvert = fin_ouverte(&p.segments);
            let sommaire = pages_sommaire.contains(&p.ancre.page) || pages_sommaire.contains(&suite.ancre.page);
            let entrees = enumeration(&suite.texte_brut()).is_some_and(|e| enumeration(&p.texte_brut()) == Some(e));
            let isole = isoles.contains(&p.ancre) || isoles.contains(&suite.ancre);
            let ligne_de_sommaire = finit_par_points_de_conduite(p.texte_brut().trim_end());
            if autre_page
                && ouvert
                && !sommaire
                && !entrees
                && !isole
                && !ligne_de_sommaire
                && !suite.texte_brut().is_empty()
            {
                fusionner(p, suite, vocab);
                fin = Some(debut_suite);
                continue;
            }
        }
        fin = Some(debut_suite);
        sortie.push(Bloc::Paragraphe(suite));
    }
    sortie
}

/// Un saut de section ne reste qu'entre deux paragraphes (les notes, rendues
/// ailleurs, ne comptent pas) : on retire les sauts en tête et en fin, ceux qui
/// touchent un titre, et tous sauf le premier d'une suite de sauts.
pub fn nettoyer_sauts(blocs: Vec<Bloc>) -> Vec<Bloc> {
    let visible = |b: &&Bloc| !matches!(b, Bloc::Note { .. });
    let est_saut = |b: &Bloc| matches!(b, Bloc::Saut { .. });
    let paragraphe = |b: Option<&Bloc>| matches!(b, Some(Bloc::Paragraphe(_)));
    let garder: Vec<bool> = (0..blocs.len())
        .map(|i| {
            !est_saut(&blocs[i])
                || (paragraphe(blocs[..i].iter().rev().find(visible))
                    && paragraphe(blocs[i + 1..].iter().find(|b| visible(b) && !est_saut(b))))
        })
        .collect();
    blocs.into_iter().zip(garder).filter_map(|(b, g)| g.then_some(b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modele::Mot;

    const BLOC: BlocTexte = BlocTexte { x0: 0.1, x1: 0.9, y1: 0.9, interligne: 0.03 };

    fn mot(texte: &str, x0: f32, x1: f32, y0: f32, taille: f32, gras: bool, italique: bool) -> Mot {
        Mot {
            texte: texte.to_string(),
            rect: Rect { x0, y0, x1, y1: y0 + 0.02 },
            taille,
            gras,
            italique,
            espace_avant: true,
        }
    }

    /// Ligne d'un seul mot, de `x0` à `x1`, à la hauteur `y0`.
    fn ligne(texte: &str, x0: f32, x1: f32, y0: f32) -> Ligne {
        ligne_taille(texte, x0, x1, y0, 12.0, false)
    }

    fn ligne_taille(texte: &str, x0: f32, x1: f32, y0: f32, taille: f32, gras: bool) -> Ligne {
        ligne_de_mots(vec![mot(texte, x0, x1, y0, taille, gras, false)])
    }

    fn ligne_de_mots(mots: Vec<Mot>) -> Ligne {
        let rect = Rect {
            x0: mots.iter().map(|m| m.rect.x0).fold(f32::MAX, f32::min),
            y0: mots.iter().map(|m| m.rect.y0).fold(f32::MAX, f32::min),
            x1: mots.iter().map(|m| m.rect.x1).fold(f32::MIN, f32::max),
            y1: mots.iter().map(|m| m.rect.y1).fold(f32::MIN, f32::max),
        };
        Ligne { mots, exposants: Vec::new(), rect }
    }

    fn textes(p: &[Paragraphe]) -> Vec<String> {
        p.iter().map(|p| p.texte_brut()).collect()
    }

    fn para(texte: &str, page: u32, y0: f32) -> Bloc {
        let r = Rect { x0: 0.1, y0, x1: 0.9, y1: y0 + 0.02 };
        Bloc::Paragraphe(Paragraphe {
            segments: vec![Segment::Texte { texte: texte.to_string(), gras: false, italique: false }],
            ancre: Ancre::depuis(page, Moitie::Entiere, r),
            taille: 12.0,
            tout_gras: false,
            nb_lignes: 1,
        })
    }

    fn texte_de(b: &Bloc) -> String {
        match b {
            Bloc::Paragraphe(p) => p.texte_brut(),
            _ => panic!("pas un paragraphe"),
        }
    }

    #[test]
    fn fin_de_paragraphe_par_blanc_a_droite() {
        let lignes = vec![
            ligne("un", 0.1, 0.9, 0.10),
            ligne("deux", 0.1, 0.66, 0.13),
            ligne("trois", 0.1, 0.9, 0.16),
        ];
        let p = paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC, &Vocabulaire::default());
        assert_eq!(textes(&p), vec!["un deux", "trois"]);
        assert_eq!(p[0].nb_lignes, 2);
    }

    #[test]
    fn ecart_vertical() {
        let lignes = vec![ligne("un", 0.1, 0.9, 0.10), ligne("deux", 0.1, 0.9, 0.16)];
        let p = paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC, &Vocabulaire::default());
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn alinea() {
        let lignes = vec![
            ligne("un", 0.1, 0.9, 0.10),
            ligne("deux", 0.1 + 0.05 * 0.8, 0.9, 0.13),
        ];
        let p = paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC, &Vocabulaire::default());
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn changement_de_taille() {
        let lignes = vec![
            ligne_taille("grand", 0.1, 0.9, 0.10, 19.0, false),
            ligne_taille("petit", 0.1, 0.9, 0.13, 15.0, false),
        ];
        assert_eq!(paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC, &Vocabulaire::default()).len(), 2);
    }

    #[test]
    fn cesure_recollee() {
        let lignes = vec![ligne("mer-", 0.1, 0.9, 0.10), ligne("veilleux", 0.1, 0.9, 0.13)];
        let p = paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC, &Vocabulaire::default());
        assert_eq!(textes(&p), vec!["merveilleux"]);

        let lignes = vec![ligne("Saint-", 0.1, 0.9, 0.10), ligne("Esprit", 0.1, 0.9, 0.13)];
        let p = paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC, &Vocabulaire::default());
        assert_eq!(textes(&p), vec!["Saint-Esprit"]);
    }

    #[test]
    fn cesures_sur_des_fins_de_mot_courantes_recollees() {
        // « les », « leurs » sont des fins de mot fréquentes après une césure (n° 3).
        for (avant, apres, attendu) in [
            ("subti-", "les", "subtiles"),
            ("cou-", "leurs", "couleurs"),
            ("cha-", "leur", "chaleur"),
            ("spirituel-", "les", "spirituelles"),
        ] {
            let lignes = vec![ligne(avant, 0.1, 0.9, 0.10), ligne(apres, 0.1, 0.9, 0.13)];
            let p = paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC, &Vocabulaire::default());
            assert_eq!(textes(&p), vec![attendu]);
        }
    }

    #[test]
    fn vocabulaire_du_livre_decide_des_cesures() {
        let lignes = vec![
            ligne_de_mots(vec![
                mot("le", 0.1, 0.2, 0.10, 12.0, false, false),
                mot("porte-monnaie,", 0.2, 0.4, 0.10, 12.0, false, false),
                mot("dit-il,", 0.4, 0.5, 0.10, 12.0, false, false),
                mot("Lui-même", 0.5, 0.6, 0.10, 12.0, false, false),
                mot("l'en-", 0.6, 0.9, 0.10, 12.0, false, false),
            ]),
            ligne("semble", 0.1, 0.9, 0.13),
        ];
        let v = Vocabulaire::mesurer(&lignes);
        assert_eq!(v.jonction("porte-", "monnaie"), Some(Cesure::GarderTiret));
        assert_eq!(v.jonction("dit-", "il,"), Some(Cesure::GarderTiret));
        assert_eq!(v.jonction("lui-", "même"), Some(Cesure::GarderTiret));
        assert_eq!(v.jonction("l'ensem-", "ble"), Some(Cesure::Recoller));
        assert_eq!(Vocabulaire::default().jonction("dit-", "il"), Some(Cesure::Recoller));
        assert_eq!(Vocabulaire::default().jonction("Saint-", "Esprit"), Some(Cesure::GarderTiret));
        assert_eq!(Vocabulaire::default().jonction("page", "12"), None);
        // Le mot recollé attesté dans le livre l'emporte sur la liste : « émoi ».
        assert_eq!(Vocabulaire::default().jonction("é-", "moi"), Some(Cesure::GarderTiret));
        let v = Vocabulaire::mesurer(&[ligne("émoi", 0.1, 0.9, 0.10)]);
        assert_eq!(v.jonction("é-", "moi"), Some(Cesure::Recoller));
    }

    #[test]
    fn trait_d_union_des_mots_composes_garde() {
        for (avant, apres, attendu) in [
            ("concentrez-", "vous", "concentrez-vous"),
            ("moment-", "là,", "moment-là,"),
            ("applique-", "t-on", "applique-t-on"),
            ("lui-", "même", "lui-même"),
            ("peut-", "être", "peut-être"),
        ] {
            let lignes = vec![ligne(avant, 0.1, 0.9, 0.10), ligne(apres, 0.1, 0.9, 0.13)];
            let p = paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC, &Vocabulaire::default());
            assert_eq!(textes(&p), vec![attendu]);
        }
    }

    #[test]
    fn trait_d_union_conditionnel() {
        let lignes = vec![ligne("mer\u{AD}", 0.1, 0.9, 0.10), ligne("veilleux", 0.1, 0.9, 0.13)];
        let p = paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC, &Vocabulaire::default());
        assert_eq!(textes(&p), vec!["merveilleux"]);

        let lignes = vec![ligne("au\u{AD}jourd'hui", 0.1, 0.9, 0.10)];
        let p = paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC, &Vocabulaire::default());
        assert_eq!(textes(&p), vec!["aujourd'hui"]);
    }

    #[test]
    fn segments_riches() {
        let mots = vec![
            mot("un", 0.10, 0.20, 0.10, 12.0, false, false),
            mot("beau", 0.21, 0.30, 0.10, 12.0, false, true),
            mot("jour", 0.31, 0.40, 0.10, 12.0, false, true),
            mot("rouge", 0.41, 0.50, 0.10, 12.0, false, false),
            mot("er", 0.50, 0.52, 0.10, 8.0, false, false),
        ];
        let mut l = ligne_de_mots(mots);
        l.mots[4].espace_avant = false;
        l.exposants = vec![4];
        let p = paragraphes_page(&[l], 1, Moitie::Entiere, BLOC, &Vocabulaire::default());
        assert_eq!(
            p[0].segments,
            vec![
                Segment::Texte { texte: "un".into(), gras: false, italique: false },
                Segment::Texte { texte: " beau jour".into(), gras: false, italique: true },
                Segment::Texte { texte: " rouge".into(), gras: false, italique: false },
                Segment::Exposant("er".into()),
            ]
        );
        assert_eq!(p[0].texte_brut(), "un beau jour rouge" .to_string() + "er");
    }

    #[test]
    fn raccord_inter_pages() {
        let blocs = vec![para("ce que", 4, 0.8), para("l'on dit", 5, 0.1)];
        let r = raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new());
        assert_eq!(r.len(), 1);
        assert_eq!(texte_de(&r[0]), "ce que l'on dit");
        assert_eq!(r[0].ancre().page, 4);

        let blocs = vec![para("la fin.", 4, 0.8), para("Suite", 5, 0.1)];
        assert_eq!(raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new()).len(), 2);

        let titre = Bloc::Titre { niveau: 1, texte: "T".into(), ancre: para("", 5, 0.1).ancre() };
        let blocs = vec![para("ce que", 4, 0.8), titre];
        assert_eq!(raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new()).len(), 2);
    }

    #[test]
    fn raccord_saute_les_notes() {
        let note = Bloc::Note {
            etiquette: "1-1".into(),
            marqueur: "1".into(),
            segments: vec![],
            ancre: para("", 4, 0.9).ancre(),
        };
        let blocs = vec![para("ce que", 4, 0.8), note, para("l'on dit", 5, 0.1)];
        let r = raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new());
        assert_eq!(r.len(), 2);
        assert_eq!(texte_de(&r[0]), "ce que l'on dit");
        assert!(matches!(r[1], Bloc::Note { .. }));
    }

    fn para_avec_fin(texte: &str, fin: Segment) -> Bloc {
        let Bloc::Paragraphe(mut p) = para(texte, 4, 0.8) else { unreachable!() };
        p.segments.push(fin);
        Bloc::Paragraphe(p)
    }

    fn appel() -> Segment {
        Segment::AppelNote { etiquette: "1-1".into(), marqueur: "1".into() }
    }

    #[test]
    fn pas_de_raccord_avec_une_page_de_sommaire() {
        let sommaire = BTreeSet::from([6]);
        let blocs = vec![para("benjamin@exemple.com", 5, 0.8), para("RÉSUMÉ", 6, 0.1)];
        assert_eq!(raccorder_pages(blocs, &Vocabulaire::default(), &sommaire, &HashSet::new()).len(), 2);
        let blocs = vec![para("TRADITION CABALISTIQUE", 6, 0.8), para("CHAPITRE 6:", 7, 0.1)];
        assert_eq!(raccorder_pages(blocs, &Vocabulaire::default(), &sommaire, &HashSet::new()).len(), 2);
    }

    #[test]
    fn raccord_ignore_l_appel_de_note_final() {
        let r = raccorder_pages(vec![para_avec_fin("la fin.", appel()), para("Suite", 5, 0.1)], &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new());
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn raccord_ignore_l_exposant_final() {
        let r = raccorder_pages(vec![
            para_avec_fin("la fin.", Segment::Exposant("1".into())),
            para("Suite", 5, 0.1),
        ], &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new());
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn raccord_malgre_un_appel_de_note_apres_un_mot() {
        let r = raccorder_pages(vec![para_avec_fin("ce que", appel()), para("l'on dit", 5, 0.1)], &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new());
        assert_eq!(r.len(), 1);
        assert_eq!(texte_de(&r[0]), "ce que1 l'on dit");
    }

    #[test]
    fn pas_de_raccord_sur_la_meme_page() {
        let blocs = vec![para("ce que", 4, 0.1), para("l'on dit", 4, 0.3)];
        assert_eq!(raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new()).len(), 2);
    }

    #[test]
    fn raccord_recolle_une_cesure() {
        let blocs = vec![para("mer-", 4, 0.8), para("veilleux", 5, 0.1)];
        let r = raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new());
        assert_eq!(texte_de(&r[0]), "merveilleux");
        let blocs = vec![para("dites-", 4, 0.8), para("vous bien", 5, 0.1)];
        assert_eq!(texte_de(&raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new())[0]), "dites-vous bien");
        let blocs = vec![para("le Saint-", 4, 0.8), para("Esprit", 5, 0.1)];
        assert_eq!(texte_de(&raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new())[0]), "le Saint-Esprit");
    }

    /// Lignes pleines (sans blanc de fin de paragraphe) aux hauteurs données.
    fn lignes_pleines(textes: &[&str]) -> Vec<Ligne> {
        textes.iter().enumerate().map(|(i, t)| ligne(t, 0.1, 0.9, 0.10 + 0.03 * i as f32)).collect()
    }

    #[test]
    fn entrees_numerotees_jamais_fusionnees() {
        let v = Vocabulaire::default();
        // Sommaire à interligne normal et lignes pleines : chaque entrée reste seule.
        let p = paragraphes_page(&lignes_pleines(&["7. La porte", "8. Le fleuve", "13. La cité", "14. La fin"]), 5, Moitie::Entiere, BLOC, &v);
        assert_eq!(textes(&p), vec!["7. La porte", "8. Le fleuve", "13. La cité", "14. La fin"]);
        // Chiffres romains, avec une entrée sur deux lignes.
        let p = paragraphes_page(&lignes_pleines(&["III. La pierre", "angulaire", "IV. Le fleuve"]), 5, Moitie::Entiere, BLOC, &v);
        assert_eq!(textes(&p), vec!["III. La pierre angulaire", "IV. Le fleuve"]);
    }

    /// Ligne de mots régulièrement espacés de `x0` à `x1` (texte justifié, points de
    /// conduite) : chaque mot occupe 80 % de sa case.
    fn ligne_reguliere(texte: &str, x0: f32, x1: f32, y0: f32) -> Ligne {
        let mots: Vec<&str> = texte.split_whitespace().collect();
        let case = (x1 - x0) / mots.len() as f32;
        ligne_de_mots(
            mots.iter()
                .enumerate()
                .map(|(i, m)| {
                    let debut = x0 + case * i as f32;
                    mot(m, debut, debut + 0.8 * case, y0, 12.0, false, false)
                })
                .collect(),
        )
    }

    #[test]
    fn lignes_d_entree_a_points_de_conduite_coupees_apres() {
        let lignes = vec![
            ligne_reguliere("Douceur et humilité . . . . 11", 0.1, 0.9, 0.10),
            ligne_reguliere("Si vous ne mourez pas . . . 31", 0.1, 0.9, 0.13),
        ];
        let p = paragraphes_page(&lignes, 205, Moitie::Entiere, BLOC, &Vocabulaire::default());
        assert_eq!(textes(&p), vec!["Douceur et humilité . . . . 11", "Si vous ne mourez pas . . . 31"]);
        // Points collés, en désordre, ou ellipse.
        for texte in ["L'alchimie spirituelle ...... 67", "La parabole .. ..... 145", "Le fleuve \u{2026} 12"] {
            assert!(ligne_entree(&ligne_reguliere(texte, 0.1, 0.9, 0.1), BLOC), "{texte}");
        }
    }

    #[test]
    fn numero_lu_l_07_apres_des_points_de_conduite() {
        for fin in ["l 07", "l07", "I 07", "I07"] {
            let texte = format!("Le miracle des deux poissons et des cinq pains . . . . {fin}");
            assert!(ligne_entree(&ligne_reguliere(&texte, 0.1, 0.9, 0.1), BLOC), "{texte}");
        }
    }

    #[test]
    fn ligne_d_entree_a_grand_blanc_coupee_apres() {
        let notes = ligne_de_mots(vec![
            mot("Notes", 0.1, 0.18, 0.10, 12.0, false, false),
            mot("175", 0.86, 0.9, 0.10, 12.0, false, false),
        ]);
        let lignes = vec![notes, ligne_reguliere("Ouvrages de Rudolf Steiner disponibles en langue", 0.1, 0.9, 0.13)];
        let p = paragraphes_page(&lignes, 3, Moitie::Droite, BLOC, &Vocabulaire::default());
        assert_eq!(textes(&p), vec!["Notes 175", "Ouvrages de Rudolf Steiner disponibles en langue"]);
    }

    #[test]
    fn limites_du_grand_blanc() {
        let v = Vocabulaire::default();
        // Deux mots, écart un peu sous 10 % de la largeur (0,07 pour 0,08) : pas d'entrée.
        let total = ligne_de_mots(vec![
            mot("Total", 0.1, 0.5, 0.10, 12.0, false, false),
            mot("1903", 0.57, 0.9, 0.10, 12.0, false, false),
        ]);
        assert!(!ligne_entree(&total, BLOC));
        let lignes = vec![total, ligne_reguliere("devant les membres de la société", 0.1, 0.9, 0.13)];
        assert_eq!(paragraphes_page(&lignes, 3, Moitie::Entiere, BLOC, &v).len(), 1);
        // Deux mots : pas d'autre écart, le seuil de 10 % suffit.
        let notes = ligne_de_mots(vec![
            mot("Notes", 0.1, 0.18, 0.10, 12.0, false, false),
            mot("175", 0.86, 0.9, 0.10, 12.0, false, false),
        ]);
        assert!(ligne_entree(&notes, BLOC));
    }

    #[test]
    fn ligne_justifiee_a_grands_ecarts_egaux_ne_coupe_pas() {
        // Trois mots étirés à écarts égaux de 0,15 (plus de 10 % de la largeur).
        let jean = ligne_de_mots(vec![
            mot("Jean", 0.1, 0.2, 0.10, 12.0, false, false),
            mot("disait", 0.35, 0.45, 0.10, 12.0, false, false),
            mot("1903", 0.6, 0.9, 0.10, 12.0, false, false),
        ]);
        assert!(!ligne_entree(&jean, BLOC));
        let lignes = vec![jean, ligne_reguliere("devant les membres de la société", 0.1, 0.9, 0.13)];
        assert_eq!(paragraphes_page(&lignes, 3, Moitie::Entiere, BLOC, &Vocabulaire::default()).len(), 1);
    }

    #[test]
    fn grand_blanc_apres_des_espaces_normales_coupe() {
        // « Première conférence, Berlin, 27 novembre 1903 » (écarts de 0,01) + blanc + « 7 ».
        let mut mots = Vec::new();
        let mut x = 0.1;
        for texte in ["Première", "conférence,", "Berlin,", "27", "novembre", "1903"] {
            mots.push(mot(texte, x, x + 0.07, 0.10, 12.0, false, true));
            x += 0.08;
        }
        mots.push(mot("7", 0.88, 0.9, 0.10, 12.0, false, false));
        let lignes = vec![
            ligne_de_mots(mots),
            ligne_reguliere("Les bases d'une théorie de la connaissance", 0.1, 0.9, 0.13),
        ];
        assert!(ligne_entree(&lignes[0], BLOC));
        let p = paragraphes_page(&lignes, 3, Moitie::Droite, BLOC, &Vocabulaire::default());
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn ligne_justifiee_finie_par_une_annee_ne_coupe_pas() {
        let lignes = vec![
            ligne_reguliere("ces conférences furent données à Berlin en 1903", 0.1, 0.9, 0.10),
            ligne_reguliere("devant les membres de la société", 0.1, 0.9, 0.13),
        ];
        let p = paragraphes_page(&lignes, 3, Moitie::Entiere, BLOC, &Vocabulaire::default());
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn entree_sur_deux_lignes_reste_un_paragraphe() {
        let lignes = vec![
            ligne_reguliere("L'élaboration des concepts et le système des catégories", 0.1, 0.9, 0.10),
            ligne_reguliere("chez Hegel . . . . . . 114", 0.1, 0.9, 0.13),
            ligne_reguliere("La culture pratique . . . . 128", 0.1, 0.9, 0.16),
        ];
        let p = paragraphes_page(&lignes, 3, Moitie::Entiere, BLOC, &Vocabulaire::default());
        assert_eq!(
            textes(&p),
            vec![
                "L'élaboration des concepts et le système des catégories chez Hegel . . . . . . 114",
                "La culture pratique . . . . 128",
            ]
        );
    }

    #[test]
    fn raccord_jamais_apres_une_entree_a_points_de_conduite() {
        let blocs = vec![
            para("Les pouvoirs de la mère pendant la gestation . . . 209", 205, 0.8),
            para("L'association Fraternité Blanche Universelle", 206, 0.1),
        ];
        assert_eq!(raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new()).len(), 2);
        // Un numéro sans points de conduite ne bloque pas le raccord (texte seul).
        let blocs = vec![para("les conférences de 1903", 5, 0.8), para("suite", 6, 0.1)];
        assert_eq!(raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new()).len(), 1);
    }

    #[test]
    fn enumeration_sans_effet_hors_d_une_entree_numerotee() {
        let v = Vocabulaire::default();
        // Le paragraphe en cours ne commençait pas par une énumération : pas de coupure.
        let p = paragraphes_page(&lignes_pleines(&["il a dit", "2. que oui"]), 5, Moitie::Entiere, BLOC, &v);
        assert_eq!(textes(&p), vec!["il a dit 2. que oui"]);
        // Énumération d'un autre type : pas de coupure.
        let p = paragraphes_page(&lignes_pleines(&["IV. Le fleuve", "5. de la vie"]), 5, Moitie::Entiere, BLOC, &v);
        assert_eq!(p.len(), 1);
        // Sans espace après le point, ce n'est pas une énumération (« 3.5 »).
        let p = paragraphes_page(&lignes_pleines(&["1. Le total", "3.5 francs"]), 5, Moitie::Entiere, BLOC, &v);
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn pas_de_raccord_avec_un_paragraphe_isole() {
        // Fin de p4 isolée (ligne refusée comme titre) : elle ne happe pas le corps de p5.
        let blocs = vec![para("ligne refusée sans point", 4, 0.8), para("corps du texte", 5, 0.1)];
        let isoles = HashSet::from([blocs[0].ancre()]);
        assert_eq!(raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &isoles).len(), 2);
        // Début de p5 isolé : il ne se colle pas au corps ouvert de p4.
        let blocs = vec![para("ce que", 4, 0.8), para("ligne refusée", 5, 0.1)];
        let isoles = HashSet::from([blocs[1].ancre()]);
        assert_eq!(raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &isoles).len(), 2);
        // Sans l'ensemble, les deux se raccordent.
        let blocs = vec![para("ce que", 4, 0.8), para("ligne refusée", 5, 0.1)];
        assert_eq!(raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new()).len(), 1);
    }

    #[test]
    fn raccord_n_unit_pas_deux_entrees_numerotees() {
        let blocs = vec![para("IV. Le fleuve de la vie", 5, 0.8), para("V. L'avènement", 6, 0.1)];
        assert_eq!(raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new()).len(), 2);
        // Une suite qui n'est pas une entrée se raccorde toujours.
        let blocs = vec![para("IV. Le fleuve de la", 5, 0.8), para("vie éternelle", 6, 0.1)];
        let r = raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new());
        assert_eq!(texte_de(&r[0]), "IV. Le fleuve de la vie éternelle");
    }

    #[test]
    fn raccord_sans_chaine_de_fusions() {
        // Seul le dernier paragraphe d'une page se raccorde au premier de la suivante.
        let blocs = vec![para("ce que", 4, 0.8), para("l'on dit", 5, 0.1), para("suite sans point", 5, 0.3)];
        let r = raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new());
        assert_eq!(r.len(), 2);
        assert_eq!(texte_de(&r[0]), "ce que l'on dit");
        assert_eq!(r[0].ancre().page, 4);
        assert_eq!(texte_de(&r[1]), "suite sans point");
        // Un paragraphe qui remplit toute une page se raccorde des deux côtés.
        let blocs = vec![para("ce que", 4, 0.8), para("l'on dit", 5, 0.1), para("encore", 6, 0.1)];
        let r = raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new());
        assert_eq!(texte_de(&r[0]), "ce que l'on dit encore");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn mesure_du_bloc() {
        let page: Vec<Ligne> = (0..10)
            .map(|i| ligne("x", 0.12, 0.88, 0.1 + 0.03 * i as f32))
            .collect();
        let court = vec![ligne("x", 0.0, 0.5, 0.1)];
        let b = mesurer_bloc(&[page, court]);
        assert!((b.x0 - 0.12).abs() < 1e-6);
        assert!((b.x1 - 0.88).abs() < 1e-6);
        assert!((b.interligne - 0.03).abs() < 1e-4);
        // Bas de la dernière ligne (0,1 + 9 × 0,03 + 0,02), pages pleines seulement.
        assert!((b.y1 - 0.39).abs() < 1e-4, "{}", b.y1);
    }

    /// Page de lignes pleines, de `y0` (haut de la première) à `fin` (haut de la
    /// dernière) ; la dernière ligne est `derniere`, de `x0` à `x1`.
    fn lignes_de(y0: f32, fin: f32, derniere: &str, (x0, x1): (f32, f32)) -> Vec<Ligne> {
        let n = ((fin - y0) / 0.03).round() as usize;
        let mut lignes: Vec<Ligne> = (0..n).map(|i| ligne("x", 0.1, 0.9, y0 + 0.03 * i as f32)).collect();
        lignes.push(ligne(derniere, x0, x1, y0 + 0.03 * n as f32));
        lignes
    }

    #[test]
    fn page_courte_mesuree_au_bas_effectif() {
        // Bas du bloc 0,9, interligne 0,03 : courte si la dernière ligne finit avant 0,825.
        assert!(!page_courte(&lignes_de(0.1, 0.86, "fin.", (0.1, 0.5)), &[], BLOC));
        assert!(page_courte(&lignes_de(0.1, 0.4, "fin.", (0.1, 0.5)), &[], BLOC));
        assert!(!page_courte(&[], &[], BLOC));
        // Avec des notes, le bas effectif est le haut de la première ligne de notes.
        let notes = vec![ligne("1. Note", 0.1, 0.9, 0.62), ligne("suite", 0.1, 0.9, 0.65)];
        assert!(!page_courte(&lignes_de(0.1, 0.55, "fin.", (0.1, 0.5)), &notes, BLOC));
        assert!(page_courte(&lignes_de(0.1, 0.49, "fin.", (0.1, 0.5)), &notes, BLOC));
    }

    #[test]
    fn page_coupee_en_plein_paragraphe_n_est_pas_courte() {
        // Une figure occupe le bas de la page : la dernière ligne, pleine et sans
        // ponctuation finale, continue sur la page suivante.
        assert!(!page_courte(&lignes_de(0.1, 0.4, "dont", (0.1, 0.9)), &[], BLOC));
        // Ligne pleine mais phrase finie, ligne courte ou ligne alignée à droite (date,
        // signature) : la page reste courte.
        assert!(page_courte(&lignes_de(0.1, 0.4, "fin.", (0.1, 0.9)), &[], BLOC));
        assert!(page_courte(&lignes_de(0.1, 0.4, "ISBN 2-85566-476-4", (0.1, 0.5)), &[], BLOC));
        assert!(page_courte(&lignes_de(0.1, 0.4, "Le Bonfin, le 20 juillet 1969", (0.5, 0.9)), &[], BLOC));
    }

    fn saut(page: u32) -> Bloc {
        Bloc::Saut { ancre: para("", page, 0.1).ancre() }
    }

    fn genres(blocs: &[Bloc]) -> String {
        blocs
            .iter()
            .map(|b| match b {
                Bloc::Paragraphe(_) => 'P',
                Bloc::Titre { .. } => 'T',
                Bloc::Note { .. } => 'N',
                Bloc::Saut { .. } => 'S',
                Bloc::EntreeSommaire { .. } => 'E',
            })
            .collect()
    }

    #[test]
    fn pas_de_raccord_a_travers_un_saut() {
        let blocs = vec![para("ISBN 2-85566-476-4", 3, 0.8), saut(4), para("Le lecteur comprendra", 4, 0.1)];
        let r = raccorder_pages(blocs, &Vocabulaire::default(), &BTreeSet::new(), &HashSet::new());
        assert_eq!(genres(&r), "PSP");
    }

    #[test]
    fn nettoyage_des_sauts() {
        let note = || Bloc::Note {
            etiquette: "1-1".into(),
            marqueur: "1".into(),
            segments: vec![],
            ancre: para("", 4, 0.9).ancre(),
        };
        let titre = || Bloc::Titre { niveau: 1, texte: "T".into(), ancre: para("", 5, 0.1).ancre() };
        let p = |page| para("x", page, 0.1);
        // En tête, en fin, en double.
        assert_eq!(genres(&nettoyer_sauts(vec![saut(1), p(1), saut(2), saut(2), p(2), saut(3)])), "PSP");
        // Contre un titre, d'un côté ou de l'autre.
        assert_eq!(genres(&nettoyer_sauts(vec![p(1), saut(2), titre(), saut(3), p(3)])), "PTP");
        // Les notes ne comptent pas comme voisines : le saut entre deux paragraphes reste,
        // celui qui ne précède que des notes s'en va.
        assert_eq!(genres(&nettoyer_sauts(vec![p(1), note(), saut(2), p(2)])), "PNSP");
        assert_eq!(genres(&nettoyer_sauts(vec![p(1), note(), saut(2), titre()])), "PNT");
        assert_eq!(genres(&nettoyer_sauts(vec![p(1), saut(2), note(), note()])), "PNN");
    }
}
