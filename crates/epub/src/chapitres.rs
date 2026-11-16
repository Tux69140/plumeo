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

/// Minuscules, sans accents ni ponctuation, espaces réduits.
fn normaliser(s: &str) -> String {
    let plie: String = s
        .chars()
        .flat_map(char::to_lowercase)
        // Accents combinants (texte décomposé).
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .map(|c| match c {
            'à' | 'â' | 'ä' | 'á' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' | 'í' => 'i',
            'ô' | 'ö' | 'ó' => 'o',
            'ù' | 'û' | 'ü' | 'ú' => 'u',
            'ç' => 'c',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        })
        .collect();
    plie.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Titre posé par le moteur sur un sommaire imprimé (`sommaire_cliquable`).
fn est_titre_de_sommaire(titre: &str) -> bool {
    matches!(normaliser(titre).as_str(), "table des matieres" | "sommaire")
}

/// Lien interne vers un titre (entrée de sommaire imprimé).
fn est_lien_vers_titre(e: &Event) -> bool {
    matches!(e, Event::Start(Tag::Link { dest_url, .. }) if dest_url.starts_with("#t-"))
}

/// Fin (exclue) du dernier bloc de premier niveau (paragraphe d'entrée) qui contient
/// un lien vers un titre.
fn fin_des_entrees(evenements: &[Event]) -> Option<usize> {
    let mut profondeur = 0usize;
    let mut avec_lien = false;
    let mut fin = None;
    for (i, e) in evenements.iter().enumerate() {
        avec_lien |= est_lien_vers_titre(e);
        match e {
            Event::Start(_) => profondeur += 1,
            Event::End(_) => {
                profondeur = profondeur.saturating_sub(1);
                if profondeur == 0 {
                    if avec_lien {
                        fin = Some(i + 1);
                    }
                    avec_lien = false;
                }
            }
            _ => {}
        }
    }
    fin
}

/// Le sommaire imprimé (premier chapitre « Table des matières » ou « Sommaire » qui
/// mène à des titres) vient juste après le chapitre du titre du livre. Ce qui suit sa
/// dernière entrée (colophon…) et ses définitions de notes restent à leur place dans
/// l'ordre de lecture : à la fin du chapitre qui le précédait.
fn placer_le_sommaire(chapitres: &mut Vec<Brouillon>) {
    let Some(s) = chapitres
        .iter()
        .position(|c| est_titre_de_sommaire(&c.titre) && c.evenements.iter().any(est_lien_vers_titre))
    else {
        return;
    };
    if s < 2 {
        return;
    }
    let mut sommaire = chapitres.remove(s);
    let fin = fin_des_entrees(&sommaire.evenements).unwrap_or(sommaire.evenements.len());
    let reste = sommaire.evenements.split_off(fin);
    let ids_du_reste: Vec<&str> = reste
        .iter()
        .filter_map(|e| match e {
            Event::Start(Tag::Heading { id: Some(id), .. }) => Some(&**id),
            _ => None,
        })
        .collect();
    let (partent, restent): (Vec<_>, Vec<_>) =
        std::mem::take(&mut sommaire.sous_titres).into_iter().partition(|(id, _)| ids_du_reste.contains(&id.as_str()));
    sommaire.sous_titres = restent;
    let precedent = &mut chapitres[s - 1];
    precedent.sous_titres.extend(partent);
    precedent.evenements.extend(reste);
    precedent.notes.append(&mut sommaire.notes);
    chapitres.insert(1, sommaire);
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
                    // Identifiant unique dans tout le livre, attribué dans l'ordre du
                    // Markdown (comme le moteur) ; recensé une fois l'ordre final connu.
                    let ancre = ancres.attribuer(&texte);
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
    placer_le_sommaire(&mut chapitres);
    for (i, c) in chapitres.iter_mut().enumerate() {
        c.preparer();
        for e in &c.evenements {
            if let Event::Start(Tag::Heading { id: Some(id), .. }) = e {
                renvois.recenser_titre(id, i);
            }
        }
        renvois.recenser(i, &c.evenements, &c.notes);
    }
    chapitres.into_iter().enumerate().map(|(i, c)| c.finir(i, titre_livre, &mut renvois)).collect()
}
