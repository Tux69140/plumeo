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
    let pos = |m: &str| c.markdown.find(m).unwrap_or_else(|| panic!("{m} absent : {}", c.markdown));
    assert!(pos("CHAPITRE UN") < pos("Premier paragraphe"));
    assert!(pos("Premier paragraphe") < pos("Deuxieme page"));
    assert!(pos("Deuxieme page") < pos("Troisieme page"));
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

fn avec_notes() -> ReglagesSerie {
    let mut r = reglages();
    r.paire.notes = 0.1;
    r.impaire.notes = 0.1;
    r
}

#[test]
fn apercu_avec_note_de_bas_de_page() {
    let p1 = [corps(200.0, "Un appel*"), corps(780.0, "* La note.")];
    let pdf = fabriquer_pdf(&[&p1]);
    let doc = DocumentPdf::ouvrir(&pdf).unwrap();
    let a = apercu_page(&doc, 1, &avec_notes()).unwrap();
    assert_eq!(a.moities[0].1, "Un appel*\n\n* La note.");
    assert!(a.points.is_empty());
    let vide = fabriquer_pdf(&[&[]]);
    let doc = DocumentPdf::ouvrir(&vide).unwrap();
    let a = apercu_page(&doc, 1, &reglages()).unwrap();
    assert_eq!(a.points[0].genre, GenrePoint::PageSansTexte);
}

#[test]
fn numero_de_page_residuel() {
    let p1 = [corps(200.0, "Du texte de page."), corps(780.0, "42")];
    let pdf = fabriquer_pdf(&[&p1]);
    let doc = DocumentPdf::ouvrir(&pdf).unwrap();
    let c = convertir_livre(&doc, &reglages(), None, &mut |_, _| {}).unwrap();
    assert!(c.retraits.iter().any(|r| r.role == Role::NumeroPage && r.texte == "42"));
    assert!(c.points.iter().any(|p| p.genre == GenrePoint::NumeroPageResiduel && p.ancre.page == 1));
    assert!(!c.markdown.contains("42"));
}

#[test]
fn forcer_titres_remplace_les_reglages() {
    let p1 = [t(100.0, "Un grand titre", 20.0, StyleFixture::Normal), corps(200.0, "Du texte de corps."), corps(220.0, "Encore du texte de corps.")];
    let pdf = fabriquer_pdf(&[&p1]);
    let doc = DocumentPdf::ouvrir(&pdf).unwrap();
    let mut r = reglages();
    r.titres = SourceTitres::GrasMajuscules;
    let sans = convertir_livre(&doc, &r, None, &mut |_, _| {}).unwrap();
    assert!(!sans.markdown.contains("# Un grand titre"), "{}", sans.markdown);
    let avec = convertir_livre(&doc, &r, Some(SourceTitres::Taille), &mut |_, _| {}).unwrap();
    assert_eq!(avec.source_titres, SourceTitres::Taille);
    assert!(avec.markdown.contains("# Un grand titre"), "{}", avec.markdown);
}

#[test]
fn double_page_coupee_en_deux() {
    let p1 = [
        TexteFixture { x_pt: 72.0, y_pt: 200.0, texte: "Texte gauche.", taille: 12.0, style: StyleFixture::Normal },
        TexteFixture { x_pt: 350.0, y_pt: 200.0, texte: "Texte droite.", taille: 12.0, style: StyleFixture::Normal },
    ];
    let pdf = fabriquer_pdf(&[&p1]);
    let doc = DocumentPdf::ouvrir(&pdf).unwrap();
    let mut r = reglages();
    r.double_page = Some(0.5);
    let c = convertir_livre(&doc, &r, None, &mut |_, _| {}).unwrap();
    let g = c.markdown.find("@p1g ").expect(&c.markdown);
    let d = c.markdown.find("@p1d ").expect(&c.markdown);
    assert!(g < d);
    assert!(c.markdown.find("Texte gauche").unwrap() < c.markdown.find("Texte droite").unwrap());
}

#[test]
fn note_de_bas_de_page() {
    let p1 = [corps(200.0, "Un appel*"), corps(780.0, "* La note.")];
    let pdf = fabriquer_pdf(&[&p1]);
    let doc = DocumentPdf::ouvrir(&pdf).unwrap();
    let c = convertir_livre(&doc, &avec_notes(), None, &mut |_, _| {}).unwrap();
    assert_eq!(c.nb_notes, 1);
    assert!(c.markdown.contains("appel[^1-*]"), "{}", c.markdown);
    assert!(c.markdown.contains("[^1-*]: La note."), "{}", c.markdown);
}

#[test]
fn notes_regroupees_en_fin_de_chapitre() {
    let p1 = [corps(200.0, "Un appel*")];
    let p2 = [corps(200.0, "NOTES"), corps(500.0, "* La note groupee.")];
    let pdf = fabriquer_pdf(&[&p1, &p2]);
    let doc = DocumentPdf::ouvrir(&pdf).unwrap();
    let c = convertir_livre(&doc, &reglages(), None, &mut |_, _| {}).unwrap();
    assert!(c.retraits.iter().any(|r| r.role == Role::TitreNotes), "{:?}", c.retraits);
    assert_eq!(c.nb_notes, 1, "{}", c.markdown);
    assert!(c.markdown.contains("[^1-*]: La note groupee."), "{}", c.markdown);
}

#[test]
fn notes_de_fin_de_livre_liees_depuis_les_chapitres_precedents() {
    use std::io::Read;
    let p1 = [t(100.0, "CHAPITRE UN", 20.0, StyleFixture::Gras), corps(200.0, "Un appel*")];
    let p2 = [t(100.0, "CHAPITRE DEUX", 20.0, StyleFixture::Gras), corps(200.0, "Un autre appel**")];
    let p3 = [corps(200.0, "NOTES"), corps(300.0, "* La premiere note."), corps(400.0, "** La seconde note.")];
    let pdf = fabriquer_pdf(&[&p1, &p2, &p3]);
    let doc = DocumentPdf::ouvrir(&pdf).unwrap();
    let c = convertir_livre(&doc, &reglages(), None, &mut |_, _| {}).unwrap();
    assert_eq!(c.nb_notes, 2, "{}", c.markdown);

    let dir = tempfile::tempdir().unwrap();
    let chemin = dir.path().join("livre.epub");
    let meta = plumeo_epub::Metadonnees {
        titre: "Livre".into(),
        auteur: String::new(),
        langue: "fr".into(),
        identifiant: "urn:uuid:12345678-1234-1234-1234-123456789abc".into(),
        modifie: "2026-11-10T12:00:00Z".into(),
    };
    plumeo_epub::generer_epub(&c.markdown, &meta, None, &chemin).unwrap();
    let mut z = zip::ZipArchive::new(std::fs::File::open(&chemin).unwrap()).unwrap();
    let mut lire = |nom: &str| {
        let mut s = String::new();
        z.by_name(nom).unwrap().read_to_string(&mut s).unwrap();
        s
    };
    let un = lire("OEBPS/chap-001.xhtml");
    let deux = lire("OEBPS/chap-002.xhtml");
    assert!(un.contains(r#"href="chap-002.xhtml#note-1""#), "{un}\n{}", c.markdown);
    assert!(deux.contains(r#"<a href="chap-001.xhtml#appel-1">↩</a>"#), "{deux}");
    assert!(deux.contains(r##"href="#note-2""##), "{deux}");
}
