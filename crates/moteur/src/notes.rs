//! Appariement des appels de note et des notes : notes en bas de page (lignes de la
//! zone de notes d'une page) et notes regroupées sous un titre « Notes » ou, sans
//! titre, à partir de la note 1 (notes Calibre « [1] » en fin de livre).
//!
//! Politique des points à vérifier : `notes_bas_de_page` ne signale que les notes
//! sans appel. Les appels (exposants) sans note ne sont signalés qu'une fois, par
//! `notes_regroupees`, après traitement du livre entier ; un `*` collé à un mot
//! sans note reste du texte, sans point.

use std::sync::LazyLock;

use plumeo_commun::normaliser;
use regex::Regex;

use crate::blocs::{Bloc, Paragraphe, Segment};
use crate::lignes::Ligne;
use crate::modele::{Ancre, Moitie, Rect};
use crate::paragraphes::{Vocabulaire, fin_ouverte, joindre, segments_des_lignes};
use crate::points::{GenrePoint, PointAVerifier};
use crate::zones::{Retrait, Role};

/// Début de note : chiffres ou « * » suivis d'une espace, d'un point ou d'une
/// parenthèse, ou numéro entre crochets (Calibre : « [3] ») seul ou suivi d'une espace.
static DEBUT_NOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:(\d{1,3}|\*{1,3})[\s.)]|\[(\d{1,3})\](?:\s|$))").unwrap());
/// Appel de note en exposant : « 3 », « * » ou « [3] ».
static APPEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:(\d{1,3}|\*{1,3})|\[(\d{1,3})\])$").unwrap());

/// Marqueur d'un appel de note en exposant, sans crochets.
fn marqueur_d_appel(texte: &str) -> Option<String> {
    let c = APPEL.captures(texte)?;
    Some(c.get(1).or(c.get(2))?.as_str().to_string())
}

/// Marqueur de note en tête de `texte`, sans crochets, et longueur (en octets) du
/// texte qui le porte, crochets compris.
fn debut_de_note(texte: &str) -> Option<(String, usize)> {
    let c = DEBUT_NOTE.captures(texte)?;
    match (c.get(1), c.get(2)) {
        (Some(m), _) => Some((m.as_str().to_string(), m.end())),
        (None, Some(m)) => Some((m.as_str().to_string(), m.end() + ']'.len_utf8())),
        (None, None) => None,
    }
}

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

/// Ponctuation après laquelle un « * » peut être un appel de note.
const AVANT_ETOILE: [char; 10] = ['.', ',', ';', ':', '!', '?', '\u{BB}', ')', '"', '\u{2019}'];

/// Fin (exclue) de l'appel de note qui commence à `k`, s'il y en a un :
/// - 1 à 3 « * » après une lettre ou une ponctuation finale, collés ou séparés par une espace ;
/// - 1 à 3 chiffres collés à un mot d'au moins deux lettres finissant par une minuscule
///   (exposant lu comme un chiffre par l'OCR : « Hartmann2 »).
///
/// L'appel n'est jamais suivi d'une lettre ou d'un chiffre.
fn fin_d_appel(c: &[char], k: usize) -> Option<usize> {
    let lettre = |i: usize| c.get(i).is_some_and(|x| x.is_alphabetic());
    let (e, permis) = if c[k] == '*' {
        let e = (k..c.len()).find(|&i| c[i] != '*').unwrap_or(c.len());
        let avant = match k.checked_sub(1).map(|i| (i, c[i])) {
            Some((i, ' ')) => i.checked_sub(1).map(|j| c[j]),
            autre => autre.map(|(_, x)| x),
        };
        let permis = (k == 0 || c[k - 1] != '*')
            && e - k <= 3
            && avant.is_some_and(|x| x.is_alphabetic() || AVANT_ETOILE.contains(&x));
        (e, permis)
    } else if c[k].is_ascii_digit() {
        let e = (k..c.len()).find(|&i| !c[i].is_ascii_digit()).unwrap_or(c.len());
        let permis = k >= 2 && e - k <= 3 && c[k - 1].is_lowercase() && lettre(k - 2);
        (e, permis)
    } else {
        return None;
    };
    (permis && c.get(e).is_none_or(|x| !x.is_alphanumeric())).then_some(e)
}

