//! Ligne de commande de Pluméo : `convertir` et `page`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand, ValueEnum};
use plumeo_epub::{Metadonnees, generer_epub};
use plumeo_moteur::metadonnees::{langue_epub, nom_sortie, proposer};
use plumeo_moteur::modele::Moitie;
use plumeo_moteur::pdf::{DocumentPdf, nettoyer_chaine};
use plumeo_moteur::reglages::{ReglagesSerie, SourceTitres};
use plumeo_moteur::{apercu_page, convertir_livre};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "plumeo-cli", about = "Convertit un PDF en EPUB")]
struct Cli {
    #[command(subcommand)]
    commande: Commande,
}

#[derive(Subcommand)]
enum Commande {
    /// Convertit un PDF en EPUB (avec Markdown, retraits et points à vérifier).
    Convertir {
        pdf: PathBuf,
        /// Fichier de réglages de la série (JSON).
        #[arg(long)]
        serie: PathBuf,
        /// Dossier de sortie, créé s'il n'existe pas.
        #[arg(long)]
        sortie: PathBuf,
        /// Force la source des titres.
        #[arg(long, value_enum)]
        titres: Option<TitresArg>,
        #[arg(long)]
        titre: Option<String>,
        #[arg(long)]
        auteur: Option<String>,
    },
    /// Affiche le texte du corps et les retraits d'une page.
    Page {
        pdf: PathBuf,
        /// Numéro de page, à partir de 1.
        n: u32,
        #[arg(long)]
        serie: PathBuf,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum TitresArg {
    Auto,
    Signets,
    GrasMajuscules,
    Taille,
}

impl From<TitresArg> for SourceTitres {
    fn from(t: TitresArg) -> Self {
        match t {
            TitresArg::Auto => SourceTitres::Auto,
            TitresArg::Signets => SourceTitres::Signets,
            TitresArg::GrasMajuscules => SourceTitres::GrasMajuscules,
            TitresArg::Taille => SourceTitres::Taille,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let resultat = match cli.commande {
        Commande::Convertir { pdf, serie, sortie, titres, titre, auteur } => {
            convertir(&pdf, &serie, &sortie, titres.map(Into::into), titre, auteur)
        }
        Commande::Page { pdf, n, serie } => page(&pdf, n, &serie),
    };
    match resultat {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erreur : {e}");
            ExitCode::from(1)
        }
    }
}

type Resultat = Result<(), Box<dyn std::error::Error>>;

fn convertir(
    chemin: &Path,
    serie: &Path,
    sortie: &Path,
    forcer: Option<SourceTitres>,
    titre: Option<String>,
    auteur: Option<String>,
) -> Resultat {
    let debut = Instant::now();
    let reglages = ReglagesSerie::lire(serie)?;
    let doc = DocumentPdf::ouvrir(chemin)?;
    let nom_fichier: String = chemin.file_stem().map(|s| s.to_string_lossy().nfc().collect()).unwrap_or_default();
    let (titre_propose, auteur_propose) = proposer(&doc.meta(), &nom_fichier);
    let titre = nettoyer_chaine(&titre.unwrap_or(titre_propose));
    let auteur = nettoyer_chaine(&auteur.unwrap_or(auteur_propose));

    let conversion = convertir_livre(&doc, &reglages, &titre, forcer, &mut |p, total| {
        eprint!("\rpage {p}/{total}");
    })?;
    eprintln!();

    let couverture = doc.rendre_png(1, 1200)?;
    let meta = Metadonnees {
        titre: titre.clone(),
        auteur: auteur.clone(),
        langue: langue_epub(&reglages.langue),
        identifiant: format!("urn:uuid:{}", Uuid::new_v5(&Uuid::NAMESPACE_URL, nom_fichier.as_bytes())),
        modifie: maintenant_utc(),
    };

    std::fs::create_dir_all(sortie)?;
    let base = nom_sortie(&titre, &auteur);
    let fichier = |ext: &str| sortie.join(format!("{base}.{ext}"));
    if fichier("epub").exists() {
        eprintln!("attention : {base}.epub existe déjà : remplacé");
    }
    generer_epub(&conversion.markdown, &meta, Some(&couverture), &fichier("epub"))?;
    std::fs::write(fichier("md"), &conversion.markdown)?;
    std::fs::write(fichier("retraits.json"), serde_json::to_string_pretty(&conversion.retraits)?)?;
    std::fs::write(fichier("points.json"), serde_json::to_string_pretty(&conversion.points)?)?;

    let nb_titres = conversion.markdown.lines().filter(|l| l.starts_with("# ")).count();
    let source = serde_json::to_value(conversion.source_titres)?;
    println!("{titre} - {auteur}");
    println!("  {} pages", conversion.nb_pages);
    println!("  titres : {} ({nb_titres} de niveau 1)", source.as_str().unwrap_or("?"));
    println!("  {} notes", conversion.nb_notes);
    println!("  {} retraits", conversion.retraits.len());
    println!("  {} points à vérifier", conversion.points.len());
    println!("  durée : {:.1} s", debut.elapsed().as_secs_f32());
    println!("  sortie : {}", fichier("epub").display());
    Ok(())
}

fn page(chemin: &Path, n: u32, serie: &Path) -> Resultat {
    let reglages = ReglagesSerie::lire(serie)?;
    let doc = DocumentPdf::ouvrir(chemin)?;
    let apercu = apercu_page(&doc, n, &reglages)?;
    for (moitie, texte, retraits) in &apercu.moities {
        let suffixe = match moitie {
            Moitie::Entiere => "",
            Moitie::Gauche => " (gauche)",
            Moitie::Droite => " (droite)",
        };
        println!("=== page {n}{suffixe} ===");
        println!("{texte}");
        println!("--- retraits ---");
        for r in retraits {
            let role = serde_json::to_value(r.role)?;
            println!("[{}] {}", role.as_str().unwrap_or("?"), r.texte);
        }
    }
    Ok(())
}

/// Heure UTC courante, `%Y-%m-%dT%H:%M:%SZ`, sans dépendance de date.
fn maintenant_utc() -> String {
    let secondes = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    formater_utc(secondes)
}

fn formater_utc(secondes: u64) -> String {
    let (jours, reste) = ((secondes / 86_400) as i64, secondes % 86_400);
    // Jours depuis 1970 vers date civile (algorithme de H. Hinnant).
    let z = jours + 719_468;
    let ere = z.div_euclid(146_097);
    let jde = z.rem_euclid(146_097);
    let ae = (jde - jde / 1_460 + jde / 36_524 - jde / 146_096) / 365;
    let jda = jde - (365 * ae + ae / 4 - ae / 100);
    let mp = (5 * jda + 2) / 153;
    let jour = jda - (153 * mp + 2) / 5 + 1;
    let mois = if mp < 10 { mp + 3 } else { mp - 9 };
    let annee = ae + ere * 400 + i64::from(mois <= 2);
    format!("{annee:04}-{mois:02}-{jour:02}T{:02}:{:02}:{:02}Z", reste / 3600, reste % 3600 / 60, reste % 60)
}

#[cfg(test)]
mod tests {
    use super::formater_utc;

    #[test]
    fn date_utc() {
        assert_eq!(formater_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(formater_utc(1_783_641_600 + 3_661), "2026-07-10T01:01:01Z");
        assert_eq!(formater_utc(951_782_400), "2000-02-29T00:00:00Z");
    }
}
