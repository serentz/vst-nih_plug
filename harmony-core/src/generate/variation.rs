//! Variation d'un passage au suivant.
//!
//! À partir du deuxième passage, chaque rendu est calculé à partir du **précédent** :
//! - silence : une note devient silence, ou un silence assez long reçoit une note ;
//! - hauteur : une note monte ou descend d'un degré de la gamme ;
//! - vélocité : plus ou moins forte que la vélocité actuelle ;
//! - durée : note allongée ou raccourcie.
//!
//! Garde-fous, pour que la ligne évolue sans s'éloigner de l'harmonie :
//! - une note déplacée sur un temps fort doit rester une note de l'accord ;
//! - la basse garde sa fondamentale à chaque changement d'accord ;
//! - le nombre de notes reste entre la moitié et une fois et demie celui du modèle ;
//! - l'étendue reste celle du modèle (± 3 demi-tons) ;
//! - la vélocité est ramenée vers celle du modèle quand elle s'en écarte trop.

use super::rng::Rng;
use super::{fit_lengths, harmonic_tones, Context, GenNote, Pattern};
use std::collections::BTreeMap;

/// Type de ligne : fixe ce que la variation a le droit de toucher.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineKind {
    Melody,
    Bass,
    /// Accords : chaque attaque (toutes les voix) varie d'un bloc ; seule la voix du haut change de hauteur.
    Chords,
}

/// Réglages de variation (choisis dans l'interface). Une probabilité à 0 désactive l'élément.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VariationParams {
    /// Probabilité, par attaque, d'un changement silence ↔ note.
    pub silence: f32,
    /// Probabilité, par attaque, d'un déplacement d'un degré (−1 ou +1).
    pub pitch: f32,
    /// Probabilité, par attaque, d'un changement de vélocité…
    pub velocity: f32,
    /// … d'au plus ± ce nombre.
    pub velocity_range: u8,
    /// Probabilité, par attaque, d'un changement de durée…
    pub length: f32,
    /// … d'au plus ± ce nombre de pas.
    pub length_range: u32,
}

impl Default for VariationParams {
    fn default() -> Self {
        Self {
            silence: 0.08,
            pitch: 0.12,
            velocity: 0.3,
            velocity_range: 10,
            length: 0.15,
            length_range: 2,
        }
    }
}

/// Repères tirés du modèle (premier passage), qui bornent la dérive.
struct Anchor {
    low: u8,
    high: u8,
    onsets: usize,
    min_onsets: usize,
    max_onsets: usize,
    velocity: i32,
}

impl Anchor {
    fn of(base: &Pattern) -> Self {
        let onsets = groups(&base.notes).len();
        let low = base.notes.iter().map(|n| n.pitch).min().unwrap_or(60);
        let high = base.notes.iter().map(|n| n.pitch).max().unwrap_or(72);
        let velocity = if base.notes.is_empty() {
            64
        } else {
            base.notes.iter().map(|n| n.velocity as i32).sum::<i32>() / base.notes.len() as i32
        };
        Self {
            low: low.saturating_sub(3),
            high: high.saturating_add(3).min(127),
            onsets,
            min_onsets: onsets.div_ceil(2).max(1),
            max_onsets: onsets * 3 / 2 + 1,
            velocity,
        }
    }
}

/// Notes regroupées par attaque (une note pour une ligne monophonique, plusieurs pour un accord).
fn groups(notes: &[GenNote]) -> BTreeMap<u32, Vec<GenNote>> {
    let mut g: BTreeMap<u32, Vec<GenNote>> = BTreeMap::new();
    for n in notes {
        g.entry(n.start).or_default().push(*n);
    }
    for v in g.values_mut() {
        v.sort_by_key(|n| n.pitch);
    }
    g
}

/// Position forte : temps 1 et 3 (une note y doit rester une note de l'accord).
fn is_strong(step: u32) -> bool {
    step % 8 == 0
}

