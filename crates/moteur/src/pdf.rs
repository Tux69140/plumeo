//! Lecture d'un PDF via pdfium : mots, signets, métadonnées, rendu.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use pdfium_render::prelude::*;
use unicode_normalization::UnicodeNormalization;

use crate::modele::{ErreurMoteur, MetaPdf, Moitie, Mot, Page, Rect, Signet};

const TOLERANCE_TAILLE: f32 = 0.5;
const SEUIL_ECART: f32 = 0.25;

static PDFIUM: OnceLock<Result<Pdfium, String>> = OnceLock::new();

fn dossiers_candidats() -> Vec<PathBuf> {
    let mut dossiers = Vec::new();
    if let Some(d) = std::env::var_os("PLUMEO_PDFIUM") {
        dossiers.push(PathBuf::from(d));
    }
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        dossiers.push(d);
    }
    dossiers.push(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../vendor/pdfium/lib")));
    dossiers
}

fn charger_pdfium() -> Result<Pdfium, String> {
    let dossiers = dossiers_candidats();
    for d in &dossiers {
        let chemin = Pdfium::pdfium_platform_library_name_at_path(d);
        if chemin.is_file() {
            let liaison = Pdfium::bind_to_library(&chemin)
                .map_err(|e| format!("{} : {e}", chemin.display()))?;
            return Ok(Pdfium::new(liaison));
        }
    }
    let essayes: Vec<String> = dossiers.iter().map(|d| d.display().to_string()).collect();
    Err(format!("chemins essayés : {}", essayes.join(", ")))
}

/// Instance unique de pdfium, chargée à la première utilisation.
pub(crate) fn pdfium() -> Result<&'static Pdfium, ErreurMoteur> {
    PDFIUM
        .get_or_init(charger_pdfium)
        .as_ref()
        .map_err(|e| ErreurMoteur::PdfiumIntrouvable(e.clone()))
}

pub struct DocumentPdf {
    doc: PdfDocument<'static>,
}

impl DocumentPdf {
    pub fn ouvrir(chemin: &Path) -> Result<DocumentPdf, ErreurMoteur> {
        let doc = pdfium()?
            .load_pdf_from_file(chemin, None)
            .map_err(|e| ErreurMoteur::PdfIllisible(format!("{} : {e}", chemin.display())))?;
        Ok(DocumentPdf { doc })
    }

    pub fn nb_pages(&self) -> u32 {
        self.doc.pages().len() as u32
    }

    fn page_pdfium(&self, numero: u32) -> Result<PdfPage<'_>, ErreurMoteur> {
        if numero == 0 {
            return Err(ErreurMoteur::Page(numero, "les pages commencent à 1".into()));
        }
        self.doc
            .pages()
            .get((numero - 1) as PdfPageIndex)
            .map_err(|e| ErreurMoteur::Page(numero, e.to_string()))
    }

    /// Lit les mots de la page `numero` (base 1).
    pub fn page(&self, numero: u32) -> Result<Page, ErreurMoteur> {
        let page = self.page_pdfium(numero)?;
        let largeur = page.width().value;
        let hauteur = page.height().value;
        let texte = page.text().map_err(|e| ErreurMoteur::Page(numero, e.to_string()))?;
        let mut assembleur = Assembleur::new(largeur, hauteur);
        for c in texte.chars().iter() {
            let Some(ch) = c.unicode_char() else { continue };
            if ch.is_whitespace() || ch.is_control() {
                assembleur.espace();
                continue;
            }
            let Ok(b) = c.loose_bounds() else { continue };
            let nom = c.font_name();
            let poids_fort = matches!(
                c.font_weight(),
                Some(PdfFontWeight::Weight600
                    | PdfFontWeight::Weight700Bold
                    | PdfFontWeight::Weight800
                    | PdfFontWeight::Weight900)
            ) || matches!(c.font_weight(), Some(PdfFontWeight::Custom(v)) if v >= 600);
            let gras = poids_fort || nom_contient(&nom, &["bold", "black", "heavy", "semibold"]);
            let italique = nom_contient(&nom, &["italic", "oblique"]) || c.font_is_italic();
            assembleur.caractere(Car {
                ch,
                gauche: b.left().value,
                droite: b.right().value,
                bas: b.bottom().value,
                haut: b.top().value,
                taille: c.scaled_font_size().value,
                gras,
                italique,
            });
        }
        Ok(Page {
            numero,
            moitie: Moitie::Entiere,
            largeur_pt: largeur,
            hauteur_pt: hauteur,
            mots: assembleur.finir(),
        })
    }

    /// Signets aplatis, dans l'ordre du document ; niveau 1 = racine.
    pub fn signets(&self) -> Vec<Signet> {
        let mut sortie = Vec::new();
        let mut courant = self.doc.bookmarks().root();
        while let Some(s) = courant {
            self.collecter_signet(&s, 1, &mut sortie);
            courant = s.next_sibling();
        }
        sortie
    }

    fn collecter_signet(&self, s: &PdfBookmark<'_>, niveau: u8, sortie: &mut Vec<Signet>) {
        if let (Some(titre), Some(page)) = (s.title(), page_cible(s)) {
            sortie.push(Signet { titre, niveau, page });
        }
        let mut enfant = s.first_child();
        while let Some(e) = enfant {
            self.collecter_signet(&e, niveau.saturating_add(1), sortie);
            enfant = e.next_sibling();
        }
    }

    pub fn meta(&self) -> MetaPdf {
        let lire = |t| {
            self.doc
                .metadata()
                .get(t)
                .map(|v| v.value().to_string())
                .unwrap_or_default()
        };
        MetaPdf {
            titre: lire(PdfDocumentMetadataTagType::Title),
            auteur: lire(PdfDocumentMetadataTagType::Author),
            producteur: lire(PdfDocumentMetadataTagType::Producer),
        }
    }

    pub fn rendre_png(&self, numero: u32, largeur_px: u32) -> Result<Vec<u8>, ErreurMoteur> {
        let erreur = |e: String| ErreurMoteur::Page(numero, e);
        let page = self.page_pdfium(numero)?;
        let config = PdfRenderConfig::new().set_target_width(largeur_px as Pixels);
        let image = page
            .render_with_config(&config)
            .and_then(|b| b.as_image())
            .map_err(|e| erreur(e.to_string()))?;
        let mut octets = Cursor::new(Vec::new());
        image
            .write_to(&mut octets, image::ImageFormat::Png)
            .map_err(|e| erreur(e.to_string()))?;
        Ok(octets.into_inner())
    }
}

