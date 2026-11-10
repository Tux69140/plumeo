//! Appariement des appels de note et des notes : notes en bas de page (lignes de la
//! zone de notes d'une page) et notes regroupées sous un titre « Notes ».
//!
//! Politique des points à vérifier : `notes_bas_de_page` ne signale que les notes
//! sans appel. Les appels (exposants) sans note ne sont signalés qu'une fois, par
//! `notes_regroupees`, après traitement du livre entier ; un `*` collé à un mot
//! sans note reste du texte, sans point.

use std::sync::LazyLock;

use regex::Regex;

use crate::blocs::{Bloc, Paragraphe, Segment};
use crate::lignes::Ligne;
use crate::modele::{Ancre, Moitie, Rect};
use crate::paragraphes::segments_des_lignes;
use crate::points::{GenrePoint, PointAVerifier};
use crate::titres::normaliser;
use crate::zones::{Retrait, Role};

static DEBUT_NOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{1,3}|\*{1,3})[\s.)]").unwrap());
static APPEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{1,3}|\*{1,3})$").unwrap());

/// Numérote les notes du livre : une étiquette `<séquence>-<marqueur>` par couple appel/note.
pub struct Etiqueteuse {
    suivant: u32,
}

impl Etiqueteuse {
    pub fn new() -> Self {
        Etiqueteuse { suivant: 1 }
    }

    pub fn etiquette(&mut self, marqueur: &str) -> String {
        let e = format!("{}-{}", self.suivant, marqueur);
        self.suivant += 1;
        e
    }
}

impl Default for Etiqueteuse {
    fn default() -> Self {
        Self::new()
    }
}

/// Une note en attente d'appariement.
struct NoteEnAttente {
    marqueur: String,
    segments: Vec<Segment>,
    ancre: Ancre,
    modele: Paragraphe,
    etiquette: Option<String>,
}

impl NoteEnAttente {
    fn nouvelle(marqueur: String, segments: Vec<Segment>, ancre: Ancre, modele: Paragraphe) -> Self {
        NoteEnAttente { marqueur, segments, ancre, modele, etiquette: None }
    }

    /// Bloc final : `Note` si appariée, sinon paragraphe « marqueur texte » et point.
    fn en_bloc(self) -> (Bloc, Option<PointAVerifier>) {
        match self.etiquette {
            Some(etiquette) => (
                Bloc::Note { etiquette, marqueur: self.marqueur, segments: self.segments, ancre: self.ancre },
                None,
            ),
            None => {
                let mut segments = self.segments;
                if !self.marqueur.is_empty() {
                    segments.insert(
                        0,
                        Segment::Texte { texte: format!("{} ", self.marqueur), gras: false, italique: false },
                    );
                }
                let message = if self.marqueur.is_empty() {
                    "Texte de zone de notes sans marqueur ni appel".to_string()
                } else {
                    format!("Note « {} » sans appel dans le texte", self.marqueur)
                };
                let point = PointAVerifier { ancre: self.ancre, genre: GenrePoint::NoteOrpheline, message };
                (Bloc::Paragraphe(Paragraphe { segments, ancre: self.ancre, ..self.modele }), Some(point))
            }
        }
    }
}

/// Appelle `apparier` pour chaque appel de note des segments (exposant ou `*` collé
/// à un mot) ; s'il renvoie une étiquette, l'appel devient `AppelNote`.
fn convertir_appels(
    segments: Vec<Segment>,
    apparier: &mut impl FnMut(&str) -> Option<String>,
) -> Vec<Segment> {
    let mut sortie = Vec::new();
    for segment in segments {
        match segment {
            Segment::Exposant(m) if APPEL.is_match(&m) => match apparier(&m) {
                Some(etiquette) => sortie.push(Segment::AppelNote { etiquette, marqueur: m }),
                None => sortie.push(Segment::Exposant(m)),
            },
            Segment::Texte { texte, gras, italique } => {
                let c: Vec<char> = texte.chars().collect();
                let mut courant = String::new();
                let mut k = 0;
                while k < c.len() {
                    if c[k] != '*' {
                        courant.push(c[k]);
                        k += 1;
                        continue;
                    }
                    let mut e = k;
                    while e < c.len() && c[e] == '*' {
                        e += 1;
                    }
                    let colle = k > 0 && c[k - 1].is_alphabetic() && e - k <= 3
                        && (e == c.len() || !c[e].is_alphanumeric());
                    let marqueur: String = c[k..e].iter().collect();
                    match colle.then(|| apparier(&marqueur)).flatten() {
                        Some(etiquette) => {
                            if !courant.is_empty() {
                                sortie.push(Segment::Texte { texte: std::mem::take(&mut courant), gras, italique });
                            }
                            sortie.push(Segment::AppelNote { etiquette, marqueur });
                        }
                        None => courant.push_str(&marqueur),
                    }
                    k = e;
                }
                if !courant.is_empty() {
                    sortie.push(Segment::Texte { texte: courant, gras, italique });
                }
            }
            autre => sortie.push(autre),
        }
    }
    sortie
}

