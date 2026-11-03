//! Analyse d'une progression captée : reconnaissance d'accords, découpage en
//! segments au pas de double croche, et estimation de la tonalité.

use crate::theory::{interval, pc_of, Chord, Mode, PcSet, PitchClass, Quality, Scale};
use crate::STEPS_PER_BEAT;

/// Note captée, quantifiée au pas (double croche). `end` est exclusif.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TimedNote {
    pub start: u32,
    pub end: u32,
    pub pitch: u8,
    pub velocity: u8,
}

impl TimedNote {
    /// Construit une note à partir de positions en temps, arrondies au pas le plus proche.
    pub fn from_beats(start: f64, duration: f64, pitch: u8, velocity: u8) -> Self {
        let s = (start * STEPS_PER_BEAT as f64).round().max(0.0) as u32;
        let e = ((start + duration) * STEPS_PER_BEAT as f64).round() as u32;
        Self {
            start: s,
            end: e.max(s + 1),
            pitch,
            velocity,
        }
    }

    fn sounds_at(&self, step: u32) -> bool {
        self.start <= step && step < self.end
    }
}

/// Poids par classe de hauteur (durée cumulée, en pas).
pub type PcWeights = [f32; 12];

/// Seuil (relatif au poids maximal) au-dessus duquel une note compte comme extension.
const EXTENSION_THRESHOLD: f32 = 0.25;

/// Reconnaît l'accord le plus plausible à partir des poids de chaque classe de
/// hauteur et de la note de basse. Renvoie `None` si aucune note ne sonne.
pub fn recognize(weights: &PcWeights, bass: Option<PitchClass>) -> Option<Chord> {
    recognize_with_bias(weights, bass, None)
}

/// Bonus donné au candidat dont la fondamentale est à la basse.
const BASS_ROOT_BONUS: f32 = 1.0;

/// Bonus donné à la fondamentale de l'accord en cours : une note de couleur
/// passagère (Eb6 ↔ Cm7 sur la même basse) ne fait pas basculer l'analyse.
const CURRENT_ROOT_BONUS: f32 = 0.8;

fn recognize_with_bias(
    weights: &PcWeights,
    bass: Option<PitchClass>,
    current_root: Option<PitchClass>,
) -> Option<Chord> {
    let max = weights.iter().cloned().fold(0.0, f32::max);
    if max <= 0.0 {
        return None;
    }
    let w: Vec<f32> = weights.iter().map(|x| x / max).collect();
    let present = PcSet::from_pcs((0..12).filter(|&pc| w[pc as usize] > 0.0));

    let mut best: Option<(f32, Chord)> = None;
    // Meilleur candidat dont la fondamentale est la basse.
    let mut best_on_bass: Option<(f32, Chord)> = None;
    for root in 0..12u8 {
        for quality in Quality::ALL {
            let tones = PcSet::from_intervals(root, quality.intervals());
            let mut score = 0.0;
            for &i in quality.intervals() {
                let pc = (root + i) % 12;
                score += if present.contains(pc) {
                    w[pc as usize]
                } else {
                    // Une fondamentale ou une tierce absente pèse plus qu'une quinte absente.
                    match i {
                        0 => -1.5,
                        3 | 4 => -1.0,
                        7 => -0.5,
                        _ => -0.8,
                    }
                };
            }
            for pc in present.iter().filter(|&pc| !tones.contains(pc)) {
                // Les 9e, 11e et 13e sont des couleurs courantes : peu pénalisées.
                let penalty = match interval(root, pc) {
                    2 | 5 | 9 => 0.3,
                    _ => 0.8,
                };
                score -= w[pc as usize] * penalty;
            }
            // Une basse sur la fondamentale est l'indice le plus fiable (accords à l'état fondamental).
            if bass == Some(root) {
                score += BASS_ROOT_BONUS;
            }
            if current_root == Some(root) {
                score += CURRENT_ROOT_BONUS;
            }
            score -= quality.complexity() * 0.15;

            let beats = |slot: &Option<(f32, Chord)>| slot.as_ref().map_or(true, |(s, _)| score > *s);
            let on_bass = bass == Some(root);
            if beats(&best) || (on_bass && beats(&best_on_bass)) {
                let mut chord = Chord::new(root, quality);
                chord.bass = bass.unwrap_or(root);
                chord.extensions = PcSet::from_pcs(present.iter().filter(|&pc| {
                    !tones.contains(pc)
                        && matches!(interval(root, pc), 2 | 5 | 9)
                        && w[pc as usize] >= EXTENSION_THRESHOLD
                }));
                if on_bass && beats(&best_on_bass) {
                    best_on_bass = Some((score, chord));
                }
                if beats(&best) {
                    best = Some((score, chord));
                }
            }
        }
    }
    // Une basse étrangère à l'accord retenu (Fmaj7 sur G) est presque toujours la vraie
    // fondamentale (G11) : on la préfère si son meilleur candidat n'est pas loin derrière.
    match (best, best_on_bass, bass) {
        (Some((s, c)), Some((sb, cb)), Some(b)) if !c.tones().contains(b) && s - sb < FOREIGN_BASS_TOLERANCE => Some(cb),
        (best, _, _) => best.map(|(_, c)| c),
    }
}

