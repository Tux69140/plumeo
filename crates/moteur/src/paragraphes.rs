//! Regroupement des lignes de corps en paragraphes, recollage des césures et
//! raccord des paragraphes coupés par un saut de page.

use std::collections::HashSet;

use crate::blocs::{Bloc, Paragraphe, Segment};
use crate::lignes::Ligne;
use crate::modele::{Moitie, Ancre, Rect};

/// Géométrie du bloc de texte, mesurée sur le livre.
#[derive(Clone, Copy, Debug)]
pub struct BlocTexte {
    pub x0: f32,
    pub x1: f32,
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

/// Mesure le bloc de texte : médianes des bords sur les pages d'au moins
/// 10 lignes (à défaut, sur toutes les pages), médiane de l'interligne.
pub fn mesurer_bloc(pages: &[Vec<Ligne>]) -> BlocTexte {
    let pleines: Vec<&Vec<Ligne>> = pages.iter().filter(|p| p.len() >= 10).collect();
    let retenues: Vec<&Vec<Ligne>> =
        if pleines.is_empty() { pages.iter().filter(|p| !p.is_empty()).collect() } else { pleines };
    let min = |p: &&Vec<Ligne>| p.iter().map(|l| l.rect.x0).fold(f32::MAX, f32::min);
    let max = |p: &&Vec<Ligne>| p.iter().map(|l| l.rect.x1).fold(f32::MIN, f32::max);
    let x0 = mediane(retenues.iter().map(min).collect());
    let x1 = mediane(retenues.iter().map(max).collect());
    let ecarts: Vec<f32> = pages
        .iter()
        .flat_map(|p| p.windows(2).map(|w| w[1].rect.y0 - w[0].rect.y0))
        .collect();
    BlocTexte { x0, x1, interligne: mediane(ecarts) }
}

/// Faut-il couper le paragraphe entre `prec` et `l` ?
fn coupure(prec: &Ligne, l: &Ligne, bloc: BlocTexte) -> bool {
    let largeur = bloc.x1 - bloc.x0;
    let retrait = |x0: f32| x0 - bloc.x0 > 0.03 * largeur;
    bloc.x1 - prec.rect.x1 > 0.15 * largeur
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
        if i == lignes.len() || coupure(&lignes[i - 1], &lignes[i], bloc) {
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
/// les deux sont sautées (elles gardent leur place) ; un titre bloque le raccord.
pub fn raccorder_pages(blocs: Vec<Bloc>, vocab: &Vocabulaire) -> Vec<Bloc> {
    let mut sortie: Vec<Bloc> = Vec::new();
    for bloc in blocs {
        let Bloc::Paragraphe(suite) = bloc else {
            sortie.push(bloc);
            continue;
        };
        let precedent = sortie.iter_mut().rev().find(|b| !matches!(b, Bloc::Note { .. }));
        if let Some(Bloc::Paragraphe(p)) = precedent {
            let autre_page = (p.ancre.page, p.ancre.moitie) != (suite.ancre.page, suite.ancre.moitie);
            let ouvert = dernier_caractere_de_texte(&p.segments)
                .is_some_and(|c| !PONCTUATION_FINALE.contains(&c));
            if autre_page && ouvert && !suite.texte_brut().is_empty() {
                fusionner(p, suite, vocab);
                continue;
            }
        }
        sortie.push(Bloc::Paragraphe(suite));
    }
    sortie
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modele::Mot;

    const BLOC: BlocTexte = BlocTexte { x0: 0.1, x1: 0.9, interligne: 0.03 };

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
        let r = raccorder_pages(blocs, &Vocabulaire::default());
        assert_eq!(r.len(), 1);
        assert_eq!(texte_de(&r[0]), "ce que l'on dit");
        assert_eq!(r[0].ancre().page, 4);

        let blocs = vec![para("la fin.", 4, 0.8), para("Suite", 5, 0.1)];
        assert_eq!(raccorder_pages(blocs, &Vocabulaire::default()).len(), 2);

        let titre = Bloc::Titre { niveau: 1, texte: "T".into(), ancre: para("", 5, 0.1).ancre() };
        let blocs = vec![para("ce que", 4, 0.8), titre];
        assert_eq!(raccorder_pages(blocs, &Vocabulaire::default()).len(), 2);
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
        let r = raccorder_pages(blocs, &Vocabulaire::default());
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
    fn raccord_ignore_l_appel_de_note_final() {
        let r = raccorder_pages(vec![para_avec_fin("la fin.", appel()), para("Suite", 5, 0.1)], &Vocabulaire::default());
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn raccord_ignore_l_exposant_final() {
        let r = raccorder_pages(vec![
            para_avec_fin("la fin.", Segment::Exposant("1".into())),
            para("Suite", 5, 0.1),
        ], &Vocabulaire::default());
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn raccord_malgre_un_appel_de_note_apres_un_mot() {
        let r = raccorder_pages(vec![para_avec_fin("ce que", appel()), para("l'on dit", 5, 0.1)], &Vocabulaire::default());
        assert_eq!(r.len(), 1);
        assert_eq!(texte_de(&r[0]), "ce que1 l'on dit");
    }

    #[test]
    fn pas_de_raccord_sur_la_meme_page() {
        let blocs = vec![para("ce que", 4, 0.1), para("l'on dit", 4, 0.3)];
        assert_eq!(raccorder_pages(blocs, &Vocabulaire::default()).len(), 2);
    }

    #[test]
    fn raccord_recolle_une_cesure() {
        let blocs = vec![para("mer-", 4, 0.8), para("veilleux", 5, 0.1)];
        let r = raccorder_pages(blocs, &Vocabulaire::default());
        assert_eq!(texte_de(&r[0]), "merveilleux");
        let blocs = vec![para("dites-", 4, 0.8), para("vous bien", 5, 0.1)];
        assert_eq!(texte_de(&raccorder_pages(blocs, &Vocabulaire::default())[0]), "dites-vous bien");
        let blocs = vec![para("le Saint-", 4, 0.8), para("Esprit", 5, 0.1)];
        assert_eq!(texte_de(&raccorder_pages(blocs, &Vocabulaire::default())[0]), "le Saint-Esprit");
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
    }
}
