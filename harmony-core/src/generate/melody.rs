//! Mélodie monophonique.
//!
//! Construction en deux moitiés A – A′ :
//! - rythme : dans chaque moitié, mesures chargées et aérées en alternance
//!   (question–réponse), la dernière mesure de la moitié plus calme (cadence) ;
//!   A′ reprend le rythme de A et change la fin ;
//! - hauteurs : A′ reprend aussi les hauteurs de A, recorrigées si l'accord change ;
//!   ailleurs, chaque note est tirée au sort parmi les notes de la gamme, avec des
//!   poids tirés des exemples analysés.
//!
//! Poids observés (9 mélodies) — sur les temps 1 et 3 : tierce 45 %, quinte 18 %,
//! 7e 16 %, fondamentale 10 %, 9e 8 % ; intervalles : secondes 41 %, tierces 26 %,
//! notes répétées 16 %, quartes et quintes 15 %, octave 1 %.

use super::rhythm::{parse_bar, STEPS_PER_BAR};
use super::rng::Rng;
use super::{apply_gate, harmonic_tones, Context, GenNote, Pattern};
use crate::theory::{Chord, Role};

#[derive(Clone, Copy, Debug)]
pub struct MelodyParams {
    /// Étendue (MIDI). Défaut D3–G4 (convention Live), environ une octave et demie.
    pub low: u8,
    pub high: u8,
    pub gate: f32,
    pub velocity: u8,
}

impl Default for MelodyParams {
    fn default() -> Self {
        Self { low: 62, high: 79, gate: 0.9, velocity: 62 }
    }
}

/// Mesures chargées (croches).
const DENSE: &[&str] = &[
    "x.x.x.x.x.x.x.x.",
    "x.x.x.x...x.x.x.",
    "x.x.x.x.x.x.x...",
    "x.x...x.x.x.x...",
    "x.x.x.x.x...x...",
];
/// Mesures aérées (réponse, respiration).
const SPARSE: &[&str] = &["x...x...x.......", "x.....x.....x...", "x...x.x.x.......", "x.x.x...x......."];
/// Fin de la première moitié (question).
const CADENCE: &[&str] = &["x...x...x...x...", "x.......x...x...", "x...x...x......."];
/// Dernière mesure de la boucle (conclusion).
const FINAL: &[&str] = &["x...x...x.......", "x.......x.......", "x...x...x...x..."];

/// Place d'une note dans la phrase.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Inside,
    /// Dernière note de la première moitié : suspensive, évite la tonique.
    QuestionEnd,
    /// Dernière note de la boucle : conclusive, attirée par la tonique.
    FinalEnd,
}

/// Rythme de chaque mesure.
fn bar_rhythms(bars: u32, rng: &mut Rng) -> Vec<&'static str> {
    let half = (bars / 2).max(1);
    let mut cells: Vec<&'static str> = Vec::with_capacity(bars as usize);
    for b in 0..bars {
        let cell = if b == bars - 1 {
            *rng.pick(FINAL)
        } else if b >= half {
            // A′ : même rythme que A, avec une variation de temps en temps.
            let src = cells[(b - half) as usize];
            if rng.chance(0.25) {
                *rng.pick(if DENSE.contains(&src) { DENSE } else { SPARSE })
            } else {
                src
            }
        } else if b == half - 1 {
            *rng.pick(CADENCE)
        } else if b % 2 == 0 {
            *rng.pick(DENSE)
        } else {
            *rng.pick(SPARSE)
        };
        cells.push(cell);
    }
    cells
}

/// Poids d'une note sur un temps fort selon sa fonction dans l'accord.
fn strong_weight(role: Role) -> f32 {
    match role {
        Role::Third => 1.0,
        Role::Fifth => 0.45,
        Role::Seventh => 0.4,
        Role::Root => 0.25,
        Role::Ninth => 0.2,
        Role::Eleventh | Role::Thirteenth => 0.05,
        Role::Other => 0.02,
    }
}

