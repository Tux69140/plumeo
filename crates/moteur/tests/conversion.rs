use plumeo_moteur::fixtures::{StyleFixture, TexteFixture, fabriquer_pdf};
use plumeo_moteur::pdf::DocumentPdf;
use plumeo_moteur::points::GenrePoint;
use plumeo_moteur::reglages::{ReglagesSerie, SourceTitres, Zones};
use plumeo_moteur::zones::Role;
use plumeo_moteur::{apercu_page, convertir_livre};

fn reglages() -> ReglagesSerie {
    let z = Zones { entete: 0.06, pied: 0.0, notes: 0.0 };
    ReglagesSerie {
        nom: "test".into(),
        langue: "fra".into(),
        paire: z.clone(),
        impaire: z,
        double_page: None,
        titres: SourceTitres::Auto,
        exceptions: Default::default(),
    }
}

fn t(y: f32, texte: &'static str, taille: f32, style: StyleFixture) -> TexteFixture {
    TexteFixture { x_pt: 72.0, y_pt: y, texte, taille, style }
}

fn en_tete() -> TexteFixture {
    t(30.0, "Mon livre", 10.0, StyleFixture::Normal)
}

fn corps(y: f32, texte: &'static str) -> TexteFixture {
    t(y, texte, 12.0, StyleFixture::Normal)
}

#[test]
fn livre_de_trois_pages() {
    let p1 = [en_tete(), t(100.0, "CHAPITRE UN", 20.0, StyleFixture::Gras), corps(200.0, "Premier paragraphe de la page un.")];
    let p2 = [en_tete(), corps(200.0, "Deuxieme page avec du texte.")];
    let p3 = [en_tete(), corps(200.0, "Troisieme page avec du texte.")];
    let pdf = fabriquer_pdf(&[&p1, &p2, &p3]);
    let doc = DocumentPdf::ouvrir(&pdf).unwrap();
    let c = convertir_livre(&doc, &reglages(), None, &mut |_, _| {}).unwrap();
    assert!(c.markdown.contains("# CHAPITRE UN"), "{}", c.markdown);
    assert!(!c.markdown.contains("Mon livre"));
    let en_tetes = c.retraits.iter().filter(|r| r.role == Role::EnTete).count();
    assert_eq!(en_tetes, 3);
    assert_eq!(c.nb_pages, 3);
}

#[test]
fn page_vide_au_milieu() {
    let p1 = [corps(200.0, "Texte de la premiere page.")];
    let p3 = [corps(200.0, "Texte de la troisieme page.")];
    let pdf = fabriquer_pdf(&[&p1, &[], &p3]);
    let doc = DocumentPdf::ouvrir(&pdf).unwrap();
    let c = convertir_livre(&doc, &reglages(), None, &mut |_, _| {}).unwrap();
    let vides: Vec<_> = c.points.iter().filter(|p| p.genre == GenrePoint::PageSansTexte).collect();
    assert_eq!(vides.len(), 1);
    assert_eq!(vides[0].ancre.page, 2);
    assert!(c.markdown.contains("premiere page"));
    assert!(c.markdown.contains("troisieme page"));
}

#[test]
fn progression_appelee() {
    let p = [corps(200.0, "Texte.")];
    let pdf = fabriquer_pdf(&[&p, &p, &p]);
    let doc = DocumentPdf::ouvrir(&pdf).unwrap();
    let mut appels = Vec::new();
    convertir_livre(&doc, &reglages(), None, &mut |a, b| appels.push((a, b))).unwrap();
    assert_eq!(appels, [(1, 3), (2, 3), (3, 3)]);
}

#[test]
fn apercu_d_une_page() {
    let p1 = [en_tete(), corps(200.0, "Contenu du corps.")];
    let pdf = fabriquer_pdf(&[&p1]);
    let doc = DocumentPdf::ouvrir(&pdf).unwrap();
    let a = apercu_page(&doc, 1, &reglages()).unwrap();
    assert_eq!(a.moities.len(), 1);
    let (_, texte, retraits) = &a.moities[0];
    assert!(texte.contains("Contenu du corps."), "{texte}");
    assert!(!texte.contains("Mon livre"));
    assert_eq!(retraits.len(), 1);
}
