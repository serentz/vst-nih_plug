//! Rythmes d'une mesure en 16 pas (doubles croches).
//!
//! Notation : `.` silence, `x` attaque, `o` attaque à l'octave supérieure,
//! `5` attaque sur la quinte (ces deux derniers ne servent qu'à la basse).

pub const STEPS_PER_BAR: u32 = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    Note,
    Octave,
    Fifth,
}

/// Attaques d'une mesure notée sur 16 caractères : `(pas, type)`.
pub fn parse_bar(pattern: &str) -> Vec<(u32, Hit)> {
    pattern
        .chars()
        .take(STEPS_PER_BAR as usize)
        .enumerate()
        .filter_map(|(i, c)| {
            let hit = match c {
                'x' => Hit::Note,
                'o' => Hit::Octave,
                '5' => Hit::Fifth,
                _ => return None,
            };
            Some((i as u32, hit))
        })
        .collect()
}

/// Rythme nommé, éventuellement sur plusieurs mesures (répétées en boucle).
#[derive(Clone, Copy, Debug)]
pub struct RhythmPreset {
    pub name: &'static str,
    pub bars: &'static [&'static str],
}

impl RhythmPreset {
    /// Attaques sur toute la longueur `length` (en pas).
    pub fn hits(&self, length: u32) -> Vec<(u32, Hit)> {
        let mut out = Vec::new();
        for bar in 0..length.div_ceil(STEPS_PER_BAR) {
            let pattern = self.bars[bar as usize % self.bars.len()];
            for (s, h) in parse_bar(pattern) {
                let step = bar * STEPS_PER_BAR + s;
                if step < length {
                    out.push((step, h));
                }
            }
        }
        out
    }
}

pub const BASS_PRESETS: &[RhythmPreset] = &[
    RhythmPreset { name: "Pop", bars: &["x.x.x.x..x..x...", "x.x.x.o..x..x.x."] },
    RhythmPreset { name: "Croches", bars: &["x.x.x.x.x.x.x.x."] },
    RhythmPreset { name: "Octaves", bars: &["x.o.x.o.x.o.x.o."] },
    RhythmPreset { name: "Syncopé", bars: &["x..x..x..x..x.x."] },
    RhythmPreset { name: "Noires", bars: &["x...x...x...x..."] },
    RhythmPreset { name: "Rondes", bars: &["x..............."] },
    RhythmPreset { name: "Contretemps", bars: &["..x...x...x...x."] },
    RhythmPreset { name: "Fondamentale-quinte", bars: &["x.......5.......", "x...5...x...5..."] },
];

/// Relance de la dernière mesure d'un cycle de basse.
pub const BASS_FILL: &str = "x..xx.x..x..x.x.";

pub const CHORD_PRESETS: &[RhythmPreset] = &[
    RhythmPreset { name: "3+3+3+3+4", bars: &["x..x..x..x..x..."] },
    RhythmPreset { name: "Pop", bars: &["x.x.x.x..x..x..."] },
    RhythmPreset { name: "Relance", bars: &["x.........x.x.x.", "x..x..x..x..x.x."] },
    RhythmPreset { name: "Croches", bars: &["x.x.x.x.x.x.x.x."] },
    RhythmPreset { name: "Blanches", bars: &["x.......x......."] },
    RhythmPreset { name: "Rondes", bars: &["x..............."] },
    RhythmPreset { name: "Stabs contretemps", bars: &["..x...x...x...x."] },
];

pub fn preset_by_name<'a>(presets: &'a [RhythmPreset], name: &str) -> Option<usize> {
    presets.iter().position(|p| p.name.eq_ignore_ascii_case(name))
}