/// Poids par hauteur candidate selon l'intervalle depuis la note précédente
/// (fréquence observée de la classe d'intervalle ÷ nombre de candidats de la classe).
fn motion_weight(semitones: i32) -> f32 {
    match semitones.abs() {
        0 => 0.16,
        1 | 2 => 0.2,
        3 | 4 => 0.13,
        5..=7 => 0.04,
        8..=12 => 0.004,
        _ => 0.0,
    }
}

struct Candidate<'a> {
    chord: &'a Chord,
    strong: bool,
    place: Place,
    previous: Option<u8>,
    /// Dernier mouvement (signé) avant cette note.
    last_move: i32,
    /// Hauteur visée par le contour en arche.
    target: f32,
}

fn score(ctx: &Context, c: &Candidate, pitch: u8) -> f32 {
    let pc = pitch % 12;
    let stable = harmonic_tones(c.chord, &ctx.scale).contains(pc);
    let mut w = if c.strong {
        if stable { strong_weight(c.chord.role(pc)).max(0.25) } else { strong_weight(c.chord.role(pc)) }
    } else if stable {
        1.0
    } else {
        // Note de passage ou broderie : seulement par mouvement conjoint.
        match c.previous {
            Some(prev) if (pitch as i32 - prev as i32).abs() <= 2 => 0.8,
            _ => 0.1,
        }
    };

    if let Some(prev) = c.previous {
        let mv = pitch as i32 - prev as i32;
        w *= motion_weight(mv);
        // Équilibre après un saut : retour par degré conjoint dans l'autre sens.
        if c.last_move.abs() > 4 {
            if mv.signum() == -c.last_move.signum() && mv.abs() <= 2 {
                w *= 2.5;
            } else if mv.signum() == c.last_move.signum() && mv.abs() > 2 {
                w *= 0.3;
            }
        }
    }

    let d = pitch as f32 - c.target;
    w *= (-(d * d) / (2.0 * 5.0 * 5.0)).exp();

    let tonic = ctx.scale.tonic;
    match c.place {
        Place::FinalEnd if pc == tonic => w *= 6.0,
        Place::FinalEnd if pc == c.chord.root => w *= 2.0,
        Place::QuestionEnd if pc == tonic => w *= 0.4,
        _ => {}
    }
    w
}

