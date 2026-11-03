//! Accords rythmés à 3 voix.
//!
//! D'après les exemples : voicing serré, mouvement minimal d'un accord à l'autre
//! (1 à 3 demi-tons par voix en moyenne), et note du haut qui alterne avec la 9e
//! sur les attaques répétées d'un même accord. Les durées et les silences viennent du rythme.

use super::rhythm::CHORD_PRESETS;
use super::rng::Rng;
use super::{fit_lengths, harmonic_tones, velocity, Context, GenNote, Pattern};
use crate::theory::{Chord, PitchClass, Quality, Scale};

#[derive(Clone, Copy, Debug)]
pub struct ChordParams {
    /// Indice dans [`CHORD_PRESETS`].
    pub preset: usize,
    /// Registre du voicing (MIDI). Défaut C2–D4 (convention Live).
    pub low: u8,
    pub high: u8,
    pub voices: usize,
    /// Alterne la note du haut avec la 9e sur les attaques répétées.
    pub color: bool,
    pub follow_changes: bool,
    pub gate: f32,
    pub velocity: u8,
}

impl Default for ChordParams {
    fn default() -> Self {
        Self {
            preset: 0,
            low: 48,
            high: 74,
            voices: 3,
            color: true,
            follow_changes: true,
            gate: 0.95,
            velocity: 64,
        }
    }
}

/// Classes de hauteur jouées pour un accord, de la fondamentale vers le haut.
/// À 3 voix, un accord de septième garde fondamentale, tierce et septième.
fn chord_pcs(chord: &Chord, scale: &Scale, voices: usize) -> Vec<PitchClass> {
    let iv = chord.quality.intervals();
    let mut intervals: Vec<u8> = if iv.len() == 4 && voices <= 3 {
        vec![iv[0], iv[1], iv[3]]
    } else {
        iv.to_vec()
    };
    if chord.quality == Quality::Power {
        // Tierce fournie par la gamme.
        let tones = harmonic_tones(chord, scale);
        if let Some(t) = [3u8, 4].into_iter().find(|t| tones.contains((chord.root + t) % 12)) {
            intervals = vec![0, t, 7];
        }
    }
    intervals.truncate(voices.max(1));
    intervals.iter().map(|i| (chord.root + i) % 12).collect()
}

/// Tous les voicings serrés (chaque renversement, chaque octave) tenant dans le registre.
fn voicings(pcs: &[PitchClass], low: u8, high: u8) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for rot in 0..pcs.len() {
        let order: Vec<PitchClass> = pcs[rot..].iter().chain(&pcs[..rot]).copied().collect();
        for bottom in (low..=high).filter(|n| n % 12 == order[0]) {
            let mut v = vec![bottom];
            for &pc in &order[1..] {
                let last = *v.last().unwrap();
                v.push(last + 1 + (pc + 12 - (last + 1) % 12) % 12);
            }
            if *v.last().unwrap() <= high {
                out.push(v);
            }
        }
    }
    out
}

fn movement(a: &[u8], b: &[u8]) -> u32 {
    a.iter().zip(b).map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs()).sum()
}

/// Voicing le plus proche du précédent ; sans précédent, le plus proche du centre du registre.
fn lead_voices(pcs: &[PitchClass], previous: Option<&[u8]>, low: u8, high: u8) -> Vec<u8> {
    let candidates = voicings(pcs, low, high);
    let center = (low as u32 + high as u32) / 2;
    candidates
        .into_iter()
        .min_by_key(|v| match previous {
            Some(prev) if prev.len() == v.len() => movement(prev, v),
            _ => {
                let mean = v.iter().map(|&n| n as u32).sum::<u32>() / v.len() as u32;
                mean.abs_diff(center)
            }
        })
        .unwrap_or_else(|| pcs.iter().map(|&pc| low + (pc + 12 - low % 12) % 12).collect())
}

pub fn generate(ctx: &Context, p: &ChordParams, rng: &mut Rng) -> Pattern {
    let length = ctx.progression.length;
    let preset = &CHORD_PRESETS[p.preset % CHORD_PRESETS.len()];
    // (pas, durée écrite ; 0 = attaque ajoutée sur un changement d'accord)
    let mut onsets: Vec<(u32, u32)> = preset.hits(length).into_iter().map(|o| (o.step, o.len)).collect();
    if p.follow_changes {
        for change in ctx.chord_changes() {
            if !onsets.iter().any(|o| o.0 == change) {
                onsets.push((change, 0));
            }
        }
    }
    onsets.sort_unstable();
    onsets.dedup_by_key(|o| o.0);

    let mut notes = Vec::new();
    let mut voicing: Option<Vec<u8>> = None;
    let mut current: Option<Chord> = None;
    let mut repeat = 0usize;
    for &(step, len) in &onsets {
        let chord = ctx.chord_at(step);
        if current.map_or(true, |c| !c.same_harmony(&chord)) {
            let pcs = chord_pcs(&chord, &ctx.scale, p.voices);
            voicing = Some(lead_voices(&pcs, voicing.as_deref(), p.low, p.high));
            current = Some(chord);
            repeat = 0;
        } else {
            repeat += 1;
        }
        let mut played = voicing.clone().unwrap_or_default();

        // Couleur : sur une attaque répétée sur deux, la note du haut glisse vers la 9e voisine.
        if p.color && repeat % 2 == 1 {
            let ninth = (chord.root + 2) % 12;
            if ctx.scale.contains(ninth) {
                if let Some(top) = played.last().copied() {
                    let target = (top.saturating_sub(2)..=top + 2)
                        .filter(|n| n % 12 == ninth && !played.contains(n))
                        .min_by_key(|n| n.abs_diff(top));
                    if let (Some(t), Some(last)) = (target, played.last_mut()) {
                        *last = t;
                    }
                }
            }
        }

        let vel = velocity(rng, p.velocity, step, 5);
        for &pitch in &played {
            let v = (vel as i32 + rng.range(-3, 3)).clamp(1, 127) as u8;
            notes.push(GenNote { start: step, len, pitch, velocity: v });
        }
    }
    fit_lengths(&mut notes, length, p.gate);
    Pattern { length, notes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::test_support::progression;
    use std::collections::BTreeMap;

    #[test]
    fn three_voices_from_the_chord() {
        let (prog, scale) = progression(8);
        let ctx = Context { progression: &prog, scale };
        let pat = generate(&ctx, &ChordParams::default(), &mut Rng::new(3));
        let mut by_onset: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
        for n in &pat.notes {
            by_onset.entry(n.start).or_default().push(n.pitch);
        }
        for (step, pitches) in by_onset {
            assert_eq!(pitches.len(), 3, "pas {step}");
            let chord = ctx.chord_at(step);
            let ninth = (chord.root + 2) % 12;
            for p in pitches {
                assert!(chord.tones().contains(p % 12) || p % 12 == ninth, "pas {step} : {p}");
            }
        }
    }

    #[test]
    fn voice_leading_stays_close() {
        let (prog, scale) = progression(4);
        let ctx = Context { progression: &prog, scale };
        let p = ChordParams { color: false, ..Default::default() };
        let pat = generate(&ctx, &p, &mut Rng::new(3));
        let mut voicings: Vec<Vec<u8>> = Vec::new();
        for seg in &prog.segments {
            let mut v: Vec<u8> = pat.notes.iter().filter(|n| n.start == seg.start).map(|n| n.pitch).collect();
            v.sort_unstable();
            voicings.push(v);
        }
        for w in voicings.windows(2) {
            assert!(movement(&w[0], &w[1]) <= 9, "{:?} -> {:?}", w[0], w[1]);
        }
    }
}
