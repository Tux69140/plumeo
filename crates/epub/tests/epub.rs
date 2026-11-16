use plumeo_epub::{Metadonnees, generer_epub};
use std::io::Read;
use std::path::Path;

fn meta() -> Metadonnees {
    Metadonnees {
        titre: "Le Livre".into(),
        auteur: "Ada Auteur".into(),
        langue: "fr".into(),
        identifiant: "urn:uuid:12345678-1234-1234-1234-123456789abc".into(),
        modifie: "2026-11-10T12:00:00Z".into(),
    }
}

/// PNG 1x1 minimal valide.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xff, 0xff, 0x3f,
    0x00, 0x05, 0xfe, 0x02, 0xfe, 0xa7, 0x35, 0x81, 0x84, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
    0x44, 0xae, 0x42, 0x60, 0x82,
];

fn fabriquer(md: &str, couv: Option<&[u8]>) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let chemin = dir.path().join("livre.epub");
    generer_epub(md, &meta(), couv, &chemin).unwrap();
    (dir, chemin)
}

fn entrees(chemin: &Path) -> Vec<(String, Vec<u8>, zip::CompressionMethod)> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(chemin).unwrap()).unwrap();
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).unwrap();
            let mut v = Vec::new();
            f.read_to_end(&mut v).unwrap();
            (f.name().unwrap().to_string(), v, f.compression())
        })
        .collect()
}

fn texte(e: &[(String, Vec<u8>, zip::CompressionMethod)], nom: &str) -> String {
    let (_, v, _) = e.iter().find(|(n, _, _)| n == nom).unwrap_or_else(|| panic!("{nom} absent"));
    String::from_utf8(v.clone()).unwrap()
}

#[test]
fn structure_du_paquet() {
    let (_d, c) = fabriquer("# A\n\nTexte.\n", None);
    let e = entrees(&c);
    assert_eq!(e[0].0, "mimetype");
    assert_eq!(e[0].2, zip::CompressionMethod::Stored);
    assert_eq!(e[0].1, b"application/epub+zip");
    for n in ["META-INF/container.xml", "OEBPS/content.opf", "OEBPS/nav.xhtml"] {
        assert!(e.iter().any(|(x, _, _)| x == n), "{n} manquant");
    }
    let opf = texte(&e, "OEBPS/content.opf");
    assert!(opf.contains("<dc:title>Le Livre</dc:title>"));
    assert!(opf.contains("<dc:creator>Ada Auteur</dc:creator>"));
    assert!(opf.contains("<dc:language>fr</dc:language>"));
    assert!(opf.contains("urn:uuid:12345678-1234-1234-1234-123456789abc"));
    assert!(opf.contains("dcterms:modified\">2026-11-10T12:00:00Z<"));
}

#[test]
fn un_fichier_par_chapitre() {
    let (_d, c) = fabriquer("# A\n\n## A1\n\nx\n\n# B\n\ny\n", None);
    let e = entrees(&c);
    assert!(e.iter().any(|(n, _, _)| n == "OEBPS/chap-001.xhtml"));
    assert!(e.iter().any(|(n, _, _)| n == "OEBPS/chap-002.xhtml"));
    assert!(!e.iter().any(|(n, _, _)| n == "OEBPS/chap-003.xhtml"));
    let nav = texte(&e, "OEBPS/nav.xhtml");
    let a = nav.find(">A<").unwrap();
    let a1 = nav.find(">A1<").unwrap();
    let b = nav.find(">B<").unwrap();
    assert!(a < a1 && a1 < b);
    assert!(nav.contains("chap-002.xhtml"));
}

#[test]
fn ancre_avant_le_premier_titre_ne_cree_pas_de_chapitre() {
    let (_d, c) = fabriquer("<!-- @p1 1,2,3,4 -->\n# A\n\nx\n", None);
    let e = entrees(&c);
    assert!(!e.iter().any(|(n, _, _)| n == "OEBPS/chap-002.xhtml"));
    assert!(texte(&e, "OEBPS/nav.xhtml").contains(">A<"));
}