/// Calcule le passage suivant à partir de `previous`. `base` est le modèle (premier passage).
pub fn vary(
    previous: &Pattern,
    base: &Pattern,
    ctx: &Context,
    kind: LineKind,
    p: &VariationParams,
    rng: &mut Rng,
) -> Pattern {
    let length = previous.length;
    let anchor = Anchor::of(base);
    let changes = ctx.chord_changes();
    let protected = |step: u32| kind == LineKind::Bass && changes.contains(&step);
    let mut g = groups(&previous.notes);

    // 1. Silence ↔ note. Le choix penche vers le nombre d'attaques du modèle,
    //    pour que la densité oscille autour de lui au lieu de dériver.
    let events = (0..g.len()).filter(|_| rng.chance(p.silence)).count();
    for _ in 0..events {
        let fill_chance = match g.len().cmp(&anchor.onsets) {
            std::cmp::Ordering::Less => 0.7,
            std::cmp::Ordering::Equal => 0.5,
            std::cmp::Ordering::Greater => 0.3,
        };
        if rng.chance(fill_chance) && g.len() < anchor.max_onsets {
            // Une note dans un silence d'au moins une croche, n'importe où dans la boucle ;
            // à défaut (silences bouchés par les allongements), une note longue est coupée en deux.
            let starts: Vec<u32> = g.keys().copied().collect();
            let end_of = |s: u32| g[&s].iter().map(|n| n.start + n.len).max().unwrap_or(s);
            let mut options: Vec<(u32, u32, u32)> = starts
                .iter()
                .enumerate()
                .filter_map(|(i, &s)| {
                    let at = (end_of(s) + 1) & !1; // calé sur la croche
                    let next = starts.get(i + 1).copied().unwrap_or(length);
                    (next >= at + 2).then_some((s, at, next))
                })
                .collect();
            if options.is_empty() {
                options = starts
                    .iter()
                    .filter(|&&s| end_of(s) >= s + 4)
                    .map(|&s| (s, (s + (end_of(s) - s) / 2) & !1, end_of(s)))
                    .filter(|&(s, at, _)| at > s)
                    .collect();
            }
            if options.is_empty() {
                continue;
            }
            let (from, at, next) = *rng.pick(&options);
            if let Some(group) = new_group(&g, from, at, (next - at).min(4), ctx, kind, &anchor, rng) {
                for n in g.get_mut(&from).unwrap() {
                    n.len = n.len.min(at - n.start);
                }
                g.insert(at, group);
            }
        } else if g.len() > anchor.min_onsets {
            let removable: Vec<u32> = g.keys().copied().filter(|&s| s != 0 && !protected(s)).collect();
            if !removable.is_empty() {
                let s = *rng.pick(&removable);
                g.remove(&s);
            }
        }
    }

    // 2. Hauteur ± 1 degré.
    for (&start, group) in g.iter_mut() {
        if protected(start) || !rng.chance(p.pitch) {
            continue;
        }
        let chord = ctx.chord_at(start);
        let tones = harmonic_tones(&chord, &ctx.scale);
        let ninth = (chord.root + 2) % 12;
        let voice = group.len() - 1; // voix du haut (seule voix d'une ligne monophonique)
        let current = group[voice].pitch;
        let first_dir = if rng.chance(0.5) { 1 } else { -1 };
        for dir in [first_dir, -first_dir] {
            let candidate = ctx.scale.step(current, dir);
            let in_range = (anchor.low..=anchor.high).contains(&candidate);
            let pc = candidate % 12;
            let harmonic_ok = match kind {
                LineKind::Chords => tones.contains(pc) || (pc == ninth && ctx.scale.contains(ninth)),
                _ => !is_strong(start) || tones.contains(pc),
            };
            let free = !group[..voice].iter().any(|n| n.pitch == candidate);
            if in_range && harmonic_ok && free {
                group[voice].pitch = candidate;
                break;
            }
        }
    }

    // 3. Vélocité, ramenée vers celle du modèle si elle s'en écarte trop.
    let r = p.velocity_range as i32;
    for group in g.values_mut() {
        if !rng.chance(p.velocity) {
            continue;
        }
        let mean = group.iter().map(|n| n.velocity as i32).sum::<i32>() / group.len() as i32;
        let mut delta = rng.range(-r, r);
        if (mean - anchor.velocity).abs() > 2 * r {
            delta = -(mean - anchor.velocity).signum() * delta.abs();
        }
        for n in group.iter_mut() {
            n.velocity = (n.velocity as i32 + delta).clamp(1, 127) as u8;
        }
    }

    // 4. Durée ± quelques pas (fit_lengths empêche ensuite tout chevauchement).
    let lr = p.length_range.max(1) as i32;
    for group in g.values_mut() {
        if !rng.chance(p.length) {
            continue;
        }
        let delta = rng.range(-lr, lr);
        for n in group.iter_mut() {
            n.len = (n.len as i32 + delta).max(1) as u32;
        }
    }

    let mut notes: Vec<GenNote> = g.into_values().flatten().collect();
    notes.sort_by_key(|n| (n.start, n.pitch));
    fit_lengths(&mut notes, length, 1.0);
    Pattern { length, notes }
}

