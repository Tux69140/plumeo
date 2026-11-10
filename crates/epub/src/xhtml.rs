//! Rendu XHTML : corps de chapitre, notes, pages complètes.

use std::collections::{HashMap, HashSet};

use pulldown_cmark::{Event, html::push_html};

pub fn echapper(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Sépare l'étiquette `<seq>-<marqueur>` en (identifiant, texte affiché).
fn decomposer(label: &str) -> (&str, &str) {
    label.split_once('-').unwrap_or((label, label))
}

fn entite_valide(reste: &str) -> bool {
    match reste.find(';') {
        Some(n) if n > 1 => reste[1..n].chars().all(|c| c.is_ascii_alphanumeric() || c == '#'),
        _ => false,
    }
}

/// Liste blanche du HTML brut : `sup`, `em`, `strong` passent tels quels ;
/// `br`, `hr`, `img` deviennent auto-fermants ; tout le reste est échappé en texte.
fn assainir_html(html: &str) -> String {
    let mut sortie = String::new();
    let mut reste = html;
    while let Some(c) = reste.chars().next() {
        if c == '<' {
            if let Some(fin) = reste.find('>') {
                let balise = &reste[1..fin];
                let (fermante, corps) = match balise.strip_prefix('/') {
                    Some(b) => (true, b),
                    None => (false, balise),
                };
                let nom_fin = corps.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(corps.len());
                let nom = corps[..nom_fin].to_ascii_lowercase();
                let sans_attributs = corps[nom_fin..].trim().is_empty();
                match nom.as_str() {
                    "sup" | "em" | "strong" if sans_attributs => {
                        sortie.push_str(&format!("<{}{nom}>", if fermante { "/" } else { "" }));
                        reste = &reste[fin + 1..];
                        continue;
                    }
                    "br" | "hr" | "img" if !fermante => {
                        let attrs = corps[nom_fin..].trim().trim_end_matches('/').trim_end();
                        if attrs.is_empty() {
                            sortie.push_str(&format!("<{nom} />"));
                        } else {
                            sortie.push_str(&format!("<{nom} {attrs} />"));
                        }
                        reste = &reste[fin + 1..];
                        continue;
                    }
                    _ => {}
                }
            }
            sortie.push_str("&lt;");
        } else if c == '&' {
            sortie.push_str(if entite_valide(reste) { "&" } else { "&amp;" });
        } else if c == '>' {
            sortie.push_str("&gt;");
        } else {
            sortie.push(c);
        }
        reste = &reste[c.len_utf8()..];
    }
    sortie
}

/// Remplace les triplets `[`, `^étiquette`, `]` (appel sans définition,
/// que pulldown-cmark laisse en texte) par un `FootnoteReference`.
fn fusionner_appels_orphelins(evenements: Vec<Event<'static>>) -> Vec<Event<'static>> {
    let mut sortie: Vec<Event<'static>> = Vec::new();
    let mut it = evenements.into_iter().peekable();
    while let Some(e) = it.next() {
        if matches!(&e, Event::Text(t) if &**t == "[") {
            if let Some(Event::Text(m)) = it.peek() {
                if m.len() > 1 && m.starts_with('^') {
                    let m = m.clone();
                    let mut copie = it.clone();
                    copie.next();
                    if matches!(copie.peek(), Some(Event::Text(t)) if &**t == "]") {
                        it.next();
                        it.next();
                        sortie.push(Event::FootnoteReference(m[1..].to_string().into()));
                        continue;
                    }
                }
            }
        }
        sortie.push(e);
    }
    sortie
}

#[derive(Default)]
struct Appels {
    definis: HashSet<String>,
    comptes: HashMap<String, usize>,
}

fn rendre(evenements: Vec<Event<'static>>, appels: &mut Appels) -> String {
    let mut rendus = Vec::new();
    for e in fusionner_appels_orphelins(evenements) {
        rendus.push(match e {
            Event::Html(h) => Event::Html(assainir_html(&h).into()),
            Event::InlineHtml(h) => Event::InlineHtml(assainir_html(&h).into()),
            Event::FootnoteReference(l) => {
                let (id, affiche) = decomposer(&l);
                let (id, affiche) = (echapper(id), echapper(affiche));
                if !appels.definis.contains(&l.to_string()) {
                    Event::Html(format!("<sup>{affiche}</sup>").into())
                } else {
                    let n = appels.comptes.entry(l.to_string()).or_insert(0);
                    *n += 1;
                    let suffixe = if *n == 1 { String::new() } else { format!("-{n}") };
                    Event::Html(
                        format!(
                            "<a epub:type=\"noteref\" id=\"appel-{id}{suffixe}\" href=\"#note-{id}\"><sup>{affiche}</sup></a>"
                        )
                        .into(),
                    )
                }
            }
            e => e,
        });
    }
    let mut sortie = String::new();
    push_html(&mut sortie, rendus.into_iter());
    sortie
}

pub fn rendre_corps(evenements: Vec<Event<'static>>, notes: Vec<(String, Vec<Event<'static>>)>) -> String {
    let mut appels = Appels { definis: notes.iter().map(|(l, _)| l.clone()).collect(), ..Appels::default() };
    let mut corps = rendre(evenements, &mut appels);
    for (label, contenu) in notes {
        let (id, _) = decomposer(&label);
        let id = echapper(id);
        corps.push_str(&format!("<aside epub:type=\"footnote\" id=\"note-{id}\">\n"));
        corps.push_str(&rendre(contenu, &mut appels));
        if appels.comptes.contains_key(&label) {
            corps.push_str(&format!("<p><a href=\"#appel-{id}\">↩</a></p>\n"));
        }
        corps.push_str("</aside>\n");
    }
    corps
}

pub fn page(langue: &str, titre: &str, corps: &str) -> String {
    let l = echapper(langue);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"{l}\" lang=\"{l}\">\n<head>\n<meta charset=\"utf-8\" />\n<title>{}</title>\n<link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\" />\n</head>\n<body>\n{corps}</body>\n</html>\n",
        echapper(titre)
    )
}
