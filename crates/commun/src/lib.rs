//! Conventions partagées par le moteur, qui écrit le Markdown interne, et par l'EPUB,
//! qui le lit : comparaison de textes, titre d'un sommaire imprimé, identifiants des
//! titres. Une seule définition, pour que les deux côtés décident toujours de même.

use std::collections::HashSet;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

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

/// Titres d'un sommaire imprimé, normalisés (`normaliser`).
const TITRES_DE_SOMMAIRE: [&str; 2] = ["table des matieres", "sommaire"];

/// Le texte est-il « Table des matières » ou « Sommaire », sans tenir compte de la casse,
/// des diacritiques ni de la ponctuation (« SOMMAIRE : ») ? Le moteur pose ce titre sur
/// un sommaire imprimé ; l'EPUB s'en sert pour placer ce sommaire en tête du livre.
pub fn est_titre_de_sommaire(texte: &str) -> bool {
    TITRES_DE_SOMMAIRE.contains(&normaliser(texte).as_str())
}

/// Caractère gardé dans un identifiant de titre : lettre ou chiffre qui peut figurer
/// dans un nom XML (un exposant « ² » ne le peut pas).
fn lettre_d_identifiant(c: char) -> bool {
    c.is_alphanumeric()
        && (c.is_ascii()
            || matches!(c as u32,
                0xC0..=0xD6 | 0xD8..=0xF6 | 0xF8..=0x2FF | 0x370..=0x37D | 0x37F..=0x1FFF
                | 0x2070..=0x218F | 0x2C00..=0x2FEF | 0x3001..=0xD7FF | 0xF900..=0xFDCF
                | 0xFDF0..=0xFFFD | 0x10000..=0xEFFFF))
}

/// Identifiant d'un titre (attribut `id` dans l'EPUB, cible des entrées de sommaire du
/// Markdown) : `t-` puis le texte en minuscules, chaque suite de caractères qui ne sont
/// ni lettre ni chiffre devenant un tiret (aucun tiret en bord). Le préfixe en fait un
/// nom XML valide.
pub fn ancre_titre(texte: &str) -> String {
    let mut sortie = String::from("t");
    let mut tiret = true;
    for c in texte.chars().flat_map(char::to_lowercase) {
        if lettre_d_identifiant(c) {
            if tiret {
                sortie.push('-');
                tiret = false;
            }
            sortie.push(c);
        } else {
            tiret = true;
        }
    }
    sortie
}

/// Identifiants des titres d'un livre, attribués dans l'ordre du Markdown : un
/// identifiant déjà pris reçoit `-2`, `-3`…
#[derive(Default)]
pub struct AncresTitres {
    pris: HashSet<String>,
}

impl AncresTitres {
    pub fn attribuer(&mut self, texte: &str) -> String {
        let base = ancre_titre(texte);
        let mut id = base.clone();
        let mut n = 1;
        while !self.pris.insert(id.clone()) {
            n += 1;
            id = format!("{base}-{n}");
        }
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normaliser_retire_diacritiques_et_ponctuation() {
        assert_eq!(normaliser("  L’Âme,  d'été! "), "l ame d ete");
        assert_eq!(normaliser("À Ä É È Ì Ò Õ Ù Ÿ Ç Š Ě"), "a a e e i o o u y c s e");
        // Texte décomposé (NFD) : accents combinants retirés.
        assert_eq!(normaliser("Matie\u{300}res :"), "matieres");
    }

    #[test]
    fn titres_de_sommaire_reconnus() {
        for oui in [
            "Table des matières",
            "TABLE DES MATIÈRES",
            "Table des Matieres",
            "Sommaire",
            "SOMMAIRE",
            "Sommaire :",
            // OCR abîmé : accents en trop ou mal placés.
            "Tâble dès mätïères",
            "Table des matìères",
        ] {
            assert!(est_titre_de_sommaire(oui), "« {oui} »");
        }
        for non in ["Table", "Sommaires et index", ""] {
            assert!(!est_titre_de_sommaire(non), "« {non} »");
        }
    }

    #[test]
    fn ancre_d_un_titre() {
        assert_eq!(ancre_titre("1 VISITE À PATMOS"), "t-1-visite-à-patmos");
        assert_eq!(ancre_titre("I. La pierre cubique"), "t-i-la-pierre-cubique");
        assert_eq!(ancre_titre("  L’Archange : « Mikhaël » !  "), "t-l-archange-mikhaël");
        assert_eq!(ancre_titre("LES 144.000 SERVITEURS DE DIEU"), "t-les-144-000-serviteurs-de-dieu");
        // Lettre ou chiffre hors des noms XML (exposant ²) : un séparateur comme un autre.
        assert_eq!(ancre_titre("E = mc²"), "t-e-mc");
        assert_eq!(ancre_titre(" … "), "t");
        assert_eq!(ancre_titre(""), "t");
    }

    #[test]
    fn ancres_de_titres_uniques() {
        let mut ancres = AncresTitres::default();
        assert_eq!(ancres.attribuer("Introduction"), "t-introduction");
        assert_eq!(ancres.attribuer("INTRODUCTION"), "t-introduction-2");
        assert_eq!(ancres.attribuer("Introduction !"), "t-introduction-3");
        assert_eq!(ancres.attribuer("Introduction 2"), "t-introduction-2-2");
        assert_eq!(ancres.attribuer("Conclusion"), "t-conclusion");
    }
}
