//! Détection des titres en cascade (signets PDF → gras + majuscules → taille)
//! et des pages de sommaire d'origine.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use regex::Regex;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::blocs::{Bloc, Paragraphe};
use crate::modele::{Ancre, Moitie, Signet};
use crate::points::{GenrePoint, PointAVerifier};
use crate::reglages::SourceTitres;

const MAX_MOTS_TITRE: usize = 12;
const MAX_LIGNES_TITRE: usize = 3;
const ECART_TAILLE_TITRE: f32 = 1.5;
const MIN_PAGES_TITRE_COURANT: usize = 5;

fn arrondi_demi(taille: f32) -> f32 {
    (taille * 2.0).round() / 2.0
}

fn nb_caracteres(p: &Paragraphe) -> usize {
    p.texte_brut().chars().count()
}

/// Taille dominante des paragraphes, pondérée par le nombre de caractères
/// (tailles arrondies à 0,5 pt). 0 s'il n'y a aucun paragraphe.
pub fn taille_corps(blocs: &[Bloc]) -> f32 {
    let mut poids: BTreeMap<i32, usize> = BTreeMap::new();
    for bloc in blocs {
        if let Bloc::Paragraphe(p) = bloc {
            let cle = (arrondi_demi(p.taille) * 2.0) as i32;
            *poids.entry(cle).or_default() += nb_caracteres(p);
        }
    }
    let mut meilleur: Option<(i32, usize)> = None;
    for (cle, poids) in poids {
        if meilleur.is_none_or(|(_, m)| poids > m) {
            meilleur = Some((cle, poids));
        }
    }
    meilleur.map_or(0.0, |(cle, _)| cle as f32 / 2.0)
}

