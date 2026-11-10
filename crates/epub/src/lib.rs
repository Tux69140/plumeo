mod chapitres;
mod paquet;
mod xhtml;

use std::path::Path;

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
    paquet::ecrire(&chapitres, meta, couverture_png, sortie)
}
