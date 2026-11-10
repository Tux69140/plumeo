//! Non-régression sur le corpus réel (hors dépôt, jamais versionné).
//!
//! ```text
//! PLUMEO_CORPUS=/chemin/du/corpus [EPUBCHECK_JAR=…/epubcheck.jar] \
//!   cargo test --release -p plumeo-moteur --test corpus -- --ignored --nocapture
//! ```
//!
//! Le manifeste `tests/corpus/manifeste.json` décrit chaque livre, sa série
//! (`tests/corpus/series/<serie>.json`) et ses attentes ; voir `docs/corpus.md`.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Instant;

use plumeo_epub::{Metadonnees, generer_epub};
use plumeo_moteur::convertir_livre;
use plumeo_moteur::modele::{Ancre, Moitie};
use plumeo_moteur::pdf::DocumentPdf;
use plumeo_moteur::reglages::{ReglagesSerie, SourceTitres, Zones};
use plumeo_moteur::zones::est_numero_page;
use regex::Regex;
use serde::Deserialize;
use unicode_normalization::UnicodeNormalization;

const RACINE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entree {
    fichier: String,
    serie: String,
    titres: SourceTitres,
    attentes: Attentes,
    derogations: Option<Derogations>,
}

/// Attentes communes qu'un livre ne peut pas tenir, pour une raison mesurée et
/// consignée dans `docs/corpus.md` ; chaque dérogation est affichée à chaque passage.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Derogations {
    raison: String,
    /// Longueur médiane minimale des paragraphes, au lieu de 80.
    mediane_paragraphes_min: Option<usize>,
    /// Pages où un bloc fait d'un seul nombre est admis (étiquette de schéma).
    #[serde(default)]
    nombres_isoles_pages: Vec<u32>,
}

/// Attentes propres à un livre ; un champ absent ou `null` n'est pas vérifié.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attentes {
    /// Source des titres réellement utilisée.
    source_titres: Option<SourceTitres>,
    /// Nombre exact de titres de niveau 1 (commençant par `prefixe_n1` s'il est donné).
    titres_n1: Option<usize>,
    /// Textes que doit contenir au moins un titre.
    #[serde(default)]
    titres_contiennent: Vec<String>,
    prefixe_n1: Option<String>,
    /// Nombre minimal de titres de niveau 1 (commençant par `prefixe_n1` s'il est donné).
    n1_min: Option<usize>,
    /// Nombre maximal de titres de niveau 1, tous préfixes confondus.
    n1_max: Option<usize>,
    /// Nombre exact de titres de niveau 2.
    n2: Option<usize>,
    /// Pages qui doivent porter au moins une note (de ce marqueur s'il est donné).
    #[serde(default)]
    notes_pages: Vec<NotePage>,
    /// Nombre minimal de notes appariées dans le livre.
    notes_min: Option<usize>,
    /// Pages qui ne doivent porter aucune note.
    #[serde(default)]
    sans_notes_pages: Vec<u32>,
    /// Motifs qu'aucun paragraphe ne doit vérifier.
    #[serde(default)]
    absent_regex: Vec<String>,
    /// Ancres en `g` et en `d` (double page).
    #[serde(default)]
    moities: bool,
    /// Durée maximale de la conversion, en secondes (mesurée en `--release`).
    duree_max_s: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NotePage {
    page: u32,
    marqueur: Option<String>,
}

enum Genre {
    Titre(usize),
    Note(String),
    Paragraphe,
}

/// Bloc relu dans le Markdown interne : ancre, genre et texte brut.
struct BlocMd {
    ancre: Ancre,
    genre: Genre,
    texte: String,
}

/// Texte brut d'une ligne de Markdown interne : sans échappements, balises,
/// délimiteurs d'emphase ni appels de note.
fn texte_brut(md: &str) -> String {
    let mut sortie = String::new();
    let mut c = md.chars().peekable();
    while let Some(x) = c.next() {
        match x {
            '\\' => sortie.extend(c.next()),
            '*' => {}
            '<' => {
                for y in c.by_ref() {
                    if y == '>' {
                        break;
                    }
                }
            }
            '[' if c.peek() == Some(&'^') => {
                for y in c.by_ref() {
                    if y == ']' {
                        break;
                    }
                }
            }
            _ => sortie.push(x),
        }
    }
    sortie.trim().to_string()
}