/// Apparie un appel à la première note libre de même marqueur.
fn apparier_avec(notes: &mut [NoteEnAttente], et: &mut Etiqueteuse, marqueur: &str) -> Option<String> {
    let note = notes.iter_mut().find(|n| n.etiquette.is_none() && n.marqueur == marqueur)?;
    let e = et.etiquette(marqueur);
    note.etiquette = Some(e.clone());
    Some(e)
}

fn union_rects(lignes: &[Ligne]) -> Rect {
    Rect {
        x0: lignes.iter().map(|l| l.rect.x0).fold(f32::MAX, f32::min),
        y0: lignes.iter().map(|l| l.rect.y0).fold(f32::MAX, f32::min),
        x1: lignes.iter().map(|l| l.rect.x1).fold(f32::MIN, f32::max),
        y1: lignes.iter().map(|l| l.rect.y1).fold(f32::MIN, f32::max),
    }
}

/// Marqueur de début de note d'une ligne de la zone de notes, s'il y en a un.
fn marqueur_de_ligne(l: &Ligne) -> Option<String> {
    let premier = l.mots.first()?;
    if l.exposants.contains(&0) {
        return APPEL.is_match(&premier.texte).then(|| premier.texte.clone());
    }
    DEBUT_NOTE.captures(&l.texte()).map(|c| c[1].to_string())
}

/// La ligne sans son marqueur (ni la ponctuation qui le suit).
fn sans_marqueur(l: &Ligne, marqueur: &str) -> Ligne {
    let mut l = l.clone();
    if l.exposants.contains(&0) {
        l.mots.remove(0);
        l.exposants = l.exposants.iter().filter(|&&i| i > 0).map(|i| i - 1).collect();
    } else {
        let reste = l.mots[0].texte[marqueur.len()..].trim_start_matches(['.', ')']).to_string();
        if reste.is_empty() {
            l.mots.remove(0);
            l.exposants = l.exposants.iter().map(|i| i.saturating_sub(1)).collect();
        } else {
            l.mots[0].texte = reste;
        }
    }
    l
}

/// Apparie les appels des paragraphes d'une page avec les notes de sa zone de notes.
/// Renvoie les paragraphes (appels convertis) puis les notes, dans l'ordre des notes.
pub fn notes_bas_de_page(
    paragraphes: Vec<Paragraphe>,
    lignes_notes: &[Ligne],
    page: u32,
    moitie: Moitie,
    et: &mut Etiqueteuse,
) -> (Vec<Bloc>, Vec<PointAVerifier>) {
    // Découpe de la zone de notes en notes.
    let mut groupes: Vec<(String, Vec<Ligne>)> = Vec::new();
    for l in lignes_notes {
        match marqueur_de_ligne(l) {
            Some(m) => {
                let reste = sans_marqueur(l, &m);
                groupes.push((m, vec![reste]));
            }
            None => match groupes.last_mut() {
                Some((_, lignes)) => lignes.push(l.clone()),
                None => groupes.push((String::new(), vec![l.clone()])),
            },
        }
    }
    let mut notes: Vec<NoteEnAttente> = groupes
        .into_iter()
        .map(|(marqueur, lignes)| {
            let ancre = Ancre::depuis(page, moitie, union_rects(&lignes));
            let modele = Paragraphe {
                segments: Vec::new(),
                ancre,
                taille: lignes[0].taille(),
                tout_gras: false,
                nb_lignes: lignes.len(),
            };
            NoteEnAttente::nouvelle(marqueur, segments_des_lignes(&lignes), ancre, modele)
        })
        .collect();

    let mut blocs = Vec::new();
    for mut p in paragraphes {
        p.segments = convertir_appels(p.segments, &mut |m| apparier_avec(&mut notes, et, m));
        blocs.push(Bloc::Paragraphe(p));
    }
    let mut points = Vec::new();
    for note in notes {
        let (bloc, point) = note.en_bloc();
        blocs.push(bloc);
        points.extend(point);
    }
    (blocs, points)
}