/// Page cible (base 1) d'un signet : action locale, sinon destination directe.
fn page_cible(s: &PdfBookmark<'_>) -> Option<u32> {
    let par_action = s.action().and_then(|a| {
        a.as_local_destination_action()
            .and_then(|l| l.destination().ok())
            .and_then(|d| d.page_index().ok())
    });
    let index = par_action.or_else(|| s.destination().and_then(|d| d.page_index().ok()))?;
    Some(index as u32 + 1)
}

fn nom_contient(nom: &str, motifs: &[&str]) -> bool {
    let nom = nom.to_lowercase();
    motifs.iter().any(|m| nom.contains(m))
}

struct Car {
    ch: char,
    gauche: f32,
    droite: f32,
    bas: f32,
    haut: f32,
    taille: f32,
    gras: bool,
    italique: bool,
}

/// Regroupe les caractères successifs en mots.
struct Assembleur {
    largeur: f32,
    hauteur: f32,
    mots: Vec<Mot>,
    en_cours: Option<Mot>,
    /// Coordonnées pdfium du dernier caractère ajouté : (droite, bas, haut).
    dernier: (f32, f32, f32),
    espace_en_attente: bool,
}

impl Assembleur {
    fn new(largeur: f32, hauteur: f32) -> Self {
        Assembleur {
            largeur,
            hauteur,
            mots: Vec::new(),
            en_cours: None,
            dernier: (0.0, 0.0, 0.0),
            espace_en_attente: false,
        }
    }

    fn espace(&mut self) {
        self.cloturer();
        self.espace_en_attente = true;
    }

    fn cloturer(&mut self) {
        if let Some(mut mot) = self.en_cours.take() {
            mot.texte = nettoyer_texte(&mot.texte);
            if !mot.texte.is_empty() {
                self.mots.push(mot);
            }
        }
    }

    fn rect(&self, c: &Car) -> Rect {
        Rect {
            x0: c.gauche / self.largeur,
            y0: 1.0 - c.haut / self.hauteur,
            x1: c.droite / self.largeur,
            y1: 1.0 - c.bas / self.hauteur,
        }
    }

    fn caractere(&mut self, c: Car) {
        let rect = self.rect(&c);
        let mut ecart = false;
        if let Some(mot) = &self.en_cours {
            let (droite, bas, haut) = self.dernier;
            let saut_horizontal = (c.gauche - droite).abs() > SEUIL_ECART * c.taille;
            let saut_vertical = (c.haut - haut).abs() > 0.5 * c.taille
                || (c.bas - bas).abs() > 0.5 * c.taille;
            let style = mot.gras != c.gras
                || mot.italique != c.italique
                || (mot.taille - c.taille).abs() > TOLERANCE_TAILLE;
            if saut_horizontal || saut_vertical || style {
                // Un écart (ou un saut de ligne) sépare les mots ; un simple
                // changement de style sans écart les colle l'un à l'autre.
                ecart = saut_horizontal || saut_vertical;
                self.cloturer();
            }
        }
        match &mut self.en_cours {
            Some(mot) => {
                mot.texte.push(c.ch);
                mot.rect.x0 = mot.rect.x0.min(rect.x0);
                mot.rect.y0 = mot.rect.y0.min(rect.y0);
                mot.rect.x1 = mot.rect.x1.max(rect.x1);
                mot.rect.y1 = mot.rect.y1.max(rect.y1);
            }
            None => {
                let espace_avant = (self.espace_en_attente || ecart) && !self.mots.is_empty();
                self.en_cours = Some(Mot {
                    texte: c.ch.to_string(),
                    rect,
                    taille: c.taille,
                    gras: c.gras,
                    italique: c.italique,
                    espace_avant,
                });
            }
        }
        self.espace_en_attente = false;
        self.dernier = (c.droite, c.bas, c.haut);
    }

    fn finir(mut self) -> Vec<Mot> {
        self.cloturer();
        self.mots
    }
}

/// Retire les caractères parasites, décompose les ligatures, normalise en NFC.
pub fn nettoyer_texte(s: &str) -> String {
    let mut sortie = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\u{fb00}' => sortie.push_str("ff"),
            '\u{fb01}' => sortie.push_str("fi"),
            '\u{fb02}' => sortie.push_str("fl"),
            '\u{fb03}' => sortie.push_str("ffi"),
            '\u{fb04}' => sortie.push_str("ffl"),
            '\u{fb05}' | '\u{fb06}' => sortie.push_str("st"),
            '\u{fffe}' | '\u{ffff}' => {}
            c if c.is_control() && c != ' ' => {}
            c => sortie.push(c),
        }
    }
    sortie.nfc().collect()
}