/// Écart de score toléré pour préférer la basse comme fondamentale quand elle est étrangère à l'accord.
const FOREIGN_BASS_TOLERANCE: f32 = 1.5;

/// Accord occupant l'intervalle de pas `[start, end)`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Segment {
    pub start: u32,
    pub end: u32,
    pub chord: Chord,
}

impl Segment {
    pub fn len(&self) -> u32 {
        self.end - self.start
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Progression {
    /// Longueur totale de la boucle, en pas.
    pub length: u32,
    pub segments: Vec<Segment>,
}

impl Progression {
    /// Accord qui sonne au pas `step` (la progression boucle).
    pub fn chord_at(&self, step: u32) -> Option<&Chord> {
        let step = step % self.length.max(1);
        self.segments
            .iter()
            .find(|s| s.start <= step && step < s.end)
            .map(|s| &s.chord)
    }
}

/// Nombre de pas pendant lesquels chaque classe de hauteur sonne entre `start` et `end`,
/// et note la plus grave entendue. Une note doublée à l'octave ne compte qu'une fois.
fn weights_over(notes: &[TimedNote], start: u32, end: u32) -> (PcWeights, Option<u8>) {
    let mut w = [0.0f32; 12];
    let mut lowest: Option<u8> = None;
    for step in start..end {
        let mut pcs = PcSet::EMPTY;
        for n in notes.iter().filter(|n| n.sounds_at(step)) {
            pcs.insert(pc_of(n.pitch));
            lowest = Some(lowest.map_or(n.pitch, |l| l.min(n.pitch)));
        }
        for pc in pcs.iter() {
            w[pc as usize] += 1.0;
        }
    }
    (w, lowest)
}

/// Découpe les notes captées en segments d'accords.
///
/// Sur chaque attaque, l'accord qui sonne est reconnu en favorisant la
/// fondamentale en cours ; un nouveau segment commence seulement si une autre
/// fondamentale l'emporte malgré ce bonus. Une note de couleur qui alterne
/// au-dessus d'un même accord (D#3 ↔ D3 sur Cm) ou un renversement passager
/// ne crée donc pas de faux changement.
///
/// Les pas sans aucune note prolongent l'accord précédent, puis les segments
/// voisins de même harmonie sont fusionnés.
pub fn segment(notes: &[TimedNote], length: u32) -> Progression {
    // (début, fondamentale reconnue à l'attaque)
    let mut cuts: Vec<(u32, Option<PitchClass>)> = vec![(0, None)];
    let mut current_root: Option<PitchClass> = None;
    let mut current_third: Option<u8> = None;
    let mut current_bass: Option<PitchClass> = None;

    let mut onsets: Vec<u32> = notes.iter().map(|n| n.start).filter(|&s| s < length).collect();
    onsets.sort_unstable();
    onsets.dedup();

    for &step in &onsets {
        let sounding: Vec<TimedNote> = notes.iter().copied().filter(|n| n.sounds_at(step)).collect();
        let (w, lowest) = weights_over(&sounding, step, step + 1);
        let bass = lowest.map(pc_of);
        // Basse inchangée : on favorise l'accord en cours. Basse qui bouge : jugement neutre,
        // sinon deux accords voisins (Bb et Gm7, trois notes communes) seraient confondus.
        let bias = if bass == current_bass { current_root } else { None };
        let Some(chord) = recognize_with_bias(&w, bass, bias) else { continue };
        // Tierce entendue (3 = mineure, 4 = majeure) : Dm → D sur la même fondamentale est un changement.
        let third = chord.quality.intervals().iter().copied().find(|&i| i == 3 || i == 4);
        match current_root {
            None => cuts[0].1 = Some(chord.root),
            Some(r) if r != chord.root => cuts.push((step, Some(chord.root))),
            Some(_) if third.is_some() && current_third.is_some() && third != current_third => {
                cuts.push((step, Some(chord.root)))
            }
            _ => {}
        }
        current_root = Some(chord.root);
        if third.is_some() {
            current_third = third;
        }
        current_bass = bass;
    }

    let mut segments: Vec<Segment> = Vec::new();
    for (i, &(start, root)) in cuts.iter().enumerate() {
        let end = cuts.get(i + 1).map_or(length, |c| c.0);
        if end <= start {
            continue;
        }
        // Le nom du segment garde la fondamentale entendue à son attaque :
        // sur Eb/Bb, une note du haut qui passe à C (notes de Cm7) reste un Eb.
        let (w, lowest) = weights_over(notes, start, end);
        let chord = match recognize_with_bias(&w, lowest.map(pc_of), root) {
            Some(c) => c,
            // Segment silencieux : prolonge l'accord précédent.
            None => match segments.last_mut() {
                Some(prev) => {
                    prev.end = end;
                    continue;
                }
                None => continue,
            },
        };
        match segments.last_mut() {
            Some(prev) if prev.chord.same_harmony(&chord) => prev.end = end,
            _ => segments.push(Segment { start, end, chord }),
        }
    }
    if let Some(first) = segments.first_mut() {
        first.start = 0;
    }
    Progression { length, segments }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct KeyEstimate {
    pub scale: Scale,
    pub score: f32,
}

/// Résultat de l'estimation de tonalité.
#[derive(Clone, PartialEq, Debug)]
pub struct KeyAnalysis {
    /// Candidats triés du plus au moins probable.
    pub candidates: Vec<KeyEstimate>,
}

/// Écart de score en dessous duquel deux tonalités sont jugées aussi plausibles.
const KEY_MARGIN: f32 = 0.1;

impl KeyAnalysis {
    pub fn best(&self) -> Option<Scale> {
        self.candidates.first().map(|k| k.scale)
    }

    /// Tonalités presque à égalité avec la meilleure (elle comprise). Typiquement
    /// une gamme et sa relative, qui ont les mêmes notes : seule l'oreille tranche,
    /// d'où le choix laissé à l'utilisateur.
    pub fn plausible(&self) -> Vec<Scale> {
        let Some(best) = self.candidates.first() else { return Vec::new() };
        let mut out: Vec<Scale> = self
            .candidates
            .iter()
            .take_while(|k| best.score - k.score < KEY_MARGIN)
            .map(|k| k.scale)
            .collect();
        // La relative a exactement les mêmes notes : elle reste toujours proposée.
        let relative = best.scale.relative();
        if !out.contains(&relative) {
            out.push(relative);
        }
        out
    }

    pub fn is_ambiguous(&self) -> bool {
        self.plausible().len() > 1
    }
}

/// Estime la tonalité d'une progression.
///
/// 1. Adéquation des notes à la gamme (durées pondérées, notes hors gamme pénalisées).
/// 2. Indices de tonique : durée des accords sur la tonique (de la bonne qualité),
///    premier et dernier accord, enchaînement dominante → tonique (en boucle).
pub fn detect_key(notes: &[TimedNote], progression: &Progression) -> KeyAnalysis {
    let (w, _) = weights_over(notes, 0, progression.length.max(1));
    let total: f32 = w.iter().sum();
    let segs = &progression.segments;
    let mut candidates = Vec::new();
    if total <= 0.0 || segs.is_empty() {
        return KeyAnalysis { candidates };
    }

    for tonic in 0..12u8 {
        for mode in Mode::ALL {
            let scale = Scale::new(tonic, mode);
            let pcs = scale.pcs();
            let inside: f32 = (0..12).filter(|&p| pcs.contains(p)).map(|p| w[p as usize]).sum();
            let fit = (inside - 3.0 * (total - inside)) / total;

            let is_tonic_chord = |c: &Chord| {
                c.root == tonic
                    && (!c.quality.has_third()
                        || matches!(c.quality, Quality::Minor | Quality::Minor7 | Quality::Minor6)
                            == mode.is_minor())
            };
            let tonic_len: u32 = segs.iter().filter(|s| is_tonic_chord(&s.chord)).map(|s| s.len()).sum();
            // Pas de bonus pour le premier accord : une boucle commence souvent ailleurs
            // que sur la tonique (ii-IV-I-vi-V).
            let mut cues = 0.3 * tonic_len as f32 / progression.length as f32;
            if is_tonic_chord(&segs[segs.len() - 1].chord) {
                cues += 0.1;
            }
            // Cadences vers la tonique (la boucle est cyclique) : V→I, IV→I.
            let resolves_from = |degree: u8| {
                let from = (tonic + degree) % 12;
                (0..segs.len()).any(|i| {
                    let next = &segs[(i + 1) % segs.len()].chord;
                    segs[i].chord.root == from && is_tonic_chord(next)
                })
            };
            if resolves_from(7) {
                cues += 0.1;
            }
            if resolves_from(5) {
                cues += 0.05;
            }
            // Le mineur harmonique n'est retenu que si sa sensible est entendue.
            let penalty = if mode == Mode::HarmonicMinor { 0.02 } else { 0.0 };
            candidates.push(KeyEstimate {
                scale,
                score: fit + cues - penalty,
            });
        }
    }
    candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    KeyAnalysis { candidates }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: u32 = 4 * STEPS_PER_BEAT;

    fn chord_notes(start: u32, end: u32, pitches: &[u8]) -> Vec<TimedNote> {
        pitches
            .iter()
            .map(|&pitch| TimedNote {
                start,
                end,
                pitch,
                velocity: 64,
            })
            .collect()
    }

    fn weights(pcs: &[u8]) -> PcWeights {
        let mut w = [0.0; 12];
        for &pc in pcs {
            w[pc as usize] = 1.0;
        }
        w
    }

    fn symbols(p: &Progression) -> Vec<(u32, String)> {
        p.segments.iter().map(|s| (s.start, s.chord.symbol())).collect()
    }

    #[test]
    fn recognizes_basic_chords() {
        assert_eq!(recognize(&weights(&[0, 4, 7]), Some(0)).unwrap().symbol(), "C");
        assert_eq!(recognize(&weights(&[0, 3, 7, 10]), Some(0)).unwrap().symbol(), "Cm7");
        // Mêmes notes, basse Eb : Eb6 plutôt que Cm7/Eb
        assert_eq!(recognize(&weights(&[0, 3, 7, 10]), Some(3)).unwrap().symbol(), "D#6");
        // Renversement
        assert_eq!(recognize(&weights(&[0, 4, 7]), Some(4)).unwrap().symbol(), "C/E");
        // Sans tierce : Eb Bb F
        assert_eq!(recognize(&weights(&[3, 10, 5]), Some(3)).unwrap().symbol(), "D#sus2");
    }

    /// Cm | Eb | Bb | Gm F — un accord par mesure, changement à la demi-mesure en fin.
    fn long_progression() -> Vec<TimedNote> {
        let mut n = Vec::new();
        n.extend(chord_notes(0, BAR, &[48, 55, 60, 63])); // Cm
        n.extend(chord_notes(BAR, 2 * BAR, &[51, 58, 63, 67])); // Eb
        n.extend(chord_notes(2 * BAR, 3 * BAR, &[46, 53, 58, 62])); // Bb
        n.extend(chord_notes(3 * BAR, 3 * BAR + 8, &[43, 50, 55, 58])); // Gm
        n.extend(chord_notes(3 * BAR + 8, 4 * BAR, &[41, 48, 53, 57])); // F
        n
    }

    #[test]
    fn segments_long_progression() {
        let p = segment(&long_progression(), 4 * BAR);
        assert_eq!(
            symbols(&p),
            vec![
                (0, "Cm".into()),
                (16, "D#".into()),
                (32, "A#".into()),
                (48, "Gm".into()),
                (56, "F".into()),
            ]
        );
    }

    #[test]
    fn segments_syncopated_changes() {
        // Changements à 0, 1.5, 4, 5.5 et 7 temps (fichiers « Short »).
        let mut n = Vec::new();
        n.extend(chord_notes(0, 6, &[48, 55, 63]));
        n.extend(chord_notes(6, 16, &[51, 58, 67]));
        n.extend(chord_notes(16, 22, &[46, 53, 62]));
        n.extend(chord_notes(22, 28, &[43, 50, 58]));
        n.extend(chord_notes(28, 32, &[41, 48, 57]));
        let p = segment(&n, 2 * BAR);
        let starts: Vec<u32> = p.segments.iter().map(|s| s.start).collect();
        assert_eq!(starts, vec![0, 6, 16, 22, 28]);
    }

    #[test]
    fn color_note_alternation_is_not_a_chord_change() {
        // Pluck : Cm rejoué tous les 3 pas, la note du haut alterne D#3 / D3.
        let mut n = Vec::new();
        for (i, start) in (0..BAR).step_by(3).enumerate() {
            let top = if i % 2 == 0 { 63 } else { 62 };
            n.extend(chord_notes(start, (start + 3).min(BAR), &[48, 55, 60, top]));
        }
        let p = segment(&n, BAR);
        assert_eq!(p.segments.len(), 1);
        assert_eq!(p.segments[0].chord.symbol(), "Cm(add9)");
    }

    #[test]
    fn neighbour_chords_sharing_notes_are_split_on_bass_change() {
        // Bb puis Gm7 : trois notes communes, seule la basse distingue les deux accords.
        let mut n = chord_notes(0, BAR, &[46, 53, 58, 62]);
        n.extend(chord_notes(BAR, 2 * BAR, &[43, 50, 58, 65]));
        let p = segment(&n, 2 * BAR);
        assert_eq!(symbols(&p), vec![(0, "A#".into()), (16, "Gm7".into())]);
    }

    #[test]
    fn same_bass_color_change_keeps_the_chord() {
        // Eb/Bb dont la note du haut passe de D# à C (notes de Cm7/Bb) : reste un Eb.
        let mut n = chord_notes(0, 6, &[46, 51, 55, 58, 63]);
        n.extend(chord_notes(6, 9, &[46, 51, 55, 58, 60]));
        n.extend(chord_notes(9, BAR, &[46, 51, 55, 58, 63]));
        let p = segment(&n, BAR);
        assert_eq!(p.segments.len(), 1);
        assert_eq!(p.segments[0].chord.root, 3);
    }

    #[test]
    fn detects_key_with_relative_ambiguity() {
        let notes = long_progression();
        let p = segment(&notes, 4 * BAR);
        let k = detect_key(&notes, &p);
        let plausible = k.plausible();
        assert!(plausible.contains(&Scale::new(10, Mode::Major)));
        assert!(plausible.contains(&Scale::new(7, Mode::NaturalMinor)));
        assert!(k.is_ambiguous());
    }

    #[test]
    fn detects_harmonic_minor_from_leading_tone() {
        // Am | Dm | E | Am : le G# impose le mineur harmonique.
        let mut n = Vec::new();
        n.extend(chord_notes(0, BAR, &[57, 60, 64]));
        n.extend(chord_notes(BAR, 2 * BAR, &[50, 57, 62, 65]));
        n.extend(chord_notes(2 * BAR, 3 * BAR, &[52, 56, 59, 64]));
        n.extend(chord_notes(3 * BAR, 4 * BAR, &[57, 60, 64]));
        let p = segment(&n, 4 * BAR);
        let k = detect_key(&n, &p);
        assert_eq!(k.best(), Some(Scale::new(9, Mode::HarmonicMinor)));
    }
}
