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

use plumeo_commun::{est_titre_de_sommaire, normaliser};
use plumeo_epub::{Metadonnees, generer_epub};
use plumeo_moteur::convertir_livre;
use plumeo_moteur::metadonnees::proposer;
use plumeo_moteur::modele::{Ancre, Moitie};
use plumeo_moteur::pdf::{DocumentPdf, nettoyer_chaine};
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
    /// Page minimale de tout titre de niveau 1 (titres du corps, pas d'un résumé).
    n1_page_min: Option<u32>,
    /// Nombre exact de titres de niveau 2.
    n2: Option<usize>,
    /// Nombre exact d'entrées de sommaire imprimé rendues cliquables.
    entrees_sommaire: Option<usize>,
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
    /// `true` : l'EPUB doit avoir un chapitre « Table des matières » ou « Sommaire » à
    /// liens, et c'est `chap-002.xhtml`. `false` (par défaut, champ absent) : un tel
    /// chapitre n'est contrôlé (position 2) que s'il existe.
    #[serde(default)]
    sommaire_en_tete: bool,
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
    /// Saut de section `* * *` : ni paragraphe ni titre, hors des statistiques.
    Saut,
    /// Entrée de sommaire imprimé `[texte](#t-…)` : hors des statistiques de paragraphes.
    Entree,
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
    let entree = Regex::new(r"^\[(.*)\]\(#[^()\s]*\)$").unwrap();
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
        } else if ligne == "* * *" {
            (Genre::Saut, String::new())
        } else if let Some(c) = entree.captures(ligne) {
            (Genre::Entree, texte_brut(&c[1]))
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

/// Titre du livre proposé comme par la CLI : métadonnées du PDF et nom du fichier.
fn titre_propose(doc: &DocumentPdf, chemin: &Path) -> String {
    let nom: String = chemin.file_stem().map(|s| s.to_string_lossy().nfc().collect()).unwrap_or_default();
    nettoyer_chaine(&proposer(&doc.meta(), &nom).0)
}

/// Vérifie que le premier bloc du Markdown est un titre `#` égal (en texte normalisé)
/// au titre du livre ; renvoie la raison de l'échec sinon.
fn verifier_titre_livre(blocs: &[BlocMd], titre_livre: &str) -> Result<(), String> {
    let voulu = normaliser(titre_livre);
    match blocs.first() {
        None => Err("Markdown sans titre du livre".into()),
        Some(b) if !matches!(b.genre, Genre::Titre(1)) => {
            Err(format!("le Markdown ne commence pas par un titre # : « {} » ({})", extrait(&b.texte), b.ancre))
        }
        Some(b) if normaliser(&b.texte) != voulu => {
            Err(format!("premier titre # « {} » différent du titre du livre « {titre_livre} »", b.texte))
        }
        Some(_) => Ok(()),
    }
}

fn meta_corpus(nom: &str) -> Metadonnees {
    Metadonnees {
        titre: nom.to_string(),
        auteur: "Corpus".into(),
        langue: "fr".into(),
        identifiant: "urn:uuid:00000000-0000-5000-8000-000000000000".into(),
        modifie: "2026-11-10T00:00:00Z".into(),
    }
}

/// Chapitres de l'EPUB (`OEBPS/chap-….xhtml`) : nom du fichier sans dossier et contenu,
/// dans l'ordre de l'archive. La navigation (`nav.xhtml`) n'en fait pas partie.
fn chapitres_epub(epub: &Path) -> Result<Vec<(String, String)>, String> {
    use std::io::Read;
    let fichier = std::fs::File::open(epub).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(fichier).map_err(|e| e.to_string())?;
    let mut chapitres = Vec::new();
    for i in 0..zip.len() {
        let mut membre = zip.by_index(i).map_err(|e| e.to_string())?;
        let nom = membre.name().map_err(|e| e.to_string())?.to_string();
        let Some(court) = nom.strip_prefix("OEBPS/chap-").filter(|_| nom.ends_with(".xhtml")) else {
            continue;
        };
        let mut texte = String::new();
        membre.read_to_string(&mut texte).map_err(|e| format!("{nom} : {e}"))?;
        chapitres.push((format!("chap-{court}"), texte));
    }
    Ok(chapitres)
}

/// Motif d'un lien d'entrée de sommaire dans un chapitre.
const LIEN_SOMMAIRE: &str = r#"href="chap-\d+\.xhtml#t-"#;

/// Nombre de liens d'entrée de sommaire (`href="chap-…#t-…"`) dans les chapitres de
/// l'EPUB ; la navigation (`nav.xhtml`) n'est pas comptée. Une entrée dont le titre
/// visé manque est rendue en texte, sans lien : elle n'est pas comptée.
fn liens_de_sommaire(epub: &Path) -> Result<usize, String> {
    Ok(compter_liens(&chapitres_epub(epub)?))
}

/// Nombre de liens d'entrée de sommaire dans les `chapitres` (voir `liens_de_sommaire`).
fn compter_liens(chapitres: &[(String, String)]) -> usize {
    let lien = Regex::new(LIEN_SOMMAIRE).unwrap();
    chapitres.iter().map(|(_, texte)| lien.find_iter(texte).count()).sum()
}

/// Vérifie que le chapitre « Table des matières » ou « Sommaire » (titre H1, à la casse,
/// aux accents et à la ponctuation près : `plumeo_commun::est_titre_de_sommaire`) qui
/// contient des liens internes est `chap-002.xhtml`, juste après la page de titre. Si
/// `attendu`, l'EPUB doit en plus avoir un tel chapitre : le contrôle ne passe pas à vide.
/// Sinon, sans chapitre de ce genre, rien à vérifier.
fn verifier_sommaire_en_tete(chapitres: &[(String, String)], attendu: bool) -> Result<(), String> {
    let h1 = Regex::new(r"(?s)<h1[^>]*>(.*?)</h1>").unwrap();
    let balises = Regex::new(r"<[^>]*>").unwrap();
    let lien = Regex::new(LIEN_SOMMAIRE).unwrap();
    let mut trouve = false;
    for (nom, texte) in chapitres {
        let Some(c) = h1.captures(texte) else { continue };
        if est_titre_de_sommaire(&balises.replace_all(&c[1], "")) && lien.is_match(texte) {
            trouve = true;
            if nom != "chap-002.xhtml" {
                return Err(format!(
                    "le chapitre « {} » à liens de sommaire est {nom} au lieu de chap-002.xhtml (juste après la page de titre)",
                    balises.replace_all(&c[1], "").trim()
                ));
            }
        }
    }
    if attendu && !trouve {
        return Err("sommaire attendu : aucun chapitre « Table des matières » ou « Sommaire » à liens dans l'EPUB \
                    (il devrait être chap-002.xhtml)"
            .into());
    }
    Ok(())
}

/// Mesures d'un EPUB généré : sa structure, puis sa validation par epubcheck si le jar
/// est donné.
struct ControleEpub {
    /// Nombre de liens d'entrée de sommaire (`compter_liens`), ou l'erreur de lecture de
    /// l'archive.
    liens: Result<usize, String>,
    /// Résultat de `verifier_sommaire_en_tete` (`Ok` si l'archive n'a pu être lue : son
    /// erreur est déjà dans `liens`).
    sommaire_en_tete: Result<(), String>,
    /// Résultat d'epubcheck (`Ok` sans jar), ou l'échec de l'appel à java.
    epubcheck: Result<(), String>,
    /// java a vraiment été appelé avec le jar d'epubcheck.
    epubcheck_lance: bool,
}

impl ControleEpub {
    /// Échecs cumulés : ceux de la structure d'abord, puis celui d'epubcheck.
    fn echecs(&self) -> Vec<String> {
        [self.liens.as_ref().err(), self.sommaire_en_tete.as_ref().err(), self.epubcheck.as_ref().err()]
            .into_iter()
            .flatten()
            .cloned()
            .collect()
    }
}

/// Lance epubcheck (`java -jar jar`) sur l'EPUB `chemin` : (java appelé, résultat). Les
/// lignes `ERROR` et `FATAL` (5 au plus) font le message d'échec, à défaut les premières
/// lignes de la sortie (jar introuvable…).
fn lancer_epubcheck(jar: &str, chemin: &Path) -> (bool, Result<(), String>) {
    let sortie = match std::process::Command::new("java").args(["-jar", jar]).arg(chemin).output() {
        Ok(sortie) => sortie,
        Err(e) => return (false, Err(format!("epubcheck : java : {e}"))),
    };
    if sortie.status.success() {
        return (true, Ok(()));
    }
    let texte = String::from_utf8_lossy(&sortie.stdout).to_string() + &String::from_utf8_lossy(&sortie.stderr);
    let mut erreurs: Vec<&str> =
        texte.lines().filter(|l| l.starts_with("ERROR") || l.starts_with("FATAL")).take(5).collect();
    if erreurs.is_empty() {
        erreurs = texte.lines().map(str::trim).filter(|l| !l.is_empty()).take(5).collect();
    }
    if erreurs.is_empty() {
        erreurs.push("échec sans message");
    }
    (true, Err(format!("epubcheck : {}", erreurs.join(" | "))))
}

/// Génère l'EPUB dans un dossier temporaire, mesure sa structure, puis le valide avec
/// epubcheck si `jar` est donné (c'est la seule étape qui en dépend). Seul l'échec de
/// `generer_epub` est une erreur : les autres sont dans le `ControleEpub`.
fn controler_epub(jar: Option<&str>, markdown: &str, nom: &str, sommaire_attendu: bool) -> Result<ControleEpub, String> {
    let dossier = tempfile::tempdir().map_err(|e| e.to_string())?;
    let chemin = dossier.path().join("livre.epub");
    generer_epub(markdown, &meta_corpus(nom), None, &chemin).map_err(|e| format!("generer_epub : {e}"))?;
    let (liens, sommaire_en_tete) = match chapitres_epub(&chemin) {
        Ok(chapitres) => (Ok(compter_liens(&chapitres)), verifier_sommaire_en_tete(&chapitres, sommaire_attendu)),
        Err(e) => (Err(format!("lecture de l'EPUB : {e}")), Ok(())),
    };
    let (epubcheck_lance, epubcheck) = jar.map_or((false, Ok(())), |jar| lancer_epubcheck(jar, &chemin));
    Ok(ControleEpub { liens, sommaire_en_tete, epubcheck, epubcheck_lance })
}

/// Libellé du résumé : « epubcheck lancé » seulement si java a vraiment été appelé.
fn libelle_epubcheck(controle: &Result<ControleEpub, String>) -> &'static str {
    match controle {
        Ok(c) if c.epubcheck_lance => "epubcheck lancé",
        _ => "epubcheck non lancé",
    }
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
    let mut titre_livre = String::new();
    let debut = Instant::now();
    let conversion = DocumentPdf::ouvrir(&chemin).and_then(|doc| {
        titre_livre = titre_propose(&doc, &chemin);
        convertir_livre(&doc, &reglages, &titre_livre, forcer, &mut |_, _| {})
    });
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
    if let Err(e) = verifier_titre_livre(&blocs, &titre_livre) {
        echecs.push(e);
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
    // EPUB toujours généré, structure toujours contrôlée avant epubcheck, qui seul dépend de
    // EPUBCHECK_JAR ; leurs échecs sont cumulés. Les liens d'entrée de sommaire sont comparés
    // plus bas aux entrées du Markdown, même si epubcheck a échoué.
    let jar = std::env::var("EPUBCHECK_JAR").ok();
    let controle = controler_epub(jar.as_deref(), &conversion.markdown, &entree.fichier, a.sommaire_en_tete);
    let liens_epub = match &controle {
        Ok(c) => {
            echecs.extend(c.echecs());
            c.liens.as_ref().ok().copied()
        }
        Err(e) => {
            echecs.push(e.clone());
            None
        }
    };

    // Attentes propres au livre.
    let titres = |niveau: usize| -> Vec<&BlocMd> {
        blocs.iter().filter(|b| matches!(b.genre, Genre::Titre(n) if n == niveau)).collect()
    };
    // Le titre du livre placé en tête est un titre de service : hors des décomptes
    // de niveau 1 et de `n1_page_min`. De même pour le titre « Table des matières » ou
    // « Sommaire » d'un sommaire imprimé rendu cliquable (suivi d'une entrée).
    let titre_norm = normaliser(&titre_livre);
    let mut n1 = titres(1);
    if n1.first().is_some_and(|b| !titre_norm.is_empty() && normaliser(&b.texte) == titre_norm) {
        n1.remove(0);
    }
    let titres_de_sommaire: Vec<&BlocMd> = blocs
        .windows(2)
        .filter(|w| {
            matches!(w[1].genre, Genre::Entree)
                && est_titre_de_sommaire(&w[0].texte)
        })
        .map(|w| &w[0])
        .collect();
    n1.retain(|b| !titres_de_sommaire.iter().any(|s| std::ptr::eq(*s, *b)));
    let entrees = blocs.iter().filter(|b| matches!(b.genre, Genre::Entree)).count();
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
    if let Some(min) = a.n1_page_min {
        for b in n1.iter().filter(|b| b.ancre.page < min) {
            echecs.push(format!("titre de niveau 1 avant la page {min} : « {} » ({})", b.texte, b.ancre));
        }
    }
    if let Some(n) = a.n2
        && titres(2).len() != n
    {
        echecs.push(format!("{} titres de niveau 2 au lieu de {n}", titres(2).len()));
    }
    if let Some(n) = a.entrees_sommaire
        && entrees != n
    {
        echecs.push(format!("{entrees} entrées de sommaire au lieu de {n}"));
    }
    // Une entrée dont le titre visé manque dans l'EPUB y devient du texte, sans avertissement.
    if let Some(liens) = liens_epub
        && liens != entrees
    {
        echecs.push(format!("{liens} liens de sommaire dans l'EPUB pour {entrees} entrées du Markdown"));
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
        "{} : {:.1} s, titres {} (n1 {}, n1{qualif} {}, n2 {}), {entrees} entrées de sommaire, {} notes, paragraphes {} (médiane {med}, max {max}), {} retraits, {} points, {}",
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
        libelle_epubcheck(&controle),
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

#[cfg(test)]
mod tests {
    use super::*;

    const ANCRE: &str = "<!-- @p1 0,0,100,100 -->";

    #[test]
    fn titre_livre_en_tete_accepte() {
        let md = format!("{ANCRE}\n# Approche de la Cité céleste\n\n{ANCRE}\nUn paragraphe.\n");
        assert_eq!(verifier_titre_livre(&lire_blocs(&md), "Approche de la cité Céleste"), Ok(()));
    }

    #[test]
    fn titre_livre_absent_refuse() {
        let md = format!("{ANCRE}\nUn paragraphe.\n\n{ANCRE}\n# Approche de la Cité céleste\n");
        assert!(verifier_titre_livre(&lire_blocs(&md), "Approche de la Cité céleste").is_err());
        assert!(verifier_titre_livre(&[], "Approche de la Cité céleste").is_err());
    }

    #[test]
    fn liens_de_sommaire_comptes_dans_l_epub() {
        // Une entrée sans titre cible est rendue en texte : elle ne compte pas.
        let md = format!(
            "{ANCRE}\n# Table\n\n{ANCRE}\n[Deux](#t-deux)\n\n{ANCRE}\n[Absent](#t-absent)\n\n{ANCRE}\n# Deux\n\n{ANCRE}\nTexte.\n"
        );
        let dossier = tempfile::tempdir().unwrap();
        let chemin = dossier.path().join("livre.epub");
        generer_epub(&md, &meta_corpus("Livre"), None, &chemin).unwrap();
        assert_eq!(liens_de_sommaire(&chemin), Ok(1));
    }

    fn chapitre(nom: &str, titre: &str, corps: &str) -> (String, String) {
        (nom.to_string(), format!("<body>\n<h1 id=\"t-x\">{titre}</h1>\n{corps}\n</body>"))
    }

    const LIEN: &str = r#"<p><a href="chap-003.xhtml#t-un">Un</a></p>"#;

    #[test]
    fn sommaire_en_tete_accepte_en_position_2_ou_absent() {
        let en_2 = [chapitre("chap-001.xhtml", "Livre", ""), chapitre("chap-002.xhtml", "Table des matières", LIEN)];
        assert_eq!(verifier_sommaire_en_tete(&en_2, false), Ok(()));
        assert_eq!(verifier_sommaire_en_tete(&en_2, true), Ok(()));
        // Sans lien interne, ou sans chapitre de sommaire : rien à vérifier tant que le
        // livre n'attend pas de sommaire.
        let sans_lien = [chapitre("chap-001.xhtml", "Livre", ""), chapitre("chap-004.xhtml", "SOMMAIRE", "<p>Un 7</p>")];
        assert_eq!(verifier_sommaire_en_tete(&sans_lien, false), Ok(()));
        let autre = [chapitre("chap-005.xhtml", "Un chapitre", LIEN)];
        assert_eq!(verifier_sommaire_en_tete(&autre, false), Ok(()));
    }

    #[test]
    fn sommaire_attendu_mais_absent_refuse() {
        // Le contrôle ne passe plus à vide : un livre qui attend un sommaire à liens
        // échoue s'il n'a aucun chapitre de ce genre.
        let sans_chapitre = [chapitre("chap-001.xhtml", "Livre", ""), chapitre("chap-002.xhtml", "Un chapitre", LIEN)];
        // « attendu » est propre à l'erreur d'absence (pas à une erreur de position).
        let e = verifier_sommaire_en_tete(&sans_chapitre, true).unwrap_err();
        assert!(e.contains("attendu") && e.contains("chap-002.xhtml"), "{e}");
        // Un chapitre « Sommaire » sans lien interne n'est pas un sommaire cliquable.
        let sans_lien = [chapitre("chap-001.xhtml", "Livre", ""), chapitre("chap-002.xhtml", "SOMMAIRE", "<p>Un 7</p>")];
        let e = verifier_sommaire_en_tete(&sans_lien, true).unwrap_err();
        assert!(e.contains("attendu"), "{e}");
        let e = verifier_sommaire_en_tete(&[], true).unwrap_err();
        assert!(e.contains("attendu"), "{e}");
        // Sans attente, ces mêmes livres passent.
        assert_eq!(verifier_sommaire_en_tete(&sans_chapitre, false), Ok(()));
    }

    #[test]
    fn sommaire_en_tete_refuse_ailleurs_qu_en_position_2() {
        for titre in ["Table des matières", "TABLE DES MATIERES", "Sommaire", "<em>SOMMAIRE</em>"] {
            let tard = [chapitre("chap-001.xhtml", "Livre", ""), chapitre("chap-004.xhtml", titre, LIEN)];
            for attendu in [false, true] {
                let e = verifier_sommaire_en_tete(&tard, attendu).unwrap_err();
                assert!(e.contains("chap-004.xhtml") && e.contains("chap-002.xhtml"), "{e}");
                assert!(!e.contains("attendu"), "{e}");
            }
        }
    }

    #[test]
    fn sommaire_imprime_a_la_fin_remonte_en_position_2() {
        let md = format!(
            "{ANCRE}\n# Livre\n\n{ANCRE}\n# Un\n\n{ANCRE}\nTexte.\n\n{ANCRE}\n# Table des matières\n\n{ANCRE}\n[Un](#t-un)\n"
        );
        let dossier = tempfile::tempdir().unwrap();
        let chemin = dossier.path().join("livre.epub");
        generer_epub(&md, &meta_corpus("Livre"), None, &chemin).unwrap();
        let chapitres = chapitres_epub(&chemin).unwrap();
        // Le chapitre du sommaire est bien trouvé, en chap-002 : sans lui, le contrôle
        // passerait à vide.
        let (_, chap_2) = chapitres.iter().find(|(nom, _)| nom == "chap-002.xhtml").expect("chap-002.xhtml");
        assert!(chap_2.contains("Table des matières</h1>") && chap_2.contains("#t-un\""), "{chap_2}");
        assert_eq!(verifier_sommaire_en_tete(&chapitres, true), Ok(()));
    }

    #[test]
    fn controle_epub_sans_epubcheck_verifie_quand_meme_la_structure() {
        let md = format!(
            "{ANCRE}\n# Livre\n\n{ANCRE}\n# Un\n\n{ANCRE}\nTexte.\n\n{ANCRE}\n# Table des matières\n\n{ANCRE}\n[Un](#t-un)\n"
        );
        let controle = controler_epub(None, &md, "Livre", true).unwrap();
        assert!(!controle.epubcheck_lance);
        assert_eq!(controle.liens, Ok(1));
        assert_eq!(controle.sommaire_en_tete, Ok(()));
        assert_eq!(controle.epubcheck, Ok(()));
        assert!(controle.echecs().is_empty());
        // Sans chapitre de sommaire, l'attente échoue aussi sans epubcheck.
        let sans = format!("{ANCRE}\n# Livre\n\n{ANCRE}\n# Un\n\n{ANCRE}\nTexte.\n");
        let controle = controler_epub(None, &sans, "Livre", true).unwrap();
        assert!(controle.sommaire_en_tete.is_err());
        assert_eq!(controler_epub(None, &sans, "Livre", false).unwrap().sommaire_en_tete, Ok(()));
    }

    #[test]
    fn controle_epub_cumule_structure_et_epubcheck() {
        // epubcheck échoue (jar introuvable, ou java absent) : la structure est mesurée
        // quand même, et les deux échecs sont rapportés, structure d'abord.
        let sans = format!("{ANCRE}\n# Livre\n\n{ANCRE}\n# Un\n\n{ANCRE}\nTexte.\n");
        let controle = controler_epub(Some("/chemin/absent/epubcheck.jar"), &sans, "Livre", true).unwrap();
        assert_eq!(controle.liens, Ok(0));
        assert!(controle.epubcheck.is_err());
        let echecs = controle.echecs();
        assert_eq!(echecs.len(), 2, "{echecs:?}");
        assert!(echecs[0].contains("attendu"), "{echecs:?}");
        assert!(echecs[1].starts_with("epubcheck : ") && echecs[1].len() > "epubcheck : ".len(), "{echecs:?}");
    }

    #[test]
    fn epubcheck_dit_lance_seulement_si_java_a_ete_appele() {
        // `generer_epub` en échec : epubcheck n'a pas été lancé, même avec un jar.
        assert_eq!(libelle_epubcheck(&Err("generer_epub : disque plein".into())), "epubcheck non lancé");
        let controle = |epubcheck_lance| ControleEpub {
            liens: Ok(0),
            sommaire_en_tete: Ok(()),
            epubcheck: Ok(()),
            epubcheck_lance,
        };
        assert_eq!(libelle_epubcheck(&Ok(controle(false))), "epubcheck non lancé");
        assert_eq!(libelle_epubcheck(&Ok(controle(true))), "epubcheck lancé");
    }

    #[test]
    fn titre_livre_different_ou_de_mauvais_niveau_refuse() {
        let autre = format!("{ANCRE}\n# Autre titre\n");
        assert!(verifier_titre_livre(&lire_blocs(&autre), "Approche de la Cité céleste").is_err());
        let niveau2 = format!("{ANCRE}\n## Approche de la Cité céleste\n");
        assert!(verifier_titre_livre(&lire_blocs(&niveau2), "Approche de la Cité céleste").is_err());
    }
}