fn lire_blocs(md: &str) -> Vec<BlocMd> {
    let mut blocs = Vec::new();
    let mut ancre: Option<Ancre> = None;
    for ligne in md.lines() {
        if let Some(a) = ligne.strip_prefix("<!-- ").and_then(|l| l.strip_suffix(" -->")) {
            ancre = Some(Ancre::from_str(a).unwrap_or_else(|e| panic!("{e}")));
            continue;
        }
        if ligne.trim().is_empty() {
            continue;
        }
        let ancre = ancre.take().unwrap_or_else(|| panic!("bloc sans ancre : {ligne}"));
        let dieses = ligne.chars().take_while(|c| *c == '#').count();
        let (genre, texte) = if dieses > 0 && ligne[dieses..].starts_with(' ') {
            (Genre::Titre(dieses), texte_brut(&ligne[dieses..]))
        } else if let Some((etiquette, reste)) = ligne.strip_prefix("[^").and_then(|l| l.split_once("]: ")) {
            let marqueur = etiquette.split_once('-').map_or("", |(_, m)| m).to_string();
            (Genre::Note(marqueur), texte_brut(reste))
        } else {
            (Genre::Paragraphe, texte_brut(ligne))
        };
        blocs.push(BlocMd { ancre, genre, texte });
    }
    blocs
}

/// Nom comparable : NFC, apostrophe typographique ramenée à l'apostrophe droite.
fn nom_normalise(s: &str) -> String {
    s.nfc().collect::<String>().replace('\u{2019}', "'")
}

/// Retrouve `relatif` sous `base` en comparant les noms normalisés, composant par composant.
fn trouver(base: &Path, relatif: &str) -> Result<PathBuf, String> {
    let mut chemin = base.to_path_buf();
    for composant in relatif.split('/') {
        let voulu = nom_normalise(composant);
        let entrees = std::fs::read_dir(&chemin).map_err(|e| format!("{} : {e}", chemin.display()))?;
        let trouve = entrees
            .filter_map(Result::ok)
            .find(|e| nom_normalise(&e.file_name().to_string_lossy()) == voulu)
            .ok_or_else(|| format!("introuvable : {composant} dans {}", chemin.display()))?;
        chemin = trouve.path();
    }
    Ok(chemin)
}

fn zones_non_nulles(r: &ReglagesSerie) -> bool {
    let non_nulle = |z: &Zones| z.entete > 0.0 || z.pied > 0.0 || z.notes > 0.0;
    non_nulle(&r.paire) || non_nulle(&r.impaire) || r.exceptions.values().any(non_nulle)
}

fn rang_moitie(m: Moitie) -> u8 {
    match m {
        Moitie::Gauche | Moitie::Entiere => 0,
        Moitie::Droite => 1,
    }
}

fn mediane(mut v: Vec<usize>) -> usize {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[v.len() / 2]
}

fn extrait(s: &str) -> String {
    let e: String = s.chars().take(80).collect();
    if e.len() < s.len() { format!("{e}…") } else { e }
}

fn source_texte(s: SourceTitres) -> String {
    serde_json::to_value(s).unwrap().as_str().unwrap_or("?").to_string()
}

fn epubcheck(jar: &str, markdown: &str, nom: &str) -> Result<(), String> {
    let dossier = tempfile::tempdir().map_err(|e| e.to_string())?;
    let chemin = dossier.path().join("livre.epub");
    let meta = Metadonnees {
        titre: nom.to_string(),
        auteur: "Corpus".into(),
        langue: "fr".into(),
        identifiant: "urn:uuid:00000000-0000-5000-8000-000000000000".into(),
        modifie: "2026-11-10T00:00:00Z".into(),
    };
    generer_epub(markdown, &meta, None, &chemin).map_err(|e| format!("generer_epub : {e}"))?;
    let sortie = std::process::Command::new("java")
        .args(["-jar", jar])
        .arg(&chemin)
        .output()
        .map_err(|e| format!("java : {e}"))?;
    if sortie.status.success() {
        return Ok(());
    }
    let texte = String::from_utf8_lossy(&sortie.stdout).to_string() + &String::from_utf8_lossy(&sortie.stderr);
    let erreurs: Vec<&str> = texte.lines().filter(|l| l.starts_with("ERROR") || l.starts_with("FATAL")).take(5).collect();
    Err(format!("epubcheck : {}", erreurs.join(" | ")))
}

