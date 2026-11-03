//! Génération des lignes jouées sur une progression : basse, accords rythmés, mélodie.
//!
//! Les règles et les valeurs par défaut viennent du document d'harmonie et de
//! l'analyse des exemples (voir chaque générateur).

pub mod bass;
pub mod chords;
pub mod engine;
pub mod melody;
pub mod rhythm;
pub mod rng;
pub mod variation;

use crate::analysis::Progression;
use crate::theory::{Chord, PcSet, Quality, Scale};

/// Note générée. `start` et `len` sont en pas (doubles croches).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GenNote {
    pub start: u32,
    pub len: u32,
    pub pitch: u8,
    pub velocity: u8,
}

/// Une boucle générée, de la même longueur que la progression.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Pattern {
    pub length: u32,
    pub notes: Vec<GenNote>,
}

/// Ce que tous les générateurs connaissent : la progression et la gamme.
pub struct Context<'a> {
    pub progression: &'a Progression,
    pub scale: Scale,
}

impl Context<'_> {
    /// Accord au pas `step` ; à défaut, l'accord de tonique de la gamme.
    pub fn chord_at(&self, step: u32) -> Chord {
        self.progression.chord_at(step).copied().unwrap_or_else(|| {
            let q = if self.scale.mode.is_minor() { Quality::Minor } else { Quality::Major };
            Chord::new(self.scale.tonic, q)
        })
    }

    /// Pas où commence chaque accord.
    pub fn chord_changes(&self) -> Vec<u32> {
        self.progression.segments.iter().map(|s| s.start).collect()
    }
}

/// Notes stables d'un accord : ses notes, plus la tierce diatonique quand l'accord
/// n'en a pas (accord de quinte captée sans tierce : la gamme la fournit).
pub fn harmonic_tones(chord: &Chord, scale: &Scale) -> PcSet {
    let mut tones = chord.tones();
    if !chord.quality.has_third() && !matches!(chord.quality, Quality::Sus2 | Quality::Sus4 | Quality::Sus4Seventh) {
        for third in [3, 4] {
            let pc = (chord.root + third) % 12;
            if scale.contains(pc) {
                tones.insert(pc);
                break;
            }
        }
    }
    tones
}

/// Ajuste les durées : une note sans durée écrite (`len` = 0) dure jusqu'à l'attaque
/// suivante × `gate` ; aucune note ne dépasse l'attaque suivante (toutes notes confondues),
/// ce qui garantit qu'une ligne monophonique ne se chevauche jamais.
pub(crate) fn fit_lengths(notes: &mut [GenNote], length: u32, gate: f32) {
    let mut starts: Vec<u32> = notes.iter().map(|n| n.start).collect();
    starts.sort_unstable();
    starts.dedup();
    for n in notes.iter_mut() {
        let next = starts.iter().copied().find(|&s| s > n.start).unwrap_or(length);
        let span = next.saturating_sub(n.start).max(1);
        if n.len == 0 {
            n.len = (span as f32 * gate).round() as u32;
        }
        n.len = n.len.clamp(1, span);
    }
}

/// Varie l'articulation pour que les silences entre les notes ne soient pas tous égaux.
/// Pour chaque attaque tirée au sort (probabilité `amount`, toutes les voix d'un accord
/// ensemble), la note devient legato (aucun silence avant la suivante), plus courte,
/// ou plus longue dans le silence disponible. À appeler après [`fit_lengths`].
pub(crate) fn vary_articulation(notes: &mut [GenNote], length: u32, amount: f32, rng: &mut rng::Rng) {
    let mut starts: Vec<u32> = notes.iter().map(|n| n.start).collect();
    starts.sort_unstable();
    starts.dedup();
    for (i, &start) in starts.iter().enumerate() {
        if !rng.chance(amount) {
            continue;
        }
        let span = starts.get(i + 1).copied().unwrap_or(length) - start;
        let Some(current) = notes.iter().find(|n| n.start == start).map(|n| n.len) else { continue };
        let new_len = match rng.range(0, 2) {
            0 => span,
            1 => (current / 2).max(1),
            _ => (current + rng.range(1, span.saturating_sub(current).max(1) as i32) as u32).min(span),
        };
        for n in notes.iter_mut().filter(|n| n.start == start) {
            n.len = new_len;
        }
    }
}

/// Vélocité humanisée : accent sur le premier temps, plus léger sur les temps,
/// plus doux sur les doubles croches impaires.
pub(crate) fn velocity(rng: &mut rng::Rng, base: u8, step: u32, humanize: i32) -> u8 {
    let accent = match step % rhythm::STEPS_PER_BAR {
        0 => 8,
        s if s % 4 == 0 => 3,
        s if s % 2 == 1 => -6,
        _ => 0,
    };
    (base as i32 + accent + rng.range(-humanize, humanize)).clamp(1, 127) as u8
}

/// Hauteur de classe `pc` la plus proche de `reference` dans `[low, high]`.
pub(crate) fn nearest_pitch(pc: u8, reference: u8, low: u8, high: u8) -> u8 {
    (low..=high)
        .filter(|n| n % 12 == pc % 12)
        .min_by_key(|&n| (n as i32 - reference as i32).abs())
        .unwrap_or_else(|| low + (pc % 12 + 12 - low % 12) % 12)
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::analysis::{segment, Progression, TimedNote};
    use crate::theory::{Mode, Scale};

    /// Cm | Eb | Bb | Gm F, sur `bars` mesures (la progression de 4 mesures boucle).
    pub fn progression(bars: u32) -> (Progression, Scale) {
        let chords: [(u32, u32, &[u8]); 5] = [
            (0, 16, &[48, 55, 63]),
            (16, 32, &[51, 58, 67]),
            (32, 48, &[46, 53, 62]),
            (48, 56, &[43, 50, 58]),
            (56, 64, &[41, 48, 57]),
        ];
        let mut notes = Vec::new();
        for rep in 0..bars.div_ceil(4) {
            for &(s, e, pitches) in &chords {
                for &pitch in pitches {
                    notes.push(TimedNote { start: rep * 64 + s, end: rep * 64 + e, pitch, velocity: 64 });
                }
            }
        }
        (segment(&notes, bars * 16), Scale::new(10, Mode::Major))
    }
}
