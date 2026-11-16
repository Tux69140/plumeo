//! Écriture du Markdown interne : chaque bloc est précédé de son ancre.

use std::collections::HashSet;

use crate::blocs::{Bloc, Segment};
use crate::modele::Ancre;

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

/// Identifiant d'un titre (attribut `id` dans l'EPUB, cible des entrées de sommaire) :
/// `t-` puis le texte en minuscules, chaque suite de caractères qui ne sont ni lettre
/// ni chiffre devenant un tiret (aucun tiret en bord). Le préfixe en fait un nom XML
/// valide. Copie exacte de `plumeo_epub::ancre_titre` (vérifiée par un test).
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

/// Identifiants des titres d'un livre, dans l'ordre : un identifiant déjà pris reçoit
/// `-2`, `-3`… (comme les titres de l'EPUB).
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

/// Échappe le texte pour le Markdown ; en début de bloc, échappe aussi ce qui
/// serait pris pour un titre, une citation, une liste ou une liste numérotée.
pub fn echapper_md(texte: &str, debut_de_bloc: bool) -> String {
    let mut sortie = String::with_capacity(texte.len() + 8);
    let mut chiffres_au_debut = debut_de_bloc;
    for (i, c) in texte.chars().enumerate() {
        let special = matches!(c, '\\' | '*' | '_' | '`' | '[' | ']' | '<' | '>' | '&')
            || (debut_de_bloc && i == 0 && matches!(c, '#' | '-' | '+' | '='))
            || (chiffres_au_debut && matches!(c, '.' | ')') && i > 0);
        if chiffres_au_debut && !c.is_ascii_digit() {
            chiffres_au_debut = false;
        }
        if special {
            sortie.push('\\');
        }
        sortie.push(c);
    }
    sortie
}

/// Fusionne les segments `Texte` voisins de même style.
fn fusionner(segments: &[Segment]) -> Vec<Segment> {
    let mut sortie: Vec<Segment> = Vec::new();
    for segment in segments {
        if let (
            Some(Segment::Texte { texte: t, gras: g, italique: i }),
            Segment::Texte { texte, gras, italique },
        ) = (sortie.last_mut(), segment)
            && g == gras
            && i == italique
        {
            t.push_str(texte);
            continue;
        }
        sortie.push(segment.clone());
    }
    sortie
}

/// Premier caractère que produira le segment (pour juger de l'encadrement des marqueurs).
fn premier_caractere(segment: Option<&Segment>) -> Option<char> {
    match segment? {
        Segment::Texte { texte, .. } => texte.chars().next(),
        Segment::Exposant(_) => Some('<'),
        Segment::AppelNote { .. } => Some('['),
    }
}

fn est_ponctuation(c: char) -> bool {
    !c.is_alphanumeric() && !c.is_whitespace()
}

/// Les délimiteurs `*` ouvrant et fermant sont-ils reconnus par CommonMark ?
fn encadrement_valide(avant: Option<char>, coeur: &str, apres: Option<char>) -> bool {
    let (Some(premier), Some(dernier)) = (coeur.chars().next(), coeur.chars().next_back()) else {
        return false;
    };
    let isole = |c: Option<char>| c.is_none_or(|c| c.is_whitespace() || est_ponctuation(c));
    let ouvrant = !est_ponctuation(premier) || isole(avant);
    let fermant = !est_ponctuation(dernier) || isole(apres);
    ouvrant && fermant
}

fn rendre_segments(segments: &[Segment]) -> String {
    let segments = fusionner(segments);
    let mut sortie = String::new();
    for (i, segment) in segments.iter().enumerate() {
        match segment {
            Segment::Texte { texte, gras, italique } => {
                let texte = if sortie.is_empty() { texte.trim_start() } else { texte.as_str() };
                let coeur = texte.trim();
                if coeur.is_empty() || (!gras && !italique) {
                    let debut = sortie.is_empty();
                    sortie.push_str(&echapper_md(texte, debut));
                    continue;
                }
                let tete = &texte[..texte.len() - texte.trim_start().len()];
                let queue = &texte[texte.trim_end().len()..];
                sortie.push_str(tete);
                let avant = sortie.chars().next_back();
                let apres = queue.chars().next().or_else(|| premier_caractere(segments.get(i + 1)));
                let (ouvre, ferme) = if encadrement_valide(avant, coeur, apres) {
                    let m = match (gras, italique) {
                        (true, true) => "***",
                        (true, false) => "**",
                        _ => "*",
                    };
                    (m, m)
                } else {
                    match (gras, italique) {
                        (true, true) => ("<strong><em>", "</em></strong>"),
                        (true, false) => ("<strong>", "</strong>"),
                        _ => ("<em>", "</em>"),
                    }
                };
                sortie.push_str(ouvre);
                sortie.push_str(&echapper_md(coeur, false));
                sortie.push_str(ferme);
                sortie.push_str(queue);
            }
            Segment::Exposant(texte) => {
                sortie.push_str("<sup>");
                sortie.push_str(&echapper_md(texte, false));
                sortie.push_str("</sup>");
            }
            Segment::AppelNote { etiquette, .. } => {
                sortie.push_str(&format!("[^{etiquette}]"));
            }
        }
    }
    // Une seule ligne : les sauts éventuels deviennent des espaces.
    sortie.replace(['\n', '\r'], " ").trim_end().to_string()
}

