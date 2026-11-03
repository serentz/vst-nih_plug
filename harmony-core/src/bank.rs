//! Banques d'inspiration : statistiques tirées d'une bibliothèque de MIDI.
//!
//! Une banque ne contient **que des comptages** — jamais une note, un motif ou un rythme
//! recopié. Tout est exprimé de façon relative (place dans la mesure, fonction de la note
//! dans l'accord, degré de la gamme), pour s'appliquer à n'importe quelle progression.
//! Les comptages bruts sont conservés : deux banques peuvent être fusionnées, et les
//! probabilités sont calculées à la lecture.

use crate::analysis::{segment, Progression, TimedNote};
use crate::library::FileKind;
use crate::theory::{Role, Scale};
use crate::STEPS_PER_BEAT;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const BANK_VERSION: u32 = 1;
const STEPS_PER_BAR: u32 = 4 * STEPS_PER_BEAT;

/// Noms des fonctions d'une note dans l'accord, dans l'ordre des histogrammes.
pub const ROLE_NAMES: [&str; 8] = ["fondamentale", "tierce", "quinte", "7e", "9e", "11e", "13e", "autre"];

pub fn role_index(r: Role) -> usize {
    match r {
        Role::Root => 0,
        Role::Third => 1,
        Role::Fifth => 2,
        Role::Seventh => 3,
        Role::Ninth => 4,
        Role::Eleventh => 5,
        Role::Thirteenth => 6,
        Role::Other => 7,
    }
}

/// Fonction d'une note dans l'accord ; un accord capté sans tierce (quinte seule)
/// reçoit la tierce de la gamme, comme pour la génération.
fn role_in(chord: &crate::theory::Chord, scale: &Scale, pc: u8) -> Role {
    let i = crate::theory::interval(chord.root, pc);
    if !chord.quality.has_third() && (i == 3 || i == 4) && scale.contains(pc) {
        Role::Third
    } else {
        chord.role(pc)
    }
}

/// Place d'une mesure dans la phrase (A–A′ : deux moitiés).
pub const PHRASE_PLACES: [&str; 4] = ["début de moitié", "milieu", "fin de 1re moitié", "dernière mesure"];

pub fn phrase_place(bar: u32, bars: u32) -> usize {
    let half = (bars / 2).max(1);
    if bar + 1 == bars {
        3
    } else if bars > 1 && bar + 1 == half {
        2
    } else if bar % half == 0 {
        0
    } else {
        1
    }
}

/// Histogramme de comptages.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Hist {
    pub counts: Vec<f64>,
}

impl Hist {
    pub fn new(size: usize) -> Self {
        Self { counts: vec![0.0; size] }
    }

    pub fn add(&mut self, i: usize, w: f64) {
        if let Some(c) = self.counts.get_mut(i) {
            *c += w;
        }
    }

    pub fn total(&self) -> f64 {
        self.counts.iter().sum()
    }

    pub fn probs(&self) -> Vec<f64> {
        let t = self.total();
        self.counts.iter().map(|c| if t > 0.0 { c / t } else { 0.0 }).collect()
    }

    pub fn mean(&self) -> f64 {
        let t = self.total();
        if t > 0.0 {
            self.counts.iter().enumerate().map(|(i, c)| i as f64 * c).sum::<f64>() / t
        } else {
            0.0
        }
    }

    pub fn merge(&mut self, other: &Hist) {
        if self.counts.len() < other.counts.len() {
            self.counts.resize(other.counts.len(), 0.0);
        }
        for (a, b) in self.counts.iter_mut().zip(&other.counts) {
            *a += b;
        }
    }
}

/// Moyenne et écart-type accumulés.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Moments {
    pub n: f64,
    pub sum: f64,
    pub sum_sq: f64,
}

impl Moments {
    pub fn add(&mut self, x: f64) {
        self.n += 1.0;
        self.sum += x;
        self.sum_sq += x * x;
    }