fn est_debut_de_section(b: &Bloc) -> bool {
    match b {
        Bloc::Titre { texte, .. } => normaliser(texte) == "notes",
        Bloc::Paragraphe(p) => normaliser(&p.texte_brut()) == "notes",
        Bloc::Note { .. } => false,
    }
}

/// Sépare le marqueur de début d'un paragraphe de note regroupée.
fn extraire_marqueur(segments: &[Segment]) -> Option<(String, Vec<Segment>)> {
    match segments.first()? {
        Segment::Exposant(m) if APPEL.is_match(m) => {
            let mut suite = segments[1..].to_vec();
            if let Some(Segment::Texte { texte, .. }) = suite.first_mut() {
                *texte = texte.trim_start().to_string();
                if texte.is_empty() {
                    suite.remove(0);
                }
            }
            Some((m.clone(), suite))
        }
        Segment::Texte { texte, gras, italique } => {
            let m = DEBUT_NOTE.captures(texte)?[1].to_string();
            let reste = texte[m.len()..].trim_start_matches(['.', ')']).trim_start().to_string();
            let mut suite = Vec::new();
            if !reste.is_empty() {
                suite.push(Segment::Texte { texte: reste, gras: *gras, italique: *italique });
            }
            suite.extend_from_slice(&segments[1..]);
            Some((m, suite))
        }
        _ => None,
    }
}

enum Element {
    Bloc(Bloc),
    Note(usize),
}