/// Convertit un livre et renvoie la liste de ses attentes non tenues.
fn verifier(corpus: &Path, entree: &Entree) -> Vec<String> {
    let mut echecs = Vec::new();
    let chemin = match trouver(corpus, &entree.fichier) {
        Ok(c) => c,
        Err(e) => return vec![e],
    };
    let serie = Path::new(RACINE).join("series").join(format!("{}.json", entree.serie));
    let reglages = match ReglagesSerie::lire(&serie) {
        Ok(r) => r,
        Err(e) => return vec![e.to_string()],
    };
    let forcer = (entree.titres != SourceTitres::Auto).then_some(entree.titres);
    let debut = Instant::now();
    let conversion = DocumentPdf::ouvrir(&chemin)
        .and_then(|doc| convertir_livre(&doc, &reglages, forcer, &mut |_, _| {}));
    let duree = debut.elapsed().as_secs_f64();
    let conversion = match conversion {
        Ok(c) => c,
        Err(e) => return vec![format!("conversion : {e}")],
    };
    let blocs = lire_blocs(&conversion.markdown);
    let a = &entree.attentes;
    let d = entree.derogations.as_ref();

    // Attentes communes.
    let nombres_admis = d.map_or(&[][..], |d| &d.nombres_isoles_pages[..]);
    for b in blocs.iter().filter(|b| !nombres_admis.contains(&b.ancre.page)) {
        if est_numero_page(&b.texte) {
            echecs.push(format!("numéro de page resté dans le texte : « {} » ({})", b.texte, b.ancre));
        }
    }
    let longueurs: Vec<usize> = blocs
        .iter()
        .filter(|b| matches!(b.genre, Genre::Paragraphe))
        .map(|b| b.texte.chars().count())
        .collect();
    let (med, max) = (mediane(longueurs.clone()), longueurs.iter().copied().max().unwrap_or(0));
    let med_min = d.and_then(|d| d.mediane_paragraphes_min).unwrap_or(80);
    if !(med_min..=1500).contains(&med) {
        echecs.push(format!("longueur médiane des paragraphes {med} hors de {med_min}..=1500"));
    }
    if max > 8000 {
        let long = blocs.iter().find(|b| matches!(b.genre, Genre::Paragraphe) && b.texte.chars().count() == max);
        echecs.push(format!("paragraphe de {max} caractères (> 8000) : {}", long.map_or(String::new(), |b| b.ancre.to_string())));
    }
    let mut precedente: Option<Ancre> = None;
    for b in blocs.iter().filter(|b| !matches!(b.genre, Genre::Note(_))) {
        if let Some(p) = precedente
            && (b.ancre.page, rang_moitie(b.ancre.moitie)) < (p.page, rang_moitie(p.moitie))
        {
            echecs.push(format!("ancres décroissantes : {p} puis {}", b.ancre));
        }
        precedente = Some(b.ancre);
    }
    if zones_non_nulles(&reglages) && conversion.retraits.is_empty() {
        echecs.push("aucun retrait malgré des zones non nulles".into());
    }
    if let Ok(jar) = std::env::var("EPUBCHECK_JAR")
        && let Err(e) = epubcheck(&jar, &conversion.markdown, &entree.fichier)
    {
        echecs.push(e);
    }

    // Attentes propres au livre.
    let titres = |niveau: usize| -> Vec<&BlocMd> {
        blocs.iter().filter(|b| matches!(b.genre, Genre::Titre(n) if n == niveau)).collect()
    };
    let n1 = titres(1);
    let n1_prefixes = n1
        .iter()
        .filter(|b| a.prefixe_n1.as_ref().is_none_or(|p| b.texte.starts_with(p.as_str())))
        .count();
    let notes: Vec<(&BlocMd, &str)> = blocs
        .iter()
        .filter_map(|b| match &b.genre {
            Genre::Note(m) => Some((b, m.as_str())),
            _ => None,
        })
        .collect();
    if let Some(s) = a.source_titres
        && s != conversion.source_titres
    {
        echecs.push(format!("source des titres {} au lieu de {}", source_texte(conversion.source_titres), source_texte(s)));
    }
    let qualif = a.prefixe_n1.as_ref().map_or(String::new(), |p| format!(" commençant par « {p} »"));
    if let Some(n) = a.titres_n1
        && n1_prefixes != n
    {
        echecs.push(format!("{n1_prefixes} titres de niveau 1{qualif} au lieu de {n}"));
    }
    if let Some(n) = a.n1_min
        && n1_prefixes < n
    {
        echecs.push(format!("{n1_prefixes} titres de niveau 1{qualif}, moins de {n}"));
    }
    if let Some(n) = a.n1_max
        && n1.len() > n
    {
        echecs.push(format!("{} titres de niveau 1, plus de {n}", n1.len()));
    }
    if let Some(n) = a.n2
        && titres(2).len() != n
    {
        echecs.push(format!("{} titres de niveau 2 au lieu de {n}", titres(2).len()));
    }
    for t in &a.titres_contiennent {
        if !blocs.iter().any(|b| matches!(b.genre, Genre::Titre(_)) && b.texte.contains(t.as_str())) {
            echecs.push(format!("aucun titre ne contient « {t} »"));
        }
    }
    for np in &a.notes_pages {
        let ok = notes
            .iter()
            .any(|(b, m)| b.ancre.page == np.page && np.marqueur.as_ref().is_none_or(|x| x == m));
        if !ok {
            echecs.push(format!("aucune note {} ancrée page {}", np.marqueur.as_deref().unwrap_or(""), np.page));
        }
    }
    if let Some(n) = a.notes_min
        && notes.len() < n
    {
        echecs.push(format!("{} notes appariées, moins de {n}", notes.len()));
    }
    for page in &a.sans_notes_pages {
        for (b, m) in notes.iter().filter(|(b, _)| b.ancre.page == *page) {
            echecs.push(format!("note « {m} » ancrée page {page} : {}", extrait(&b.texte)));
        }
    }
    for motif in &a.absent_regex {
        let re = Regex::new(motif).unwrap_or_else(|e| panic!("motif {motif} : {e}"));
        for b in blocs.iter().filter(|b| matches!(b.genre, Genre::Paragraphe) && re.is_match(&b.texte)) {
            echecs.push(format!("paragraphe vérifiant {motif} ({}) : {}", b.ancre, extrait(&b.texte)));
        }
    }
    if a.moities {
        for m in [Moitie::Gauche, Moitie::Droite] {
            if !blocs.iter().any(|b| b.ancre.moitie == m) {
                echecs.push(format!("aucune ancre en {m:?}"));
            }
        }
    }
    if let Some(max) = a.duree_max_s
        && duree > max
    {
        echecs.push(format!("conversion en {duree:.1} s, plus de {max} s"));
    }

    println!(
        "{} : {:.1} s, titres {} (n1 {}, n1{qualif} {}, n2 {}), {} notes, paragraphes {} (médiane {med}, max {max}), {} retraits, {} points",
        entree.fichier,
        duree,
        source_texte(conversion.source_titres),
        n1.len(),
        n1_prefixes,
        titres(2).len(),
        notes.len(),
        longueurs.len(),
        conversion.retraits.len(),
        conversion.points.len(),
    );
    if let Some(d) = d {
        println!("  DÉROGATION : {}", d.raison);
    }
    echecs
}

#[test]
#[ignore]
fn corpus() {
    let corpus = std::env::var("PLUMEO_CORPUS")
        .expect("PLUMEO_CORPUS doit désigner le dossier du corpus (ex. PLUMEO_CORPUS=/home/…/RAG)");
    let manifeste = Path::new(RACINE).join("manifeste.json");
    let texte = std::fs::read_to_string(&manifeste).unwrap_or_else(|e| panic!("{} : {e}", manifeste.display()));
    let entrees: Vec<Entree> = serde_json::from_str(&texte).unwrap_or_else(|e| panic!("manifeste : {e}"));
    let mut bilan = Vec::new();
    for entree in &entrees {
        for echec in verifier(Path::new(&corpus), entree) {
            println!("  ÉCHEC : {echec}");
            bilan.push(format!("{} : {echec}", entree.fichier));
        }
    }
    assert!(bilan.is_empty(), "{} attente(s) non tenue(s) :\n{}", bilan.len(), bilan.join("\n"));
}
