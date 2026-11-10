//! Fabrication de PDF de test (feature `fixtures`). Aucun PDF n'est versionné.

use pdfium_render::prelude::*;

use crate::pdf::pdfium;

#[derive(Clone, Copy, Debug)]
pub enum StyleFixture {
    Normal,
    Gras,
    Italique,
}

#[derive(Clone, Copy, Debug)]
pub struct TexteFixture {
    pub x_pt: f32,
    /// Distance depuis le haut de la page.
    pub y_pt: f32,
    pub texte: &'static str,
    pub taille: f32,
    pub style: StyleFixture,
}

const LARGEUR_A4: f32 = 595.0;
const HAUTEUR_A4: f32 = 842.0;

/// Fabrique un PDF A4 (595×842 pt, Helvetica) d'une page par élément de `pages`.
pub fn fabriquer_pdf(pages: &[&[TexteFixture]]) -> tempfile::TempPath {
    let pdfium = pdfium().expect("pdfium doit être disponible pour les fixtures");
    let mut doc = pdfium.create_new_pdf().expect("création du PDF");
    let normal = doc.fonts_mut().helvetica();
    let gras = doc.fonts_mut().helvetica_bold();
    let italique = doc.fonts_mut().helvetica_oblique();
    for textes in pages {
        let mut page = doc
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::Custom(
                PdfPoints::new(LARGEUR_A4),
                PdfPoints::new(HAUTEUR_A4),
            ))
            .expect("création de la page");
        for t in *textes {
            let police = match t.style {
                StyleFixture::Normal => normal,
                StyleFixture::Gras => gras,
                StyleFixture::Italique => italique,
            };
            page.objects_mut()
                .create_text_object(
                    PdfPoints::new(t.x_pt),
                    PdfPoints::new(HAUTEUR_A4 - t.y_pt),
                    t.texte,
                    police,
                    PdfPoints::new(t.taille),
                )
                .expect("ajout du texte");
        }
    }
    let fichier = tempfile::Builder::new()
        .suffix(".pdf")
        .tempfile()
        .expect("fichier temporaire");
    doc.save_to_file(fichier.path()).expect("écriture du PDF");
    fichier.into_temp_path()
}
