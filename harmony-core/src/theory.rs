//! Notes, ensembles de classes de hauteur, accords et gammes.

use std::fmt;

/// Classe de hauteur : 0 = C, 1 = C#, … 11 = B.
pub type PitchClass = u8;

pub const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

pub fn pc_of(midi: u8) -> PitchClass {
    midi % 12
}

pub fn pc_name(pc: PitchClass) -> &'static str {
    NOTE_NAMES[(pc % 12) as usize]
}

/// Nom d'une note MIDI dans la convention d'Ableton Live (60 = C3).
pub fn note_name(midi: u8) -> String {
    format!("{}{}", pc_name(pc_of(midi)), midi as i32 / 12 - 2)
}

/// Intervalle ascendant de `from` vers `to`, en demi-tons (0..12).
pub fn interval(from: PitchClass, to: PitchClass) -> u8 {
    (to + 12 - from % 12) % 12
}

/// Ensemble de classes de hauteur (bit n = classe n).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PcSet(u16);

impl PcSet {
    pub const EMPTY: PcSet = PcSet(0);

    pub fn from_pcs(pcs: impl IntoIterator<Item = PitchClass>) -> Self {
        let mut s = Self::EMPTY;
        for pc in pcs {
            s.insert(pc);
        }
        s
    }

    /// Ensemble formé par `root` + chaque intervalle.
    pub fn from_intervals(root: PitchClass, intervals: &[u8]) -> Self {
        Self::from_pcs(intervals.iter().map(|i| (root + i) % 12))
    }

    pub fn insert(&mut self, pc: PitchClass) {
        self.0 |= 1 << (pc % 12);
    }

    pub fn contains(self, pc: PitchClass) -> bool {
        self.0 & (1 << (pc % 12)) != 0
    }

    pub fn len(self) -> u32 {
        self.0.count_ones()
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn iter(self) -> impl Iterator<Item = PitchClass> {
        (0..12).filter(move |&pc| self.contains(pc))
    }
}

impl fmt::Debug for PcSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<_> = self.iter().map(pc_name).collect();
        write!(f, "{{{}}}", names.join(" "))
    }
}

/// Qualité d'un accord, définie par ses intervalles depuis la fondamentale.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Quality {
    Major,
    Minor,
    Diminished,
    Augmented,
    Sus2,
    Sus4,
    /// Fondamentale + quinte, sans tierce.
    Power,
    Major7,
    Minor7,
    Dominant7,
    HalfDiminished7,
    Diminished7,
    Major6,
    Minor6,
}

impl Quality {
    pub const ALL: [Quality; 14] = [
        Quality::Major,
        Quality::Minor,
        Quality::Diminished,
        Quality::Augmented,
        Quality::Sus2,
        Quality::Sus4,
        Quality::Power,
        Quality::Major7,
        Quality::Minor7,
        Quality::Dominant7,
        Quality::HalfDiminished7,
        Quality::Diminished7,
        Quality::Major6,
        Quality::Minor6,
    ];

    pub fn intervals(self) -> &'static [u8] {
        match self {
            Quality::Major => &[0, 4, 7],
            Quality::Minor => &[0, 3, 7],
            Quality::Diminished => &[0, 3, 6],
            Quality::Augmented => &[0, 4, 8],
            Quality::Sus2 => &[0, 2, 7],
            Quality::Sus4 => &[0, 5, 7],
            Quality::Power => &[0, 7],
            Quality::Major7 => &[0, 4, 7, 11],
            Quality::Minor7 => &[0, 3, 7, 10],
            Quality::Dominant7 => &[0, 4, 7, 10],
            Quality::HalfDiminished7 => &[0, 3, 6, 10],
            Quality::Diminished7 => &[0, 3, 6, 9],
            Quality::Major6 => &[0, 4, 7, 9],
            Quality::Minor6 => &[0, 3, 7, 9],
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            Quality::Major => "",
            Quality::Minor => "m",
            Quality::Diminished => "dim",
            Quality::Augmented => "aug",
            Quality::Sus2 => "sus2",
            Quality::Sus4 => "sus4",
            Quality::Power => "5",
            Quality::Major7 => "maj7",
            Quality::Minor7 => "m7",
            Quality::Dominant7 => "7",
            Quality::HalfDiminished7 => "m7b5",
            Quality::Diminished7 => "dim7",
            Quality::Major6 => "6",
            Quality::Minor6 => "m6",
        }
    }

    /// Coût de complexité : à score égal, l'accord le plus simple l'emporte.
    pub fn complexity(self) -> f32 {
        match self {
            Quality::Major | Quality::Minor => 0.0,
            Quality::Power => 0.2,
            Quality::Sus2 | Quality::Sus4 => 0.4,
            Quality::Minor7 | Quality::Dominant7 | Quality::Major7 => 0.5,
            Quality::Diminished | Quality::Major6 | Quality::Minor6 => 0.7,
            Quality::HalfDiminished7 => 0.8,
            Quality::Augmented | Quality::Diminished7 => 1.0,
        }
    }

    /// Vrai si la qualité contient une tierce (majeure ou mineure).
    pub fn has_third(self) -> bool {
        self.intervals().iter().any(|&i| i == 3 || i == 4)
    }
}

/// Fonction d'une note par rapport à un accord.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Role {
    Root,
    Third,
    Fifth,
    Seventh,
    Ninth,
    Eleventh,
    Thirteenth,
    /// Note étrangère à l'accord et sans fonction d'extension usuelle.
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Chord {
    pub root: PitchClass,
    pub quality: Quality,
    /// Note la plus grave (renversement si différente de la fondamentale).
    pub bass: PitchClass,
    /// Extensions entendues (9e, 11e, 13e) en plus de la qualité.
    pub extensions: PcSet,
}

