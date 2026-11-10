//! Moteur de conversion PDF → EPUB de Pluméo.

#[cfg(feature = "fixtures")]
pub mod fixtures;
pub mod blocs;
pub mod convertir;
pub mod double_page;
pub mod lignes;
pub mod markdown;
pub mod metadonnees;
pub mod modele;
pub mod notes;
pub mod paragraphes;
pub mod pdf;
pub mod points;
pub mod reglages;
pub mod titres;
pub mod zones;

pub use convertir::{ApercuPage, Conversion, apercu_page, convertir_livre};
pub use markdown::{echapper_md, ecrire_markdown};
