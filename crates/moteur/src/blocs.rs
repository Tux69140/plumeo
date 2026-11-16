//! Modèle des blocs produits par le moteur : paragraphes, titres, notes et entrées de sommaire.

use crate::modele::Ancre;

#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    Texte { texte: String, gras: bool, italique: bool },
    Exposant(String),
    AppelNote { etiquette: String, marqueur: String },
}

impl Segment {
    fn texte_brut(&self) -> &str {
        match self {
            Segment::Texte { texte, .. } => texte,
            Segment::Exposant(texte) => texte,
            Segment::AppelNote { marqueur, .. } => marqueur,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Paragraphe {
    pub segments: Vec<Segment>,
    pub ancre: Ancre,
    pub taille: f32,
    pub tout_gras: bool,
    pub nb_lignes: usize,
}

impl Paragraphe {
    pub fn texte_brut(&self) -> String {
        self.segments.iter().map(Segment::texte_brut).collect()
    }
}

#[derive(Clone, Debug)]
pub enum Bloc {
    Paragraphe(Paragraphe),
    Titre { niveau: u8, texte: String, ancre: Ancre },
    Note { etiquette: String, marqueur: String, segments: Vec<Segment>, ancre: Ancre },
    /// Saut de section : la page précédente s'arrête bien avant le bas du bloc de texte
    /// (page liminaire, colophon, fin de partie). Ancre : première ligne de la page suivante.
    Saut { ancre: Ancre },
    /// Entrée d'un sommaire imprimé : lien vers le titre d'identifiant `cible`
    /// (voir `markdown::ancre_titre`).
    EntreeSommaire { segments: Vec<Segment>, cible: String, ancre: Ancre },
}

impl Bloc {
    pub fn ancre(&self) -> Ancre {
        match self {
            Bloc::Paragraphe(p) => p.ancre,
            Bloc::Titre { ancre, .. }
            | Bloc::Note { ancre, .. }
            | Bloc::Saut { ancre }
            | Bloc::EntreeSommaire { ancre, .. } => *ancre,
        }
    }
}