/// Appelle `apparier` pour chaque appel de note des segments (exposant, « * » ou
/// chiffres collés : voir `fin_d_appel`) ; s'il renvoie une étiquette, l'appel
/// devient `AppelNote`. Sans note correspondante, le texte reste inchangé.
fn convertir_appels(
    segments: Vec<Segment>,
    apparier: &mut impl FnMut(&str) -> Option<String>,
) -> Vec<Segment> {
    let mut sortie = Vec::new();
    for segment in segments {
        match segment {
            Segment::Exposant(m) => {
                let appel = marqueur_d_appel(&m).and_then(|marqueur| Some((apparier(&marqueur)?, marqueur)));
                sortie.push(match appel {
                    Some((etiquette, marqueur)) => Segment::AppelNote { etiquette, marqueur },
                    None => Segment::Exposant(m),
                });
            }
            Segment::Texte { texte, gras, italique } => {
                let c: Vec<char> = texte.chars().collect();
                let mut courant = String::new();
                let mut k = 0;
                while k < c.len() {
                    if let Some(e) = fin_d_appel(&c, k) {
                        let marqueur: String = c[k..e].iter().collect();
                        if let Some(etiquette) = apparier(&marqueur) {
                            let tete = courant.trim_end().len();
                            courant.truncate(tete);
                            if !courant.is_empty() {
                                sortie.push(Segment::Texte { texte: std::mem::take(&mut courant), gras, italique });
                            }
                            sortie.push(Segment::AppelNote { etiquette, marqueur });
                            k = e;
                            continue;
                        }
                    }
                    courant.push(c[k]);
                    k += 1;
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

/// Marqueur de début de note d'une ligne de la zone de notes, s'il y en a un, et
/// longueur (en octets) du texte qui le porte dans le premier mot.
fn marqueur_de_ligne(l: &Ligne) -> Option<(String, usize)> {
    let premier = l.mots.first()?;
    if l.exposants.contains(&0) {
        return marqueur_d_appel(&premier.texte).map(|m| (m, premier.texte.len()));
    }
    debut_de_note(&l.texte())
}

/// La ligne sans son marqueur (`longueur` octets en tête du premier mot) ni la
/// ponctuation qui le suit.
fn sans_marqueur(l: &Ligne, longueur: usize) -> Ligne {
    let mut l = l.clone();
    if l.exposants.contains(&0) {
        l.mots.remove(0);
        l.exposants = l.exposants.iter().filter(|&&i| i > 0).map(|i| i - 1).collect();
    } else {
        let reste = l.mots[0].texte[longueur..].trim_start_matches(['.', ')']).to_string();
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
    vocab: &Vocabulaire,
) -> (Vec<Bloc>, Vec<PointAVerifier>) {
    // Découpe de la zone de notes en notes.
    let mut groupes: Vec<(String, Vec<Ligne>)> = Vec::new();
    for l in lignes_notes {
        match marqueur_de_ligne(l) {
            Some((m, longueur)) => {
                let reste = sans_marqueur(l, longueur);
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
            NoteEnAttente::nouvelle(marqueur, segments_des_lignes(&lignes, vocab), ancre, modele)
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
        Bloc::Note { .. } | Bloc::Saut { .. } | Bloc::EntreeSommaire { .. } => false,
    }
}

/// Sépare le marqueur de début d'un paragraphe de note regroupée.
fn extraire_marqueur(segments: &[Segment]) -> Option<(String, Vec<Segment>)> {
    match segments.first()? {
        Segment::Exposant(m) => {
            let m = marqueur_d_appel(m)?;
            let mut suite = segments[1..].to_vec();
            if let Some(Segment::Texte { texte, .. }) = suite.first_mut() {
                *texte = texte.trim_start().to_string();
                if texte.is_empty() {
                    suite.remove(0);
                }
            }
            Some((m, suite))
        }
        Segment::Texte { texte, gras, italique } => {
            let (m, longueur) = debut_de_note(texte)?;
            let reste = texte[longueur..].trim_start_matches(['.', ')']).trim_start().to_string();
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

/// Note suivante restée collée après une fin de phrase : « 6. » ou « [6] » (groupe 1 :
/// le marqueur, groupe 2 ou 3 : son numéro).
static NOTE_SUIVANTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[.)»]\s+((\d{1,3})\.\s|\[(\d{1,3})\](?:\s|$))").unwrap());

/// Détache d'un paragraphe de note numérotée les notes suivantes qui y sont restées
/// collées (« … introduction. 6. Kant : … », « … 5e partie. [32] ») : seul le numéro
/// qui suit le précédent (n + 1), après une fin de phrase, ouvre une nouvelle note.
fn notes_enchainees(marqueur: String, segments: Vec<Segment>) -> Vec<(String, Vec<Segment>)> {
    let mut sortie = Vec::new();
    let (mut marqueur, mut reste) = (marqueur, segments);
    while let Ok(numero) = marqueur.parse::<u32>() {
        let suivant = (numero + 1).to_string();
        let coupe = reste.iter().enumerate().find_map(|(i, s)| match s {
            Segment::Texte { texte, .. } => NOTE_SUIVANTE
                .captures_iter(texte)
                .find(|c| c.get(2).or(c.get(3)).is_some_and(|n| n.as_str() == suivant))
                .map(|c| (i, c.get(1).expect("groupe 1").start(), c.get(0).expect("groupe 0").end())),
            _ => None,
        });
        let Some((i, debut, fin)) = coupe else { break };
        let Segment::Texte { texte, gras, italique } = reste[i].clone() else { unreachable!() };
        let mut note: Vec<Segment> = reste[..i].to_vec();
        let avant = texte[..debut].trim_end();
        if !avant.is_empty() {
            note.push(Segment::Texte { texte: avant.to_string(), gras, italique });
        }
        let mut suite = Vec::new();
        if fin < texte.len() {
            suite.push(Segment::Texte { texte: texte[fin..].to_string(), gras, italique });
        }
        suite.extend_from_slice(&reste[i + 1..]);
        sortie.push((std::mem::replace(&mut marqueur, suivant), note));
        reste = suite;
    }
    sortie.push((marqueur, reste));
    sortie
}

/// Nombre de blocs, après celui qui ouvre la note 1, où chercher les notes 2 et 3
/// d'une section de notes implicite.
const PORTEE_SECTION_IMPLICITE: usize = 8;

/// Le paragraphe ouvre-t-il la note `numero` ?
fn ouvre_la_note(b: &Bloc, numero: &str) -> bool {
    matches!(b, Bloc::Paragraphe(p) if extraire_marqueur(&p.segments).is_some_and(|(m, _)| m == numero))
}

/// Les notes `attendues` trouvent-elles toutes un appel dans `portee` ? Appariement à
/// blanc, à la condition de l'appariement réel (`apparier_avec`) : un appel ne compte
/// que si une note de ce numéro attend encore, et chaque note prend un seul appel.
fn toutes_appelees(portee: &[Bloc], attendues: &[&str]) -> bool {
    let mut libres = attendues.to_vec();
    for b in portee {
        if libres.is_empty() {
            break;
        }
        if let Bloc::Paragraphe(p) = b {
            convertir_appels(p.segments.clone(), &mut |m| {
                let k = libres.iter().position(|&n| n == m)?;
                libres.remove(k);
                Some(String::new())
            });
        }
    }
    libres.is_empty()
}

/// Section de notes sans titre « Notes » (notes Calibre en fin de livre) : elle s'ouvre
/// au paragraphe `blocs[i]` de la note 1 si les notes 2 et 3 ouvrent des paragraphes
/// parmi les 8 blocs suivants et si les appels « 1 », « 2 » et « 3 » les précèdent
/// dans la portée (un appel « 1 » égaré ne suffit pas à faire d'une liste des notes).
fn ouvre_section_implicite(blocs: &[Bloc], i: usize, portee: &[Bloc]) -> bool {
    if !ouvre_la_note(&blocs[i], "1") {
        return false;
    }
    let suivants = &blocs[i + 1..blocs.len().min(i + 1 + PORTEE_SECTION_IMPLICITE)];
    ["2", "3"].iter().all(|n| suivants.iter().any(|b| ouvre_la_note(b, n)))
        && toutes_appelees(portee, &["1", "2", "3"])
}

/// La (demi-)page `page` porte-t-elle un paragraphe qui ouvre une note, parmi `blocs` ?
fn page_a_marqueur(blocs: &[Bloc], page: (u32, Moitie)) -> bool {
    blocs.iter().any(|b| match b {
        Bloc::Paragraphe(p) => p.ancre.demi_page() == page && extraire_marqueur(&p.segments).is_some(),
        _ => false,
    })
}

/// Un titre termine la section de notes, sauf un intertitre (niveau 2 ou plus,
/// « Chapitre 1 ») directement suivi d'un paragraphe qui commence par un marqueur
/// de note : les notes de fin de livre sont souvent groupées par chapitre.
fn termine_la_section(blocs: &[Bloc], j: usize) -> bool {
    let Bloc::Titre { niveau, .. } = &blocs[j] else { return false };
    let suivi_d_une_note =
        matches!(blocs.get(j + 1), Some(Bloc::Paragraphe(p)) if extraire_marqueur(&p.segments).is_some());
    *niveau == 1 || est_debut_de_section(&blocs[j]) || !suivi_d_une_note
}

enum Element {
    Bloc(Bloc),
    Note(usize),
}

/// Apparie les notes regroupées sous un titre « Notes », ou dans une section implicite
/// (`ouvre_section_implicite`), avec les appels qui les précèdent (depuis la section
/// précédente). Les appels restés sans note, dans tout le livre, sont signalés ici.
pub fn notes_regroupees(
    blocs: Vec<Bloc>,
    et: &mut Etiqueteuse,
    vocab: &Vocabulaire,
) -> (Vec<Bloc>, Vec<Retrait>, Vec<PointAVerifier>) {
    let mut sortie: Vec<Bloc> = Vec::new();
    let mut retraits = Vec::new();
    let mut points = Vec::new();
    let mut debut_portee = 0;
    let mut i = 0;
    while i < blocs.len() {
        let implicite = !est_debut_de_section(&blocs[i]);
        let debut_notes = if !implicite {
            let (ancre, texte) = match &blocs[i] {
                Bloc::Titre { texte, ancre, .. } => (*ancre, texte.clone()),
                Bloc::Paragraphe(p) => (p.ancre, p.texte_brut()),
                Bloc::Note { .. } | Bloc::Saut { .. } | Bloc::EntreeSommaire { .. } => unreachable!(),
            };
            retraits.push(Retrait { ancre, role: Role::TitreNotes, texte });
            i + 1
        } else if ouvre_section_implicite(&blocs, i, &sortie[debut_portee..]) {
            // Pas de titre à retirer : la section commence à la note 1.
            i
        } else {
            sortie.push(blocs[i].clone());
            i += 1;
            continue;
        };
        let mut fin = (i + 1..blocs.len()).find(|&j| termine_la_section(&blocs, j)).unwrap_or(blocs.len());

        let mut notes: Vec<NoteEnAttente> = Vec::new();
        let mut elements: Vec<Element> = Vec::new();
        // (Demi-)page du dernier paragraphe pris dans les notes.
        let mut derniere: Option<(u32, Moitie)> = None;
        for (j, bloc) in blocs.iter().enumerate().take(fin).skip(debut_notes) {
            let Bloc::Paragraphe(p) = bloc else {
                elements.push(Element::Bloc(bloc.clone()));
                continue;
            };
            let page = (p.ancre.page, p.ancre.moitie);
            // Une (demi-)page sans aucun marqueur de note n'appartient pas à la section :
            // elle se ferme avant elle (épigraphe, fin de livre après des notes).
            if derniere.is_some_and(|d| d != page) && !page_a_marqueur(&blocs[j..fin], page) {
                fin = j;
                break;
            }
            let nouvelles = if let Some((m, segments)) = extraire_marqueur(&p.segments) {
                notes_enchainees(m, segments)
            } else if let Some(note) = notes.last_mut() {
                // Section implicite : un paragraphe sans marqueur ne continue la note que
                // si elle est encore vide (marqueur seul) ou si elle reste ouverte en bas
                // de (demi-)page et qu'il ouvre la suivante. Sinon la section s'arrête là.
                if implicite
                    && !note.segments.is_empty()
                    && !(derniere != Some(page) && fin_ouverte(&note.segments))
                {
                    fin = j;
                    break;
                }
                // Suite de la note précédente (ou son texte, si son marqueur était seul),
                // qui peut contenir le début de la suivante.
                if note.segments.is_empty() {
                    note.segments = p.segments.clone();
                } else {
                    joindre(&mut note.segments, p.segments.clone(), vocab);
                }
                let mut parties = notes_enchainees(note.marqueur.clone(), std::mem::take(&mut note.segments));
                note.segments = parties.remove(0).1;
                parties
            } else {
                elements.push(Element::Bloc(bloc.clone()));
                Vec::new()
            };
            for (m, segments) in nouvelles {
                elements.push(Element::Note(notes.len()));
                notes.push(NoteEnAttente::nouvelle(m, segments, p.ancre, p.clone()));
            }
            derniere = Some(page);
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
            notes_bas_de_page(vec![p], &[ligne("* Lire la conférence", 0.9)], 3, Moitie::Entiere, &mut et, &Vocabulaire::default());
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
            notes_bas_de_page(vec![p], &[ligne("* Une précision", 0.9)], 3, Moitie::Entiere, &mut et, &Vocabulaire::default());
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
    fn appel_etoile_apres_ponctuation_et_espace() {
        // Scan OCR : l'appel « * » suit un point, séparé par une espace.
        let mut et = Etiqueteuse::new();
        let p = para(vec![texte("le corps éthérique. * Le corps")], 36, 0.2);
        let (blocs, points) =
            notes_bas_de_page(vec![p], &[ligne("* Lire la conférence", 0.9)], 36, Moitie::Entiere, &mut et, &Vocabulaire::default());
        assert!(points.is_empty());
        let Bloc::Paragraphe(p) = &blocs[0] else { panic!() };
        assert_eq!(
            p.segments,
            vec![
                texte("le corps éthérique."),
                Segment::AppelNote { etiquette: "1-*".into(), marqueur: "*".into() },
                texte(" Le corps"),
            ]
        );
    }

    #[test]
    fn appel_en_chiffres_colles_au_mot() {
        // Scan OCR : l'exposant est lu comme un chiffre collé au mot (« Hartmann2 »).
        let mut et = Etiqueteuse::new();
        let blocs = vec![
            bloc_para(vec![texte("pour Hartmann2 la vérité, le CO2 et 1923.")], 8, 0.2),
            titre("Notes", 2, 91),
            bloc_para(vec![texte("2. Edouard von Hartmann (1842-1906).")], 91, 0.2),
        ];
        let (blocs, _, points) = notes_regroupees(blocs, &mut et, &Vocabulaire::default());
        assert!(points.is_empty());
        let Bloc::Paragraphe(p) = &blocs[0] else { panic!() };
        assert_eq!(
            p.segments,
            vec![
                texte("pour Hartmann"),
                Segment::AppelNote { etiquette: "1-2".into(), marqueur: "2".into() },
                texte(" la vérité, le CO2 et 1923."),
            ]
        );
        assert_eq!(texte_note(&blocs[1]), "Edouard von Hartmann (1842-1906).");
    }

    #[test]
    fn etoile_collee_sans_note_reste_du_texte() {
        let mut et = Etiqueteuse::new();
        let p = para(vec![texte("un double* mot")], 3, 0.2);
        let (blocs, points) = notes_bas_de_page(vec![p], &[], 3, Moitie::Entiere, &mut et, &Vocabulaire::default());
        assert!(points.is_empty());
        let Bloc::Paragraphe(p) = &blocs[0] else { panic!() };
        assert_eq!(p.segments, vec![texte("un double* mot")]);
    }

    #[test]
    fn note_sur_deux_lignes() {
        let mut et = Etiqueteuse::new();
        let p = para(vec![texte("Mot"), Segment::Exposant("1".into())], 2, 0.2);
        let lignes = [ligne("1 Voir le chapitre", 0.88), ligne("suite de la note", 0.91)];
        let (blocs, points) = notes_bas_de_page(vec![p], &lignes, 2, Moitie::Entiere, &mut et, &Vocabulaire::default());
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
        let (blocs, retraits, points) = notes_regroupees(blocs, &mut et, &Vocabulaire::default());
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
        let (blocs, points) = notes_bas_de_page(vec![p], &[], 2, Moitie::Entiere, &mut et, &Vocabulaire::default());
        assert!(points.is_empty());
        let Bloc::Paragraphe(p) = &blocs[0] else { panic!() };
        assert_eq!(p.segments[1], Segment::Exposant("2".into()));
    }

    #[test]
    fn note_sans_appel() {
        let mut et = Etiqueteuse::new();
        let p = para(vec![texte("Rien ici")], 2, 0.2);
        let (blocs, points) =
            notes_bas_de_page(vec![p], &[ligne("3 texte", 0.9)], 2, Moitie::Entiere, &mut et, &Vocabulaire::default());
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
        let (blocs, points) = notes_bas_de_page(vec![p1, p2], &[], 2, Moitie::Entiere, &mut et, &Vocabulaire::default());
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
        let (blocs, retraits, points) = notes_regroupees(blocs, &mut et, &Vocabulaire::default());
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
        let (blocs, retraits, points) = notes_regroupees(blocs, &mut et, &Vocabulaire::default());
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
    fn notes_de_fin_de_livre_coupees_par_des_intertitres() {
        let mut et = Etiqueteuse::new();
        let blocs = vec![
            titre("Chapitre 1", 1, 1),
            bloc_para(vec![texte("Mot"), Segment::Exposant("1".into())], 1, 0.2),
            titre("Chapitre 2", 1, 2),
            bloc_para(vec![texte("Autre"), Segment::Exposant("1".into())], 2, 0.2),
            titre("Notes", 1, 3),
            titre("Chapitre 1", 2, 3),
            bloc_para(vec![texte("1 Note du premier")], 3, 0.2),
            titre("Chapitre 2", 2, 3),
            bloc_para(vec![texte("1 Note du second")], 3, 0.4),
            titre("Suite", 2, 4),
            bloc_para(vec![texte("Texte ordinaire.")], 4, 0.2),
        ];
        let (blocs, retraits, points) = notes_regroupees(blocs, &mut et, &Vocabulaire::default());
        assert!(points.is_empty(), "{points:?}");
        assert_eq!(retraits.len(), 1);
        let notes: Vec<(&str, String)> = blocs
            .iter()
            .filter_map(|b| match b {
                Bloc::Note { etiquette, .. } => Some((etiquette.as_str(), texte_note(b))),
                _ => None,
            })
            .collect();
        assert_eq!(notes, vec![("1-1", "Note du premier".into()), ("2-1", "Note du second".into())]);
        // Les intertitres de la section restent des titres ; la section s'arrête au
        // titre qui n'est pas suivi d'une note.
        let titres = blocs.iter().filter(|b| matches!(b, Bloc::Titre { niveau: 2, .. })).count();
        assert_eq!(titres, 3);
        assert!(matches!(blocs.last(), Some(Bloc::Paragraphe(p)) if p.texte_brut() == "Texte ordinaire."));
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
        let (blocs, retraits, points) = notes_regroupees(blocs, &mut et, &Vocabulaire::default());
        assert_eq!(retraits.len(), 1);
        assert_eq!(texte_note(&blocs[1]), "Début fin de la note");
        let Bloc::Paragraphe(p) = &blocs[2] else { panic!() };
        assert_eq!(p.texte_brut(), "7 Sans appel");
        assert_eq!(points.len(), 1);
    }

    #[test]
    fn notes_regroupees_enchainees_dans_un_paragraphe() {
        // Scan : deux notes numérotées sans retrait ni blanc entre elles forment un seul
        // paragraphe ; la note suivante (numéro + 1) en est détachée.
        let mut et = Etiqueteuse::new();
        let blocs = vec![
            bloc_para(
                vec![texte("Kant"), Segment::Exposant("5".into()), texte(" et Kant"), Segment::Exposant("6".into())],
                1,
                0.2,
            ),
            titre("Notes", 2, 2),
            bloc_para(vec![texte("5. Kant (1724-1804); «Critique», introduction. 6. Kant: voir note 5.")], 2, 0.2),
        ];
        let (blocs, _, points) = notes_regroupees(blocs, &mut et, &Vocabulaire::default());
        assert!(points.is_empty());
        assert_eq!(blocs.len(), 3);
        assert_eq!(texte_note(&blocs[1]), "Kant (1724-1804); «Critique», introduction.");
        assert_eq!(texte_note(&blocs[2]), "Kant: voir note 5.");

        // La note suivante peut aussi arriver dans un paragraphe de suite (retrait suspendu).
        let blocs = vec![
            bloc_para(vec![texte("Wolff"), Segment::Exposant("3".into()), texte(" et Hume"), Segment::Exposant("4".into())], 1, 0.2),
            titre("Notes", 2, 2),
            bloc_para(vec![texte("3. Christian Wolff, «Philosophia rationalis sive")], 2, 0.2),
            bloc_para(vec![texte("logica» 1728. 4. David Hume, 1739.")], 2, 0.3),
        ];
        let (blocs, _, points) = notes_regroupees(blocs, &mut et, &Vocabulaire::default());
        assert!(points.is_empty());
        assert_eq!(texte_note(&blocs[1]), "Christian Wolff, «Philosophia rationalis sive logica» 1728.");
        assert_eq!(texte_note(&blocs[2]), "David Hume, 1739.");
    }

    #[test]
    fn suite_de_note_regroupee_recolle_la_cesure() {
        let mut et = Etiqueteuse::new();
        let blocs = vec![
            bloc_para(vec![texte("Baumann"), Segment::Exposant("23".into())], 1, 0.2),
            titre("Notes", 2, 2),
            bloc_para(vec![texte("23. Julius Baumann, professeur à l'uni-")], 2, 0.2),
            bloc_para(vec![texte("versité de Göttingen, philo-")], 2, 0.3),
            bloc_para(vec![texte("sophe.")], 2, 0.4),
        ];
        let (blocs, _, points) = notes_regroupees(blocs, &mut et, &Vocabulaire::default());
        assert!(points.is_empty());
        assert_eq!(texte_note(&blocs[1]), "Julius Baumann, professeur à l'université de Göttingen, philosophe.");
    }

    #[test]
    fn note_regroupee_a_marqueur_exposant_sans_espace_en_tete() {
        let mut et = Etiqueteuse::new();
        let blocs = vec![
            bloc_para(vec![texte("Mot"), Segment::Exposant("7".into())], 1, 0.2),
            titre("Notes", 2, 2),
            bloc_para(vec![Segment::Exposant("7".into()), texte(" Texte sept")], 2, 0.2),
        ];
        let (blocs, _, points) = notes_regroupees(blocs, &mut et, &Vocabulaire::default());
        assert!(points.is_empty());
        assert_eq!(texte_note(&blocs[1]), "Texte sept");
    }

    fn notes_du_livre(blocs: &[Bloc]) -> Vec<(String, String, String)> {
        blocs
            .iter()
            .filter_map(|b| match b {
                Bloc::Note { etiquette, marqueur, .. } => Some((etiquette.clone(), marqueur.clone(), texte_note(b))),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn notes_calibre_entre_crochets_sans_titre() {
        // Calibre : appels « [1] » en exposant, notes en fin de livre sans titre « Notes » ;
        // le marqueur est seul dans son paragraphe, ou collé à la fin de la note précédente.
        let mut et = Etiqueteuse::new();
        let blocs = vec![
            titre("Chapitre", 1, 1),
            bloc_para(
                vec![
                    texte("Un"),
                    Segment::Exposant("[1]".into()),
                    texte(" deux"),
                    Segment::Exposant("[2]".into()),
                    texte(" trois"),
                    Segment::Exposant("[3]".into()),
                    texte(" quatre"),
                    Segment::Exposant("[4]".into()),
                    texte(" ."),
                ],
                1,
                0.2,
            ),
            Bloc::Saut { ancre: Ancre::depuis(2, Moitie::Entiere, rect(0.1)) },
            bloc_para(vec![texte("[1]")], 2, 0.1),
            bloc_para(vec![texte("Texte un.")], 2, 0.15),
            bloc_para(vec![texte("[2]")], 2, 0.2),
            bloc_para(vec![texte("Texte deux.")], 2, 0.25),
            bloc_para(vec![texte("[3]")], 2, 0.3),
            bloc_para(vec![texte("Voir le tome 26, 5e partie. [4]")], 2, 0.35),
            bloc_para(vec![texte("Texte quatre.")], 2, 0.4),
        ];
        let (blocs, retraits, points) = notes_regroupees(blocs, &mut et, &Vocabulaire::default());
        assert!(points.is_empty(), "{points:?}");
        assert!(retraits.is_empty());
        let Bloc::Paragraphe(p) = &blocs[1] else { panic!() };
        assert_eq!(p.segments[1], Segment::AppelNote { etiquette: "1-1".into(), marqueur: "1".into() });
        assert_eq!(p.segments[3], Segment::AppelNote { etiquette: "2-2".into(), marqueur: "2".into() });
        assert_eq!(p.segments[7], Segment::AppelNote { etiquette: "4-4".into(), marqueur: "4".into() });
        assert_eq!(
            notes_du_livre(&blocs),
            vec![
                ("1-1".into(), "1".into(), "Texte un.".into()),
                ("2-2".into(), "2".into(), "Texte deux.".into()),
                ("3-3".into(), "3".into(), "Voir le tome 26, 5e partie.".into()),
                ("4-4".into(), "4".into(), "Texte quatre.".into()),
            ]
        );
        assert!(matches!(blocs[2], Bloc::Saut { .. }));
        assert_eq!(blocs.len(), 7);
    }

    /// Paragraphe du corps portant les appels en exposant `marqueurs`.
    fn appels(marqueurs: &[&str]) -> Bloc {
        let segments =
            marqueurs.iter().flat_map(|m| [texte(" mot"), Segment::Exposant((*m).into())]).collect();
        bloc_para(segments, 1, 0.2)
    }

    /// Liste « k. Point k » de 1 à `n`.
    fn liste(n: usize) -> Vec<Bloc> {
        (1..=n).map(|k| bloc_para(vec![texte(&format!("{k}. Point {k}"))], 4, 0.1 * k as f32)).collect()
    }

    fn sans_section(blocs: Vec<Bloc>) {
        let (sortie, retraits, _) = notes_regroupees(blocs.clone(), &mut Etiqueteuse::new(), &Vocabulaire::default());
        assert!(retraits.is_empty());
        assert!(notes_du_livre(&sortie).is_empty());
        // Aucun paragraphe recollé à un autre : le corps reste tel quel, appels compris.
        assert_eq!(format!("{sortie:?}"), format!("{blocs:?}"));
    }

    #[test]
    fn section_implicite_en_chiffres_simples() {
        let mut et = Etiqueteuse::new();
        let blocs = vec![
            appels(&["1", "2", "3"]),
            bloc_para(vec![texte("1. Texte un")], 9, 0.2),
            bloc_para(vec![texte("2. Texte deux")], 9, 0.3),
            bloc_para(vec![texte("3. Texte trois")], 9, 0.4),
            bloc_para(vec![texte("4. Texte quatre")], 9, 0.5),
        ];
        let (blocs, retraits, points) = notes_regroupees(blocs, &mut et, &Vocabulaire::default());
        assert!(retraits.is_empty());
        let notes = notes_du_livre(&blocs);
        assert_eq!(notes.len(), 3);
        assert_eq!(notes[1], ("2-2".into(), "2".into(), "Texte deux".into()));
        // La note 4 n'a pas d'appel : paragraphe rendu tel quel, signalé.
        assert_eq!(points.len(), 1);
        assert!(matches!(blocs.last(), Some(Bloc::Paragraphe(p)) if p.texte_brut() == "4 Texte quatre"));
    }

    #[test]
    fn liste_numerotee_sans_appel_ni_suite_n_ouvre_pas_de_section() {
        // Sans appel avant la liste.
        let mut blocs = vec![bloc_para(vec![texte("Trois points :")], 4, 0.05)];
        blocs.extend(liste(3));
        sans_section(blocs);
        // Avec les appels, mais sans point 3.
        let mut blocs = vec![appels(&["1", "2", "3"])];
        blocs.extend(liste(2));
        sans_section(blocs);
        // Avec les appels, mais le point 3 trop loin (9e bloc après le point 1).
        let mut blocs = vec![appels(&["1", "2", "3"])];
        blocs.extend(liste(2));
        blocs.extend((0..7).map(|_| bloc_para(vec![texte("Texte.")], 5, 0.5)));
        blocs.push(bloc_para(vec![texte("3. Point 3")], 6, 0.1));
        sans_section(blocs);
    }

    #[test]
    fn un_appel_1_egare_n_ouvre_pas_de_section() {
        // Un seul appel « 1 » sans note, puis une liste 1. 2. 3. suivie du corps : la
        // liste et les paragraphes qui suivent restent du texte, sans recollage.
        let mut blocs = vec![appels(&["1"])];
        blocs.extend(liste(3));
        blocs.push(bloc_para(vec![texte("Suite du chapitre.")], 5, 0.1));
        blocs.push(bloc_para(vec![texte("Autre paragraphe.")], 5, 0.2));
        sans_section(blocs);
    }

    #[test]
    fn appels_2_et_3_sans_appel_1_n_ouvrent_pas_de_section() {
        let mut blocs = vec![appels(&["2", "3"])];
        blocs.extend(liste(3));
        sans_section(blocs);
    }

    #[test]
    fn note_3_au_8e_bloc_suivant_ouvre_la_section() {
        // Après le point 1 : point 2 (1er bloc), 6 sauts (2e à 7e), point 3 (8e bloc).
        // Des sauts, neutres pour les notes : des paragraphes sans marqueur fermeraient
        // la section (même page : règles a et b ; page sans marqueur).
        let mut blocs = vec![appels(&["1", "2", "3"])];
        blocs.extend(liste(2));
        blocs.extend((0..6).map(|_| Bloc::Saut { ancre: Ancre::depuis(4, Moitie::Entiere, rect(0.5)) }));
        blocs.push(bloc_para(vec![texte("3. Point 3")], 4, 0.6));
        let (sortie, _, points) = notes_regroupees(blocs, &mut Etiqueteuse::new(), &Vocabulaire::default());
        assert!(points.is_empty(), "{points:?}");
        let marqueurs: Vec<String> = notes_du_livre(&sortie).into_iter().map(|(_, m, _)| m).collect();
        assert_eq!(marqueurs, ["1", "2", "3"]);
    }

    /// Paragraphe sur la moitié droite d'une double page.
    fn bloc_droite(t: &str, page: u32, y0: f32) -> Bloc {
        Bloc::Paragraphe(Paragraphe {
            ancre: Ancre::depuis(page, Moitie::Droite, rect(y0)),
            ..para(vec![texte(t)], page, y0)
        })
    }

    /// Paragraphes rendus tels quels (texte et ancre), dans l'ordre.
    fn paragraphes(blocs: &[Bloc]) -> Vec<(String, Ancre)> {
        blocs
            .iter()
            .filter_map(|b| match b {
                Bloc::Paragraphe(p) => Some((p.texte_brut(), p.ancre)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn section_implicite_marqueur_seul_puis_texte_puis_corps() {
        // Règle (a) : le paragraphe sans marqueur devient le texte d'une note encore vide
        // (marqueur seul, ou « [n+1] » détaché de la fin de la note précédente). Un
        // second paragraphe sans marqueur termine la section : il reste du corps.
        let blocs = vec![
            appels(&["[1]", "[2]", "[3]", "[4]"]),
            bloc_para(vec![texte("[1]")], 2, 0.1),
            bloc_para(vec![texte("Texte un.")], 2, 0.15),
            bloc_para(vec![texte("[2]")], 2, 0.2),
            bloc_para(vec![texte("Texte deux.")], 2, 0.25),
            bloc_para(vec![texte("[3]")], 2, 0.3),
            bloc_para(vec![texte("Texte trois. [4]")], 2, 0.35),
            bloc_para(vec![texte("Texte quatre.")], 2, 0.4),
            bloc_para(vec![texte("Corps qui suit.")], 2, 0.5),
            bloc_para(vec![texte("Autre corps.")], 2, 0.6),
        ];
        let attendu = paragraphes(&blocs[8..]);
        let (sortie, _, points) = notes_regroupees(blocs, &mut Etiqueteuse::new(), &Vocabulaire::default());
        assert!(points.is_empty(), "{points:?}");
        let textes: Vec<String> = notes_du_livre(&sortie).into_iter().map(|(_, _, t)| t).collect();
        assert_eq!(textes, ["Texte un.", "Texte deux.", "Texte trois.", "Texte quatre."]);
        assert_eq!(paragraphes(&sortie)[1..], attendu[..]);
    }

    #[test]
    fn section_implicite_note_coupee_par_la_page() {
        // Règle (b) : le premier paragraphe de la page suivante (qui porte la note 4)
        // continue une note restée ouverte ; un paragraphe sans marqueur après une note
        // fermée, sur la même page, termine la section.
        let blocs = vec![
            appels(&["1", "2", "3", "4"]),
            bloc_para(vec![texte("1. Texte un.")], 9, 0.7),
            bloc_para(vec![texte("2. Texte deux.")], 9, 0.8),
            bloc_para(vec![texte("3. Texte trois qui")], 9, 0.9),
            bloc_para(vec![texte("continue page suivante.")], 10, 0.1),
            bloc_para(vec![texte("4. Texte quatre.")], 10, 0.2),
            bloc_para(vec![texte("Corps.")], 10, 0.3),
        ];
        let attendu = paragraphes(&blocs[6..]);
        let (sortie, _, points) = notes_regroupees(blocs, &mut Etiqueteuse::new(), &Vocabulaire::default());
        assert!(points.is_empty(), "{points:?}");
        assert_eq!(texte_note(&sortie[3]), "Texte trois qui continue page suivante.");
        assert_eq!(texte_note(&sortie[4]), "Texte quatre.");
        assert_eq!(paragraphes(&sortie)[1..], attendu[..]);

        // Note fermée en bas de page : le premier paragraphe de la page suivante est du
        // corps, même si la page porte ensuite un marqueur.
        let blocs = vec![
            appels(&["1", "2", "3"]),
            bloc_para(vec![texte("1. Texte un.")], 9, 0.7),
            bloc_para(vec![texte("2. Texte deux.")], 9, 0.8),
            bloc_para(vec![texte("3. Texte trois.")], 9, 0.9),
            bloc_para(vec![texte("Corps de la page suivante.")], 10, 0.1),
            bloc_para(vec![texte("4. Point quatre.")], 10, 0.2),
        ];
        let attendu = paragraphes(&blocs[4..]);
        let (sortie, _, _) = notes_regroupees(blocs, &mut Etiqueteuse::new(), &Vocabulaire::default());
        assert_eq!(notes_du_livre(&sortie).len(), 3);
        assert_eq!(paragraphes(&sortie)[1..], attendu[..]);
    }

    #[test]
    fn bibliographie_numerotee_puis_corps_de_la_demi_page_suivante() {
        // Cas du type de Connaissance p. 95d-96d, dans une section implicite : une
        // bibliographie « 1 … 29 » ouvre la section (appels 1, 2, 3 dans la portée) ;
        // l'épigraphe de la demi-page suivante reste du corps, chaque paragraphe avec
        // son ancre.
        let mut blocs = vec![appels(&["1", "2", "3"])];
        blocs.extend((1..=29).map(|k| bloc_droite(&format!("{k} Ouvrage {k} (1904). (T)"), 95, 0.1 + 0.02 * k as f32)));
        let corps = [
            "C'est ainsi que la pensée abstraite",
            "devient vivante.",
            "R. Steiner",
            "La Philosophie de la Liberté",
        ];
        blocs.extend(corps.iter().enumerate().map(|(k, t)| bloc_droite(t, 96, 0.2 + 0.1 * k as f32)));
        let attendu = paragraphes(&blocs[30..]);
        let (sortie, _, _) = notes_regroupees(blocs, &mut Etiqueteuse::new(), &Vocabulaire::default());
        let rendus = paragraphes(&sortie);
        assert_eq!(rendus[rendus.len() - corps.len()..], attendu[..]);
        assert!(matches!(sortie.last(), Some(Bloc::Paragraphe(p)) if p.texte_brut() == "La Philosophie de la Liberté"));
    }

    #[test]
    fn demi_page_sans_marqueur_ferme_la_section() {
        // Connaissance p. 95d-96d : la note « 29 », ouverte, finit la demi-page 95d ; la
        // demi-page 96d n'a aucun marqueur de note : la section se ferme avant elle, ses
        // paragraphes restent du corps avec leurs ancres.
        let blocs = vec![
            appels(&["1", "29"]),
            titre("NOTES", 2, 91),
            bloc_droite("1 Texte un.", 91, 0.2),
            bloc_droite("29 Données de base, Editions du Centre Triades, Paris", 95, 0.8),
            bloc_droite("compositeur travaille d'après des bases", 96, 0.1),
            bloc_droite("établies par la science musicale.", 96, 0.15),
            bloc_droite("R. Steiner", 96, 0.8),
        ];
        let attendu = paragraphes(&blocs[4..]);
        let (sortie, retraits, points) = notes_regroupees(blocs, &mut Etiqueteuse::new(), &Vocabulaire::default());
        assert!(points.is_empty(), "{points:?}");
        assert_eq!(retraits.len(), 1);
        let notes = notes_du_livre(&sortie);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[1].2, "Données de base, Editions du Centre Triades, Paris");
        assert_eq!(paragraphes(&sortie)[1..], attendu[..]);
    }

    #[test]
    fn note_continuee_sur_une_page_qui_porte_un_marqueur() {
        // La page suivante commence par la fin de la note 1, puis ouvre la note 2 :
        // elle porte un marqueur, la section continue et la suite reste dans la note 1.
        let blocs = vec![
            appels(&["1", "2"]),
            titre("Notes", 2, 9),
            bloc_para(vec![texte("1 Texte un qui")], 9, 0.8),
            bloc_para(vec![texte("continue ici.")], 10, 0.1),
            bloc_para(vec![texte("2 Texte deux.")], 10, 0.2),
        ];
        let (sortie, _, points) = notes_regroupees(blocs, &mut Etiqueteuse::new(), &Vocabulaire::default());
        assert!(points.is_empty(), "{points:?}");
        let textes: Vec<String> = notes_du_livre(&sortie).into_iter().map(|(_, _, t)| t).collect();
        assert_eq!(textes, ["Texte un qui continue ici.", "Texte deux."]);
        assert_eq!(paragraphes(&sortie).len(), 1);
    }

    #[test]
    fn note_en_bas_de_page_entre_crochets() {
        let mut et = Etiqueteuse::new();
        let p = para(vec![texte("Mot"), Segment::Exposant("[4]".into())], 2, 0.2);
        let (blocs, points) =
            notes_bas_de_page(vec![p], &[ligne("[4] Voir plus loin", 0.9)], 2, Moitie::Entiere, &mut et, &Vocabulaire::default());
        assert!(points.is_empty());
        let Bloc::Paragraphe(p) = &blocs[0] else { panic!() };
        assert_eq!(p.segments[1], Segment::AppelNote { etiquette: "1-4".into(), marqueur: "4".into() });
        assert_eq!(notes_du_livre(&blocs), vec![("1-4".into(), "4".into(), "Voir plus loin".into())]);
    }
}