/// Minuscules, sans diacritiques, tout non-alphanumérique devient espace, espaces réduits.
pub fn normaliser(s: &str) -> String {
    let nettoye: String = s
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    nettoye.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Les signets servent-ils de plan : assez nombreux, pas un par page, pas des « Page N ».
pub fn signets_utilisables(signets: &[Signet], nb_pages: u32) -> bool {
    if signets.len() < 3 {
        return false;
    }
    let pages: BTreeSet<u32> = signets.iter().map(|s| s.page).collect();
    if pages.len() < 3 {
        return false;
    }
    if signets.len() as f32 >= 0.9 * nb_pages as f32 {
        return false;
    }
    let factices = Regex::new(r"(?i)^(page|p\.?)\s*\d+$|^\d+$|\.(jpe?g|png|tiff?)$").unwrap();
    let nb_factices = signets.iter().filter(|s| factices.is_match(s.titre.trim())).count();
    (nb_factices as f32) < 0.5 * signets.len() as f32
}

fn nb_mots(texte: &str) -> usize {
    texte.split_whitespace().count()
}

fn taux_majuscules(texte: &str) -> f32 {
    let lettres = texte.chars().filter(|c| c.is_alphabetic()).count();
    if lettres == 0 {
        return 0.0;
    }
    texte.chars().filter(|c| c.is_uppercase()).count() as f32 / lettres as f32
}

fn court(p: &Paragraphe, texte: &str) -> bool {
    nb_mots(texte) <= MAX_MOTS_TITRE && p.nb_lignes <= MAX_LIGNES_TITRE
}

fn candidat_gras_majuscules(p: &Paragraphe) -> bool {
    let texte = p.texte_brut();
    p.tout_gras && taux_majuscules(&texte) >= 0.8 && court(p, &texte)
}

fn candidat_taille(p: &Paragraphe, corps: f32) -> bool {
    p.taille >= corps + ECART_TAILLE_TITRE && court(p, &p.texte_brut())
}

/// Pages dont la plupart des paragraphes ressemblent à des lignes de sommaire.
fn pages_sommaire(blocs: &[Bloc]) -> BTreeSet<u32> {
    let fin_numero = Regex::new(r"\s\d{1,4}$").unwrap();
    let points = Regex::new(r"\.{3,}").unwrap();
    let mut par_page: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
    for bloc in blocs {
        if let Bloc::Paragraphe(p) = bloc {
            let texte = p.texte_brut();
            let texte = texte.trim();
            let compte = par_page.entry(p.ancre.page).or_default();
            compte.0 += 1;
            if fin_numero.is_match(texte) || points.is_match(texte) {
                compte.1 += 1;
            }
        }
    }
    par_page
        .into_iter()
        .filter(|(_, (total, numerotes))| *numerotes >= 4 && *numerotes * 2 >= *total)
        .map(|(page, _)| page)
        .collect()
}

fn est_sommaire(bloc: &Bloc, pages: &BTreeSet<u32>) -> bool {
    matches!(bloc, Bloc::Paragraphe(_)) && pages.contains(&bloc.ancre().page)
}

/// Titres issus des signets. Retourne les blocs et les points de signets introuvables.
fn titres_depuis_signets(
    blocs: Vec<Bloc>,
    signets: &[Signet],
    pages_som: &BTreeSet<u32>,
) -> (Vec<Bloc>, Vec<PointAVerifier>) {
    let mut points = Vec::new();
    // Remplacements : indice de début → (nombre de blocs avalés, titre).
    let mut remplacements: HashMap<usize, (usize, Bloc)> = HashMap::new();
    let mut utilises: HashSet<usize> = HashSet::new();
    // Insertions avant l'indice (blocs.len() = à la fin).
    let mut insertions: BTreeMap<usize, Vec<Bloc>> = BTreeMap::new();

    for signet in signets {
        let niveau = signet.niveau.clamp(1, 3);
        let cible = normaliser(&signet.titre);
        let trouve = if cible.is_empty() {
            None
        } else {
            [signet.page, signet.page + 1]
                .into_iter()
                .find_map(|page| chercher_signet(&blocs, &cible, page, &utilises, pages_som))
        };
        match trouve {
            Some((debut, fin, texte)) => {
                utilises.extend(debut..fin);
                let ancre = blocs[debut].ancre();
                remplacements.insert(debut, (fin - debut, Bloc::Titre { niveau, texte, ancre }));
            }
            None => {
                let existant = blocs.iter().position(|b| b.ancre().page == signet.page);
                let (position, ancre) = match existant {
                    Some(i) => (i, blocs[i].ancre()),
                    None => (
                        blocs.iter().position(|b| b.ancre().page > signet.page).unwrap_or(blocs.len()),
                        Ancre { page: signet.page, moitie: Moitie::Entiere, rect: [0, 0, 1000, 1000] },
                    ),
                };
                insertions.entry(position).or_default().push(Bloc::Titre {
                    niveau,
                    texte: signet.titre.trim().to_string(),
                    ancre,
                });
                points.push(PointAVerifier {
                    ancre,
                    genre: GenrePoint::TitreSuspect,
                    message: format!(
                        "signet « {} » introuvable page {}",
                        signet.titre.trim(),
                        signet.page
                    ),
                });
            }
        }
    }

    let mut resultat = Vec::with_capacity(blocs.len());
    let mut a_sauter = 0;
    for (i, bloc) in blocs.iter().enumerate() {
        if let Some(titres) = insertions.remove(&i) {
            resultat.extend(titres);
        }
        if a_sauter > 0 {
            a_sauter -= 1;
            continue;
        }
        if let Some((n, titre)) = remplacements.remove(&i) {
            resultat.push(titre);
            a_sauter = n - 1;
        } else {
            resultat.push(bloc.clone());
        }
    }
    if let Some(titres) = insertions.remove(&blocs.len()) {
        resultat.extend(titres);
    }
    (resultat, points)
}

/// Cherche le paragraphe (ou la suite de paragraphes) qui porte le titre du signet sur
/// `page` ou la suivante. Les correspondances s'arrêtent à une frontière de mot ; une
/// correspondance exacte est préférée à une simple correspondance de début ; un résultat
/// qui n'est pas court (plus de 12 mots ou 3 lignes) est refusé.
fn chercher_signet(
    blocs: &[Bloc],
    cible: &str,
    page: u32,
    utilises: &HashSet<usize>,
    pages_som: &BTreeSet<u32>,
) -> Option<(usize, usize, String)> {
    let libre = |i: usize| {
        !utilises.contains(&i) && matches!(blocs[i], Bloc::Paragraphe(_)) && !est_sommaire(&blocs[i], pages_som)
    };
    let mut premier_prefixe = None;
    for page_cherchee in [page, page + 1] {
        for debut in 0..blocs.len() {
            if blocs[debut].ancre().page != page_cherchee || !libre(debut) {
                continue;
            }
            let mut textes: Vec<String> = Vec::new();
            let mut lignes = 0;
            let mut fin = debut;
            loop {
                let Bloc::Paragraphe(p) = &blocs[fin] else { break };
                textes.push(p.texte_brut().trim().to_string());
                lignes += p.nb_lignes;
                fin += 1;
                let texte = textes.join(" ");
                let norm = normaliser(&texte);
                if norm.is_empty() {
                    break;
                }
                let court = nb_mots(&texte) <= MAX_MOTS_TITRE && lignes <= MAX_LIGNES_TITRE;
                if norm == cible && court {
                    return Some((debut, fin, texte));
                }
                if norm.strip_prefix(cible).is_some_and(|reste| reste.starts_with(' ')) {
                    if court && premier_prefixe.is_none() {
                        premier_prefixe = Some((debut, fin, texte));
                    }
                    break;
                }
                let debut_du_titre = cible.strip_prefix(norm.as_str()).is_some_and(|r| r.starts_with(' '));
                if !debut_du_titre || fin >= blocs.len() || !libre(fin) {
                    break;
                }
            }
        }
    }
    premier_prefixe
}

/// Transforme en titres les paragraphes retenus par `candidat`, niveaux par taille.
fn titres_par_candidats(
    blocs: Vec<Bloc>,
    pages_som: &BTreeSet<u32>,
    candidat: impl Fn(&Paragraphe) -> bool,
) -> Vec<Bloc> {
    let est_candidat = |b: &Bloc| match b {
        Bloc::Paragraphe(p) => !pages_som.contains(&p.ancre.page) && candidat(p),
        _ => false,
    };
    let mut tailles: Vec<i32> = blocs
        .iter()
        .filter(|b| est_candidat(b))
        .filter_map(|b| match b {
            Bloc::Paragraphe(p) => Some((arrondi_demi(p.taille) * 2.0) as i32),
            _ => None,
        })
        .collect();
    tailles.sort_unstable_by(|a, b| b.cmp(a));
    tailles.dedup();

    blocs
        .into_iter()
        .map(|b| {
            if !est_candidat(&b) {
                return b;
            }
            let Bloc::Paragraphe(p) = b else { unreachable!() };
            let cle = (arrondi_demi(p.taille) * 2.0) as i32;
            let rang = tailles.iter().position(|t| *t == cle).unwrap_or(0);
            Bloc::Titre {
                niveau: (rang + 1).min(3) as u8,
                texte: p.texte_brut().trim().to_string(),
                ancre: p.ancre,
            }
        })
        .collect()
}

/// Fusionne les titres adjacents de même niveau, même page et même moitié.
fn fusionner_titres_consecutifs(blocs: Vec<Bloc>) -> Vec<Bloc> {
    let mut resultat: Vec<Bloc> = Vec::with_capacity(blocs.len());
    for bloc in blocs {
        if let (
            Bloc::Titre { niveau, texte, ancre },
            Some(Bloc::Titre { niveau: n0, texte: t0, ancre: a0 }),
        ) = (&bloc, resultat.last_mut())
        {
            if niveau == n0 && ancre.page == a0.page && ancre.moitie == a0.moitie {
                t0.push(' ');
                t0.push_str(texte);
                continue;
            }
        }
        resultat.push(bloc);
    }
    resultat
}

/// Un même titre sur au moins 5 pages distinctes est probablement un titre courant.
fn titres_courants_suspects(blocs: &[Bloc]) -> Vec<PointAVerifier> {
    let mut pages: HashMap<String, BTreeSet<u32>> = HashMap::new();
    for bloc in blocs {
        if let Bloc::Titre { texte, ancre, .. } = bloc {
            pages.entry(normaliser(texte)).or_default().insert(ancre.page);
        }
    }
    blocs
        .iter()
        .filter_map(|bloc| match bloc {
            Bloc::Titre { texte, ancre, .. }
                if pages.get(&normaliser(texte)).is_some_and(|p| p.len() >= MIN_PAGES_TITRE_COURANT) =>
            {
                Some(PointAVerifier {
                    ancre: *ancre,
                    genre: GenrePoint::TitreSuspect,
                    message: format!(
                        "le titre « {} » se répète sur {} pages : titre courant possible",
                        texte,
                        pages[&normaliser(texte)].len()
                    ),
                })
            }
            _ => None,
        })
        .collect()
}

/// Détecte les titres selon `source` (ou la cascade en mode `Auto`) et les pages de
/// sommaire. Retourne les blocs, les points à vérifier et la source réellement utilisée.
pub fn detecter_titres(
    blocs: Vec<Bloc>,
    signets: &[Signet],
    nb_pages: u32,
    source: SourceTitres,
) -> (Vec<Bloc>, Vec<PointAVerifier>, SourceTitres) {
    let pages_som = pages_sommaire(&blocs);
    let mut points: Vec<PointAVerifier> = pages_som
        .iter()
        .filter_map(|page| blocs.iter().find(|b| b.ancre().page == *page))
        .map(|bloc| PointAVerifier {
            ancre: bloc.ancre(),
            genre: GenrePoint::Sommaire,
            message: format!(
                "la page {} semble être un sommaire d'origine : conservée comme texte",
                bloc.ancre().page
            ),
        })
        .collect();

    let corps = taille_corps(&blocs);
    let effective = match source {
        SourceTitres::Auto => {
            let nb_gras_maj = blocs
                .iter()
                .filter(|b| match b {
                    Bloc::Paragraphe(p) => !pages_som.contains(&p.ancre.page) && candidat_gras_majuscules(p),
                    _ => false,
                })
                .count();
            if signets_utilisables(signets, nb_pages) {
                SourceTitres::Signets
            } else if nb_gras_maj >= 3 {
                SourceTitres::GrasMajuscules
            } else {
                SourceTitres::Taille
            }
        }
        autre => autre,
    };

    let blocs = match effective {
        SourceTitres::Signets => {
            let (blocs, manquants) = titres_depuis_signets(blocs, signets, &pages_som);
            points.extend(manquants);
            blocs
        }
        SourceTitres::GrasMajuscules => {
            fusionner_titres_consecutifs(titres_par_candidats(blocs, &pages_som, candidat_gras_majuscules))
        }
        _ => fusionner_titres_consecutifs(titres_par_candidats(blocs, &pages_som, |p| {
            candidat_taille(p, corps)
        })),
    };
    points.extend(titres_courants_suspects(&blocs));
    (blocs, points, effective)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocs::Segment;

    fn ancre(page: u32) -> Ancre {
        Ancre { page, moitie: Moitie::Entiere, rect: [100, 100, 900, 200] }
    }

    fn para(page: u32, texte: &str, taille: f32, gras: bool, lignes: usize) -> Bloc {
        Bloc::Paragraphe(Paragraphe {
            segments: vec![Segment::Texte { texte: texte.into(), gras, italique: false }],
            ancre: ancre(page),
            taille,
            tout_gras: gras,
            nb_lignes: lignes,
        })
    }

    fn corps(page: u32, texte: &str) -> Bloc {
        para(page, texte, 12.0, false, 5)
    }

    fn signet(titre: &str, niveau: u8, page: u32) -> Signet {
        Signet { titre: titre.into(), niveau, page }
    }

    fn titres(blocs: &[Bloc]) -> Vec<(u8, String)> {
        blocs
            .iter()
            .filter_map(|b| match b {
                Bloc::Titre { niveau, texte, .. } => Some((*niveau, texte.clone())),
                _ => None,
            })
            .collect()
    }

    fn texte_long() -> String {
        "Un long paragraphe de texte courant qui occupe plusieurs lignes du livre. ".repeat(4)
    }

    #[test]
    fn signets_rejetes_si_un_par_page() {
        let un_par_page: Vec<Signet> = (1..=207).map(|p| signet(&format!("Chapitre {p}"), 1, p)).collect();
        assert!(!signets_utilisables(&un_par_page, 207));
        let pages: Vec<Signet> = (1..=96).map(|p| signet(&format!("Page {p}"), 1, p)).collect();
        assert!(!signets_utilisables(&pages, 96));
        let bons: Vec<Signet> = (1..=73).map(|i| signet(&format!("Chapitre {i}"), 1, i * 2 - 1)).collect();
        assert!(signets_utilisables(&bons, 136));
    }

    #[test]
    fn titre_depuis_signet() {
        let blocs = vec![corps(2, &texte_long()), para(3, "1. VISITE À PATMOS", 12.0, false, 1), corps(3, &texte_long())];
        let signets = [signet("1. VISITE À PATMOS", 1, 3), signet("B", 1, 5), signet("C", 1, 7)];
        let (blocs, _, source) = detecter_titres(blocs, &signets, 100, SourceTitres::Signets);
        assert_eq!(source, SourceTitres::Signets);
        assert_eq!(titres(&blocs)[0], (1, "1. VISITE À PATMOS".to_string()));
        assert_eq!(blocs.len(), 3 + 2); // deux titres insérés pour B et C
    }

    #[test]
    fn titre_multiligne_depuis_signet() {
        let blocs = vec![
            para(4, "LE SENS DE L’AMOUR", 12.0, false, 1),
            para(4, "DANS LE MONDE", 12.0, false, 1),
            corps(4, &texte_long()),
        ];
        let signets = [signet("LE SENS DE L’AMOUR DANS LE MONDE", 1, 4)];
        let (blocs, points, _) = detecter_titres(blocs, &signets, 100, SourceTitres::Signets);
        assert_eq!(titres(&blocs), vec![(1, "LE SENS DE L’AMOUR DANS LE MONDE".to_string())]);
        assert_eq!(blocs.len(), 2);
        assert!(points.is_empty());
    }

    #[test]
    fn signet_introuvable() {
        let blocs = vec![corps(6, &texte_long())];
        let signets = [signet("Épilogue", 1, 6)];
        let (blocs, points, _) = detecter_titres(blocs, &signets, 100, SourceTitres::Signets);
        assert!(matches!(&blocs[0], Bloc::Titre { texte, niveau: 1, .. } if texte == "Épilogue"));
        assert!(matches!(blocs[1], Bloc::Paragraphe(_)));
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].genre, GenrePoint::TitreSuspect);
        assert!(points[0].message.contains("Épilogue") && points[0].message.contains('6'));
    }

    #[test]
    fn signet_court_ne_avale_pas_le_corps() {
        let blocs = vec![corps(3, "Il était une fois un livre très long."), corps(3, &texte_long())];
        let (blocs, points, _) = detecter_titres(blocs, &[signet("I", 1, 3)], 100, SourceTitres::Signets);
        assert!(matches!(&blocs[0], Bloc::Titre { texte, .. } if texte == "I"));
        assert!(matches!(blocs[1], Bloc::Paragraphe(_)));
        assert_eq!(points.len(), 1);
    }

    #[test]
    fn signet_refuse_un_long_paragraphe() {
        let long = format!("Introduction de la méthode {}", vec!["mot"; 20].join(" "));
        let blocs = vec![para(3, &long, 12.0, false, 2)];
        let (blocs, points, _) = detecter_titres(blocs, &[signet("Introduction", 1, 3)], 100, SourceTitres::Signets);
        assert!(matches!(&blocs[0], Bloc::Titre { texte, .. } if texte == "Introduction"));
        assert!(matches!(blocs[1], Bloc::Paragraphe(_)));
        assert_eq!(points.len(), 1);
    }

    #[test]
    fn signet_prefere_la_correspondance_exacte() {
        let blocs = vec![para(3, "Introduction générale", 12.0, false, 1), para(3, "Introduction", 12.0, false, 1)];
        let (blocs, points, _) = detecter_titres(blocs, &[signet("Introduction", 1, 3)], 100, SourceTitres::Signets);
        assert!(matches!(&blocs[0], Bloc::Paragraphe(_)));
        assert!(matches!(&blocs[1], Bloc::Titre { texte, .. } if texte == "Introduction"));
        assert!(points.is_empty());
    }

    #[test]
    fn signet_page_vide_insere_avant_la_page_suivante() {
        let blocs = vec![corps(3, &texte_long()), corps(5, &texte_long())];
        let (blocs, _, _) = detecter_titres(blocs, &[signet("Partie", 1, 4)], 100, SourceTitres::Signets);
        assert!(matches!(blocs[0], Bloc::Paragraphe(_)));
        assert!(matches!(&blocs[1], Bloc::Titre { ancre, .. } if ancre.page == 4 && ancre.rect == [0, 0, 1000, 1000]));
        assert!(matches!(blocs[2], Bloc::Paragraphe(_)));
    }

    #[test]
    fn gras_majuscules() {
        let mut blocs = Vec::new();
        for n in 1..=10 {
            blocs.push(para(n, &format!("CHAPITRE {n}:"), 12.0, true, 1));
            blocs.push(corps(n, &texte_long()));
        }
        blocs.push(para(11, "Un titre en gras minuscule", 12.0, true, 1));
        let (blocs, _, source) = detecter_titres(blocs, &[], 11, SourceTitres::Auto);
        assert_eq!(source, SourceTitres::GrasMajuscules);
        let t = titres(&blocs);
        assert_eq!(t.len(), 10);
        assert!(t.iter().all(|(n, _)| *n == 1));
        assert!(matches!(blocs.last(), Some(Bloc::Paragraphe(_))));
    }

    #[test]
    fn par_taille_niveaux() {
        let long: String = vec!["mot"; 20].join(" ");
        let blocs = vec![
            corps(1, &texte_long()),
            para(2, "Grand titre", 18.0, false, 1),
            corps(2, &texte_long()),
            para(3, "Sous-titre", 14.0, false, 1),
            corps(3, &texte_long()),
            para(4, &long, 18.0, false, 2),
        ];
        let (blocs, _, source) = detecter_titres(blocs, &[], 4, SourceTitres::Auto);
        assert_eq!(source, SourceTitres::Taille);
        assert_eq!(titres(&blocs), vec![(1, "Grand titre".to_string()), (2, "Sous-titre".to_string())]);
        assert!(matches!(blocs[5], Bloc::Paragraphe(_)));
    }

    #[test]
    fn titres_consecutifs_fusionnes() {
        let blocs = vec![
            corps(1, &texte_long()),
            para(2, "CHAPITRE 1:", 18.0, true, 1),
            para(2, "LES ANGES", 18.0, true, 1),
            corps(2, &texte_long()),
        ];
        let (blocs, _, _) = detecter_titres(blocs, &[], 2, SourceTitres::Taille);
        assert_eq!(titres(&blocs), vec![(1, "CHAPITRE 1: LES ANGES".to_string())]);
        assert_eq!(blocs.len(), 3);
    }

    #[test]
    fn sommaire_non_titre() {
        let mut blocs = vec![corps(1, &texte_long())];
        for (i, nom) in ["Visite à Patmos", "Les anges", "Le ciel", "La terre", "La mer", "Le feu"].iter().enumerate() {
            blocs.push(para(2, &format!("{nom} ...... {}", 12 + i), 18.0, true, 1));
        }
        let (blocs, points, _) = detecter_titres(blocs, &[], 10, SourceTitres::Taille);
        assert!(titres(&blocs).is_empty());
        assert_eq!(blocs.len(), 7);
        let sommaires: Vec<_> = points.iter().filter(|p| p.genre == GenrePoint::Sommaire).collect();
        assert_eq!(sommaires.len(), 1);
        assert_eq!(sommaires[0].ancre.page, 2);
    }

    #[test]
    fn titre_courant_suspect() {
        let mut blocs = Vec::new();
        for page in 1..=6 {
            blocs.push(para(page, "LE LIVRE DES ANGES", 12.0, true, 1));
            blocs.push(corps(page, &texte_long()));
        }
        let (_, points, _) = detecter_titres(blocs, &[], 6, SourceTitres::GrasMajuscules);
        let suspects: Vec<_> = points.iter().filter(|p| p.genre == GenrePoint::TitreSuspect).collect();
        assert_eq!(suspects.len(), 6);
    }

    #[test]
    fn normaliser_retire_diacritiques_et_ponctuation() {
        assert_eq!(normaliser("  L’Âme,  d'été! "), "l ame d ete");
    }

    #[test]
    fn taille_corps_pondere_par_caracteres() {
        let blocs = vec![para(1, "court", 18.0, false, 1), corps(1, &texte_long()), para(1, "x", 12.2, false, 1)];
        assert_eq!(taille_corps(&blocs), 12.0);
    }
}
