//! Classement des lignes d'une page : corps, notes, en-tête, pied, numéro résiduel.

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::lignes::Ligne;
use crate::modele::{Ancre, Page};
use crate::reglages::Zones;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    EnTete,
    Pied,
    NumeroPage,
    TitreNotes,
}

#[derive(Clone, Debug, Serialize)]
pub struct Retrait {
    pub ancre: Ancre,
    pub role: Role,
    pub texte: String,
}

pub struct PageClassee {
    pub corps: Vec<Ligne>,
    pub notes: Vec<Ligne>,
    pub retraits: Vec<Retrait>,
    pub numero_residuel: bool,
}

static NUMERO_PAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^\s*(?:Page\s+\d+\s+(?:de|sur|/)\s+\d+|\d+\s*/\s*\d+|\.{2,}\s*/\s*\d+|-\s*\d+\s*-|\d+(?:\s+\d+)?)\s*$",
    )
    .expect("motif de numéro de page valide")
});

/// Un « numéro » plus grand que le texte de la page d'au moins cet écart (en points)
/// est un numéro de chapitre, pas un numéro de page.
const ECART_NUMERO_CHAPITRE: f32 = 1.5;

pub fn classer_lignes(page: &Page, lignes: Vec<Ligne>, zones: &Zones) -> PageClassee {
    let mut resultat = PageClassee {
        corps: Vec::new(),
        notes: Vec::new(),
        retraits: Vec::new(),
        numero_residuel: false,
    };
    let retirer = |ligne: &Ligne, role| Retrait {
        ancre: Ancre::depuis(page.numero, page.moitie, ligne.rect),
        role,
        texte: ligne.texte(),
    };
    for ligne in lignes {
        let c = (ligne.rect.y0 + ligne.rect.y1) / 2.0;
        if c < zones.entete {
            resultat.retraits.push(retirer(&ligne, Role::EnTete));
        } else if c > 1.0 - zones.pied {
            resultat.retraits.push(retirer(&ligne, Role::Pied));
        } else if zones.notes > 0.0 && c > 1.0 - zones.pied - zones.notes {
            resultat.notes.push(ligne);
        } else {
            resultat.corps.push(ligne);
        }
    }
    // Numéro résiduel : première ou dernière ligne du corps seulement, et pas plus
    // grande que le texte de la page (sinon c'est un numéro de chapitre).
    let taille_texte = mediane(resultat.corps.iter().map(Ligne::taille).collect());
    let est_numero = |ligne: &Ligne| est_numero_page(&ligne.texte()) && ligne.taille() < taille_texte + ECART_NUMERO_CHAPITRE;
    if resultat.corps.last().is_some_and(est_numero) {
        let ligne = resultat.corps.pop().expect("corps non vide");
        resultat.retraits.push(retirer(&ligne, Role::NumeroPage));
        resultat.numero_residuel = true;
    }
    if resultat.corps.first().is_some_and(est_numero) {
        let ligne = resultat.corps.remove(0);
        resultat.retraits.push(retirer(&ligne, Role::NumeroPage));
        resultat.numero_residuel = true;
    }
    resultat
}

fn mediane(mut valeurs: Vec<f32>) -> f32 {
    if valeurs.is_empty() {
        return 0.0;
    }
    valeurs.sort_by(|a, b| a.total_cmp(b));
    valeurs[valeurs.len() / 2]
}

/// Le texte entier est-il un numéro de page (« 12 », « - 12 - », « 92 93 », « Page 3 de 9 »…) ?
pub fn est_numero_page(texte: &str) -> bool {
    NUMERO_PAGE.is_match(texte)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modele::{Moitie, Mot, Rect};

    fn ligne(texte: &str, y: f32) -> Ligne {
        let rect = Rect { x0: 0.1, y0: y - 0.01, x1: 0.9, y1: y + 0.01 };
        let mots = texte
            .split(' ')
            .map(|t| Mot {
                texte: t.into(),
                rect,
                taille: 10.0,
                gras: false,
                italique: false,
                espace_avant: true,
            })
            .collect();
        Ligne { mots, exposants: vec![], rect }
    }

    fn page() -> Page {
        Page { numero: 5, moitie: Moitie::Entiere, largeur_pt: 400.0, hauteur_pt: 600.0, mots: vec![] }
    }

    #[test]
    fn classe_par_centre() {
        let zones = Zones { entete: 0.07, pied: 0.05, notes: 0.10 };
        let lignes = vec![
            ligne("haut", 0.03),
            ligne("milieu", 0.50),
            ligne("note", 0.90),
            ligne("bas", 0.97),
        ];
        let r = classer_lignes(&page(), lignes, &zones);
        assert_eq!(r.corps.len(), 1);
        assert_eq!(r.corps[0].texte(), "milieu");
        assert_eq!(r.notes.len(), 1);
        assert_eq!(r.notes[0].texte(), "note");
        let roles: Vec<_> = r.retraits.iter().map(|t| (t.role, t.texte.as_str())).collect();
        assert_eq!(roles, vec![(Role::EnTete, "haut"), (Role::Pied, "bas")]);
        assert_eq!(r.retraits[0].ancre.page, 5);
        assert!(!r.numero_residuel);
    }

    #[test]
    fn numero_residuel_retire_et_signale() {
        let zones = Zones::default();
        for numero in ["- 12 -", "92 93"] {
            let lignes = vec![ligne("texte", 0.4), ligne(numero, 0.8)];
            let r = classer_lignes(&page(), lignes, &zones);
            assert_eq!(r.corps.len(), 1, "{numero}");
            assert_eq!(r.retraits.len(), 1);
            assert_eq!(r.retraits[0].role, Role::NumeroPage);
            assert_eq!(r.retraits[0].texte, numero);
            assert!(r.numero_residuel);
        }
        let lignes = vec![ligne("texte", 0.4), ligne("12 juin 1924", 0.8)];
        let r = classer_lignes(&page(), lignes, &zones);
        assert_eq!(r.corps.len(), 2);
        assert!(r.retraits.is_empty());
        assert!(!r.numero_residuel);
    }

    #[test]
    fn numero_de_chapitre_plus_grand_que_le_texte_garde() {
        // Calibre : le numéro de chapitre « 1 », en 19,4 pt au-dessus d'un texte en 15 pt.
        let mut numero = ligne("1", 0.12);
        numero.mots[0].taille = 19.4;
        let lignes = vec![numero, ligne("VISITE À PATMOS", 0.17), ligne("Aujourd'hui je voudrais", 0.25)];
        let r = classer_lignes(&page(), lignes, &Zones::default());
        assert_eq!(r.corps.len(), 3);
        assert!(r.retraits.is_empty());
        assert!(!r.numero_residuel);
    }
}
