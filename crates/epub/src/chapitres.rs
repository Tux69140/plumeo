//! Lecture du Markdown interne et découpage en chapitres.

use pulldown_cmark::{CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::xhtml;

pub struct Chapitre {
    pub titre: String,
    /// Titres de niveau 2 : (identifiant du titre, voir `xhtml::ancre_titre`, texte).
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

    /// Fusionne les appels orphelins (corps et notes) avant recensement et rendu.
    fn preparer(&mut self) {
        self.evenements = xhtml::fusionner_appels_orphelins(std::mem::take(&mut self.evenements));
        for (_, contenu) in &mut self.notes {
            *contenu = xhtml::fusionner_appels_orphelins(std::mem::take(contenu));
        }
    }

    fn finir(self, indice: usize, titre_livre: &str, renvois: &mut xhtml::Renvois) -> Chapitre {
        Chapitre {
            titre: if self.titre.trim().is_empty() { titre_livre.to_string() } else { self.titre },
            sous_titres: self.sous_titres,
            corps: xhtml::rendre_corps(indice, self.evenements, self.notes, renvois),
        }
    }
}

fn est_ancre(e: &Event) -> bool {
    matches!(e, Event::Html(h) if h.trim_start().starts_with("<!-- @"))
}

pub fn decouper(markdown: &str, titre_livre: &str) -> Vec<Chapitre> {
    let parser = Parser::new_ext(markdown, Options::ENABLE_FOOTNOTES);
    let mut chapitres: Vec<Brouillon> = Vec::new();
    let mut courant = Brouillon::nouveau(titre_livre.to_string());
    // Titre en cours : niveau, texte, position de son événement d'ouverture (qui reçoit
    // l'identifiant une fois le texte connu).
    let mut dans_titre: Option<(HeadingLevel, String, usize)> = None;
    let mut ancres = xhtml::AncresTitres::default();
    // Les renvois (notes, titres) sont résolus sur tout le livre avant le rendu.
    let mut renvois = xhtml::Renvois::default();
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
            Event::Start(Tag::Heading { level, classes, attrs, .. }) => {
                if level == HeadingLevel::H1 {
                    if courant.significatif || !chapitres.is_empty() || !courant.notes.is_empty() {
                        chapitres.push(courant);
                    }
                    courant = Brouillon::nouveau(String::new());
                }
                courant.significatif = true;
                dans_titre = Some((level, String::new(), courant.evenements.len()));
                courant.evenements.push(Event::Start(Tag::Heading { level, id: None, classes, attrs }));
            }
            Event::End(TagEnd::Heading(niveau)) => {
                if let Some((n, texte, position)) = dans_titre.take() {
                    let texte = texte.trim().to_string();
                    // Identifiant unique dans tout le livre ; le chapitre courant sera
                    // le prochain poussé.
                    let ancre = ancres.attribuer(&texte);
                    renvois.recenser_titre(&ancre, chapitres.len());
                    if let Event::Start(Tag::Heading { id, .. }) = &mut courant.evenements[position] {
                        *id = Some(CowStr::from(ancre.clone()));
                    }
                    if n == HeadingLevel::H1 {
                        courant.titre = texte;
                    } else if n == HeadingLevel::H2 {
                        courant.sous_titres.push((ancre, texte));
                    }
                }
                courant.evenements.push(Event::End(TagEnd::Heading(niveau)));
            }
            e => {
                if let Some((_, texte, _)) = dans_titre.as_mut() {
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
        chapitres.push(courant);
    }
    for (i, c) in chapitres.iter_mut().enumerate() {
        c.preparer();
        renvois.recenser(i, &c.evenements, &c.notes);
    }
    chapitres.into_iter().enumerate().map(|(i, c)| c.finir(i, titre_livre, &mut renvois)).collect()
}