/// Apparie les notes regroupées sous un titre « Notes » avec les appels qui les
/// précèdent (depuis la section précédente). Les appels restés sans note, dans
/// tout le livre, sont signalés ici.
pub fn notes_regroupees(
    blocs: Vec<Bloc>,
    et: &mut Etiqueteuse,
) -> (Vec<Bloc>, Vec<Retrait>, Vec<PointAVerifier>) {
    let mut sortie: Vec<Bloc> = Vec::new();
    let mut retraits = Vec::new();
    let mut points = Vec::new();
    let mut debut_portee = 0;
    let mut i = 0;
    while i < blocs.len() {
        if !est_debut_de_section(&blocs[i]) {
            sortie.push(blocs[i].clone());
            i += 1;
            continue;
        }
        let fin = (i + 1..blocs.len())
            .find(|&j| matches!(blocs[j], Bloc::Titre { .. }))
            .unwrap_or(blocs.len());
        let (ancre, texte) = match &blocs[i] {
            Bloc::Titre { texte, ancre, .. } => (*ancre, texte.clone()),
            Bloc::Paragraphe(p) => (p.ancre, p.texte_brut()),
            Bloc::Note { .. } => unreachable!(),
        };
        retraits.push(Retrait { ancre, role: Role::TitreNotes, texte });

        let mut notes: Vec<NoteEnAttente> = Vec::new();
        let mut elements: Vec<Element> = Vec::new();
        for bloc in &blocs[i + 1..fin] {
            let Bloc::Paragraphe(p) = bloc else {
                elements.push(Element::Bloc(bloc.clone()));
                continue;
            };
            if let Some((m, segments)) = extraire_marqueur(&p.segments) {
                elements.push(Element::Note(notes.len()));
                notes.push(NoteEnAttente::nouvelle(m, segments, p.ancre, p.clone()));
            } else if let Some(note) = notes.last_mut() {
                note.segments.push(Segment::Texte { texte: " ".into(), gras: false, italique: false });
                note.segments.extend(p.segments.iter().cloned());
            } else {
                elements.push(Element::Bloc(bloc.clone()));
            }
        }

        for bloc in &mut sortie[debut_portee..] {
            if let Bloc::Paragraphe(p) = bloc {
                p.segments = convertir_appels(std::mem::take(&mut p.segments), &mut |m| {
                    apparier_avec(&mut notes, et, m)
                });
            }
        }

        let mut notes: Vec<Option<NoteEnAttente>> = notes.into_iter().map(Some).collect();
        for element in elements {
            match element {
                Element::Bloc(b) => sortie.push(b),
                Element::Note(n) => {
                    let (bloc, point) = notes[n].take().expect("note utilisée une fois").en_bloc();
                    sortie.push(bloc);
                    points.extend(point);
                }
            }
        }
        debut_portee = sortie.len();
        i = fin;
    }

    // Appels restés sans note, dans tout le livre.
    for bloc in &sortie {
        if let Bloc::Paragraphe(p) = bloc {
            for s in &p.segments {
                if let Segment::Exposant(m) = s {
                    if APPEL.is_match(m) {
                        points.push(PointAVerifier {
                            ancre: p.ancre,
                            genre: GenrePoint::NoteOrpheline,
                            message: format!("Appel de note « {m} » sans note correspondante"),
                        });
                    }
                }
            }
        }
    }
    (sortie, retraits, points)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modele::Mot;

    fn mot(texte: &str, x0: f32, y0: f32) -> Mot {
        Mot {
            texte: texte.to_string(),
            rect: Rect { x0, y0, x1: x0 + 0.05, y1: y0 + 0.02 },
            taille: 9.0,
            gras: false,
            italique: false,
            espace_avant: true,
        }
    }

    fn ligne(texte: &str, y0: f32) -> Ligne {
        let mots: Vec<Mot> =
            texte.split(' ').enumerate().map(|(i, t)| mot(t, 0.1 + 0.06 * i as f32, y0)).collect();
        let rect = Rect {
            x0: mots[0].rect.x0,
            y0,
            x1: mots.last().unwrap().rect.x1,
            y1: y0 + 0.02,
        };
        Ligne { mots, exposants: vec![], rect }
    }

    fn rect(y0: f32) -> Rect {
        Rect { x0: 0.1, y0, x1: 0.9, y1: y0 + 0.02 }
    }

    fn texte(t: &str) -> Segment {
        Segment::Texte { texte: t.into(), gras: false, italique: false }
    }

    fn para(segments: Vec<Segment>, page: u32, y0: f32) -> Paragraphe {
        Paragraphe {
            segments,
            ancre: Ancre::depuis(page, Moitie::Entiere, rect(y0)),
            taille: 12.0,
            tout_gras: false,
            nb_lignes: 1,
        }
    }

    fn bloc_para(segments: Vec<Segment>, page: u32, y0: f32) -> Bloc {
        Bloc::Paragraphe(para(segments, page, y0))
    }

    fn titre(t: &str, niveau: u8, page: u32) -> Bloc {
        Bloc::Titre { niveau, texte: t.into(), ancre: Ancre::depuis(page, Moitie::Entiere, rect(0.1)) }
    }

    fn texte_note(b: &Bloc) -> String {
        match b {
            Bloc::Note { segments, .. } => {
                Paragraphe { segments: segments.clone(), ..para(vec![], 1, 0.0) }.texte_brut()
            }
            _ => panic!("pas une note"),
        }
    }

    #[test]
    fn etiqueteuse() {
        let mut et = Etiqueteuse::new();
        assert_eq!(et.etiquette("12"), "1-12");
        assert_eq!(et.etiquette("*"), "2-*");
    }

    #[test]
    fn note_etoile_appariee() {
        let mut et = Etiqueteuse::new();
        let p = para(vec![texte("Un mot"), Segment::Exposant("*".into())], 3, 0.2);
        let (blocs, points) =
            notes_bas_de_page(vec![p], &[ligne("* Lire la conférence", 0.9)], 3, Moitie::Entiere, &mut et);
        assert!(points.is_empty());
        assert_eq!(blocs.len(), 2);
        let Bloc::Paragraphe(p) = &blocs[0] else { panic!() };
        assert_eq!(p.segments[1], Segment::AppelNote { etiquette: "1-*".into(), marqueur: "*".into() });
        let Bloc::Note { etiquette, marqueur, ancre, .. } = &blocs[1] else { panic!() };
        assert_eq!((etiquette.as_str(), marqueur.as_str()), ("1-*", "*"));
        assert_eq!(ancre.page, 3);
        assert_eq!(texte_note(&blocs[1]), "Lire la conférence");
    }

    #[test]
    fn appel_etoile_colle_au_mot() {
        let mut et = Etiqueteuse::new();
        let p = para(vec![texte("un double* mot")], 3, 0.2);
        let (blocs, points) =
            notes_bas_de_page(vec![p], &[ligne("* Une précision", 0.9)], 3, Moitie::Entiere, &mut et);
        assert!(points.is_empty());
        let Bloc::Paragraphe(p) = &blocs[0] else { panic!() };
        assert_eq!(
            p.segments,
            vec![
                texte("un double"),
                Segment::AppelNote { etiquette: "1-*".into(), marqueur: "*".into() },
                texte(" mot"),
            ]
        );
        assert!(matches!(blocs[1], Bloc::Note { .. }));
    }

    #[test]
    fn etoile_collee_sans_note_reste_du_texte() {
        let mut et = Etiqueteuse::new();
        let p = para(vec![texte("un double* mot")], 3, 0.2);
        let (blocs, points) = notes_bas_de_page(vec![p], &[], 3, Moitie::Entiere, &mut et);
        assert!(points.is_empty());
        let Bloc::Paragraphe(p) = &blocs[0] else { panic!() };
        assert_eq!(p.segments, vec![texte("un double* mot")]);
    }

    #[test]
    fn note_sur_deux_lignes() {
        let mut et = Etiqueteuse::new();
        let p = para(vec![texte("Mot"), Segment::Exposant("1".into())], 2, 0.2);
        let lignes = [ligne("1 Voir le chapitre", 0.88), ligne("suite de la note", 0.91)];
        let (blocs, points) = notes_bas_de_page(vec![p], &lignes, 2, Moitie::Entiere, &mut et);
        assert!(points.is_empty());
        assert_eq!(blocs.len(), 2);
        assert_eq!(texte_note(&blocs[1]), "Voir le chapitre suite de la note");
        let Bloc::Note { ancre, .. } = &blocs[1] else { panic!() };
        assert!(ancre.rect[1] <= 880 && ancre.rect[3] >= 910);
    }

    #[test]
    fn appel_sans_note() {
        let mut et = Etiqueteuse::new();
        let blocs = vec![bloc_para(vec![texte("Mot"), Segment::Exposant("2".into())], 2, 0.2)];
        let (blocs, retraits, points) = notes_regroupees(blocs, &mut et);
        assert!(retraits.is_empty());
        let Bloc::Paragraphe(p) = &blocs[0] else { panic!() };
        assert_eq!(p.segments[1], Segment::Exposant("2".into()));
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].genre, GenrePoint::NoteOrpheline);
    }

    #[test]
    fn appel_sans_note_en_bas_de_page_n_est_pas_signale_par_cette_fonction() {
        let mut et = Etiqueteuse::new();
        let p = para(vec![texte("Mot"), Segment::Exposant("2".into())], 2, 0.2);
        let (blocs, points) = notes_bas_de_page(vec![p], &[], 2, Moitie::Entiere, &mut et);
        assert!(points.is_empty());
        let Bloc::Paragraphe(p) = &blocs[0] else { panic!() };
        assert_eq!(p.segments[1], Segment::Exposant("2".into()));
    }

    #[test]
    fn note_sans_appel() {
        let mut et = Etiqueteuse::new();
        let p = para(vec![texte("Rien ici")], 2, 0.2);
        let (blocs, points) =
            notes_bas_de_page(vec![p], &[ligne("3 texte", 0.9)], 2, Moitie::Entiere, &mut et);
        assert_eq!(blocs.len(), 2);
        let Bloc::Paragraphe(p) = &blocs[1] else { panic!("attendu un paragraphe") };
        assert_eq!(p.texte_brut(), "3 texte");
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].genre, GenrePoint::NoteOrpheline);
    }

    #[test]
    fn liste_numerotee_hors_zone_reste_texte() {
        let mut et = Etiqueteuse::new();
        let p1 = para(vec![texte("1. Premier point")], 2, 0.2);
        let p2 = para(vec![texte("2. Second point")], 2, 0.3);
        let (blocs, points) = notes_bas_de_page(vec![p1, p2], &[], 2, Moitie::Entiere, &mut et);
        assert!(points.is_empty());
        assert_eq!(blocs.len(), 2);
        assert!(blocs.iter().all(|b| matches!(b, Bloc::Paragraphe(_))));
    }

    fn corps_avec_appels(page: u32) -> Bloc {
        bloc_para(
            vec![texte("Un"), Segment::Exposant("1".into()), texte(" deux"), Segment::Exposant("2".into())],
            page,
            0.2,
        )
    }

    #[test]
    fn section_notes_regroupees() {
        let mut et = Etiqueteuse::new();
        let blocs = vec![
            titre("Chapitre", 1, 1),
            corps_avec_appels(1),
            titre("NOTES", 2, 2),
            bloc_para(vec![texte("1 Texte un")], 2, 0.2),
            bloc_para(vec![texte("2 Texte deux")], 2, 0.3),
        ];
        let (blocs, retraits, points) = notes_regroupees(blocs, &mut et);
        assert!(points.is_empty());
        assert_eq!(retraits.len(), 1);
        assert_eq!(retraits[0].role, Role::TitreNotes);
        assert_eq!(retraits[0].texte, "NOTES");
        assert_eq!(blocs.len(), 4);
        let Bloc::Paragraphe(p) = &blocs[1] else { panic!() };
        assert_eq!(p.segments[1], Segment::AppelNote { etiquette: "1-1".into(), marqueur: "1".into() });
        assert_eq!(p.segments[3], Segment::AppelNote { etiquette: "2-2".into(), marqueur: "2".into() });
        let Bloc::Note { etiquette, marqueur, .. } = &blocs[2] else { panic!() };
        assert_eq!((etiquette.as_str(), marqueur.as_str()), ("1-1", "1"));
        assert_eq!(texte_note(&blocs[2]), "Texte un");
        assert!(matches!(&blocs[3], Bloc::Note { etiquette, .. } if etiquette == "2-2"));
    }

    #[test]
    fn numerotation_qui_repart() {
        let mut et = Etiqueteuse::new();
        let chapitre = |n: u32| {
            vec![
                titre(&format!("Chapitre {n}"), 1, n * 10),
                bloc_para(vec![texte("Mot"), Segment::Exposant("1".into())], n * 10, 0.2),
                titre("Notes", 2, n * 10 + 1),
                bloc_para(vec![texte(&format!("1 Note du chapitre {n}"))], n * 10 + 1, 0.2),
            ]
        };
        let mut blocs = chapitre(1);
        blocs.extend(chapitre(2));
        let (blocs, retraits, points) = notes_regroupees(blocs, &mut et);
        assert!(points.is_empty());
        assert_eq!(retraits.len(), 2);
        let etiquettes: Vec<&str> = blocs
            .iter()
            .filter_map(|b| match b {
                Bloc::Note { etiquette, .. } => Some(etiquette.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(etiquettes, vec!["1-1", "2-1"]);
        let Bloc::Paragraphe(p) = &blocs[4] else { panic!() };
        assert!(matches!(&p.segments[1], Segment::AppelNote { etiquette, .. } if etiquette == "2-1"));
        assert_eq!(texte_note(&blocs[2]), "Note du chapitre 1");
    }

    #[test]
    fn notes_regroupees_prolongees_et_orphelines() {
        let mut et = Etiqueteuse::new();
        let blocs = vec![
            bloc_para(vec![texte("Mot"), Segment::Exposant("1".into())], 1, 0.2),
            bloc_para(vec![texte("Notes")], 2, 0.1),
            bloc_para(vec![texte("1 Début")], 2, 0.2),
            bloc_para(vec![texte("fin de la note")], 2, 0.3),
            bloc_para(vec![Segment::Exposant("7".into()), texte(" Sans appel")], 2, 0.4),
        ];
        let (blocs, retraits, points) = notes_regroupees(blocs, &mut et);
        assert_eq!(retraits.len(), 1);
        assert_eq!(texte_note(&blocs[1]), "Début fin de la note");
        let Bloc::Paragraphe(p) = &blocs[2] else { panic!() };
        assert_eq!(p.texte_brut(), "7 Sans appel");
        assert_eq!(points.len(), 1);
    }

    #[test]
    fn note_regroupee_a_marqueur_exposant_sans_espace_en_tete() {
        let mut et = Etiqueteuse::new();
        let blocs = vec![
            bloc_para(vec![texte("Mot"), Segment::Exposant("7".into())], 1, 0.2),
            titre("Notes", 2, 2),
            bloc_para(vec![Segment::Exposant("7".into()), texte(" Texte sept")], 2, 0.2),
        ];
        let (blocs, _, points) = notes_regroupees(blocs, &mut et);
        assert!(points.is_empty());
        assert_eq!(texte_note(&blocs[1]), "Texte sept");
    }
}