#[test]
fn contenu_avant_le_premier_titre() {
    let (_d, c) = fabriquer("Juste du texte.\n", None);
    let e = entrees(&c);
    assert!(e.iter().any(|(n, _, _)| n == "OEBPS/chap-001.xhtml"));
    assert!(!e.iter().any(|(n, _, _)| n == "OEBPS/chap-002.xhtml"));
    assert!(texte(&e, "OEBPS/nav.xhtml").contains(">Le Livre<"));
}

#[test]
fn notes_noteref_et_retour() {
    let (_d, c) = fabriquer("# T\n\nTexte[^3-1].\n\n[^3-1]: La note.\n", None);
    let e = entrees(&c);
    let x = texte(&e, "OEBPS/chap-001.xhtml");
    assert!(x.contains(r##"<a epub:type="noteref" id="appel-3" href="#note-3"><sup>1</sup></a>"##), "{x}");
    let aside = x.find(r#"<aside epub:type="footnote" id="note-3">"#).expect("aside");
    let fin = &x[aside..];
    assert!(fin.contains("La note."));
    assert!(fin.contains(r##"<a href="#appel-3">↩</a>"##));
}

#[test]
fn marqueur_etoile() {
    let (_d, c) = fabriquer("# T\n\nTexte[^4-*].\n\n[^4-*]: Note.\n", None);
    let x = texte(&entrees(&c), "OEBPS/chap-001.xhtml");
    assert!(x.contains(r##"id="appel-4" href="#note-4"><sup>*</sup></a>"##), "{x}");
}

#[test]
fn xhtml_bien_forme() {
    let md = "<!-- @p1 1,2,3,4 -->\n# T\n\n<!-- @p1 1,2,3,4 -->\nUn \\& \\<b\\> et 1<sup>er</sup> « guillemets » \"droits\" <em>x</em>\\\nligne\n\n---\n\nFin[^1-2].\n\n<!-- @p2 1,2,3,4 -->\n[^1-2]: n & \\<b\\>\n";
    let (_d, c) = fabriquer(md, Some(PNG));
    let e = entrees(&c);
    let mut n = 0;
    for (nom, v, _) in &e {
        if nom.ends_with(".xhtml") || nom.ends_with(".opf") {
            let s = std::str::from_utf8(v).unwrap();
            roxmltree::Document::parse(s).unwrap_or_else(|er| panic!("{nom}: {er}\n{s}"));
            n += 1;
        }
    }
    assert!(n >= 4);
}

#[test]
fn ancres_absentes() {
    let md = "<!-- @p12 120,145,880,210 -->\n# T\n\n<!-- @p6g 1,2,3,4 -->\nTexte[^1-1].\n\n<!-- @p12 1,2,3,4 -->\n[^1-1]: N.\n";
    let (_d, c) = fabriquer(md, None);
    for (nom, v, _) in entrees(&c) {
        assert!(!String::from_utf8_lossy(&v).contains("@p"), "{nom}");
    }
}

#[test]
fn couverture() {
    let (_d, c) = fabriquer("# A\n\nx\n", Some(PNG));
    let e = entrees(&c);
    assert!(e.iter().any(|(n, _, _)| n == "OEBPS/images/couverture.png"));
    assert!(e.iter().any(|(n, _, _)| n == "OEBPS/couverture.xhtml"));
    let opf = texte(&e, "OEBPS/content.opf");
    assert!(opf.contains("properties=\"cover-image\""));
    let spine = &opf[opf.find("<spine").unwrap()..];
    let premier = spine.find("<itemref").unwrap();
    assert!(spine[premier..].starts_with("<itemref idref=\"couverture\""), "{spine}");
}

fn chap1(md: &str) -> String {
    let (_d, c) = fabriquer(md, None);
    texte(&entrees(&c), "OEBPS/chap-001.xhtml")
}

#[test]
fn saut_de_section_en_filet() {
    let x = chap1("# T\n\n<!-- @p3 1,2,3,4 -->\nAvant\n\n<!-- @p4 1,2,3,4 -->\n* * *\n\n<!-- @p4 1,2,3,4 -->\nApres\n");
    assert!(x.contains("<p>Avant</p>\n<hr />\n<p>Apres</p>"), "{x}");
}

#[test]
fn html_brut_assaini() {
    let x = chap1("# T\n\nA <img src=\"a.png\"> et <BR> puis <span>x\n");
    assert!(x.contains("<img src=\"a.png\" />"), "{x}");
    assert!(x.contains("<br />"), "{x}");
    assert!(x.contains("&lt;span>x") || x.contains("&lt;span&gt;x"), "{x}");
    assert!(!x.contains("<span"), "{x}");
}

#[test]
fn em_desequilibre_est_une_erreur() {
    let dir = tempfile::tempdir().unwrap();
    let r = generer_epub("# T\n\nA <em>b\n\nc</em>\n", &meta(), None, &dir.path().join("x.epub"));
    assert!(matches!(r, Err(plumeo_epub::ErreurEpub::XhtmlInvalide { chapitre: 1, .. })), "{r:?}");
}

#[test]
fn metadonnees_invalides_refusees_avant_ecriture() {
    let dir = tempfile::tempdir().unwrap();
    for (titre, auteur) in [("T\u{1}x", "Ada"), ("Titre", "A\u{1}da")] {
        let chemin = dir.path().join("x.epub");
        let m = Metadonnees { titre: titre.into(), auteur: auteur.into(), ..meta() };
        let r = generer_epub("# A\n\nTexte.\n", &m, None, &chemin);
        assert!(
            matches!(&r, Err(plumeo_epub::ErreurEpub::XmlInvalide { fichier, .. }) if fichier == "content.opf"),
            "{r:?}"
        );
        assert!(!chemin.exists());
    }
}

#[test]
fn appel_sans_definition_et_definition_sans_appel() {
    let x = chap1("# T\n\nA[^9-1] sans def.\n");
    assert!(x.contains("A<sup>1</sup> sans def."), "{x}");
    assert!(!x.contains("[^"), "{x}");
    let x = chap1("# T\n\nA.\n\n[^2-5]: Orpheline.\n");
    assert!(x.contains("id=\"note-2\"") && x.contains("Orpheline."), "{x}");
    assert!(!x.contains("href=\"#appel-2\""), "{x}");
}

#[test]
fn appel_repete_ids_uniques() {
    let x = chap1("# T\n\nA[^1-1] B[^1-1]\n\n[^1-1]: N.\n");
    assert_eq!(x.matches("id=\"appel-1\"").count(), 1, "{x}");
    assert_eq!(x.matches("id=\"appel-1-2\"").count(), 1, "{x}");
    assert!(x.contains("<a href=\"#appel-1\">↩</a>"));
}

#[test]
fn notes_de_fin_de_livre_liees_entre_chapitres() {
    let md = "# A\n\nAppel[^1-1] ici.\n\n# B\n\nSuite[^2-2].\n\n[^1-1]: La note.\n\n[^2-2]: Autre.\n";
    let (_d, c) = fabriquer(md, None);
    let e = entrees(&c);
    let a = texte(&e, "OEBPS/chap-001.xhtml");
    let b = texte(&e, "OEBPS/chap-002.xhtml");
    assert!(
        a.contains(r#"<a epub:type="noteref" id="appel-1" href="chap-002.xhtml#note-1"><sup>1</sup></a>"#),
        "{a}"
    );
    let note1 = &b[b.find(r#"<aside epub:type="footnote" id="note-1">"#).expect("aside 1")..];
    assert!(note1.contains(r##"<a href="chap-001.xhtml#appel-1">↩</a>"##), "{b}");
    assert!(b.contains(r##"id="appel-2" href="#note-2""##), "{b}");
    assert!(b.contains(r##"<a href="#appel-2">↩</a>"##), "{b}");
}

#[test]
fn appel_repete_dans_deux_chapitres_ids_uniques_dans_le_livre() {
    let md = "# A\n\nX[^1-1]\n\n# B\n\nY[^1-1]\n\n[^1-1]: N.\n";
    let (_d, c) = fabriquer(md, None);
    let e = entrees(&c);
    let a = texte(&e, "OEBPS/chap-001.xhtml");
    let b = texte(&e, "OEBPS/chap-002.xhtml");
    assert!(a.contains(r##"id="appel-1" href="chap-002.xhtml#note-1""##), "{a}");
    assert!(b.contains(r##"id="appel-1-2" href="#note-1""##), "{b}");
    assert!(b.contains(r##"<a href="chap-001.xhtml#appel-1">↩</a>"##), "{b}");
}

#[test]
fn titre_vide_retombe_sur_le_titre_du_livre() {
    let (_d, c) = fabriquer("#\n\nx\n", None);
    let e = entrees(&c);
    assert!(texte(&e, "OEBPS/nav.xhtml").contains(">Le Livre<"));
    assert!(texte(&e, "OEBPS/chap-001.xhtml").contains("<title>Le Livre</title>"));
}

#[test]
fn titres_identifies_dans_tout_le_livre() {
    let (_d, c) = fabriquer("# A\n\n## A1\n\n### Détail\n\nx\n\n# B\n\n## A1\n\ny\n", None);
    let e = entrees(&c);
    let a = texte(&e, "OEBPS/chap-001.xhtml");
    let b = texte(&e, "OEBPS/chap-002.xhtml");
    assert!(a.contains(r#"<h1 id="t-a">A</h1>"#), "{a}");
    assert!(a.contains(r#"<h2 id="t-a1">A1</h2>"#), "{a}");
    assert!(a.contains(r#"<h3 id="t-détail">Détail</h3>"#), "{a}");
    assert!(b.contains(r#"<h1 id="t-b">B</h1>"#), "{b}");
    assert!(b.contains(r#"<h2 id="t-a1-2">A1</h2>"#), "{b}");
    let nav = texte(&e, "OEBPS/nav.xhtml");
    assert!(nav.contains(r##"<a href="chap-001.xhtml#t-a1">A1</a>"##), "{nav}");
    assert!(nav.contains(r##"<a href="chap-002.xhtml#t-a1-2">A1</a>"##), "{nav}");
    assert!(!nav.contains("#s1"), "{nav}");
}

#[test]
fn liens_du_sommaire_vers_le_fichier_du_titre() {
    let md = "# Table des matières\n\n[**1. VISITE À PATMOS**](#t-1-visite-à-patmos)\n\n[Perdu](#t-perdu)\n\n[Ailleurs](https://exemple.org)\n\n# 1 VISITE À PATMOS\n\nx\n";
    let (_d, c) = fabriquer(md, None);
    let e = entrees(&c);
    let table = texte(&e, "OEBPS/chap-001.xhtml");
    assert!(table.contains(r##"<a href="chap-002.xhtml#t-1-visite-à-patmos"><strong>1. VISITE À PATMOS</strong></a>"##), "{table}");
    // Sans cible connue : texte simple, aucun lien cassé.
    assert!(table.contains("<p>Perdu</p>"), "{table}");
    assert!(table.contains("<p>Ailleurs</p>"), "{table}");
    assert_eq!(table.matches("<a ").count(), 1, "{table}");
    assert!(texte(&e, "OEBPS/chap-002.xhtml").contains(r#"<h1 id="t-1-visite-à-patmos">"#));
}

#[test]
fn ancre_titre_publique() {
    assert_eq!(plumeo_epub::ancre_titre("1 VISITE À PATMOS"), "t-1-visite-à-patmos");
}

#[test]
#[ignore]
fn epubcheck() {
    let Ok(jar) = std::env::var("EPUBCHECK_JAR") else {
        eprintln!("EPUBCHECK_JAR non défini");
        return;
    };
    let md = "<!-- @p1 1,2,3,4 -->\n# Table des matières\n\n[**1. Un été à Patmos**](#t-1-un-été-à-patmos)\n\n[Sous](#t-sous)\n\n[Perdu](#t-perdu)\n\n# 1 Un été à Patmos\n\nTexte[^1-1] et[^2-*] puis[^3-3].\n\n## Sous\n\nSuite 1<sup>er</sup>.\n\n[^1-1]: N1.\n\n[^2-*]: N2.\n\n# Deux\n\nAutre.\n\n[^3-3]: N3, en fin de livre.\n";
    let (_d, c) = fabriquer(md, Some(PNG));
    let s = std::process::Command::new("java").args(["-jar", &jar]).arg(&c).status().unwrap();
    assert!(s.success());
}

/// Titre (`<title>`) de chaque fichier de chapitre, dans l'ordre des fichiers.
fn titres_des_chapitres(e: &[(String, Vec<u8>, zip::CompressionMethod)]) -> Vec<String> {
    let mut noms: Vec<&String> =
        e.iter().map(|(n, _, _)| n).filter(|n| n.starts_with("OEBPS/chap-")).collect();
    noms.sort();
    noms.iter()
        .map(|n| {
            let x = texte(e, n);
            let debut = x.find("<title>").unwrap() + "<title>".len();
            x[debut..x.find("</title>").unwrap()].to_string()
        })
        .collect()
}

const LIVRE_A_SOMMAIRE_DE_FIN: &str = "# Livre\n\ntexte\n\n# Ch 1\n\n[**Ch 1**](#t-ch-1) ailleurs…\n\n# Table des matières\n\n[**Ch 1**](#t-ch-1)\n\ncolophon\n";

#[test]
fn sommaire_de_fin_deplace_apres_la_page_de_titre() {
    let (_d, c) = fabriquer(LIVRE_A_SOMMAIRE_DE_FIN, None);
    let e = entrees(&c);
    assert_eq!(titres_des_chapitres(&e), ["Livre", "Table des matières", "Ch 1"]);
    let table = texte(&e, "OEBPS/chap-002.xhtml");
    assert!(table.contains(r##"<a href="chap-003.xhtml#t-ch-1"><strong>Ch 1</strong></a>"##), "{table}");
    assert!(!table.contains("colophon"), "{table}");
    let ch1 = texte(&e, "OEBPS/chap-003.xhtml");
    assert!(ch1.contains(r#"<h1 id="t-ch-1">Ch 1</h1>"#), "{ch1}");
    // La partie qui suit la dernière entrée reste à sa place : fin du chapitre précédent.
    let ailleurs = ch1.find("ailleurs").expect("texte du chapitre");
    let colophon = ch1.find("<p>colophon</p>").expect("colophon resté en place");
    assert!(ailleurs < colophon, "{ch1}");
    assert!(ch1.contains(r##"<a href="chap-003.xhtml#t-ch-1"><strong>Ch 1</strong></a>"##), "{ch1}");
}

#[test]
fn sommaire_deplace_la_nav_et_la_spine_suivent() {
    let (_d, c) = fabriquer(LIVRE_A_SOMMAIRE_DE_FIN, None);
    let e = entrees(&c);
    let nav = texte(&e, "OEBPS/nav.xhtml");
    assert!(nav.contains(r#"<li><a href="chap-001.xhtml">Livre</a>"#), "{nav}");
    assert!(nav.contains(r#"<li><a href="chap-002.xhtml">Table des matières</a>"#), "{nav}");
    assert!(nav.contains(r#"<li><a href="chap-003.xhtml">Ch 1</a>"#), "{nav}");
    let livre = nav.find(">Livre<").unwrap();
    let table = nav.find(">Table des matières</a>").unwrap();
    let ch1 = nav.find(">Ch 1<").unwrap();
    assert!(livre < table && table < ch1, "{nav}");
    let opf = texte(&e, "OEBPS/content.opf");
    let spine = &opf[opf.find("<spine").unwrap()..];
    let i: Vec<usize> = ["chap-001", "chap-002", "chap-003"].iter().map(|c| spine.find(c).unwrap()).collect();
    assert!(i[0] < i[1] && i[1] < i[2], "{spine}");
}

#[test]
fn sommaire_en_majuscules_deplace_aussi() {
    let md = "# Livre\n\nx\n\n# Ch 1\n\ny\n\n# TABLE DES MATIÈRES\n\n[Ch 1](#t-ch-1)\n";
    let (_d, c) = fabriquer(md, None);
    let e = entrees(&c);
    assert_eq!(titres_des_chapitres(&e), ["Livre", "TABLE DES MATIÈRES", "Ch 1"]);
    let md = "# Livre\n\nx\n\n# Ch 1\n\ny\n\n# SOMMAIRE\n\n[Ch 1](#t-ch-1)\n";
    let (_d, c) = fabriquer(md, None);
    assert_eq!(titres_des_chapitres(&entrees(&c)), ["Livre", "SOMMAIRE", "Ch 1"]);
}

#[test]
fn sommaire_deja_en_deuxieme_position_ne_bouge_pas() {
    let md = "# Livre\n\nx\n\n# Sommaire\n\n[Ch 1](#t-ch-1)\n\nsuite du sommaire\n\n# Ch 1\n\ny\n";
    let (_d, c) = fabriquer(md, None);
    let e = entrees(&c);
    assert_eq!(titres_des_chapitres(&e), ["Livre", "Sommaire", "Ch 1"]);
    let table = texte(&e, "OEBPS/chap-002.xhtml");
    assert!(table.contains("<p>suite du sommaire</p>"), "{table}");
    assert!(table.contains(r#"<a href="chap-003.xhtml#t-ch-1">Ch 1</a>"#), "{table}");
}

#[test]
fn sommaire_sans_lien_interne_ne_bouge_pas() {
    let md = "# Livre\n\nx\n\n# Ch 1\n\ny\n\n# Sommaire\n\nCh 1 ........ 5\n\n[Site](https://exemple.org)\n";
    let (_d, c) = fabriquer(md, None);
    let e = entrees(&c);
    assert_eq!(titres_des_chapitres(&e), ["Livre", "Ch 1", "Sommaire"]);
    assert!(texte(&e, "OEBPS/chap-003.xhtml").contains("Ch 1 ........ 5"));
}

#[test]
fn livre_sans_sommaire_inchange() {
    let md = "# Livre\n\nx\n\n# Ch 1\n\n[Ch 2](#t-ch-2)\n\n# Ch 2\n\nz\n";
    let (_d, c) = fabriquer(md, None);
    let e = entrees(&c);
    assert_eq!(titres_des_chapitres(&e), ["Livre", "Ch 1", "Ch 2"]);
    assert!(texte(&e, "OEBPS/chap-002.xhtml").contains(r#"<a href="chap-003.xhtml#t-ch-2">Ch 2</a>"#));
}

#[test]
fn note_du_colophon_reste_en_place_avec_son_aller_retour() {
    let md = "# Livre\n\nx\n\n# Ch 1\n\ny\n\n# Sommaire\n\n[Ch 1](#t-ch-1)\n\nColophon[^5-1].\n\n[^5-1]: Note du colophon.\n";
    let (_d, c) = fabriquer(md, None);
    let e = entrees(&c);
    assert_eq!(titres_des_chapitres(&e), ["Livre", "Sommaire", "Ch 1"]);
    let ch1 = texte(&e, "OEBPS/chap-003.xhtml");
    assert!(ch1.contains(r##"<a epub:type="noteref" id="appel-5" href="#note-5"><sup>1</sup></a>"##), "{ch1}");
    let aside = &ch1[ch1.find(r#"<aside epub:type="footnote" id="note-5">"#).expect("aside")..];
    assert!(aside.contains("Note du colophon."), "{ch1}");
    assert!(aside.contains(r##"<a href="#appel-5">↩</a>"##), "{ch1}");
    assert!(!texte(&e, "OEBPS/chap-002.xhtml").contains("note-5"));
}

#[test]
fn sous_titre_apres_le_sommaire_reste_en_place() {
    let md = "# Livre\n\nx\n\n# Ch 1\n\ny\n\n# Sommaire\n\n[Ch 1](#t-ch-1)\n\n## Achevé d'imprimer\n\nz\n";
    let (_d, c) = fabriquer(md, None);
    let e = entrees(&c);
    assert_eq!(titres_des_chapitres(&e), ["Livre", "Sommaire", "Ch 1"]);
    assert!(texte(&e, "OEBPS/chap-003.xhtml").contains(r#"<h2 id="t-achevé-d-imprimer">"#));
    let nav = texte(&e, "OEBPS/nav.xhtml");
    assert!(nav.contains(r##"<a href="chap-003.xhtml#t-achevé-d-imprimer">Achevé d'imprimer</a>"##), "{nav}");
}

#[test]
fn est_titre_de_sommaire_public() {
    let oui = ["Table des matières", "TABLE DES MATIÈRES", "Table des Matieres", "Sommaire", "SOMMAIRE"];
    for oui in oui.into_iter().chain(["Sommaire :", "Tâble dès mätïères"]) {
        assert!(plumeo_epub::est_titre_de_sommaire(oui), "« {oui} »");
    }
    for non in ["Table", "Sommaires et index", ""] {
        assert!(!plumeo_epub::est_titre_de_sommaire(non), "« {non} »");
    }
}

#[test]
fn deux_sommaires_a_liens_seul_le_premier_est_deplace() {
    let md = "# Livre\n\nx\n\n# Ch 1\n\ny\n\n# Sommaire\n\n[Ch 1](#t-ch-1)\n\n# Ch 2\n\nz\n\n\
              # Sommaire\n\n[Ch 2](#t-ch-2)\n";
    let (_d, c) = fabriquer(md, None);
    let e = entrees(&c);
    assert_eq!(titres_des_chapitres(&e), ["Livre", "Sommaire", "Ch 1", "Ch 2", "Sommaire"]);
    let premier = texte(&e, "OEBPS/chap-002.xhtml");
    assert!(premier.contains(r#"<a href="chap-003.xhtml#t-ch-1">Ch 1</a>"#), "{premier}");
    let second = texte(&e, "OEBPS/chap-005.xhtml");
    assert!(second.contains(r#"<a href="chap-004.xhtml#t-ch-2">Ch 2</a>"#), "{second}");
}

#[test]
fn sommaire_en_position_0_ne_bouge_pas() {
    let md = "# Sommaire\n\n[Ch 1](#t-ch-1)\n\napres les entrees\n\n# Ch 1\n\ny\n\n# Ch 2\n\nz\n";
    let (_d, c) = fabriquer(md, None);
    let e = entrees(&c);
    assert_eq!(titres_des_chapitres(&e), ["Sommaire", "Ch 1", "Ch 2"]);
    let table = texte(&e, "OEBPS/chap-001.xhtml");
    assert!(table.contains("<p>apres les entrees</p>"), "{table}");
    assert!(table.contains(r#"<a href="chap-002.xhtml#t-ch-1">Ch 1</a>"#), "{table}");
}

#[test]
fn sommaire_sans_suite_laisse_le_chapitre_precedent_inchange() {
    // Rien après la dernière entrée : le chapitre précédent ne reçoit rien. Le livre
    // est le même que si le sommaire avait été écrit à sa place.
    let deplace = "# Livre\n\nx\n\n# Ch 1\n\ny\n\n## Sous\n\nw\n\n# Sommaire\n\n[Ch 1](#t-ch-1)\n";
    let en_place = "# Livre\n\nx\n\n# Sommaire\n\n[Ch 1](#t-ch-1)\n\n# Ch 1\n\ny\n\n## Sous\n\nw\n";
    let (_d1, c1) = fabriquer(deplace, None);
    let (_d2, c2) = fabriquer(en_place, None);
    let (e1, e2) = (entrees(&c1), entrees(&c2));
    assert_eq!(titres_des_chapitres(&e1), ["Livre", "Sommaire", "Ch 1"]);
    for nom in ["OEBPS/chap-001.xhtml", "OEBPS/chap-002.xhtml", "OEBPS/chap-003.xhtml", "OEBPS/nav.xhtml"] {
        assert_eq!(texte(&e1, nom), texte(&e2, nom), "{nom}");
    }
}
