//! Types partagés par tout le moteur.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Rectangle en fractions de la page (0..1), origine en haut à gauche.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Moitie {
    Entiere,
    Gauche,
    Droite,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mot {
    pub texte: String,
    pub rect: Rect,
    pub taille: f32,
    pub gras: bool,
    pub italique: bool,
    pub espace_avant: bool,
}

#[derive(Clone, Debug)]
pub struct Page {
    pub numero: u32,
    pub moitie: Moitie,
    pub largeur_pt: f32,
    pub hauteur_pt: f32,
    pub mots: Vec<Mot>,
}

/// Position d'un bloc dans le PDF : page (base 1), moitié, rectangle en millièmes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ancre {
    pub page: u32,
    pub moitie: Moitie,
    pub rect: [u16; 4],
}

impl Ancre {
    pub fn depuis(page: u32, moitie: Moitie, r: Rect) -> Ancre {
        let m = |v: f32| (v * 1000.0).round().clamp(0.0, 1000.0) as u16;
        Ancre { page, moitie, rect: [m(r.x0), m(r.y0), m(r.x1), m(r.y1)] }
    }
}

impl fmt::Display for Ancre {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let suffixe = match self.moitie {
            Moitie::Entiere => "",
            Moitie::Gauche => "g",
            Moitie::Droite => "d",
        };
        let [x0, y0, x1, y1] = self.rect;
        write!(f, "@p{}{} {},{},{},{}", self.page, suffixe, x0, y0, x1, y1)
    }
}

impl FromStr for Ancre {
    type Err = ErreurMoteur;

    fn from_str(s: &str) -> Result<Ancre, ErreurMoteur> {
        let invalide = || ErreurMoteur::Reglages(format!("ancre invalide : « {s} »"));
        let (tete, rect) = s.trim().split_once(' ').ok_or_else(invalide)?;
        let tete = tete.strip_prefix("@p").ok_or_else(invalide)?;
        let (chiffres, moitie) = match tete.chars().last() {
            Some('g') => (&tete[..tete.len() - 1], Moitie::Gauche),
            Some('d') => (&tete[..tete.len() - 1], Moitie::Droite),
            _ => (tete, Moitie::Entiere),
        };
        let page = chiffres.parse().map_err(|_| invalide())?;
        let valeurs: Vec<u16> = rect
            .split(',')
            .map(|v| v.trim().parse().map_err(|_| invalide()))
            .collect::<Result<_, _>>()?;
        let rect: [u16; 4] = valeurs.try_into().map_err(|_| invalide())?;
        Ok(Ancre { page, moitie, rect })
    }
}

#[derive(Clone, Debug)]
pub struct Signet {
    pub titre: String,
    pub niveau: u8,
    pub page: u32,
}

#[derive(Clone, Debug, Default)]
pub struct MetaPdf {
    pub titre: String,
    pub auteur: String,
    pub producteur: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurMoteur {
    #[error("PDF illisible : {0}")]
    PdfIllisible(String),
    #[error("bibliothèque pdfium introuvable : {0}")]
    PdfiumIntrouvable(String),
    #[error("page {0} : {1}")]
    Page(u32, String),
    #[error("réglages invalides : {0}")]
    Reglages(String),
}
