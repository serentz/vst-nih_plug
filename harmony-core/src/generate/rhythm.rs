//! Rythmes d'une mesure en 16 pas (doubles croches).
//!
//! Notation, un caractère par pas :
//! - `x` attaque ; `o` attaque à l'octave supérieure ; `5` attaque sur la quinte
//!   (ces deux derniers ne servent qu'à la basse) ;
//! - `-` prolonge la note précédente ;
//! - `.` silence. Le silence est une note à part entière : les rythmes l'écrivent explicitement.
//!
//! Exemple : `x-.x-.x-.x-.x-..` = cinq notes d'une croche séparées par des silences.

pub const STEPS_PER_BAR: u32 = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    Note,
    Octave,
    Fifth,
}

/// Attaque d'un rythme. `len` = 0 : durée non écrite (attaque ajoutée par un générateur),
/// la note dure alors jusqu'à l'attaque suivante.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Onset {
    pub step: u32,
    pub hit: Hit,
    pub len: u32,
}

/// Attaques d'une mesure notée sur 16 caractères (pas relatifs au début de la mesure).
pub fn parse_bar(pattern: &str) -> Vec<Onset> {
    let chars: Vec<char> = pattern.chars().take(STEPS_PER_BAR as usize).collect();
    let mut out = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        let hit = match c {
            'x' => Hit::Note,
            'o' => Hit::Octave,
            '5' => Hit::Fifth,
            _ => continue,
        };
        let held = chars[i + 1..].iter().take_while(|&&c| c == '-').count() as u32;
        out.push(Onset { step: i as u32, hit, len: 1 + held });
    }
    out
}

/// Rythme nommé, éventuellement sur plusieurs mesures (répétées en boucle).
#[derive(Clone, Copy, Debug)]
pub struct RhythmPreset {
    pub name: &'static str,
    pub bars: &'static [&'static str],
}

impl RhythmPreset {
    /// Attaques sur toute la longueur `length` (en pas).
    pub fn hits(&self, length: u32) -> Vec<Onset> {
        let mut out = Vec::new();
        for bar in 0..length.div_ceil(STEPS_PER_BAR) {
            let pattern = self.bars[bar as usize % self.bars.len()];
            for o in parse_bar(pattern) {
                let step = bar * STEPS_PER_BAR + o.step;
                if step < length {
                    out.push(Onset { step, ..o });
                }
            }
        }
        out
    }
}

pub const BASS_PRESETS: &[RhythmPreset] = &[
    RhythmPreset { name: "Pop", bars: &["x-x-x-x-.x-.x-..", "x-x.x-o--x..x-x."] },
    RhythmPreset { name: "Croches", bars: &["x.x.x-x.x.x.x-x.", "x.x-x.x.x-x.x.x."] },
    RhythmPreset { name: "Octaves", bars: &["x.o.x.o-x.o.x-o.", "x-o.x.o.x.o-x.o."] },
    RhythmPreset { name: "Syncopé", bars: &["x-.x-.x-.x-.x-x.", "x--x.-x-..x-x---"] },
    RhythmPreset { name: "Noires", bars: &["x--.x--.x--.x--.", "x---x-.x--.x--.."] },
    RhythmPreset { name: "Rondes", bars: &["x-----------....", "x-------.....x-."] },
    RhythmPreset { name: "Contretemps", bars: &["..x-..x-..x-..x-", "..x-..x...x-..x."] },
    RhythmPreset { name: "Fondamentale-quinte", bars: &["x-----..5-----..", "x--.5--.x--.5--."] },
    RhythmPreset { name: "Irrégulier", bars: &["x--x-.x.x---.x-.", "x-.x---.x-x.x---"] },
];

/// Relance de la dernière mesure d'un cycle de basse.
pub const BASS_FILL: &str = "x-.xx-x-.x-.x-x.";

pub const CHORD_PRESETS: &[RhythmPreset] = &[
    RhythmPreset { name: "3+3+3+3+4", bars: &["x-.x-.x-.x-.x-..", "x--x-.x--x-.x---"] },
    RhythmPreset { name: "Pop", bars: &["x-x-x-x-.x-.x-..", "x-x-x.x--x-.x-x."] },
    RhythmPreset { name: "Relance", bars: &["x-------..x-x-x.", "x-.x-.x-.x-.x-x."] },
    RhythmPreset { name: "Croches", bars: &["x.x.x.x-x.x.x.x-"] },
    RhythmPreset { name: "Blanches", bars: &["x------.x------.", "x-------x-----.."] },
    RhythmPreset { name: "Rondes", bars: &["x-------------.."] },
    RhythmPreset { name: "Stabs contretemps", bars: &["..x...x-..x...x.", "..x-..x...x-..x."] },
    RhythmPreset { name: "Irrégulier", bars: &["x--x-.x.x---.x-.", "x-.x---.x-x.x---"] },
];

pub fn preset_by_name(presets: &[RhythmPreset], name: &str) -> Option<usize> {
    presets.iter().position(|p| p.name.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_and_rests() {
        let o = parse_bar("x-.x---.o.......");
        assert_eq!(
            o,
            vec![
                Onset { step: 0, hit: Hit::Note, len: 2 },
                Onset { step: 3, hit: Hit::Note, len: 4 },
                Onset { step: 8, hit: Hit::Octave, len: 1 },
            ]
        );
    }
}
