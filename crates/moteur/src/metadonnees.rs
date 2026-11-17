//! Titre et auteur proposés à partir du nom de fichier et des métadonnées PDF,
//! nom du fichier de sortie, code de langue de l'EPUB.

use std::sync::LazyLock;

use plumeo_commun::normaliser;
use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::modele::MetaPdf;

static PREFIXE_NUMERO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\[(]?\d+[\])]?[\s_-]+").unwrap());
static AUTEUR_ENTRE_PARENTHESES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(.+?)\s*\(([^()]+)\)$").unwrap());
static PARENTHESE_EDITION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\([^()]*dition[^()]*\)\s*$").unwrap());

const AUTEUR_INCONNU: &str = "Inconnu";
const LONGUEUR_MAX_NOM: usize = 150;

fn preparer_nom(nom: &str) -> String {
    let mut s: String = nom.nfc().collect();
    while let Some(m) = PREFIXE_NUMERO.find(&s) {
        s = s[m.end()..].to_string();
    }
    s.replace('_', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn est_candidat_auteur(cote: &str) -> bool {
    if cote.contains(',') {
        return true;
    }
    let mots: Vec<&str> = cote.split_whitespace().collect();
    (2..=4).contains(&mots.len())
        && mots.iter().all(|m| m.chars().next().is_some_and(char::is_uppercase))
}

/// « Nom, Prénom » devient « Prénom Nom ».
fn prenom_nom(auteur: &str) -> String {
    match auteur.split_once(',') {
        Some((nom, prenom)) if !prenom.trim().is_empty() => {
            format!("{} {}", prenom.trim(), nom.trim())
        }
        _ => auteur.trim().to_string(),
    }
}

/// Sépare le nom de fichier préparé en (titre, auteur éventuel).
fn separer(nom: &str) -> (String, Option<String>) {
    if let Some(c) = AUTEUR_ENTRE_PARENTHESES.captures(nom) {
        return (c[1].trim().to_string(), Some(prenom_nom(&c[2])));
    }
    if let Some((gauche, droite)) = nom.split_once(" - ") {
        let (gauche, droite) = (gauche.trim(), droite.trim());
        let auteur_a_gauche = est_candidat_auteur(gauche) && !est_candidat_auteur(droite);
        return if auteur_a_gauche {
            (droite.to_string(), Some(prenom_nom(gauche)))
        } else {
            (gauche.to_string(), Some(prenom_nom(droite)))
        };
    }
    (nom.to_string(), None)
}

/// Propose (titre, auteur) ; `nom_fichier` est sans extension.
pub fn proposer(meta: &MetaPdf, nom_fichier: &str) -> (String, String) {
    let (mut titre, auteur_fichier) = separer(&preparer_nom(nom_fichier));
    let titre_meta = PARENTHESE_EDITION.replace(meta.titre.trim(), "").trim().to_string();
    let norme_fichier = normaliser(&titre);
    let norme_meta = normaliser(&titre_meta);
    if !norme_fichier.is_empty()
        && norme_meta.starts_with(&norme_fichier)
        && norme_meta.len() > norme_fichier.len()
    {
        titre = titre_meta;
    }
    let auteur = auteur_fichier
        .filter(|a| !a.is_empty())
        .or_else(|| {
            let a = meta.auteur.trim();
            (a.split_whitespace().count() >= 2).then(|| a.to_string())
        })
        .unwrap_or_else(|| AUTEUR_INCONNU.to_string());
    (titre, auteur)
}

/// Nom de fichier de sortie, sans extension : `{titre} - {auteur}`.
pub fn nom_sortie(titre: &str, auteur: &str) -> String {
    format!("{titre} - {auteur}")
        .nfc()
        .filter(|c| !c.is_control())
        .map(|c| if "/\\:*?\"<>|".contains(c) { '_' } else { c })
        .take(LONGUEUR_MAX_NOM)
        .collect()
}

/// Code ISO 639-3 (Tesseract) vers le code BCP 47 attendu par l'EPUB.
pub fn langue_epub(code: &str) -> String {
    match code {
        "fra" => "fr",
        "deu" => "de",
        "eng" => "en",
        autre => autre,
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(titre: &str, auteur: &str) -> MetaPdf {
        MetaPdf { titre: titre.into(), auteur: auteur.into(), producteur: String::new() }
    }

    #[test]
    fn proposer_sur_le_corpus() {
        let cas = [
            ("Aïvanhov, Omraam Mikhaël - Approche de la Cité céleste", "Aïvanhov, Omraam Mikhaël", "Approche de la Cité céleste", "Approche de la Cité céleste", "Omraam Mikhaël Aïvanhov"),
            ("Anges et l’arbre de vie - Omraam Mikhaël Aïvanhov", "Microsoft Word - F - Les Anges et l'Abre de Vie", "Windows", "Anges et l’arbre de vie", "Omraam Mikhaël Aïvanhov"),
            ("L’alchimie_spirituelle_Tome_2_Omraam_Mikhaël_Aïvanhov", "", "", "L’alchimie spirituelle Tome 2 Omraam Mikhaël Aïvanhov", "Inconnu"),
            ("Chronique de l'Akasha - Rudolf Steiner", "Chronique de l'Akasha", "Rudolf Steiner", "Chronique de l'Akasha", "Rudolf Steiner"),
            ("Le sens de l'amour (Rudolf Steiner)", "Le sens de l'amour", "Rudolf Steiner", "Le sens de l'amour", "Rudolf Steiner"),
            ("Steiner Rudolf - Connaissance Logique Pensée pratique", "", "", "Connaissance Logique Pensée pratique", "Steiner Rudolf"),
            ("[02] Lettres Sur La Méditation Occulte - Alice Bailey", "Livres AAB", "AAB", "Lettres Sur La Méditation Occulte", "Alice Bailey"),
            ("Alice Bailey - Initiation humaine et solaire", "Initiation humaine et solaire: Une connaissance approfondie est révélée (French Edition)", "Alice Bailey", "Initiation humaine et solaire: Une connaissance approfondie est révélée", "Alice Bailey"),
            ("guerison esoterique_ Une connai - Alice Bailey", "guérison ésotérique: Une connaissance approfondie est révélée (French Edition)", "Alice Bailey", "guérison ésotérique: Une connaissance approfondie est révélée", "Alice Bailey"),
        ];
        for (fichier, mt, ma, titre, auteur) in cas {
            let (t, a) = proposer(&meta(mt, ma), fichier);
            assert_eq!((t.as_str(), a.as_str()), (titre, auteur), "{fichier}");
        }
    }

    #[test]
    fn nom_sortie_sur() {
        assert_eq!(nom_sortie("a/b: c", "X"), "a_b_ c - X");
        let nfd = "e\u{301}te\u{301}";
        assert_eq!(nom_sortie(nfd, "X"), "été - X");
        assert!(nom_sortie(&"é".repeat(400), "X").chars().count() <= 150);
    }

    #[test]
    fn langue_iso_vers_bcp47() {
        assert_eq!(langue_epub("fra"), "fr");
        assert_eq!(langue_epub("deu"), "de");
        assert_eq!(langue_epub("eng"), "en");
        assert_eq!(langue_epub("spa"), "spa");
    }
}
