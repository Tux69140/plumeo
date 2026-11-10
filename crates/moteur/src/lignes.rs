//! Regroupement des mots en lignes, avec détection des exposants (appels de note).

use crate::modele::{Mot, Rect};

#[derive(Clone, Debug)]
pub struct Ligne {
    pub mots: Vec<Mot>,
    /// Indices dans `mots` des mots détectés comme exposants.
    pub exposants: Vec<usize>,
    pub rect: Rect,
}

impl Ligne {
    pub fn texte(&self) -> String {
        let mut sortie = String::new();
        for (i, mot) in self.mots.iter().enumerate() {
            if i > 0 && mot.espace_avant && !self.exposants.contains(&i) {
                sortie.push(' ');
            }
            sortie.push_str(&mot.texte);
        }
        sortie
    }

    /// Taille médiane des mots, exposants exclus.
    pub fn taille(&self) -> f32 {
        let tailles: Vec<f32> = self
            .mots
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.exposants.contains(i))
            .map(|(_, m)| m.taille)
            .collect();
        if tailles.is_empty() {
            // Ligne faite uniquement d'exposants : on retombe sur tous les mots.
            return mediane(self.mots.iter().map(|m| m.taille).collect());
        }
        mediane(tailles)
    }

    pub fn tout_gras(&self) -> bool {
        !self.mots.is_empty() && self.mots.iter().all(|m| m.gras)
    }
}

fn mediane(mut valeurs: Vec<f32>) -> f32 {
    if valeurs.is_empty() {
        return 0.0;
    }
    valeurs.sort_by(|a, b| a.total_cmp(b));
    let n = valeurs.len();
    if n % 2 == 1 {
        valeurs[n / 2]
    } else {
        (valeurs[n / 2 - 1] + valeurs[n / 2]) / 2.0
    }
}

fn centre_y(m: &Mot) -> f32 {
    (m.rect.y0 + m.rect.y1) / 2.0
}

fn hauteur(m: &Mot) -> f32 {
    m.rect.y1 - m.rect.y0
}

/// Regroupe les mots en lignes, triées de haut en bas.
///
/// Les mots sont traités du plus haut au plus bas (hauteur de rectangle) : le
/// premier mot d'une ligne en est donc le plus grand, et son rectangle vertical
/// sert de référence. Un mot rejoint la ligne dont la référence contient son
/// centre vertical ; un mot nettement plus petit (exposant, ponctuation) peut
/// aussi la rejoindre par simple chevauchement, la ligne au plus grand
/// chevauchement l'emportant.
pub fn regrouper_en_lignes(mots: &[Mot]) -> Vec<Ligne> {
    let mut ordre: Vec<&Mot> = mots.iter().collect();
    ordre.sort_by(|a, b| hauteur(b).total_cmp(&hauteur(a)));

    // (référence y0, référence y1, mots)
    let mut groupes: Vec<(f32, f32, Vec<Mot>)> = Vec::new();
    for mot in ordre {
        let c = centre_y(mot);
        let mut meilleur: Option<(usize, f32)> = None;
        for (i, (y0, y1, _)) in groupes.iter().enumerate() {
            let chevauchement = mot.rect.y1.min(*y1) - mot.rect.y0.max(*y0);
            let contient = c >= *y0 && c <= *y1;
            let petit = hauteur(mot) < 0.8 * (y1 - y0);
            if (contient || (petit && chevauchement > 0.0))
                && meilleur.is_none_or(|(_, m)| chevauchement > m)
            {
                meilleur = Some((i, chevauchement));
            }
        }
        match meilleur {
            Some((i, _)) => groupes[i].2.push(mot.clone()),
            None => groupes.push((mot.rect.y0, mot.rect.y1, vec![mot.clone()])),
        }
    }
    groupes.sort_by(|a, b| (a.0 + a.1).total_cmp(&(b.0 + b.1)));
    groupes.into_iter().map(|(_, _, m)| construire_ligne(m)).collect()
}

fn construire_ligne(mut mots: Vec<Mot>) -> Ligne {
    mots.sort_by(|a, b| a.rect.x0.total_cmp(&b.rect.x0));
    let exposants = detecter_exposants(&mots);
    let rect = Rect {
        x0: mots.iter().map(|m| m.rect.x0).fold(f32::MAX, f32::min),
        y0: mots.iter().map(|m| m.rect.y0).fold(f32::MAX, f32::min),
        x1: mots.iter().map(|m| m.rect.x1).fold(f32::MIN, f32::max),
        y1: mots.iter().map(|m| m.rect.y1).fold(f32::MIN, f32::max),
    };
    Ligne { mots, exposants, rect }
}

