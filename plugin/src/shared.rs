//! État partagé entre le thread audio, le travail en arrière-plan et l'interface.
//!
//! Le thread audio n'écrit que des atomiques ; ce qui demande une allocation
//! (progression, passages) est écrit par l'arrière-plan dans `display`.

use harmony_core::generate::Pattern;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU8};
use std::sync::Mutex;

pub const STATE_WAITING: u8 = 0;
pub const STATE_CAPTURING: u8 = 1;
pub const STATE_BUILDING: u8 = 2;
pub const STATE_PLAYING: u8 = 3;

/// Passages gardés pour l'affichage.
pub const DISPLAY_PASSES: usize = 6;

/// Passage calculé, étiqueté pour que l'interface sache lequel est joué.
pub struct Tagged {
    /// Incrémenté à chaque nouveau modèle (capture ou réglage modifié).
    pub generation: u32,
    /// Numéro du passage dans la génération (1 = modèle).
    pub index: u32,
    pub pattern: Pattern,
}

#[derive(Default)]
pub struct Display {
    pub length: u32,
    /// (début, durée, symbole) de chaque accord reconnu.
    pub chords: Vec<(u32, u32, String)>,
    pub key: String,
    pub alternatives: Vec<String>,
    pub generation: u32,
    /// Derniers passages calculés : (numéro, passage).
    pub passes: VecDeque<(u32, Pattern)>,
}

#[derive(Default)]
pub struct Shared {
    pub state: AtomicU8,
    /// Position dans la boucle (en pas) ; pendant la capture, pas écoulés depuis son début.
    pub position: AtomicI32,
    /// Longueur de la capture en cours ou de la boucle jouée (en pas).
    pub length: AtomicU32,
    pub playing_generation: AtomicU32,
    pub playing_index: AtomicU32,
    /// Demandes de l'interface, consommées par le thread audio.
    pub relearn_request: AtomicBool,
    pub model_request: AtomicBool,
    pub display: Mutex<Display>,
}
