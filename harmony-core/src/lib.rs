//! Moteur musical d'Harmogen : théorie (notes, gammes, accords), analyse
//! d'une progression d'accords captée (découpage, reconnaissance, tonalité)
//! et génération des lignes (basse, accords rythmés, mélodie).
//!
//! Le temps est exprimé en pas de double croche (1/16 de la grille Ableton,
//! 4 pas par temps) : c'est la résolution minimale d'un changement d'accord.

pub mod analysis;
pub mod bank;
pub mod generate;
pub mod library;
pub mod theory;

/// Nombre de pas par temps (pas = double croche).
pub const STEPS_PER_BEAT: u32 = 4;
