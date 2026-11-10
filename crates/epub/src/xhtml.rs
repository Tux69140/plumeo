//! Rendu XHTML : corps de chapitre, notes, pages complètes.

use pulldown_cmark::{Event, html::push_html};

pub fn echapper(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Sépare l'étiquette `<seq>-<marqueur>` en (identifiant, texte affiché).
fn decomposer(label: &str) -> (&str, &str) {
    label.split_once('-').unwrap_or((label, label))
}

fn normaliser_vides(html: &str) -> String {
    html.replace("<br>", "<br />").replace("<hr>", "<hr />")
}

fn rendre(evenements: impl Iterator<Item = Event<'static>>) -> String {
    let mut sortie = String::new();
    push_html(&mut sortie, evenements.map(|e| match e {
        Event::Html(h) => Event::Html(normaliser_vides(&h).into()),
        Event::InlineHtml(h) => Event::InlineHtml(normaliser_vides(&h).into()),
        Event::FootnoteReference(l) => {
            let (id, affiche) = decomposer(&l);
            Event::Html(
                format!(
                    "<a epub:type=\"noteref\" id=\"appel-{}\" href=\"#note-{}\"><sup>{}</sup></a>",
                    echapper(id),
                    echapper(id),
                    echapper(affiche)
                )
                .into(),
            )
        }
        e => e,
    }));
    sortie
}

pub fn rendre_corps(evenements: Vec<Event<'static>>, notes: Vec<(String, Vec<Event<'static>>)>) -> String {
    let mut corps = rendre(evenements.into_iter());
    for (label, contenu) in notes {
        let (id, _) = decomposer(&label);
        let id = echapper(id);
        corps.push_str(&format!("<aside epub:type=\"footnote\" id=\"note-{id}\">\n"));
        corps.push_str(&rendre(contenu.into_iter()));
        corps.push_str(&format!("<p><a href=\"#appel-{id}\">↩</a></p>\n</aside>\n"));
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
