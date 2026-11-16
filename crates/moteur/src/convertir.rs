//! Conversion d'un livre (ou d'une page) : toutes les étapes du moteur, de la
//! lecture du PDF au Markdown ancré.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::blocs::{Bloc, Segment};
use crate::double_page::couper_double_page;
use crate::lignes::{Ligne, regrouper_en_lignes};
use crate::markdown::ecrire_markdown;
use crate::modele::{Ancre, DemiPage, ErreurMoteur, Moitie, Page};
use crate::notes::{Etiqueteuse, notes_bas_de_page, notes_regroupees};
use crate::paragraphes::{Vocabulaire, mesurer_bloc, nettoyer_sauts, page_courte, paragraphes_page, raccorder_pages};
use crate::pdf::DocumentPdf;
use crate::points::{GenrePoint, PointAVerifier};
use crate::reglages::{ReglagesSerie, SourceTitres};
use crate::titres::{detecter_titres, placer_titre_livre, sommaire_cliquable};
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

/// Vocabulaire des lignes de corps et de notes, pour trancher les tirets de fin de ligne.
fn vocabulaire_de(lues: &[PageLue]) -> Vocabulaire {
    Vocabulaire::mesurer(lues.iter().flat_map(|l| l.classee.corps.iter().chain(&l.classee.notes)))
}

fn corps_de(lues: &[PageLue]) -> Vec<Vec<Ligne>> {
    lues.iter().map(|l| l.classee.corps.clone()).collect()
}

/// Convertit tout le livre en Markdown ancré. `titre_livre` (titre retenu pour l'EPUB)
/// devient le premier titre de niveau 1 du Markdown ; vide, il est ignoré.
pub fn convertir_livre(
    doc: &DocumentPdf,
    reglages: &ReglagesSerie,
    titre_livre: &str,
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
    let vocab = vocabulaire_de(&lues);
    let mut etiqueteuse = Etiqueteuse::new();
    let mut blocs: Vec<Bloc> = Vec::new();
    // La dernière page avec du corps s'est-elle arrêtée bien avant le bas du bloc ?
    let mut apres_page_courte = false;
    for lue in &lues {
        let (page, classee) = (&lue.page, &lue.classee);
        let paragraphes = paragraphes_page(&classee.corps, page.numero, page.moitie, bloc, &vocab);
        if classee.corps.is_empty() && classee.notes.is_empty() {
            points.push(point_page_sans_texte(page));
        }
        if let Some(premiere) = classee.corps.first() {
            if apres_page_courte {
                blocs.push(Bloc::Saut { ancre: Ancre::depuis(page.numero, page.moitie, premiere.rect) });
            }
            apres_page_courte = page_courte(&classee.corps, &classee.notes, bloc);
        }
        let (de_page, pts) =
            notes_bas_de_page(paragraphes, &classee.notes, page.numero, page.moitie, &mut etiqueteuse, &vocab);
        blocs.extend(de_page);
        points.extend(pts);
    }

    let source = forcer_titres.unwrap_or(reglages.titres);
    let (blocs, pts, source_titres, retraits_titres, illisibles, mut sommaires) =
        detecter_titres(blocs, &doc.signets(), nb_pages, source);
    points.extend(pts);
    retraits.extend(retraits_titres);
    // Avant le raccord des pages (le titre ne s'y recolle pas) et le nettoyage des
    // sauts (aucun saut ne reste contre lui).
    let blocs = placer_titre_livre(blocs, titre_livre);
    let (blocs, retraits_notes, pts) = notes_regroupees(blocs, &mut etiqueteuse, &vocab);
    retraits.extend(retraits_notes);
    points.extend(pts);
    // Après le regroupement des notes (qui peut retirer un titre « Notes ») : les
    // identifiants visés par le sommaire sont ceux des titres définitifs.
    let (blocs, pts, moities_cliquables) = sommaire_cliquable(blocs, titre_livre);
    reunir_sommaires(&mut points, &mut sommaires, pts, moities_cliquables);
    let blocs = nettoyer_sauts(raccorder_pages(blocs, &vocab, &sommaires, &illisibles));

    let nb_notes = blocs.iter().filter(|b| matches!(b, Bloc::Note { .. })).count();
    Ok(Conversion { markdown: ecrire_markdown(&blocs), retraits, points, source_titres, nb_pages, nb_notes })
}