pub fn generate(ctx: &Context, p: &MelodyParams, rng: &mut Rng) -> Pattern {
    let length = ctx.progression.length;
    let bars = length.div_ceil(STEPS_PER_BAR).max(1);
    let half = (bars / 2).max(1);
    let half_len = half * STEPS_PER_BAR;
    let cells = bar_rhythms(bars, rng);

    let onsets: Vec<u32> = cells
        .iter()
        .enumerate()
        .flat_map(|(b, cell)| parse_bar(cell).into_iter().map(move |(s, _)| b as u32 * STEPS_PER_BAR + s))
        .filter(|&s| s < length)
        .collect();
    let question_end = onsets.iter().rposition(|&s| s < half_len).filter(|_| bars > 1);

    let center = (p.low as f32 + p.high as f32) / 2.0;
    let amp = (p.high - p.low) as f32 / 3.0;
    let mut pitches: Vec<u8> = Vec::with_capacity(onsets.len());
    let mut last_move = 0;
    for (i, &start) in onsets.iter().enumerate() {
        let next = onsets.get(i + 1).copied().unwrap_or(length);
        let strong = start % 8 == 0 || next - start >= 6;
        let chord = ctx.chord_at(start);
        let previous = pitches.last().copied();
        let bar = start / STEPS_PER_BAR;

        // A′ : reprise de la note correspondante de A quand la mesure a le même rythme.
        let echo = (bar >= half && bar + 1 < bars && cells[bar as usize] == cells[(bar - half) as usize])
            .then(|| onsets.iter().position(|&s| s == start - half_len))
            .flatten();
        let pitch = if let Some(j) = echo {
            let mut pitch = pitches[j];
            let tones = harmonic_tones(&chord, &ctx.scale);
            let same_chord = ctx.chord_at(onsets[j]).same_harmony(&chord);
            if strong && !same_chord && !tones.contains(pitch % 12) {
                // L'accord a changé : on se replace sur la note stable la plus proche.
                pitch = (p.low..=p.high)
                    .filter(|n| tones.contains(n % 12))
                    .min_by_key(|n| n.abs_diff(pitch))
                    .unwrap_or(pitch);
            }
            pitch
        } else {
            let place = if i + 1 == onsets.len() {
                Place::FinalEnd
            } else if Some(i) == question_end {
                Place::QuestionEnd
            } else {
                Place::Inside
            };
            let t = (start % half_len) as f32 / half_len as f32;
            let cand = Candidate {
                chord: &chord,
                strong,
                place,
                previous,
                last_move,
                target: center - amp / 2.0 + amp * (std::f32::consts::PI * t).sin(),
            };
            let pool: Vec<u8> = (p.low..=p.high).filter(|n| ctx.scale.contains(n % 12)).collect();
            let weights: Vec<f32> = pool.iter().map(|&n| score(ctx, &cand, n)).collect();
            match rng.weighted(&weights) {
                Some(k) => pool[k],
                None => previous.unwrap_or(center as u8),
            }
        };
        if let Some(prev) = previous {
            last_move = pitch as i32 - prev as i32;
        }
        pitches.push(pitch);
    }

    let mut notes: Vec<GenNote> = onsets
        .iter()
        .zip(&pitches)
        .map(|(&start, &pitch)| {
            let accent = if start % 8 == 0 { 6 } else { 0 };
            let vel = (p.velocity as i32 + accent + rng.range(-6, 6)).clamp(1, 127) as u8;
            GenNote { start, len: 0, pitch, velocity: vel }
        })
        .collect();
    apply_gate(&mut notes, length, p.gate);
    // La note finale est tenue jusqu'au bout de la boucle.
    if let Some(last) = notes.last_mut() {
        last.len = length - last.start;
    }
    Pattern { length, notes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::test_support::progression;

    fn melodies(bars: u32, seeds: u64) -> Vec<(Pattern, Vec<Chord>)> {
        let (prog, scale) = progression(bars);
        let ctx = Context { progression: &prog, scale };
        (0..seeds)
            .map(|seed| {
                let pat = generate(&ctx, &MelodyParams::default(), &mut Rng::new(seed));
                let chords = pat.notes.iter().map(|n| ctx.chord_at(n.start)).collect();
                (pat, chords)
            })
            .collect()
    }

    #[test]
    fn melody_is_monophonic_in_scale_and_range() {
        let p = MelodyParams::default();
        for (pat, _) in melodies(8, 30) {
            for w in pat.notes.windows(2) {
                assert!(w[0].start + w[0].len <= w[1].start);
            }
            for n in &pat.notes {
                assert!((p.low..=p.high).contains(&n.pitch));
                assert!(crate::theory::Scale::new(10, crate::theory::Mode::Major).contains(n.pitch % 12));
            }
        }
    }

    #[test]
    fn strong_beats_land_on_chord_tones() {
        let (mut total, mut stable) = (0, 0);
        for (pat, chords) in melodies(8, 30) {
            for (n, chord) in pat.notes.iter().zip(&chords) {
                if n.start % 8 == 0 {
                    total += 1;
                    if chord.tones().contains(n.pitch % 12) || chord.role(n.pitch % 12) == Role::Seventh {
                        stable += 1;
                    }
                }
            }
        }
        // Exemples : 89 % de notes de l'accord ou de 7e sur les temps 1 et 3.
        assert!(stable * 100 / total >= 80, "{stable}/{total}");
    }

    #[test]
    fn second_half_echoes_the_first() {
        // La mesure 5 reprend la mesure 1 (rythme et hauteurs), sauf variation (25 %).
        let (prog, scale) = progression(8);
        let ctx = Context { progression: &prog, scale };
        let same = (0..40)
            .filter(|&seed| {
                let pat = generate(&ctx, &MelodyParams::default(), &mut Rng::new(seed));
                let a: Vec<(u32, u8)> = pat.notes.iter().filter(|n| n.start < 16).map(|n| (n.start, n.pitch)).collect();
                let b: Vec<(u32, u8)> =
                    pat.notes.iter().filter(|n| (64..80).contains(&n.start)).map(|n| (n.start - 64, n.pitch)).collect();
                a == b
            })
            .count();
        assert!(same >= 20, "{same}/40");
    }
}