/// Un exposant est petit (<= 0,8 x taille médiane) et son bas est plus haut que
/// le bas médian de la ligne d'au moins 25 % de la hauteur de ligne.
fn detecter_exposants(mots: &[Mot]) -> Vec<usize> {
    let seuil = 0.8 * mediane(mots.iter().map(|m| m.taille).collect());
    let petit = |m: &Mot| m.taille <= seuil;
    let normaux: Vec<&Mot> = mots.iter().filter(|m| !petit(m)).collect();
    if normaux.is_empty() {
        return Vec::new();
    }
    // Hauteur de ligne : mot non exposant le plus haut. Bas médian : mesuré sur
    // les mots de taille normale, pour que les exposants ne le tirent pas vers le haut.
    let hauteur_ligne = normaux.iter().map(|m| hauteur(m)).fold(0.0, f32::max);
    let bas_median = mediane(normaux.iter().map(|m| m.rect.y1).collect());
    mots.iter()
        .enumerate()
        .filter(|(_, m)| petit(m) && bas_median - m.rect.y1 >= 0.25 * hauteur_ligne)
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mot(texte: &str, x0: f32, y0: f32, x1: f32, y1: f32, taille: f32) -> Mot {
        Mot {
            texte: texte.to_string(),
            rect: Rect { x0, y0, x1, y1 },
            taille,
            gras: false,
            italique: false,
            espace_avant: true,
        }
    }

    #[test]
    fn deux_lignes_separees() {
        let mots = vec![
            mot("a", 0.10, 0.10, 0.12, 0.12, 12.0),
            mot("b", 0.13, 0.10, 0.15, 0.12, 12.0),
            mot("c", 0.16, 0.10, 0.18, 0.12, 12.0),
            mot("d", 0.10, 0.13, 0.12, 0.15, 12.0),
            mot("e", 0.13, 0.13, 0.15, 0.15, 12.0),
        ];
        let lignes = regrouper_en_lignes(&mots);
        assert_eq!(lignes.len(), 2);
        assert_eq!(lignes[0].texte(), "a b c");
        assert_eq!(lignes[1].texte(), "d e");
    }

    #[test]
    fn ordre_horizontal() {
        let mots = vec![
            mot("c", 0.16, 0.10, 0.18, 0.12, 12.0),
            mot("a", 0.10, 0.10, 0.12, 0.12, 12.0),
            mot("b", 0.13, 0.10, 0.15, 0.12, 12.0),
        ];
        let lignes = regrouper_en_lignes(&mots);
        assert_eq!(lignes.len(), 1);
        assert_eq!(lignes[0].texte(), "a b c");
    }

    #[test]
    fn exposant_detecte() {
        let mots = vec![
            mot("texte", 0.10, 0.100, 0.20, 0.120, 12.0),
            mot("3", 0.20, 0.100, 0.21, 0.114, 7.0),
        ];
        let lignes = regrouper_en_lignes(&mots);
        assert_eq!(lignes.len(), 1);
        assert_eq!(lignes[0].exposants, vec![1]);
        assert_eq!(lignes[0].texte(), "texte3");
        assert_eq!(lignes[0].taille(), 12.0);
    }

    #[test]
    fn petit_texte_sur_la_ligne_de_base_n_est_pas_exposant() {
        let mots = vec![
            mot("texte", 0.10, 0.100, 0.20, 0.120, 12.0),
            mot("3", 0.20, 0.108, 0.21, 0.120, 7.0),
        ];
        let lignes = regrouper_en_lignes(&mots);
        assert!(lignes[0].exposants.is_empty());
    }

    #[test]
    fn exposant_dont_le_centre_depasse_le_haut_du_corps_reste_sur_sa_ligne() {
        let mots = vec![
            mot("texte", 0.10, 0.100, 0.20, 0.120, 12.0),
            mot("3", 0.20, 0.090, 0.21, 0.105, 7.0),
        ];
        let lignes = regrouper_en_lignes(&mots);
        assert_eq!(lignes.len(), 1);
        assert_eq!(lignes[0].exposants, vec![1]);
        assert_eq!(lignes[0].texte(), "texte3");
    }

    #[test]
    fn exposant_de_la_ligne_2_ne_s_attache_pas_a_la_ligne_1() {
        let mots = vec![
            mot("un", 0.10, 0.100, 0.20, 0.120, 12.0),
            mot("deux", 0.10, 0.122, 0.20, 0.142, 12.0),
            // chevauche un peu la ligne 1 mais surtout la ligne 2
            mot("7", 0.20, 0.118, 0.21, 0.132, 7.0),
        ];
        let lignes = regrouper_en_lignes(&mots);
        assert_eq!(lignes.len(), 2);
        assert_eq!(lignes[0].texte(), "un");
        assert_eq!(lignes[1].texte(), "deux7");
        assert_eq!(lignes[1].exposants, vec![1]);
    }
}