impl Chord {
    pub fn new(root: PitchClass, quality: Quality) -> Self {
        Self {
            root,
            quality,
            bass: root,
            extensions: PcSet::EMPTY,
        }
    }

    /// Notes de l'accord (hors extensions).
    pub fn tones(&self) -> PcSet {
        PcSet::from_intervals(self.root, self.quality.intervals())
    }

    pub fn role(&self, pc: PitchClass) -> Role {
        let i = interval(self.root, pc);
        if self.tones().contains(pc) {
            return match i {
                0 => Role::Root,
                3 | 4 => Role::Third,
                6 | 7 | 8 => Role::Fifth,
                // 6e des accords de sixte et 7e diminuée : traitées comme 7e
                9 | 10 | 11 => Role::Seventh,
                2 | 5 => Role::Third, // sus2 / sus4 remplacent la tierce
                _ => Role::Other,
            };
        }
        match i {
            2 => Role::Ninth,
            5 => Role::Eleventh,
            9 => Role::Thirteenth,
            10 | 11 => Role::Seventh,
            _ => Role::Other,
        }
    }

    /// Symbole usuel : `Cm`, `Cm(add9)`, `Eb/G`.
    pub fn symbol(&self) -> String {
        let mut s = format!("{}{}", pc_name(self.root), self.quality.suffix());
        let ext: Vec<&str> = self
            .extensions
            .iter()
            .map(|pc| match interval(self.root, pc) {
                2 => "add9",
                5 => "add11",
                9 => "add13",
                _ => "?",
            })
            .collect();
        if !ext.is_empty() {
            s.push_str(&format!("({})", ext.join(",")));
        }
        if self.bass != self.root {
            s.push('/');
            s.push_str(pc_name(self.bass));
        }
        s
    }

    /// Même accord, sans tenir compte du renversement ni des extensions.
    pub fn same_harmony(&self, other: &Chord) -> bool {
        self.root == other.root && self.quality == other.quality
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.symbol())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Mode {
    Major,
    NaturalMinor,
    HarmonicMinor,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Major, Mode::NaturalMinor, Mode::HarmonicMinor];

    pub fn intervals(self) -> &'static [u8; 7] {
        match self {
            Mode::Major => &[0, 2, 4, 5, 7, 9, 11],
            Mode::NaturalMinor => &[0, 2, 3, 5, 7, 8, 10],
            Mode::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
        }
    }

    pub fn is_minor(self) -> bool {
        self != Mode::Major
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Major => "majeur",
            Mode::NaturalMinor => "mineur",
            Mode::HarmonicMinor => "mineur harmonique",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Scale {
    pub tonic: PitchClass,
    pub mode: Mode,
}

impl Scale {
    pub fn new(tonic: PitchClass, mode: Mode) -> Self {
        Self { tonic, mode }
    }

    pub fn pcs(&self) -> PcSet {
        PcSet::from_intervals(self.tonic, self.mode.intervals())
    }

    pub fn contains(&self, pc: PitchClass) -> bool {
        self.pcs().contains(pc)
    }

    /// Degré (0..7) de `pc` dans la gamme, ou `None` si `pc` n'en fait pas partie.
    pub fn degree_of(&self, pc: PitchClass) -> Option<usize> {
        let i = interval(self.tonic, pc);
        self.mode.intervals().iter().position(|&x| x == i)
    }

    /// Déplace une note MIDI de `steps` degrés dans la gamme (« hauteur ±1 »).
    /// Une note hors gamme est d'abord ramenée sur le degré inférieur le plus proche.
    pub fn step(&self, midi: u8, steps: i32) -> u8 {
        let mut note = midi as i32;
        while !self.contains(note.rem_euclid(12) as u8) {
            note -= 1;
        }
        let dir = steps.signum();
        for _ in 0..steps.abs() {
            note += dir;
            while !self.contains(note.rem_euclid(12) as u8) {
                note += dir;
            }
        }
        note.clamp(0, 127) as u8
    }

    /// Relative : Bb majeur <-> G mineur (mêmes notes, autre tonique).
    pub fn relative(&self) -> Scale {
        match self.mode {
            Mode::Major => Scale::new((self.tonic + 9) % 12, Mode::NaturalMinor),
            _ => Scale::new((self.tonic + 3) % 12, Mode::Major),
        }
    }

    pub fn name(&self) -> String {
        format!("{} {}", pc_name(self.tonic), self.mode.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_note_names() {
        assert_eq!(note_name(60), "C3");
        assert_eq!(note_name(72), "C4");
        assert_eq!(note_name(46), "A#1");
    }

    #[test]
    fn chord_symbols_and_roles() {
        let mut cm = Chord::new(0, Quality::Minor);
        assert_eq!(cm.symbol(), "Cm");
        assert_eq!(cm.role(3), Role::Third);
        assert_eq!(cm.role(10), Role::Seventh);
        assert_eq!(cm.role(2), Role::Ninth);
        cm.extensions.insert(2);
        cm.bass = 3;
        assert_eq!(cm.symbol(), "Cm(add9)/D#");
    }

    #[test]
    fn scale_steps_by_degree() {
        let g_minor = Scale::new(7, Mode::NaturalMinor);
        assert_eq!(g_minor.step(62, 1), 63); // D -> Eb
        assert_eq!(g_minor.step(70, -1), 69); // Bb -> A
        assert_eq!(g_minor.step(65, 2), 69); // F -> G -> A
        assert_eq!(g_minor.step(66, 0), 65); // F# hors gamme -> F
        assert_eq!(g_minor.relative(), Scale::new(10, Mode::Major));
    }
}
