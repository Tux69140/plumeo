//! Découpe d'une page PDF qui contient deux pages de livre côte à côte.

use crate::modele::{Moitie, Mot, Page, Rect};

/// Coupe au pli (fraction de largeur) ; un mot va du côté de son centre.
pub fn couper_double_page(page: Page, pli: f32) -> [Page; 2] {
    let mut gauche = Vec::new();
    let mut droite = Vec::new();
    for mot in page.mots {
        let centre = (mot.rect.x0 + mot.rect.x1) / 2.0;
        if centre < pli {
            gauche.push(renormaliser(mot, 0.0, pli));
        } else {
            droite.push(renormaliser(mot, pli, 1.0 - pli));
        }
    }
    let moitie = |moitie, mots| Page {
        numero: page.numero,
        moitie,
        largeur_pt: page.largeur_pt / 2.0,
        hauteur_pt: page.hauteur_pt,
        mots,
    };
    [moitie(Moitie::Gauche, gauche), moitie(Moitie::Droite, droite)]
}

fn renormaliser(mut mot: Mot, debut: f32, largeur: f32) -> Mot {
    let x = |v: f32| ((v - debut) / largeur).clamp(0.0, 1.0);
    mot.rect = Rect { x0: x(mot.rect.x0), x1: x(mot.rect.x1), ..mot.rect };
    mot
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mot(texte: &str, x0: f32, x1: f32) -> Mot {
        Mot {
            texte: texte.into(),
            rect: Rect { x0, y0: 0.4, x1, y1: 0.42 },
            taille: 10.0,
            gras: false,
            italique: false,
            espace_avant: true,
        }
    }

    #[test]
    fn coupe_au_pli() {
        let page = Page {
            numero: 6,
            moitie: Moitie::Entiere,
            largeur_pt: 842.0,
            hauteur_pt: 595.0,
            mots: vec![mot("a", 0.2, 0.3), mot("b", 0.7, 0.8)],
        };
        let [g, d] = couper_double_page(page, 0.5);
        assert_eq!((g.moitie, d.moitie), (Moitie::Gauche, Moitie::Droite));
        assert_eq!((g.numero, d.numero), (6, 6));
        assert_eq!((g.largeur_pt, d.largeur_pt), (421.0, 421.0));
        assert_eq!(g.hauteur_pt, 595.0);
        assert_eq!(g.mots.len(), 1);
        assert_eq!(g.mots[0].texte, "a");
        assert!((g.mots[0].rect.x0 - 0.4).abs() < 1e-5);
        assert!((g.mots[0].rect.x1 - 0.6).abs() < 1e-5);
        assert_eq!(d.mots[0].texte, "b");
        assert!((d.mots[0].rect.x0 - 0.4).abs() < 1e-5);
        assert_eq!(d.mots[0].rect.y0, 0.4);
    }
}
