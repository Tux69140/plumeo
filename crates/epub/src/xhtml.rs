//! Rendu XHTML : corps de chapitre, notes, pages complètes.

use std::collections::HashMap;

use pulldown_cmark::{Event, Tag, TagEnd, html::push_html};

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

/// Nom du fichier XHTML du chapitre d'indice `i` (base 0).
pub fn nom_chapitre(i: usize) -> String {
    format!("chap-{:03}.xhtml", i + 1)
}

/// Remplace les triplets `[`, `^étiquette`, `]` (appel sans définition,
/// que pulldown-cmark laisse en texte) par un `FootnoteReference`.
pub fn fusionner_appels_orphelins(evenements: Vec<Event<'static>>) -> Vec<Event<'static>> {
    let mut sortie: Vec<Event<'static>> = Vec::new();
    let mut it = evenements.into_iter().peekable();
    while let Some(e) = it.next() {
        if matches!(&e, Event::Text(t) if &**t == "[")
            && let Some(Event::Text(m)) = it.peek()
            && m.len() > 1
            && m.starts_with('^')
        {
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
        sortie.push(e);
    }
    sortie
}

/// Renvois à l'échelle du livre. Notes : un appel peut se trouver dans un autre
/// chapitre que sa définition (notes de fin de livre) ; les identifiants `appel-…`
/// sont numérotés sur tout le livre, donc uniques. Titres : un lien `#id` (entrée du
/// sommaire imprimé) mène au fichier du chapitre qui porte ce titre.
#[derive(Default)]
pub struct Renvois {
    /// Identifiant de titre → chapitre qui le porte.
    titres: HashMap<String, usize>,
    /// Étiquette → chapitre de sa (première) définition.
    definitions: HashMap<String, usize>,
    /// Étiquette → chapitre de son premier appel.
    premiers_appels: HashMap<String, usize>,
    /// Appels déjà rendus par étiquette.
    comptes: HashMap<String, usize>,
}

impl Renvois {
    pub fn recenser_titre(&mut self, id: &str, chapitre: usize) {
        self.titres.insert(id.to_string(), chapitre);
    }

    /// Recense les appels et définitions d'un chapitre, dans l'ordre du rendu
    /// (corps puis contenu des notes).
    pub fn recenser(
        &mut self,
        chapitre: usize,
        evenements: &[Event<'static>],
        notes: &[(String, Vec<Event<'static>>)],
    ) {
        let mut appels = |evts: &[Event<'static>]| {
            for e in evts {
                if let Event::FootnoteReference(l) = e {
                    self.premiers_appels.entry(l.to_string()).or_insert(chapitre);
                }
            }
        };
        appels(evenements);
        for (_, contenu) in notes {
            appels(contenu);
        }
        for (label, _) in notes {
            self.definitions.entry(label.clone()).or_insert(chapitre);
        }
    }
}

/// Lien vers `#<prefixe>-<id>`, préfixé du fichier cible s'il diffère du chapitre courant.
fn lien(cible: usize, courant: usize, prefixe: &str, id: &str) -> String {
    let fichier = if cible == courant { String::new() } else { nom_chapitre(cible) };
    format!("{fichier}#{prefixe}-{id}")
}

fn rendre(evenements: Vec<Event<'static>>, chapitre: usize, renvois: &mut Renvois) -> String {
    let mut rendus = Vec::new();
    // Pour chaque lien ouvert : est-il rendu (cible connue) ?
    let mut liens: Vec<bool> = Vec::new();
    for e in evenements {
        rendus.push(match e {
            // Lien vers un titre : vers le fichier de son chapitre ; sans cible connue,
            // texte simple (pas de lien cassé).
            Event::Start(Tag::Link { dest_url, .. }) => {
                let cible = dest_url.strip_prefix('#').and_then(|id| Some((id, *renvois.titres.get(id)?)));
                liens.push(cible.is_some());
                match cible {
                    Some((id, c)) => Event::Html(format!("<a href=\"{}#{}\">", nom_chapitre(c), echapper(id)).into()),
                    None => continue,
                }
            }
            Event::End(TagEnd::Link) => match liens.pop() {
                Some(true) => Event::Html("</a>".into()),
                _ => continue,
            },
            Event::Html(h) => Event::Html(assainir_html(&h).into()),
            Event::InlineHtml(h) => Event::InlineHtml(assainir_html(&h).into()),
            Event::FootnoteReference(l) => {
                let (id, affiche) = decomposer(&l);
                let (id, affiche) = (echapper(id), echapper(affiche));
                match renvois.definitions.get(&*l) {
                    None => Event::Html(format!("<sup>{affiche}</sup>").into()),
                    Some(&definition) => {
                        let n = renvois.comptes.entry(l.to_string()).or_insert(0);
                        *n += 1;
                        let suffixe = if *n == 1 { String::new() } else { format!("-{n}") };
                        let href = lien(definition, chapitre, "note", &id);
                        Event::Html(
                            format!(
                                "<a epub:type=\"noteref\" id=\"appel-{id}{suffixe}\" \
                                 href=\"{href}\"><sup>{affiche}</sup></a>"
                            )
                            .into(),
                        )
                    }
                }
            }
            e => e,
        });
    }
    let mut sortie = String::new();
    push_html(&mut sortie, rendus.into_iter());
    sortie
}

/// Corps XHTML du chapitre d'indice `chapitre` ; `evenements` et `notes` doivent
/// avoir été recensés dans `renvois` (avec les appels orphelins déjà fusionnés).
pub fn rendre_corps(
    chapitre: usize,
    evenements: Vec<Event<'static>>,
    notes: Vec<(String, Vec<Event<'static>>)>,
    renvois: &mut Renvois,
) -> String {
    let mut corps = rendre(evenements, chapitre, renvois);
    for (label, contenu) in notes {
        let (id, _) = decomposer(&label);
        let id = echapper(id);
        corps.push_str(&format!("<aside epub:type=\"footnote\" id=\"note-{id}\">\n"));
        corps.push_str(&rendre(contenu, chapitre, renvois));
        if let Some(&appel) = renvois.premiers_appels.get(&label) {
            corps.push_str(&format!("<p><a href=\"{}\">↩</a></p>\n", lien(appel, chapitre, "appel", &id)));
        }
        corps.push_str("</aside>\n");
    }
    corps
}

pub fn page(langue: &str, titre: &str, corps: &str) -> String {
    let l = echapper(langue);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" \
         xml:lang=\"{l}\" lang=\"{l}\">\n\
         <head>\n\
         <meta charset=\"utf-8\" />\n\
         <title>{}</title>\n\
         <link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\" />\n\
         </head>\n\
         <body>\n\
         {corps}</body>\n\
         </html>\n",
        echapper(titre)
    )
}