    pub fn mean(&self) -> f64 {
        if self.n > 0.0 { self.sum / self.n } else { 0.0 }
    }

    pub fn sd(&self) -> f64 {
        if self.n > 1.0 { (self.sum_sq / self.n - self.mean().powi(2)).max(0.0).sqrt() } else { 0.0 }
    }

    pub fn merge(&mut self, o: &Moments) {
        self.n += o.n;
        self.sum += o.sum;
        self.sum_sq += o.sum_sq;
    }
}

/// Rythme et humanisation d'une famille de lignes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RhythmStats {
    /// Attaques comptées sur chaque pas de la mesure, par place dans la phrase.
    pub onsets: Vec<Vec<f64>>,
    /// Mesures comptées par place dans la phrase.
    pub bars: Vec<f64>,
    /// Durée des notes, en pas (0–32).
    pub length: Hist,
    /// Silence avant l'attaque suivante, en pas (0–16).
    pub gap: Hist,
    /// Attaques par mesure (0–16).
    pub per_bar: Hist,
    /// Vélocité selon le pas de la mesure.
    pub velocity: Vec<Moments>,
}

impl Default for RhythmStats {
    fn default() -> Self {
        Self {
            onsets: vec![vec![0.0; 16]; 4],
            bars: vec![0.0; 4],
            length: Hist::new(33),
            gap: Hist::new(17),
            per_bar: Hist::new(17),
            velocity: vec![Moments::default(); 16],
        }
    }
}

impl RhythmStats {
    /// Probabilité d'une attaque sur chaque pas, pour une place dans la phrase.
    pub fn onset_probs(&self, place: usize) -> Vec<f64> {
        let bars = self.bars[place];
        self.onsets[place].iter().map(|c| if bars > 0.0 { c / bars } else { 0.0 }).collect()
    }

    /// Ajoute des attaques `(pas, durée, vélocité)` triées, sur une boucle de `length` pas.
    fn add(&mut self, onsets: &[(u32, u32, u8)], length: u32) {
        let bars = length.div_ceil(STEPS_PER_BAR).max(1);
        for b in 0..bars {
            self.bars[phrase_place(b, bars)] += 1.0;
            let n = onsets.iter().filter(|o| o.0 / STEPS_PER_BAR == b).count();
            self.per_bar.add(n.min(16), 1.0);
        }
        for (i, &(start, len, vel)) in onsets.iter().enumerate() {
            let bar = start / STEPS_PER_BAR;
            let step = (start % STEPS_PER_BAR) as usize;
            self.onsets[phrase_place(bar, bars)][step] += 1.0;
            self.length.add(len.min(32) as usize, 1.0);
            self.velocity[step].add(vel as f64);
            if let Some(next) = onsets.get(i + 1) {
                self.gap.add(next.0.saturating_sub(start + len).min(16) as usize, 1.0);
            }
        }
    }

