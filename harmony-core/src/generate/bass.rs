//! Ligne de basse monophonique.
//!
//! D'après les exemples : presque uniquement des fondamentales (98 %), répétées ou
//! sautant à l'octave ; une attaque à chaque changement d'accord, même syncopé ;
//! la dernière mesure du cycle relance (« fill ») vers le premier accord.

use super::rhythm::{parse_bar, Hit, Onset, BASS_FILL, BASS_PRESETS, STEPS_PER_BAR};
use super::rng::Rng;
use super::{fit_lengths, nearest_pitch, vary_articulation, velocity, Context, GenNote, Pattern};

#[derive(Clone, Copy, Debug)]
pub struct BassParams {
    /// Indice dans [`BASS_PRESETS`].
    pub preset: usize,
    /// Registre des fondamentales (MIDI). Défaut A1–A2 (convention Live), le registre des exemples.
    pub low: u8,
    pub high: u8,
    /// Remplace la dernière mesure par une relance, avec note d'approche vers le premier accord.
    pub fill_last_bar: bool,
    /// Ajoute une attaque à chaque changement d'accord absent du rythme.
    pub follow_changes: bool,
    /// Durée des attaques ajoutées aux changements d'accord, en proportion de l'écart
    /// jusqu'à l'attaque suivante (les rythmes écrivent eux-mêmes leurs durées et silences).
    pub gate: f32,
    /// Part des attaques dont l'articulation varie (legato, plus courte, plus longue).
    pub articulation: f32,
    pub velocity: u8,
}

impl Default for BassParams {
    fn default() -> Self {
        Self {
            preset: 0,
            low: 45,
            high: 57,
            fill_last_bar: true,
            follow_changes: true,
            gate: 0.85,
            articulation: 0.35,
            velocity: 72,
        }
    }
}

pub fn generate(ctx: &Context, p: &BassParams, rng: &mut Rng) -> Pattern {
    let length = ctx.progression.length;
    let preset = &BASS_PRESETS[p.preset % BASS_PRESETS.len()];
    let mut hits = preset.hits(length);

    let fill_start = (length >= 2 * STEPS_PER_BAR && p.fill_last_bar)
        .then(|| (length - 1) / STEPS_PER_BAR * STEPS_PER_BAR);
    if let Some(start) = fill_start {
        hits.retain(|o| o.step < start);
        hits.extend(parse_bar(BASS_FILL).into_iter().map(|o| Onset { step: start + o.step, ..o }));
    }
    if p.follow_changes {
        for change in ctx.chord_changes() {
            if !hits.iter().any(|o| o.step == change) {
                hits.push(Onset { step: change, hit: Hit::Note, len: 0 });
            }
        }
    }
    hits.sort_by_key(|o| o.step);
    hits.dedup_by_key(|o| o.step);

    let center = ((p.low as u16 + p.high as u16) / 2) as u8;
    let mut notes = Vec::with_capacity(hits.len());
    let mut root_pitch = center;
    let mut current_root = None;
    for (i, &Onset { step, hit, len }) in hits.iter().enumerate() {
        let chord = ctx.chord_at(step);
        if current_root != Some(chord.root) {
            // Fondamentale la plus proche de la précédente : mouvement de basse minimal.
            root_pitch = nearest_pitch(chord.root, root_pitch, p.low, p.high);
            current_root = Some(chord.root);
        }
        let mut pitch = match hit {
            Hit::Note => root_pitch,
            Hit::Octave => root_pitch.saturating_add(12).min(127),
            Hit::Fifth => nearest_pitch((chord.root + 7) % 12, root_pitch + 4, p.low, p.high + 7),
        };

        // Dans la relance, la dernière attaque peut approcher le premier accord par degré conjoint.
        let is_last = i + 1 == hits.len();
        if is_last && fill_start.is_some_and(|f| step >= f) && rng.chance(0.6) {
            let next_root = ctx.chord_at(0).root;
            let target = nearest_pitch(next_root, root_pitch, p.low, p.high);
            let from_below = target > p.low + 1 && (rng.chance(0.5) || target + 2 > p.high);
            pitch = if from_below { ctx.scale.step(target, -1) } else { ctx.scale.step(target, 1) };
        }

        notes.push(GenNote {
            start: step,
            len,
            pitch,
            velocity: velocity(rng, p.velocity, step, 5),
        });
    }
    fit_lengths(&mut notes, length, p.gate);
    vary_articulation(&mut notes, length, p.articulation, rng);
    Pattern { length, notes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::test_support::progression;

    #[test]
    fn bass_plays_the_root_on_every_chord_change() {
        let (prog, scale) = progression(8);
        let ctx = Context { progression: &prog, scale };
        let pat = generate(&ctx, &BassParams::default(), &mut Rng::new(1));
        for seg in &prog.segments {
            let note = pat.notes.iter().find(|n| n.start == seg.start).expect("attaque au changement");
            assert_eq!(note.pitch % 12, seg.chord.root, "pas {}", seg.start);
        }
    }

    #[test]
    fn bass_is_monophonic_and_in_range() {
        let (prog, scale) = progression(8);
        let ctx = Context { progression: &prog, scale };
        for seed in 0..20 {
            let p = BassParams { preset: seed as usize, ..Default::default() };
            let pat = generate(&ctx, &p, &mut Rng::new(seed));
            for w in pat.notes.windows(2) {
                assert!(w[0].start + w[0].len <= w[1].start);
            }
            assert!(pat.notes.iter().all(|n| (p.low..=p.high + 12).contains(&n.pitch)));
        }
    }

    #[test]
    fn same_seed_same_bass() {
        let (prog, scale) = progression(4);
        let ctx = Context { progression: &prog, scale };
        let a = generate(&ctx, &BassParams::default(), &mut Rng::new(7));
        let b = generate(&ctx, &BassParams::default(), &mut Rng::new(7));
        assert_eq!(a, b);
    }
}
