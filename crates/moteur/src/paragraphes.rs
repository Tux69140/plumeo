//! Regroupement des lignes de corps en paragraphes, recollage des césures et
//! raccord des paragraphes coupés par un saut de page.

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

fn commence_par_minuscule(texte: &str) -> bool {
    texte.chars().next().is_some_and(char::is_lowercase)
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

pub(crate) fn segments_des_lignes(lignes: &[Ligne]) -> Vec<Segment> {
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
            } else if finit_par_cesure(&dernier.texte)
                && !ligne.exposants.contains(&(ligne.mots.len() - 1))
                && commence_par_minuscule(&suivante.texte())
            {
                retirer_dernier_caractere(&mut segments);
                espace_en_tete = false;
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

fn paragraphe_de(lignes: &[Ligne], page: u32, moitie: Moitie) -> Paragraphe {
    let rect = Rect {
        x0: lignes.iter().map(|l| l.rect.x0).fold(f32::MAX, f32::min),
        y0: lignes.iter().map(|l| l.rect.y0).fold(f32::MAX, f32::min),
        x1: lignes.iter().map(|l| l.rect.x1).fold(f32::MIN, f32::max),
        y1: lignes.iter().map(|l| l.rect.y1).fold(f32::MIN, f32::max),
    };
    Paragraphe {
        segments: segments_des_lignes(lignes),
        ancre: Ancre::depuis(page, moitie, rect),
        taille: mediane(lignes.iter().map(Ligne::taille).collect()),
        tout_gras: lignes.iter().all(Ligne::tout_gras),
        nb_lignes: lignes.len(),
    }
}

pub fn paragraphes_page(lignes: &[Ligne], page: u32, moitie: Moitie, bloc: BlocTexte) -> Vec<Paragraphe> {
    let mut resultat = Vec::new();
    let mut debut = 0;
    for i in 1..=lignes.len() {
        if i == lignes.len() || coupure(&lignes[i - 1], &lignes[i], bloc) {
            resultat.push(paragraphe_de(&lignes[debut..i], page, moitie));
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

/// Colle `suite` à `p` (paragraphe coupé par un saut de page).
fn fusionner(p: &mut Paragraphe, suite: Paragraphe) {
    let cesure = finit_par_cesure(&p.texte_brut())
        && matches!(p.segments.last(), Some(Segment::Texte { .. }))
        && commence_par_minuscule(&suite.texte_brut());
    if cesure {
        retirer_dernier_caractere(&mut p.segments);
    } else {
        let (gras, italique) = match p.segments.last() {
            Some(Segment::Texte { gras, italique, .. }) => (*gras, *italique),
            _ => (false, false),
        };
        ajouter_texte(&mut p.segments, " ", gras, italique);
    }
    for segment in suite.segments {
        match segment {
            Segment::Texte { texte, gras, italique } => ajouter_texte(&mut p.segments, &texte, gras, italique),
            autre => p.segments.push(autre),
        }
    }
    p.tout_gras = p.tout_gras && suite.tout_gras;
    p.nb_lignes += suite.nb_lignes;
}

/// Raccorde les paragraphes coupés par un saut de page. Les notes situées entre
/// les deux sont sautées (elles gardent leur place) ; un titre bloque le raccord.
pub fn raccorder_pages(blocs: Vec<Bloc>) -> Vec<Bloc> {
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
                fusionner(p, suite);
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
        let p = paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC);
        assert_eq!(textes(&p), vec!["un deux", "trois"]);
        assert_eq!(p[0].nb_lignes, 2);
    }

    #[test]
    fn ecart_vertical() {
        let lignes = vec![ligne("un", 0.1, 0.9, 0.10), ligne("deux", 0.1, 0.9, 0.16)];
        let p = paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC);
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn alinea() {
        let lignes = vec![
            ligne("un", 0.1, 0.9, 0.10),
            ligne("deux", 0.1 + 0.05 * 0.8, 0.9, 0.13),
        ];
        let p = paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC);
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn changement_de_taille() {
        let lignes = vec![
            ligne_taille("grand", 0.1, 0.9, 0.10, 19.0, false),
            ligne_taille("petit", 0.1, 0.9, 0.13, 15.0, false),
        ];
        assert_eq!(paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC).len(), 2);
    }

    #[test]
    fn cesure_recollee() {
        let lignes = vec![ligne("mer-", 0.1, 0.9, 0.10), ligne("veilleux", 0.1, 0.9, 0.13)];
        let p = paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC);
        assert_eq!(textes(&p), vec!["merveilleux"]);

        let lignes = vec![ligne("peut-", 0.1, 0.9, 0.10), ligne("Être", 0.1, 0.9, 0.13)];
        let p = paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC);
        assert_eq!(textes(&p), vec!["peut- Être"]);
    }

    #[test]
    fn trait_d_union_conditionnel() {
        let lignes = vec![ligne("mer\u{AD}", 0.1, 0.9, 0.10), ligne("veilleux", 0.1, 0.9, 0.13)];
        let p = paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC);
        assert_eq!(textes(&p), vec!["merveilleux"]);

        let lignes = vec![ligne("au\u{AD}jourd'hui", 0.1, 0.9, 0.10)];
        let p = paragraphes_page(&lignes, 1, Moitie::Entiere, BLOC);
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
        let p = paragraphes_page(&[l], 1, Moitie::Entiere, BLOC);
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
        let r = raccorder_pages(blocs);
        assert_eq!(r.len(), 1);
        assert_eq!(texte_de(&r[0]), "ce que l'on dit");
        assert_eq!(r[0].ancre().page, 4);

        let blocs = vec![para("la fin.", 4, 0.8), para("Suite", 5, 0.1)];
        assert_eq!(raccorder_pages(blocs).len(), 2);

        let titre = Bloc::Titre { niveau: 1, texte: "T".into(), ancre: para("", 5, 0.1).ancre() };
        let blocs = vec![para("ce que", 4, 0.8), titre];
        assert_eq!(raccorder_pages(blocs).len(), 2);
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
        let r = raccorder_pages(blocs);
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
        let r = raccorder_pages(vec![para_avec_fin("la fin.", appel()), para("Suite", 5, 0.1)]);
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn raccord_ignore_l_exposant_final() {
        let r = raccorder_pages(vec![
            para_avec_fin("la fin.", Segment::Exposant("1".into())),
            para("Suite", 5, 0.1),
        ]);
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn raccord_malgre_un_appel_de_note_apres_un_mot() {
        let r = raccorder_pages(vec![para_avec_fin("ce que", appel()), para("l'on dit", 5, 0.1)]);
        assert_eq!(r.len(), 1);
        assert_eq!(texte_de(&r[0]), "ce que1 l'on dit");
    }

    #[test]
    fn pas_de_raccord_sur_la_meme_page() {
        let blocs = vec![para("ce que", 4, 0.1), para("l'on dit", 4, 0.3)];
        assert_eq!(raccorder_pages(blocs).len(), 2);
    }

    #[test]
    fn raccord_recolle_une_cesure() {
        let blocs = vec![para("mer-", 4, 0.8), para("veilleux", 5, 0.1)];
        let r = raccorder_pages(blocs);
        assert_eq!(texte_de(&r[0]), "merveilleux");
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
