//! Réglages d'une série de PDF (fichier JSON).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::modele::{ErreurMoteur, Moitie};

/// Hauteurs en fraction de page ; `notes` = bande au-dessus du pied ; 0 = aucune.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Zones {
    pub entete: f32,
    pub pied: f32,
    pub notes: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceTitres {
    #[default]
    Auto,
    Signets,
    GrasMajuscules,
    Taille,
}

fn fra() -> String {
    "fra".to_string()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReglagesSerie {
    pub nom: String,
    #[serde(default = "fra")]
    pub langue: String,
    pub paire: Zones,
    pub impaire: Zones,
    /// Position du pli, fraction de largeur.
    #[serde(default)]
    pub double_page: Option<f32>,
    #[serde(default)]
    pub titres: SourceTitres,
    /// Clé « 12 », « 6g » ou « 6d ».
    #[serde(default)]
    pub exceptions: BTreeMap<String, Zones>,
}

impl ReglagesSerie {
    pub fn lire(chemin: &Path) -> Result<Self, ErreurMoteur> {
        let texte = std::fs::read_to_string(chemin)
            .map_err(|e| ErreurMoteur::Reglages(format!("{} : {e}", chemin.display())))?;
        serde_json::from_str(&texte)
            .map_err(|e| ErreurMoteur::Reglages(format!("{} : {e}", chemin.display())))
    }

    /// Exception éventuelle d'abord, sinon parité (gauche = paire, droite = impaire).
    pub fn zones_pour(&self, page: u32, moitie: Moitie) -> &Zones {
        let cle = match moitie {
            Moitie::Entiere => page.to_string(),
            Moitie::Gauche => format!("{page}g"),
            Moitie::Droite => format!("{page}d"),
        };
        if let Some(zones) = self.exceptions.get(&cle) {
            return zones;
        }
        let paire = match moitie {
            Moitie::Gauche => true,
            Moitie::Droite => false,
            Moitie::Entiere => page.is_multiple_of(2),
        };
        if paire { &self.paire } else { &self.impaire }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serie() -> ReglagesSerie {
        let z = |e| Zones { entete: e, pied: 0.0, notes: 0.0 };
        ReglagesSerie {
            nom: "x".into(),
            langue: "fra".into(),
            paire: z(0.1),
            impaire: z(0.2),
            double_page: None,
            titres: SourceTitres::Auto,
            exceptions: BTreeMap::new(),
        }
    }

    #[test]
    fn zones_par_parite() {
        let s = serie();
        assert_eq!(s.zones_pour(12, Moitie::Entiere), &s.paire);
        assert_eq!(s.zones_pour(13, Moitie::Entiere), &s.impaire);
        assert_eq!(s.zones_pour(7, Moitie::Gauche), &s.paire);
        assert_eq!(s.zones_pour(6, Moitie::Droite), &s.impaire);
    }

    #[test]
    fn exception_prioritaire() {
        let mut s = serie();
        let exc = Zones { entete: 0.5, pied: 0.4, notes: 0.3 };
        s.exceptions.insert("29".into(), exc.clone());
        s.exceptions.insert("6g".into(), exc.clone());
        assert_eq!(s.zones_pour(29, Moitie::Entiere), &exc);
        assert_eq!(s.zones_pour(6, Moitie::Gauche), &exc);
        assert_eq!(s.zones_pour(6, Moitie::Droite), &s.impaire);
    }

    #[test]
    fn json_minimal() {
        let json = r#"{"nom":"x","paire":{"entete":0.07,"pied":0,"notes":0},"impaire":{"entete":0.07,"pied":0,"notes":0}}"#;
        let s: ReglagesSerie = serde_json::from_str(json).unwrap();
        assert_eq!(s.langue, "fra");
        assert_eq!(s.titres, SourceTitres::Auto);
        assert_eq!(s.double_page, None);
        assert!(s.exceptions.is_empty());
    }
}