fn bloc_ancre(sortie: &mut Vec<String>, ancre: Ancre, ligne: String) {
    sortie.push(format!("<!-- {ancre} -->\n{ligne}\n"));
}

/// Écrit les blocs en Markdown. Les définitions de notes vont en fin de chapitre
/// (avant le titre de niveau 1 suivant, ou à la fin).
pub fn ecrire_markdown(blocs: &[Bloc]) -> String {
    let mut sortie: Vec<String> = Vec::new();
    let mut notes: Vec<(Ancre, String)> = Vec::new();
    let vider = |sortie: &mut Vec<String>, notes: &mut Vec<(Ancre, String)>| {
        for (ancre, ligne) in notes.drain(..) {
            bloc_ancre(sortie, ancre, ligne);
        }
    };
    for bloc in blocs {
        match bloc {
            Bloc::Paragraphe(p) => bloc_ancre(&mut sortie, p.ancre, rendre_segments(&p.segments)),
            Bloc::Titre { niveau, texte, ancre } => {
                if *niveau == 1 {
                    vider(&mut sortie, &mut notes);
                }
                let diese = "#".repeat((*niveau).clamp(1, 3) as usize);
                let texte = echapper_md(&texte.replace(['\n', '\r'], " "), true);
                bloc_ancre(&mut sortie, *ancre, format!("{diese} {texte}"));
            }
            Bloc::Note { etiquette, segments, ancre, .. } => {
                notes.push((*ancre, format!("[^{etiquette}]: {}", rendre_segments(segments))));
            }
            Bloc::Saut { ancre } => bloc_ancre(&mut sortie, *ancre, "* * *".into()),
            Bloc::EntreeSommaire { segments, cible, ancre } => {
                bloc_ancre(&mut sortie, *ancre, format!("[{}](#{cible})", rendre_segments(segments)));
            }
        }
    }
    vider(&mut sortie, &mut notes);
    sortie.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocs::Paragraphe;
    use crate::modele::Moitie;

    fn ancre() -> Ancre {
        Ancre { page: 12, moitie: Moitie::Entiere, rect: [120, 145, 880, 210] }
    }

    fn texte(t: &str, gras: bool, italique: bool) -> Segment {
        Segment::Texte { texte: t.into(), gras, italique }
    }

    fn para(segments: Vec<Segment>) -> Bloc {
        Bloc::Paragraphe(Paragraphe { segments, ancre: ancre(), taille: 12.0, tout_gras: false, nb_lignes: 1 })
    }

    fn titre(niveau: u8, t: &str) -> Bloc {
        Bloc::Titre { niveau, texte: t.into(), ancre: ancre() }
    }

    #[test]
    fn echappement() {
        assert_eq!(echapper_md("# 3 & <b> *x* [^1]", true), "\\# 3 \\& \\<b\\> \\*x\\* \\[^1\\]");
        assert_eq!(echapper_md("1. Visite", true), "1\\. Visite");
        assert_eq!(echapper_md("1. Visite", false), "1. Visite");
        assert_eq!(echapper_md("12) a", true), "12\\) a");
        assert_eq!(echapper_md("a1. b", true), "a1. b");
    }

    #[test]
    fn style_et_espaces() {
        let b = para(vec![texte("Il dit ", false, false), texte("vraiment ", false, true), texte("oui", false, false)]);
        assert!(ecrire_markdown(&[b]).contains("\nIl dit *vraiment* oui\n"));
        let b = para(vec![texte("un", false, false), texte(" beau jour", false, true), texte(" x", true, true)]);
        assert!(ecrire_markdown(&[b]).contains("\nun *beau jour* ***x***\n"));
        let b = para(vec![texte("a", false, false), texte(" ", false, true), texte("b", false, false)]);
        assert!(ecrire_markdown(&[b]).contains("\na b\n"));
    }

    fn rendu(segments: Vec<Segment>) -> String {
        let md = ecrire_markdown(&[para(segments)]);
        md.lines().nth(1).unwrap().to_string()
    }

    #[test]
    fn debut_de_bloc_apres_espaces() {
        assert_eq!(rendu(vec![texte(" 1. Visite", false, false)]), "1\\. Visite");
        assert_eq!(rendu(vec![texte(" # x", false, false)]), "\\# x");
        assert_eq!(rendu(vec![texte(" ", false, false), texte("- x", false, false)]), "\\- x");
    }

    #[test]
    fn encadrement_des_marqueurs() {
        // ponctuation collée à une lettre : balises HTML
        assert_eq!(
            rendu(vec![texte("a ", false, false), texte("(mot)", false, true), texte("suite", false, false)]),
            "a <em>(mot)</em>suite"
        );
        assert_eq!(
            rendu(vec![texte("mot", false, false), texte("(x)", false, true)]),
            "mot<em>(x)</em>"
        );
        assert_eq!(rendu(vec![texte("mot", false, false), texte("ab", true, false)]), "mot**ab**");
        assert_eq!(
            rendu(vec![texte("a ", false, false), texte("(x)", true, true), texte("b", false, false)]),
            "a <strong><em>(x)</em></strong>b"
        );
        // ponctuation entourée d'espaces : marqueurs normaux
        assert_eq!(
            rendu(vec![texte("a ", false, false), texte("(mot)", false, true), texte(" b", false, false)]),
            "a *(mot)* b"
        );
    }

    #[test]
    fn segments_voisins_fusionnes() {
        assert_eq!(rendu(vec![texte("a", false, true), texte("b", false, true)]), "*ab*");
        assert_eq!(
            rendu(vec![texte("x ", false, false), texte("gras ", true, false), texte("ital", false, true)]),
            "x **gras** *ital*"
        );
    }

    #[test]
    fn exposants_et_appels() {
        let b = para(vec![
            texte("1", false, false),
            Segment::Exposant("er".into()),
            texte(" mot", false, false),
            Segment::AppelNote { etiquette: "7-12".into(), marqueur: "12".into() },
        ]);
        assert!(ecrire_markdown(&[b]).contains("\n1<sup>er</sup> mot[^7-12]\n"));
    }

    #[test]
    fn ancres_et_titres() {
        let md = ecrire_markdown(&[titre(2, "Titre"), para(vec![texte("Texte", false, false)])]);
        let a = "<!-- @p12 120,145,880,210 -->";
        assert_eq!(md, format!("{a}\n## Titre\n\n{a}\nTexte\n"));
    }

    #[test]
    fn saut_de_section() {
        let md = ecrire_markdown(&[para(vec![texte("Avant", false, false)]), Bloc::Saut { ancre: ancre() }]);
        let a = "<!-- @p12 120,145,880,210 -->";
        assert_eq!(md, format!("{a}\nAvant\n\n{a}\n* * *\n"));
        // Un paragraphe fait d'étoiles n'est jamais pris pour un saut.
        assert_eq!(rendu(vec![texte("* * *", false, false)]), "\\* \\* \\*");
    }

    #[test]
    fn titre_sur_une_seule_ligne() {
        let md = ecrire_markdown(&[titre(1, "Un\r\ndeux\rtrois")]);
        assert_eq!(md.lines().nth(1), Some("# Un  deux trois"), "{md}");
    }

    #[test]
    fn ancre_d_un_titre() {
        assert_eq!(ancre_titre("1 VISITE À PATMOS"), "t-1-visite-à-patmos");
        assert_eq!(ancre_titre("I. La pierre cubique"), "t-i-la-pierre-cubique");
        assert_eq!(ancre_titre("  L’Archange : « Mikhaël » !  "), "t-l-archange-mikhaël");
        // Lettre ou chiffre hors des noms XML (exposant ²) : un séparateur comme un autre.
        assert_eq!(ancre_titre("E = mc²"), "t-e-mc");
        assert_eq!(ancre_titre(" … "), "t");
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

    #[test]
    fn entree_de_sommaire_en_lien() {
        let entree = Bloc::EntreeSommaire {
            segments: vec![texte("1. VISITE À PATMOS", true, false)],
            cible: "t-1-visite-à-patmos".into(),
            ancre: ancre(),
        };
        let md = ecrire_markdown(&[entree]);
        assert_eq!(md.lines().nth(1), Some("[**1. VISITE À PATMOS**](#t-1-visite-à-patmos)"), "{md}");
        let entree = Bloc::EntreeSommaire {
            segments: vec![texte("Préface [suite] ........ 7", false, false)],
            cible: "t-préface".into(),
            ancre: ancre(),
        };
        let md = ecrire_markdown(&[entree]);
        assert_eq!(md.lines().nth(1), Some("[Préface \\[suite\\] ........ 7](#t-préface)"), "{md}");
    }

    #[test]
    fn notes_en_fin_de_chapitre() {
        let appel = Segment::AppelNote { etiquette: "1-1".into(), marqueur: "1".into() };
        let note = Bloc::Note {
            etiquette: "1-1".into(),
            marqueur: "1".into(),
            segments: vec![texte("Note", false, false)],
            ancre: ancre(),
        };
        let md = ecrire_markdown(&[titre(1, "A"), para(vec![texte("x", false, false), appel]), note, titre(1, "B")]);
        let def = md.find("[^1-1]: Note").unwrap();
        assert!(md[..def].ends_with("-->\n"));
        assert!(def < md.find("# B").unwrap());
        assert!(def > md.find("x[^1-1]").unwrap());
    }
}
