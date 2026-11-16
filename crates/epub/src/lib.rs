mod chapitres;
mod paquet;
mod xhtml;

use std::path::Path;

pub use chapitres::est_titre_de_sommaire;
pub use xhtml::ancre_titre;

pub struct Metadonnees {
    pub titre: String,
    pub auteur: String,
    pub langue: String,
    pub identifiant: String,
    pub modifie: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurEpub {
    #[error("écriture : {0}")]
    Ecriture(#[from] std::io::Error),
    #[error("zip : {0}")]
    Zip(String),
    #[error("XHTML invalide dans le chapitre {chapitre} : {detail}")]
    XhtmlInvalide { chapitre: usize, detail: String },
    #[error("XML invalide dans {fichier} : {detail}")]
    XmlInvalide { fichier: String, detail: String },
}

pub fn generer_epub(
    markdown: &str,
    meta: &Metadonnees,
    couverture_png: Option<&[u8]>,
    sortie: &Path,
) -> Result<(), ErreurEpub> {
    let chapitres = chapitres::decouper(markdown, &meta.titre);
    for (i, c) in chapitres.iter().enumerate() {
        let page = xhtml::page(&meta.langue, &c.titre, &c.corps);
        if let Err(e) = roxmltree::Document::parse(&page) {
            return Err(ErreurEpub::XhtmlInvalide { chapitre: i + 1, detail: e.to_string() });
        }
    }
    // Métadonnées et titres arrivent aussi dans l'OPF et la nav : un caractère
    // interdit en XML y rendrait l'EPUB illisible.
    let opf = paquet::opf(&chapitres, meta, couverture_png.is_some());
    let nav = paquet::nav(&chapitres, meta);
    for (fichier, xml) in [("content.opf", &opf), ("nav.xhtml", &nav)] {
        if let Err(e) = roxmltree::Document::parse(xml) {
            return Err(ErreurEpub::XmlInvalide { fichier: fichier.into(), detail: e.to_string() });
        }
    }
    paquet::ecrire(&chapitres, meta, &opf, &nav, couverture_png, sortie)
}