    fn merge(&mut self, o: &RhythmStats) {
        for (a, b) in self.onsets.iter_mut().zip(&o.onsets) {
            for (x, y) in a.iter_mut().zip(b) {
                *x += y;
            }
        }
        for (a, b) in self.bars.iter_mut().zip(&o.bars) {
            *a += b;
        }
        self.length.merge(&o.length);
        self.gap.merge(&o.gap);
        self.per_bar.merge(&o.per_bar);
        for (a, b) in self.velocity.iter_mut().zip(&o.velocity) {
            a.merge(b);
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MelodyStats {
    pub files: u32,
    pub rhythm: RhythmStats,
    /// Rythme par tempo (tempo minimal du dossier : 90, 115, 130, 160).
    pub by_tempo: BTreeMap<u32, RhythmStats>,
    /// Fonction des notes sur les temps 1 et 3, et ailleurs.
    pub strong_roles: Hist,
    pub weak_roles: Hist,
    /// Mouvement d'une note à la suivante en degrés de la gamme (−7…+7, indice + 7).
    pub degree_moves: Hist,
    /// Notes hors gamme.
    pub chromatic: f64,
    pub notes: f64,
    /// Sauts (> 4 demi-tons) et sauts suivis d'un retour conjoint en sens inverse.
    pub leaps: f64,
    pub leaps_recovered: f64,
    /// Mesures de la 2e moitié comparées à la 1re : même rythme, mêmes hauteurs.
    pub echo_compared: f64,
    pub echo_rhythm: f64,
    pub echo_pitches: f64,
    pub low: Moments,
    pub high: Moments,
}

impl Default for MelodyStats {
    fn default() -> Self {
        Self {
            files: 0,
            rhythm: RhythmStats::default(),
            by_tempo: BTreeMap::new(),
            strong_roles: Hist::new(8),
            weak_roles: Hist::new(8),
            degree_moves: Hist::new(15),
            chromatic: 0.0,
            notes: 0.0,
            leaps: 0.0,
            leaps_recovered: 0.0,
            echo_compared: 0.0,
            echo_rhythm: 0.0,
            echo_pitches: 0.0,
            low: Moments::default(),
            high: Moments::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BassStats {
    pub files: u32,
    pub rhythm: RhythmStats,
    pub roles: Hist,
    /// Mouvement d'une note à la suivante en demi-tons (−12…+12, indice + 12).
    pub moves: Hist,
    /// Changements d'accord observés, attaqués par la basse, attaqués sur la fondamentale.
    pub changes: f64,
    pub changes_hit: f64,
    pub changes_root: f64,
    pub low: Moments,
    pub high: Moments,
}

impl Default for BassStats {
    fn default() -> Self {
        Self {
            files: 0,
            rhythm: RhythmStats::default(),
            roles: Hist::new(8),
            moves: Hist::new(25),
            changes: 0.0,
            changes_hit: 0.0,
            changes_root: 0.0,
            low: Moments::default(),
            high: Moments::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ChordStats {
    pub files: u32,
    pub rhythm: RhythmStats,
    /// Nombre de notes par attaque (0–8).
    pub voices: Hist,
    /// Note de basse : fondamentale, tierce, quinte, autre.
    pub inversion: Hist,
    /// Écart entre la note la plus grave et la plus aiguë, en demi-tons (0–36).
    pub spread: Hist,
    pub top_roles: Hist,
    pub attacks: f64,
    pub doubled_root: f64,
    /// Attaques contenant une 7e, une 9e, une 11e ou une 13e.
    pub colored: f64,
    pub low: Moments,
    pub high: Moments,
}

impl Default for ChordStats {
    fn default() -> Self {
        Self {
            files: 0,
            rhythm: RhythmStats::default(),
            voices: Hist::new(9),
            inversion: Hist::new(4),
            spread: Hist::new(37),
            top_roles: Hist::new(8),
            attacks: 0.0,
            doubled_root: 0.0,
            colored: 0.0,
            low: Moments::default(),
            high: Moments::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ArpStats {
    pub files: u32,
    pub rhythm: RhythmStats,
    /// Mouvement en notes de l'accord empilées (−4…+4, indice + 4), sur un même accord.
    pub moves: Hist,
    /// Étendue parcourue par mesure, en demi-tons (0–36).
    pub span: Hist,
    pub roles: Hist,
    /// Attaques sur une grille de triolets plutôt que de doubles croches.
    pub triplet_onsets: f64,
    pub onsets: f64,
    pub low: Moments,
    pub high: Moments,
}

impl Default for ArpStats {
    fn default() -> Self {
        Self {
            files: 0,
            rhythm: RhythmStats::default(),
            moves: Hist::new(9),
            span: Hist::new(37),
            roles: Hist::new(8),
            triplet_onsets: 0.0,
            onsets: 0.0,
            low: Moments::default(),
            high: Moments::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Bank {
    pub version: u32,
    pub name: String,
    /// Origine des statistiques (chemin analysé), pour mémoire.
    pub source: String,
    pub melody: MelodyStats,
    pub bass: BassStats,
    pub fast_chords: ChordStats,
    pub slow_chords: ChordStats,
    pub arps: ArpStats,
}

/// Note lue dans un fichier, en temps (non quantifiée, pour reconnaître les triolets).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawNote {
    pub start: f64,
    pub len: f64,
    pub pitch: u8,
    pub velocity: u8,
}

impl RawNote {
    fn timed(&self) -> TimedNote {
        TimedNote::from_beats(self.start, self.len, self.pitch, self.velocity)
    }
}

/// Un fichier à analyser, avec son contexte.
pub struct FileInput<'a> {
    pub kind: FileKind,
    pub notes: &'a [RawNote],
    /// Gamme du dossier (pour les degrés).
    pub scale: Scale,
    /// Progressions du dossier (fichiers « Chord Progression » découpés), pour les lignes
    /// qui ne portent pas leur propre harmonie (basses).
    pub harmonies: &'a [Progression],
}

/// Sépare la voix du haut (mélodie, arpège) de l'accompagnement en accords mélangé dans le fichier.
/// L'accompagnement = notes des attaques à plusieurs notes, sauf la plus aiguë ; la voix du haut =
/// la note la plus aiguë de chaque attaque, si elle dépasse le haut de l'accompagnement.
pub fn split_top_line(notes: &[RawNote]) -> (Vec<RawNote>, Vec<RawNote>) {
    let mut groups: BTreeMap<i64, Vec<RawNote>> = BTreeMap::new();
    for n in notes {
        groups.entry((n.start * 48.0).round() as i64).or_default().push(*n);
    }
    let mut accompaniment: Vec<RawNote> = Vec::new();
    for g in groups.values_mut() {
        g.sort_by_key(|n| n.pitch);
        if g.len() > 1 {
            accompaniment.extend_from_slice(&g[..g.len() - 1]);
        }
    }
    let mut acc_pitches: Vec<u8> = accompaniment.iter().map(|n| n.pitch).collect();
    acc_pitches.sort_unstable();
    let ceiling = acc_pitches.get(acc_pitches.len() * 9 / 10).copied().unwrap_or(0);
    let mut line = Vec::new();
    for g in groups.values() {
        let top = *g.last().unwrap();
        if top.pitch > ceiling {
            line.push(top);
        } else if g.len() == 1 {
            accompaniment.push(top);
        }
    }
    (line, accompaniment)
}

fn loop_length(notes: &[TimedNote]) -> u32 {
    notes.iter().map(|n| n.end).max().unwrap_or(0).div_ceil(STEPS_PER_BAR).max(1) * STEPS_PER_BAR
}

/// Part (pondérée par la durée) des notes qui appartiennent à l'accord du moment.
fn fit(progression: &Progression, notes: &[TimedNote]) -> f64 {
    let (mut inside, mut total) = (0.0, 0.0);
    for n in notes {
        let w = (n.end - n.start) as f64;
        total += w;
        if progression.chord_at(n.start).is_some_and(|c| c.tones().contains(n.pitch % 12)) {
            inside += w;
        }
    }
    if total > 0.0 { inside / total } else { 0.0 }
}

fn best_harmony<'a>(candidates: &'a [Progression], notes: &[TimedNote]) -> Option<&'a Progression> {
    candidates.iter().max_by(|a, b| fit(a, notes).partial_cmp(&fit(b, notes)).unwrap())
}

/// Indice diatonique absolu : nombre de notes de la gamme sous `pitch`.
fn diatonic_index(scale: &Scale, pitch: u8) -> Option<i32> {
    scale
        .contains(pitch % 12)
        .then(|| (0..pitch).filter(|p| scale.contains(p % 12)).count() as i32)
}

/// Attaques d'une ligne : (pas, durée, vélocité) ; une seule par pas (la plus aiguë).
fn onsets_of(notes: &[TimedNote]) -> Vec<(u32, u32, u8)> {
    let mut by_step: BTreeMap<u32, (u32, u8)> = BTreeMap::new();
    for n in notes {
        let e = by_step.entry(n.start).or_insert((0, n.velocity));
        e.0 = e.0.max(n.end - n.start);
        e.1 = e.1.max(n.velocity);
    }
    by_step.into_iter().map(|(s, (l, v))| (s, l, v)).collect()
}

impl Bank {
    pub fn new(name: &str, source: &str) -> Self {
        Self {
            version: BANK_VERSION,
            name: name.to_string(),
            source: source.to_string(),
            melody: MelodyStats::default(),
            bass: BassStats::default(),
            fast_chords: ChordStats::default(),
            slow_chords: ChordStats::default(),
            arps: ArpStats::default(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let bank: Bank = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if bank.version != BANK_VERSION {
            return Err(format!("version de banque {} (attendue {BANK_VERSION})", bank.version));
        }
        Ok(bank)
    }

    /// Analyse un fichier et ajoute ses statistiques à la banque.
    pub fn add_file(&mut self, input: FileInput) {
        match input.kind {
            FileKind::Melody { bpm_min } => self.add_melody(&input, bpm_min),
            FileKind::Arp => self.add_arp(&input),
            FileKind::Bass => self.add_bass(&input),
            FileKind::FastChords => add_chords(&mut self.fast_chords, input.notes),
            FileKind::SlowChords => add_chords(&mut self.slow_chords, input.notes),
            FileKind::ChordProgression => {}
        }
    }

    fn add_melody(&mut self, input: &FileInput, bpm_min: u32) {
        let (line, acc) = split_top_line(input.notes);
        let line: Vec<TimedNote> = line.iter().map(RawNote::timed).collect();
        let acc: Vec<TimedNote> = acc.iter().map(RawNote::timed).collect();
        if line.len() < 4 {
            return;
        }
        let length = loop_length(&line).max(loop_length(&acc));
        let own = (!acc.is_empty()).then(|| segment(&acc, length));
        let Some(harmony) = own.as_ref().or_else(|| best_harmony(input.harmonies, &line)) else { return };
        let m = &mut self.melody;
        m.files += 1;
        let onsets = onsets_of(&line);
        m.rhythm.add(&onsets, length);
        m.by_tempo.entry(bpm_min).or_default().add(&onsets, length);

        for n in &line {
            let Some(chord) = harmony.chord_at(n.start) else { continue };
            let role = role_index(role_in(chord, &input.scale, n.pitch % 12));
            if n.start % 8 == 0 { m.strong_roles.add(role, 1.0) } else { m.weak_roles.add(role, 1.0) }
            m.notes += 1.0;
            if !input.scale.contains(n.pitch % 12) {
                m.chromatic += 1.0;
            }
        }
        for w in line.windows(3) {
            let semis = w[1].pitch as i32 - w[0].pitch as i32;
            if semis.abs() > 4 {
                m.leaps += 1.0;
                let next = w[2].pitch as i32 - w[1].pitch as i32;
                if next.signum() == -semis.signum() && next.abs() <= 2 {
                    m.leaps_recovered += 1.0;
                }
            }
        }
        for w in line.windows(2) {
            if let (Some(a), Some(b)) = (diatonic_index(&input.scale, w[0].pitch), diatonic_index(&input.scale, w[1].pitch)) {
                m.degree_moves.add(((b - a).clamp(-7, 7) + 7) as usize, 1.0);
            }
        }
        // Reprise A–A′ : chaque mesure de la 1re moitié (sauf la dernière) contre sa jumelle.
        let bars = length / STEPS_PER_BAR;
        let half = bars / 2;
        for b in 0..half.saturating_sub(1) {
            let bar = |k: u32| -> Vec<(u32, u8)> {
                line.iter()
                    .filter(|n| n.start / STEPS_PER_BAR == k)
                    .map(|n| (n.start % STEPS_PER_BAR, n.pitch))
                    .collect()
            };
            let (a, b2) = (bar(b), bar(b + half));
            m.echo_compared += 1.0;
            if a.iter().map(|x| x.0).eq(b2.iter().map(|x| x.0)) {
                m.echo_rhythm += 1.0;
                if a == b2 {
                    m.echo_pitches += 1.0;
                }
            }
        }
        m.low.add(line.iter().map(|n| n.pitch).min().unwrap() as f64);
        m.high.add(line.iter().map(|n| n.pitch).max().unwrap() as f64);
    }

    fn add_bass(&mut self, input: &FileInput) {
        let line: Vec<TimedNote> = input.notes.iter().map(RawNote::timed).collect();
        if line.len() < 2 {
            return;
        }
        let Some(harmony) = best_harmony(input.harmonies, &line) else { return };
        let length = loop_length(&line);
        let b = &mut self.bass;
        b.files += 1;
        b.rhythm.add(&onsets_of(&line), length);
        for n in &line {
            if let Some(chord) = harmony.chord_at(n.start) {
                b.roles.add(role_index(role_in(chord, &input.scale, n.pitch % 12)), 1.0);
            }
        }
        for w in line.windows(2) {
            b.moves.add(((w[1].pitch as i32 - w[0].pitch as i32).clamp(-12, 12) + 12) as usize, 1.0);
        }
        // Changements d'accord de la progression, répétée sur toute la ligne.
        let reps = length.div_ceil(harmony.length.max(1));
        for r in 0..reps {
            for seg in &harmony.segments {
                let at = r * harmony.length + seg.start;
                if at >= length {
                    continue;
                }
                b.changes += 1.0;
                if let Some(n) = line.iter().find(|n| n.start == at) {
                    b.changes_hit += 1.0;
                    if n.pitch % 12 == seg.chord.root {
                        b.changes_root += 1.0;
                    }
                }
            }
        }
        b.low.add(line.iter().map(|n| n.pitch).min().unwrap() as f64);
        b.high.add(line.iter().map(|n| n.pitch).max().unwrap() as f64);
    }

    fn add_arp(&mut self, input: &FileInput) {
        let (raw_line, acc) = split_top_line(input.notes);
        let line: Vec<TimedNote> = raw_line.iter().map(RawNote::timed).collect();
        let acc: Vec<TimedNote> = acc.iter().map(RawNote::timed).collect();
        if line.len() < 4 {
            return;
        }
        let length = loop_length(&line).max(loop_length(&acc));
        let own = (!acc.is_empty()).then(|| segment(&acc, length));
        let Some(harmony) = own.as_ref().or_else(|| best_harmony(input.harmonies, &line)) else { return };
        let a = &mut self.arps;
        a.files += 1;
        a.rhythm.add(&onsets_of(&line), length);
        for n in &raw_line {
            a.onsets += 1.0;
            let sixteenth = (n.start * 4.0 - (n.start * 4.0).round()).abs() < 0.05;
            let triplet = (n.start * 12.0 - (n.start * 12.0).round()).abs() < 0.05;
            if triplet && !sixteenth {
                a.triplet_onsets += 1.0;
            }
        }
        for n in &line {
            if let Some(chord) = harmony.chord_at(n.start) {
                a.roles.add(role_index(role_in(chord, &input.scale, n.pitch % 12)), 1.0);
            }
        }
        // Position dans l'accord empilé : nombre de notes de l'accord sous la note.
        for w in line.windows(2) {
            let (Some(c0), Some(c1)) = (harmony.chord_at(w[0].start), harmony.chord_at(w[1].start)) else { continue };
            if !c0.same_harmony(c1) {
                continue;
            }
            let tones = c0.tones();
            let index = |p: u8| (0..p).filter(|q| tones.contains(q % 12)).count() as i32;
            a.moves.add(((index(w[1].pitch) - index(w[0].pitch)).clamp(-4, 4) + 4) as usize, 1.0);
        }
        for bar in 0..length / STEPS_PER_BAR {
            let pitches: Vec<u8> = line.iter().filter(|n| n.start / STEPS_PER_BAR == bar).map(|n| n.pitch).collect();
            if let (Some(lo), Some(hi)) = (pitches.iter().min(), pitches.iter().max()) {
                a.span.add((hi - lo).min(36) as usize, 1.0);
            }
        }
        a.low.add(line.iter().map(|n| n.pitch).min().unwrap() as f64);
        a.high.add(line.iter().map(|n| n.pitch).max().unwrap() as f64);
    }

    /// Fusionne une autre banque dans celle-ci.
    pub fn merge(&mut self, o: &Bank) {
        let m = &mut self.melody;
        m.files += o.melody.files;
        m.rhythm.merge(&o.melody.rhythm);
        for (k, v) in &o.melody.by_tempo {
            m.by_tempo.entry(*k).or_default().merge(v);
        }
        m.strong_roles.merge(&o.melody.strong_roles);
        m.weak_roles.merge(&o.melody.weak_roles);
        m.degree_moves.merge(&o.melody.degree_moves);
        m.chromatic += o.melody.chromatic;
        m.notes += o.melody.notes;
        m.leaps += o.melody.leaps;
        m.leaps_recovered += o.melody.leaps_recovered;
        m.echo_compared += o.melody.echo_compared;
        m.echo_rhythm += o.melody.echo_rhythm;
        m.echo_pitches += o.melody.echo_pitches;
        m.low.merge(&o.melody.low);
        m.high.merge(&o.melody.high);
        let b = &mut self.bass;
        b.files += o.bass.files;
        b.rhythm.merge(&o.bass.rhythm);
        b.roles.merge(&o.bass.roles);
        b.moves.merge(&o.bass.moves);
        b.changes += o.bass.changes;
        b.changes_hit += o.bass.changes_hit;
        b.changes_root += o.bass.changes_root;
        b.low.merge(&o.bass.low);
        b.high.merge(&o.bass.high);
        for (c, oc) in [(&mut self.fast_chords, &o.fast_chords), (&mut self.slow_chords, &o.slow_chords)] {
            c.files += oc.files;
            c.rhythm.merge(&oc.rhythm);
            c.voices.merge(&oc.voices);
            c.inversion.merge(&oc.inversion);
            c.spread.merge(&oc.spread);
            c.top_roles.merge(&oc.top_roles);
            c.attacks += oc.attacks;
            c.doubled_root += oc.doubled_root;
            c.colored += oc.colored;
            c.low.merge(&oc.low);
            c.high.merge(&oc.high);
        }
        let a = &mut self.arps;
        a.files += o.arps.files;
        a.rhythm.merge(&o.arps.rhythm);
        a.moves.merge(&o.arps.moves);
        a.span.merge(&o.arps.span);
        a.roles.merge(&o.arps.roles);
        a.triplet_onsets += o.arps.triplet_onsets;
        a.onsets += o.arps.onsets;
        a.low.merge(&o.arps.low);
        a.high.merge(&o.arps.high);
    }
}

/// Accords rythmés : l'harmonie est celle que jouent les accords eux-mêmes.
fn add_chords(c: &mut ChordStats, raw: &[RawNote]) {
    let notes: Vec<TimedNote> = raw.iter().map(RawNote::timed).collect();
    if notes.is_empty() {
        return;
    }
    let length = loop_length(&notes);
    let harmony = segment(&notes, length);
    c.files += 1;
    c.rhythm.add(&onsets_of(&notes), length);
    let mut groups: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    for n in &notes {
        groups.entry(n.start).or_default().push(n.pitch);
    }
    for (start, mut pitches) in groups {
        pitches.sort_unstable();
        let Some(chord) = harmony.chord_at(start) else { continue };
        c.attacks += 1.0;
        c.voices.add(pitches.len().min(8), 1.0);
        let bass_role = match chord.role(pitches[0] % 12) {
            Role::Root => 0,
            Role::Third => 1,
            Role::Fifth => 2,
            _ => 3,
        };
        c.inversion.add(bass_role, 1.0);
        c.spread.add((pitches[pitches.len() - 1] - pitches[0]).min(36) as usize, 1.0);
        c.top_roles.add(role_index(chord.role(pitches[pitches.len() - 1] % 12)), 1.0);
        if pitches.iter().filter(|p| *p % 12 == chord.root).count() >= 2 {
            c.doubled_root += 1.0;
        }
        if pitches.iter().any(|p| {
            matches!(chord.role(p % 12), Role::Seventh | Role::Ninth | Role::Eleventh | Role::Thirteenth)
        }) {
            c.colored += 1.0;
        }
        c.low.add(pitches[0] as f64);
        c.high.add(pitches[pitches.len() - 1] as f64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theory::Mode;

    fn raw(start: f64, len: f64, pitch: u8) -> RawNote {
        RawNote { start, len, pitch, velocity: 80 }
    }

    /// Accompagnement Cm | Eb (2 mesures) + mélodie au-dessus.
    fn melody_file() -> Vec<RawNote> {
        let mut n = vec![raw(0.0, 4.0, 48), raw(0.0, 4.0, 55), raw(4.0, 4.0, 51), raw(4.0, 4.0, 58)];
        for (i, p) in [75u8, 74, 72, 70, 67, 70, 72, 74].iter().enumerate() {
            n.push(raw(i as f64, 0.5, *p));
        }
        n
    }

    #[test]
    fn top_line_is_split_from_accompaniment() {
        let (line, acc) = split_top_line(&melody_file());
        assert_eq!(line.len(), 8);
        assert_eq!(acc.len(), 4);
        assert!(line.iter().all(|n| n.pitch >= 67));
    }

    #[test]
    fn melody_statistics_are_counts_only_and_survive_json() {
        let mut bank = Bank::new("test", "mémoire");
        let notes = melody_file();
        bank.add_file(FileInput {
            kind: FileKind::Melody { bpm_min: 130 },
            notes: &notes,
            scale: Scale::new(10, Mode::Major),
            harmonies: &[],
        });
        let m = &bank.melody;
        assert_eq!(m.files, 1);
        assert_eq!(m.notes, 8.0);
        // Eb sur Cm au temps 1 : tierce ; G sur Eb au temps 5 (pas 16) : tierce.
        assert_eq!(m.strong_roles.counts[1], 2.0);
        // Boucle de 2 mesures : la mesure 1 est aussi la fin de la 1re moitié.
        assert_eq!(m.rhythm.onset_probs(2)[0], 1.0);
        assert!(m.by_tempo.contains_key(&130));
        let back = Bank::from_json(&bank.to_json()).unwrap();
        assert_eq!(back, bank);
    }

    #[test]
    fn merging_adds_counts() {
        let mut a = Bank::new("a", "");
        let notes = melody_file();
        let input = || FileInput {
            kind: FileKind::Melody { bpm_min: 130 },
            notes: &notes,
            scale: Scale::new(10, Mode::Major),
            harmonies: &[],
        };
        a.add_file(input());
        let mut b = a.clone();
        b.merge(&a);
        assert_eq!(b.melody.files, 2);
        assert_eq!(b.melody.notes, 16.0);
    }

    #[test]
    fn phrase_places() {
        let places: Vec<usize> = (0..8).map(|b| phrase_place(b, 8)).collect();
        assert_eq!(places, vec![0, 1, 1, 2, 0, 1, 1, 3]);
    }
}
