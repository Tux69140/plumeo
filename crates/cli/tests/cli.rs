use std::process::Command;

use plumeo_moteur::fixtures::{StyleFixture, TexteFixture, fabriquer_pdf};

fn t(y: f32, texte: &'static str, taille: f32, style: StyleFixture) -> TexteFixture {
    TexteFixture { x_pt: 72.0, y_pt: y, texte, taille, style }
}

const SERIE: &str = r#"{"nom":"test","langue":"fra",
 "paire":{"entete":0.06,"pied":0.0,"notes":0.0},
 "impaire":{"entete":0.06,"pied":0.0,"notes":0.0}}"#;

fn preparer() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let p1 = [
        t(30.0, "Mon livre", 10.0, StyleFixture::Normal),
        t(100.0, "CHAPITRE UN", 20.0, StyleFixture::Gras),
        t(200.0, "Premier paragraphe de la page un.", 12.0, StyleFixture::Normal),
    ];
    let p2 = [
        t(30.0, "Mon livre", 10.0, StyleFixture::Normal),
        t(200.0, "Deuxieme page avec du texte.", 12.0, StyleFixture::Normal),
    ];
    let pdf = fabriquer_pdf(&[&p1, &p2]);
    let dossier = tempfile::tempdir().unwrap();
    let copie = dossier.path().join("Jean Dupont - Mon petit livre.pdf");
    std::fs::copy(&pdf, &copie).unwrap();
    let serie = dossier.path().join("serie.json");
    std::fs::write(&serie, SERIE).unwrap();
    (dossier, copie, serie)
}

#[test]
fn cli_convertit_un_pdf_fabrique() {
    let (dossier, pdf, serie) = preparer();
    let sortie = dossier.path().join("sortie/sous-dossier");
    let r = Command::new(env!("CARGO_BIN_EXE_plumeo-cli"))
        .arg("convertir").arg(&pdf)
        .arg("--serie").arg(&serie)
        .arg("--sortie").arg(&sortie)
        .output().unwrap();
    let err = String::from_utf8_lossy(&r.stderr);
    assert!(r.status.success(), "{err}");
    let base = "Mon petit livre - Jean Dupont";
    for ext in ["epub", "md", "retraits.json", "points.json"] {
        assert!(sortie.join(format!("{base}.{ext}")).is_file(), "{ext} manquant");
    }
    let epub = std::fs::File::open(sortie.join(format!("{base}.epub"))).unwrap();
    let archive = zip::ZipArchive::new(epub).unwrap();
    assert!(archive.len() > 2);
    let md = std::fs::read_to_string(sortie.join(format!("{base}.md"))).unwrap();
    assert!(md.contains("# CHAPITRE UN"), "{md}");
    let resume = String::from_utf8_lossy(&r.stdout);
    assert!(resume.contains("2 pages"), "{resume}");
}

#[test]
fn cli_page_affiche_corps_et_retraits() {
    let (_d, pdf, serie) = preparer();
    let r = Command::new(env!("CARGO_BIN_EXE_plumeo-cli"))
        .arg("page").arg(&pdf).arg("1").arg("--serie").arg(&serie)
        .output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let s = String::from_utf8_lossy(&r.stdout);
    assert!(s.contains("=== page 1"), "{s}");
    assert!(s.contains("Premier paragraphe"), "{s}");
    assert!(s.contains("--- retraits ---"), "{s}");
    assert!(s.contains("[en-tete] Mon livre"), "{s}");
}

#[test]
fn cli_echoue_avec_code_1_et_message() {
    let (d, _pdf, serie) = preparer();
    let r = Command::new(env!("CARGO_BIN_EXE_plumeo-cli"))
        .arg("page").arg(d.path().join("absent.pdf")).arg("1").arg("--serie").arg(&serie)
        .output().unwrap();
    assert_eq!(r.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&r.stderr).contains("erreur"));
}