/// Nouvelle attaque au pas `at`, dérivée de l'attaque précédente `from`.
#[allow(clippy::too_many_arguments)]
fn new_group(
    g: &BTreeMap<u32, Vec<GenNote>>,
    from: u32,
    at: u32,
    len: u32,
    ctx: &Context,
    kind: LineKind,
    anchor: &Anchor,
    rng: &mut Rng,
) -> Option<Vec<GenNote>> {
    let from = &g[&from];
    let chord = ctx.chord_at(at);
    let tones = harmonic_tones(&chord, &ctx.scale);
    let velocity = from.iter().map(|n| n.velocity).min().unwrap_or(64).saturating_sub(4).max(1);
    match kind {
        // Accord : voicing d'une attaque qui joue déjà le même accord, la plus proche possible.
        LineKind::Chords => g
            .iter()
            .filter(|(&s, _)| ctx.chord_at(s).same_harmony(&chord))
            .min_by_key(|(&s, _)| s.abs_diff(at))
            .map(|(_, src)| src.iter().map(|n| GenNote { start: at, len, velocity, ..*n }).collect()),
        // Basse : la fondamentale de l'accord en cours, à l'octave de la note précédente.
        LineKind::Bass => {
            let prev = from[0].pitch;
            let pitch = super::nearest_pitch(chord.root, prev, anchor.low, anchor.high);
            Some(vec![GenNote { start: at, len, pitch, velocity }])
        }
        // Mélodie : note voisine (même note ou degré conjoint), stable si elle tombe sur un temps fort.
        LineKind::Melody => {
            let prev = from[0].pitch;
            let mut options: Vec<u8> = [-1, 0, 1]
                .iter()
                .map(|&d| ctx.scale.step(prev, d))
                .filter(|n| (anchor.low..=anchor.high).contains(n))
                .filter(|n| !is_strong(at) || tones.contains(n % 12))
                .collect();
            options.dedup();
            (!options.is_empty()).then(|| {
                let pitch = *rng.pick(&options);
                vec![GenNote { start: at, len, pitch, velocity }]
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::test_support::progression;
    use crate::generate::{bass, chords, melody};

    fn passes(kind: LineKind, count: usize, seed: u64) -> (Vec<Pattern>, crate::analysis::Progression) {
        let (prog, scale) = progression(8);
        let ctx = Context { progression: &prog, scale };
        let mut rng = Rng::new(seed);
        let base = match kind {
            LineKind::Melody => melody::generate(&ctx, &Default::default(), &mut rng),
            LineKind::Bass => bass::generate(&ctx, &Default::default(), &mut rng),
            LineKind::Chords => chords::generate(&ctx, &Default::default(), &mut rng),
        };
        // Variation volontairement forte pour éprouver les garde-fous.
        let p = VariationParams { silence: 0.3, pitch: 0.4, velocity: 0.6, length: 0.4, ..Default::default() };
        let mut out = vec![base.clone()];
        for _ in 1..count {
            let next = vary(out.last().unwrap(), &base, &ctx, kind, &p, &mut rng);
            out.push(next);
        }
        (out, prog)
    }

    #[test]
    fn each_pass_differs_from_the_previous() {
        let (ps, _) = passes(LineKind::Melody, 10, 1);
        let changed = ps.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(changed >= 8, "{changed}/9");
    }

    #[test]
    fn melody_stays_monophonic_in_scale_and_harmonic_after_many_passes() {
        let (ps, prog) = passes(LineKind::Melody, 60, 2);
        let scale = crate::theory::Scale::new(10, crate::theory::Mode::Major);
        let base_len = groups(&ps[0].notes).len();
        for pat in &ps {
            for w in pat.notes.windows(2) {
                assert!(w[0].start + w[0].len <= w[1].start);
            }
            for n in &pat.notes {
                assert!(scale.contains(n.pitch % 12));
                if is_strong(n.start) {
                    let chord = prog.chord_at(n.start).unwrap();
                    // Une 7e ou 9e écrite par le modèle peut rester ; une note déplacée ou ajoutée, non.
                    let from_base = ps[0].notes.iter().any(|b| b.start == n.start && b.pitch == n.pitch);
                    assert!(
                        harmonic_tones(chord, &scale).contains(n.pitch % 12) || from_base,
                        "temps fort hors accord au pas {}",
                        n.start
                    );
                }
            }
            let count = pat.notes.len();
            assert!(count >= base_len.div_ceil(2) && count <= base_len * 3 / 2 + 1, "{count} notes");
        }
    }

    #[test]
    fn bass_keeps_its_roots_on_chord_changes() {
        let (ps, prog) = passes(LineKind::Bass, 60, 3);
        for pat in &ps {
            for seg in &prog.segments {
                let n = pat.notes.iter().find(|n| n.start == seg.start).expect("attaque au changement");
                assert_eq!(n.pitch % 12, seg.chord.root);
            }
        }
    }

    #[test]
    fn chords_keep_three_voices_and_velocity_stays_anchored() {
        let (ps, _) = passes(LineKind::Chords, 60, 4);
        let base_vel = Anchor::of(&ps[0]).velocity;
        for pat in &ps {
            for (_, g) in groups(&pat.notes) {
                assert_eq!(g.len(), 3);
            }
            let v = Anchor::of(pat).velocity;
            assert!((v - base_vel).abs() <= 25, "vélocité moyenne {v} (modèle {base_vel})");
        }
    }

    #[test]
    fn density_oscillates_around_the_model() {
        for kind in [LineKind::Melody, LineKind::Bass, LineKind::Chords] {
            let (ps, _) = passes(kind, 100, 6);
            let base = groups(&ps[0].notes).len() as f32;
            let mean = ps[50..].iter().map(|p| groups(&p.notes).len() as f32).sum::<f32>() / 50.0;
            assert!((mean / base - 1.0).abs() < 0.2, "{kind:?} : {mean} attaques en moyenne (modèle {base})");
        }
    }

    #[test]
    fn same_seed_same_passes() {
        assert_eq!(passes(LineKind::Melody, 8, 5).0, passes(LineKind::Melody, 8, 5).0);
    }
}
