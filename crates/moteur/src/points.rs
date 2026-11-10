//! Points à vérifier : situations douteuses signalées à l'utilisateur.

use serde::Serialize;

use crate::modele::Ancre;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GenrePoint {
    TitreSuspect,
    NumeroPageResiduel,
    NoteOrpheline,
    Sommaire,
    PageSansTexte,
}

#[derive(Clone, Debug, Serialize)]
pub struct PointAVerifier {
    pub ancre: Ancre,
    pub genre: GenrePoint,
    pub message: String,
}
