//! Assemblage du conteneur EPUB (zip, OPF, nav).

use std::io::Write;
use std::path::Path;

use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::chapitres::Chapitre;
use crate::xhtml::{echapper, page};
use crate::{ErreurEpub, Metadonnees};

const CSS: &str = include_str!("style.css");

const CONTAINER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n<rootfiles>\n<rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\" />\n</rootfiles>\n</container>\n";

fn zip_err(e: zip::result::ZipError) -> ErreurEpub {
    match e {
        zip::result::ZipError::Io(e) => ErreurEpub::Ecriture(e),
        e => ErreurEpub::Zip(e.to_string()),
    }
}

fn nom_chapitre(i: usize) -> String {
    format!("chap-{:03}.xhtml", i + 1)
}

fn nav(chapitres: &[Chapitre], meta: &Metadonnees) -> String {
    let mut s = String::from("<nav epub:type=\"toc\" id=\"toc\">\n<h1>Table des matières</h1>\n<ol>\n");
    for (i, c) in chapitres.iter().enumerate() {
        let f = nom_chapitre(i);
        s.push_str(&format!("<li><a href=\"{f}\">{}</a>", echapper(&c.titre)));
        if !c.sous_titres.is_empty() {
            s.push_str("\n<ol>\n");
            for (id, t) in &c.sous_titres {
                s.push_str(&format!("<li><a href=\"{f}#{id}\">{}</a></li>\n", echapper(t)));
            }
            s.push_str("</ol>\n");
        }
        s.push_str("</li>\n");
    }
    s.push_str("</ol>\n</nav>\n");
    page(&meta.langue, &meta.titre, &s)
}

fn opf(chapitres: &[Chapitre], meta: &Metadonnees, couverture: bool) -> String {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"id-livre\" xml:lang=\"");
    s.push_str(&echapper(&meta.langue));
    s.push_str("\">\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n");
    s.push_str(&format!("<dc:identifier id=\"id-livre\">{}</dc:identifier>\n", echapper(&meta.identifiant)));
    s.push_str(&format!("<dc:title>{}</dc:title>\n", echapper(&meta.titre)));
    if !meta.auteur.is_empty() {
        s.push_str(&format!("<dc:creator>{}</dc:creator>\n", echapper(&meta.auteur)));
    }
    s.push_str(&format!("<dc:language>{}</dc:language>\n", echapper(&meta.langue)));
    s.push_str(&format!("<meta property=\"dcterms:modified\">{}</meta>\n", echapper(&meta.modifie)));
    if couverture {
        s.push_str("<meta name=\"cover\" content=\"couverture-img\" />\n");
    }
    s.push_str("</metadata>\n<manifest>\n");
    s.push_str("<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\" />\n");
    s.push_str("<item id=\"style\" href=\"style.css\" media-type=\"text/css\" />\n");
    if couverture {
        s.push_str("<item id=\"couverture-img\" href=\"images/couverture.png\" media-type=\"image/png\" properties=\"cover-image\" />\n");
        s.push_str("<item id=\"couverture\" href=\"couverture.xhtml\" media-type=\"application/xhtml+xml\" />\n");
    }
    for i in 0..chapitres.len() {
        s.push_str(&format!("<item id=\"chap-{0:03}\" href=\"{1}\" media-type=\"application/xhtml+xml\" />\n", i + 1, nom_chapitre(i)));
    }
    s.push_str("</manifest>\n<spine>\n");
    if couverture {
        s.push_str("<itemref idref=\"couverture\" />\n");
    }
    for i in 0..chapitres.len() {
        s.push_str(&format!("<itemref idref=\"chap-{:03}\" />\n", i + 1));
    }
    s.push_str("</spine>\n</package>\n");
    s
}

pub fn ecrire(
    chapitres: &[Chapitre],
    meta: &Metadonnees,
    couverture_png: Option<&[u8]>,
    sortie: &Path,
) -> Result<(), ErreurEpub> {
    let fichier = std::fs::File::create(sortie)?;
    let mut z = ZipWriter::new(fichier);
    let stocke = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let comprime = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    let mut ajouter = |nom: &str, opts: SimpleFileOptions, data: &[u8]| -> Result<(), ErreurEpub> {
        z.start_file(nom, opts).map_err(zip_err)?;
        z.write_all(data)?;
        Ok(())
    };

    ajouter("mimetype", stocke, b"application/epub+zip")?;
    ajouter("META-INF/container.xml", comprime, CONTAINER.as_bytes())?;
    ajouter("OEBPS/content.opf", comprime, opf(chapitres, meta, couverture_png.is_some()).as_bytes())?;
    ajouter("OEBPS/nav.xhtml", comprime, nav(chapitres, meta).as_bytes())?;
    ajouter("OEBPS/style.css", comprime, CSS.as_bytes())?;
    if let Some(png) = couverture_png {
        ajouter("OEBPS/images/couverture.png", comprime, png)?;
        let corps = format!(
            "<div class=\"couverture\"><img src=\"images/couverture.png\" alt=\"Couverture\" /></div>\n"
        );
        ajouter("OEBPS/couverture.xhtml", comprime, page(&meta.langue, &meta.titre, &corps).as_bytes())?;
    }
    for (i, c) in chapitres.iter().enumerate() {
        let xml = page(&meta.langue, &c.titre, &c.corps);
        ajouter(&format!("OEBPS/{}", nom_chapitre(i)), comprime, xml.as_bytes())?;
    }
    z.finish().map_err(zip_err)?;
    Ok(())
}
