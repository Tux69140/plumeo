//! Lecture du Markdown interne et découpage en chapitres.

use pulldown_cmark::{CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::xhtml;

pub struct Chapitre {
    pub titre: String,
    /// Titres de niveau 2 : (identifiant d'ancre, texte).
    pub sous_titres: Vec<(String, String)>,
    /// Corps XHTML (notes comprises).
    pub corps: String,
}

struct Brouillon {
    titre: String,
    sous_titres: Vec<(String, String)>,
    evenements: Vec<Event<'static>>,
    notes: Vec<(String, Vec<Event<'static>>)>,
    significatif: bool,
}

impl Brouillon {
    fn nouveau(titre: String) -> Self {
        Brouillon { titre, sous_titres: Vec::new(), evenements: Vec::new(), notes: Vec::new(), significatif: false }
    }

    fn finir(self, titre_livre: &str) -> Chapitre {
        Chapitre {
            titre: if self.titre.trim().is_empty() { titre_livre.to_string() } else { self.titre },
            sous_titres: self.sous_titres,
            corps: xhtml::rendre_corps(self.evenements, self.notes),
        }
    }
}

fn est_ancre(e: &Event) -> bool {
    matches!(e, Event::Html(h) if h.trim_start().starts_with("<!-- @"))
}

pub fn decouper(markdown: &str, titre_livre: &str) -> Vec<Chapitre> {
    let parser = Parser::new_ext(markdown, Options::ENABLE_FOOTNOTES);
    let mut chapitres: Vec<Chapitre> = Vec::new();
    let mut courant = Brouillon::nouveau(titre_livre.to_string());
    let mut dans_titre: Option<(HeadingLevel, String)> = None;
    let mut dans_note: Option<(String, Vec<Event<'static>>)> = None;

    for evt in parser {
        if est_ancre(&evt) {
            continue;
        }
        let evt = evt.into_static();
        match evt {
            Event::Start(Tag::FootnoteDefinition(label)) => {
                dans_note = Some((label.to_string(), Vec::new()));
                continue;
            }
            Event::End(TagEnd::FootnoteDefinition) => {
                if let Some(n) = dans_note.take() {
                    courant.notes.push(n);
                }
                continue;
            }
            _ => {}
        }
        if let Some((_, contenu)) = dans_note.as_mut() {
            contenu.push(evt);
            continue;
        }
        match evt {
            Event::Start(Tag::Heading { level: HeadingLevel::H1, .. }) => {
                if courant.significatif || !chapitres.is_empty() || !courant.notes.is_empty() {
                    chapitres.push(courant.finir(titre_livre));
                }
                courant = Brouillon::nouveau(String::new());
                courant.significatif = true;
                dans_titre = Some((HeadingLevel::H1, String::new()));
                courant.evenements.push(Event::Start(Tag::Heading {
                    level: HeadingLevel::H1,
                    id: None,
                    classes: vec![],
                    attrs: vec![],
                }));
            }
            Event::Start(Tag::Heading { level: HeadingLevel::H2, classes, attrs, .. }) => {
                let id = format!("s{}", courant.sous_titres.len() + 1);
                dans_titre = Some((HeadingLevel::H2, String::new()));
                courant.significatif = true;
                courant.evenements.push(Event::Start(Tag::Heading {
                    level: HeadingLevel::H2,
                    id: Some(CowStr::from(id)),
                    classes,
                    attrs,
                }));
            }
            Event::End(TagEnd::Heading(niveau)) => {
                if let Some((n, texte)) = dans_titre.take() {
                    let texte = texte.trim().to_string();
                    if n == HeadingLevel::H1 {
                        courant.titre = texte;
                    } else if n == HeadingLevel::H2 {
                        let id = format!("s{}", courant.sous_titres.len() + 1);
                        courant.sous_titres.push((id, texte));
                    }
                }
                courant.evenements.push(Event::End(TagEnd::Heading(niveau)));
            }
            e => {
                if let Some((_, texte)) = dans_titre.as_mut() {
                    if let Event::Text(t) | Event::Code(t) = &e {
                        texte.push_str(t);
                    }
                }
                if !matches!(
                    &e,
                    Event::SoftBreak | Event::HardBreak | Event::Start(Tag::HtmlBlock) | Event::End(TagEnd::HtmlBlock)
                )
                    && !matches!(&e, Event::Text(t) if t.trim().is_empty())
                {
                    courant.significatif = true;
                }
                courant.evenements.push(e);
            }
        }
    }
    if courant.significatif || chapitres.is_empty() {
        chapitres.push(courant.finir(titre_livre));
    }
    chapitres
}