/// Ajoute aux points et aux (demi-)pages de sommaire de `detecter_titres` (`points`,
/// `sommaires`) ceux du sommaire cliquable (`pts`, `moities`). Ces (demi-)pages ne se
/// raccordent pas (`raccorder_pages`) ; l'autre moitié d'une page double, si elle n'en
/// est pas une, se raccorde normalement. Un seul point par page : celui du sommaire
/// cliquable remplace « conservée comme texte » d'une page déjà reconnue comme sommaire.
fn reunir_sommaires(
    points: &mut Vec<PointAVerifier>,
    sommaires: &mut BTreeSet<DemiPage>,
    pts: Vec<PointAVerifier>,
    moities: BTreeSet<DemiPage>,
) {
    sommaires.extend(moities);
    for point in pts {
        match points.iter_mut().find(|p| p.genre == GenrePoint::Sommaire && p.ancre.page == point.ancre.page) {
            Some(ancien) => *ancien = point,
            None => points.push(point),
        }
    }
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
    let vocab = vocabulaire_de(&lues);
    let mut etiqueteuse = Etiqueteuse::new();
    let mut points = Vec::new();
    let mut moities = Vec::new();
    for lue in lues {
        let (page, classee) = (&lue.page, &lue.classee);
        if classee.corps.is_empty() && classee.notes.is_empty() {
            points.push(point_page_sans_texte(page));
        }
        let paragraphes = paragraphes_page(&classee.corps, page.numero, page.moitie, bloc, &vocab);
        let (blocs, pts) =
            notes_bas_de_page(paragraphes, &classee.notes, page.numero, page.moitie, &mut etiqueteuse, &vocab);
        points.extend(pts);
        let texte = blocs
            .iter()
            .map(|b| match b {
                Bloc::Paragraphe(p) => texte_des_segments(&p.segments),
                Bloc::EntreeSommaire { segments, .. } => texte_des_segments(segments),
                Bloc::Note { marqueur, segments, .. } => {
                    format!("{marqueur} {}", texte_des_segments(segments))
                }
                Bloc::Titre { texte, .. } => texte.clone(),
                Bloc::Saut { .. } => String::new(),
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::blocs::Paragraphe;

    fn para(page: u32, moitie: Moitie, texte: &str) -> Bloc {
        Bloc::Paragraphe(Paragraphe {
            segments: vec![Segment::Texte { texte: texte.into(), gras: false, italique: false }],
            ancre: Ancre { page, moitie, rect: [100, 100, 900, 130] },
            taille: 12.0,
            tout_gras: false,
            nb_lignes: 1,
        })
    }

    fn point(page: u32, moitie: Moitie, message: &str) -> PointAVerifier {
        PointAVerifier {
            ancre: Ancre { page, moitie, rect: [100, 100, 900, 130] },
            genre: GenrePoint::Sommaire,
            message: message.into(),
        }
    }

    #[test]
    fn deux_moities_de_sommaire_un_seul_point_aucun_raccord() {
        // `detecter_titres` a vu un sommaire en p3d ; le sommaire cliquable couvre p3g et p3d.
        let mut points = vec![point(3, Moitie::Droite, "conservée comme texte")];
        let mut sommaires = BTreeSet::from([(3, Moitie::Droite)]);
        let cliquables = BTreeSet::from([(3, Moitie::Gauche), (3, Moitie::Droite)]);
        reunir_sommaires(&mut points, &mut sommaires, vec![point(3, Moitie::Gauche, "cliquable")], cliquables);
        assert_eq!(points.len(), 1);
        assert_eq!((points[0].ancre.moitie, points[0].message.as_str()), (Moitie::Gauche, "cliquable"));
        assert_eq!(sommaires, BTreeSet::from([(3, Moitie::Gauche), (3, Moitie::Droite)]));
        // Aucune des deux moitiés ne se raccorde, ni entre elles, ni à leurs voisines.
        let blocs = || {
            vec![
                para(2, Moitie::Droite, "fin de p2d"),
                para(3, Moitie::Gauche, "entrée de p3g"),
                para(3, Moitie::Droite, "entrée de p3d"),
                para(4, Moitie::Gauche, "début de p4g"),
            ]
        };
        let vocab = Vocabulaire::default();
        assert_eq!(raccorder_pages(blocs(), &vocab, &sommaires, &HashSet::new()).len(), 4);
        // Témoin : sans les sommaires, tout se raccorde.
        assert_eq!(raccorder_pages(blocs(), &vocab, &BTreeSet::new(), &HashSet::new()).len(), 1);
    }
}
