//! Conversion d'un livre (ou d'une page) : toutes les étapes du moteur, de la
//! lecture du PDF au Markdown ancré.

use serde::Serialize;

use crate::blocs::{Bloc, Segment};
use crate::double_page::couper_double_page;
use crate::lignes::{Ligne, regrouper_en_lignes};
use crate::markdown::ecrire_markdown;
use crate::modele::{Ancre, ErreurMoteur, Moitie, Page};
use crate::notes::{Etiqueteuse, notes_bas_de_page, notes_regroupees};
use crate::paragraphes::{mesurer_bloc, paragraphes_page, raccorder_pages};
use crate::pdf::DocumentPdf;
use crate::points::{GenrePoint, PointAVerifier};
use crate::reglages::{ReglagesSerie, SourceTitres};
use crate::titres::detecter_titres;
use crate::zones::{PageClassee, Retrait, Role, classer_lignes};

#[derive(Debug, Serialize)]
pub struct Conversion {
    #[serde(skip)]
    pub markdown: String,
    pub retraits: Vec<Retrait>,
    pub points: Vec<PointAVerifier>,
    pub source_titres: SourceTitres,
    pub nb_pages: u32,
    pub nb_notes: usize,
}

#[derive(Debug)]
pub struct ApercuPage {
    /// Pour chaque (demi-)page : texte du corps et retraits.
    pub moities: Vec<(Moitie, String, Vec<Retrait>)>,
    pub points: Vec<PointAVerifier>,
}

/// (demi-)page lue, classée.
struct PageLue {
    page: Page,
    classee: PageClassee,
}

fn lire_page(doc: &DocumentPdf, numero: u32, reglages: &ReglagesSerie) -> Result<Vec<PageLue>, ErreurMoteur> {
    let page = doc.page(numero)?;
    let moities = match reglages.double_page {
        Some(pli) => couper_double_page(page, pli).into_iter().collect(),
        None => vec![page],
    };
    Ok(moities
        .into_iter()
        .map(|page| {
            let lignes = regrouper_en_lignes(&page.mots);
            let classee = classer_lignes(&page, lignes, reglages.zones_pour(page.numero, page.moitie));
            PageLue { page, classee }
        })
        .collect())
}

fn ancre_entiere(page: &Page) -> Ancre {
    Ancre { page: page.numero, moitie: page.moitie, rect: [0, 0, 1000, 1000] }
}

fn corps_de(lues: &[PageLue]) -> Vec<Vec<Ligne>> {
    lues.iter().map(|l| l.classee.corps.clone()).collect()
}

pub fn convertir_livre(
    doc: &DocumentPdf,
    reglages: &ReglagesSerie,
    forcer_titres: Option<SourceTitres>,
    progression: &mut dyn FnMut(u32, u32),
) -> Result<Conversion, ErreurMoteur> {
    let nb_pages = doc.nb_pages();
    let mut lues: Vec<PageLue> = Vec::new();
    let mut retraits: Vec<Retrait> = Vec::new();
    let mut points: Vec<PointAVerifier> = Vec::new();
    for numero in 1..=nb_pages {
        for mut lue in lire_page(doc, numero, reglages)? {
            retraits.extend(lue.classee.retraits.iter().cloned());
            if lue.classee.numero_residuel {
                let ancre = lue
                    .classee
                    .retraits
                    .iter()
                    .find(|r| r.role == Role::NumeroPage)
                    .map_or_else(|| ancre_entiere(&lue.page), |r| r.ancre);
                points.push(PointAVerifier {
                    ancre,
                    genre: GenrePoint::NumeroPageResiduel,
                    message: "un numéro de page a été retiré du corps du texte".into(),
                });
            }
            lue.page.mots = Vec::new(); // le détail des mots n'est plus utile
            lues.push(lue);
        }
        progression(numero, nb_pages);
    }

    let bloc = mesurer_bloc(&corps_de(&lues));
    let mut etiqueteuse = Etiqueteuse::new();
    let mut blocs: Vec<Bloc> = Vec::new();
    for lue in &lues {
        let (page, classee) = (&lue.page, &lue.classee);
        let paragraphes = paragraphes_page(&classee.corps, page.numero, page.moitie, bloc);
        if classee.corps.is_empty() && classee.notes.is_empty() {
            points.push(point_page_sans_texte(page));
        }
        let (de_page, pts) =
            notes_bas_de_page(paragraphes, &classee.notes, page.numero, page.moitie, &mut etiqueteuse);
        blocs.extend(de_page);
        points.extend(pts);
    }

    let source = forcer_titres.unwrap_or(reglages.titres);
    let (blocs, pts, source_titres) = detecter_titres(blocs, &doc.signets(), nb_pages, source);
    points.extend(pts);
    let (blocs, retraits_notes, pts) = notes_regroupees(blocs, &mut etiqueteuse);
    retraits.extend(retraits_notes);
    points.extend(pts);
    let blocs = raccorder_pages(blocs);

    let nb_notes = blocs.iter().filter(|b| matches!(b, Bloc::Note { .. })).count();
    Ok(Conversion { markdown: ecrire_markdown(&blocs), retraits, points, source_titres, nb_pages, nb_notes })
}

fn texte_des_segments(segments: &[Segment]) -> String {
    segments
        .iter()
        .map(|s| match s {
            Segment::Texte { texte, .. } | Segment::Exposant(texte) => texte.as_str(),
            Segment::AppelNote { marqueur, .. } => marqueur.as_str(),
        })
        .collect()
}

/// Texte d'une seule page (paragraphes puis notes « marqueur texte »), avec ses
/// retraits et ses points ; le bloc est mesuré sur cette page seule.
pub fn apercu_page(doc: &DocumentPdf, numero: u32, reglages: &ReglagesSerie) -> Result<ApercuPage, ErreurMoteur> {
    let lues = lire_page(doc, numero, reglages)?;
    let bloc = mesurer_bloc(&corps_de(&lues));
    let mut etiqueteuse = Etiqueteuse::new();
    let mut points = Vec::new();
    let mut moities = Vec::new();
    for lue in lues {
        let (page, classee) = (&lue.page, &lue.classee);
        if classee.corps.is_empty() && classee.notes.is_empty() {
            points.push(point_page_sans_texte(page));
        }
        let paragraphes = paragraphes_page(&classee.corps, page.numero, page.moitie, bloc);
        let (blocs, pts) =
            notes_bas_de_page(paragraphes, &classee.notes, page.numero, page.moitie, &mut etiqueteuse);
        points.extend(pts);
        let texte = blocs
            .iter()
            .map(|b| match b {
                Bloc::Paragraphe(p) => texte_des_segments(&p.segments),
                Bloc::Note { marqueur, segments, .. } => {
                    format!("{marqueur} {}", texte_des_segments(segments))
                }
                Bloc::Titre { texte, .. } => texte.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        moities.push((page.moitie, texte, lue.classee.retraits));
    }
    Ok(ApercuPage { moities, points })
}

fn point_page_sans_texte(page: &Page) -> PointAVerifier {
    PointAVerifier {
        ancre: ancre_entiere(page),
        genre: GenrePoint::PageSansTexte,
        message: format!("la page {} ne contient aucun texte", page.numero),
    }
}
