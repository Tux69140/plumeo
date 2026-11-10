use plumeo_moteur::fixtures::{StyleFixture, TexteFixture, fabriquer_pdf};
use plumeo_moteur::modele::{Ancre, ErreurMoteur, Moitie};
use plumeo_moteur::pdf::{DocumentPdf, nettoyer_texte};

#[test]
fn lit_les_mots_avec_position_et_taille() {
    let pdf = fabriquer_pdf(&[&[TexteFixture {
        x_pt: 72.0,
        y_pt: 100.0,
        texte: "Bonjour le monde",
        taille: 12.0,
        style: StyleFixture::Normal,
    }]]);
    let doc = DocumentPdf::ouvrir(&pdf).unwrap();
    assert_eq!(doc.nb_pages(), 1);
    let p = doc.page(1).unwrap();
    let textes: Vec<_> = p.mots.iter().map(|m| m.texte.as_str()).collect();
    assert_eq!(textes, ["Bonjour", "le", "monde"]);
    assert!((p.mots[0].taille - 12.0).abs() <= 0.5);
    assert!((p.mots[0].rect.x0 - 72.0 / 595.0).abs() < 0.01);
    assert!(p.mots[0].rect.y0 < 100.0 / 842.0 && p.mots[0].rect.y1 > 90.0 / 842.0);
    assert!(!p.mots[0].espace_avant && p.mots[1].espace_avant);
}

#[test]
fn reconnait_gras_et_italique() {
    let pdf = fabriquer_pdf(&[&[
        TexteFixture { x_pt: 72.0, y_pt: 100.0, texte: "Gras", taille: 12.0, style: StyleFixture::Gras },
        TexteFixture { x_pt: 72.0, y_pt: 130.0, texte: "Penche", taille: 12.0, style: StyleFixture::Italique },
        TexteFixture { x_pt: 72.0, y_pt: 160.0, texte: "Droit", taille: 12.0, style: StyleFixture::Normal },
    ]]);
    let doc = DocumentPdf::ouvrir(&pdf).unwrap();
    let p = doc.page(1).unwrap();
    let mot = |t: &str| p.mots.iter().find(|m| m.texte == t).unwrap().clone();
    let (g, i, d) = (mot("Gras"), mot("Penche"), mot("Droit"));
    assert!(g.gras && !g.italique);
    assert!(i.italique && !i.gras);
    assert!(!d.gras && !d.italique);
}

#[test]
fn pdf_illisible_donne_une_erreur() {
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), b"pas un pdf").unwrap();
    assert!(matches!(DocumentPdf::ouvrir(f.path()), Err(ErreurMoteur::PdfIllisible(_))));
}

#[test]
fn nettoie_les_caracteres_parasites() {
    assert_eq!(nettoyer_texte("ef\u{fb01}cace\u{2}\u{fffe}"), "efficace");
    assert_eq!(nettoyer_texte("Cite\u{301}"), "Cité"); // NFC
}

#[test]
fn ancre_aller_retour() {
    let a: Ancre = "@p6g 120,145,880,210".parse().unwrap();
    assert_eq!((a.page, a.moitie, a.rect), (6, Moitie::Gauche, [120, 145, 880, 210]));
    assert_eq!(a.to_string(), "@p6g 120,145,880,210");
}

#[test]
fn rend_une_page_en_png() {
    let pdf = fabriquer_pdf(&[&[TexteFixture {
        x_pt: 72.0,
        y_pt: 100.0,
        texte: "Bonjour",
        taille: 12.0,
        style: StyleFixture::Normal,
    }]]);
    let doc = DocumentPdf::ouvrir(&pdf).unwrap();
    let png = doc.rendre_png(1, 300).unwrap();
    assert!(png.starts_with(b"\x89PNG"));
}

#[test]
fn tiret_de_fin_de_ligne_conserve() {
    // pdfium rend le tiret qui termine une ligne par le caractère de contrôle \u{2} :
    // il ne doit pas être pris pour une espace.
    let t = |y_pt, texte| TexteFixture { x_pt: 72.0, y_pt, texte, taille: 12.0, style: StyleFixture::Normal };
    let pdf = fabriquer_pdf(&[&[t(100.0, "Un mer-"), t(114.0, "veilleux jour")]]);
    let doc = DocumentPdf::ouvrir(&pdf).unwrap();
    let p = doc.page(1).unwrap();
    let textes: Vec<_> = p.mots.iter().map(|m| m.texte.as_str()).collect();
    assert_eq!(textes, ["Un", "mer-", "veilleux", "jour"]);
}
