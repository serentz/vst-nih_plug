//! Travail en arrière-plan (jamais sur le thread audio) : analyse de la progression
//! captée, génération du modèle, calcul des passages suivants.
//!
//! Le thread audio n'échange avec lui que par des tâches (`Task`) et par la file
//! `ready`, qu'il consulte avec `try_lock` pour ne jamais attendre.

use harmony_core::analysis::{detect_key, segment, Progression, TimedNote};
use harmony_core::generate::engine::PassEngine;
use harmony_core::generate::rng::Rng;
use harmony_core::generate::variation::{LineKind, VariationParams};
use harmony_core::generate::{bass, chords, melody, Context, Pattern};
use harmony_core::theory::{Mode, Scale};
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Arc, Mutex};

/// Passages toujours prêts d'avance dans la file partagée.
pub const READY_PASSES: usize = 2;

/// Réglages qui déterminent la génération (instantané des paramètres du plugin).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub line: LineKind,
    pub bars: u32,
    /// -1 : tonalité détectée ; 0–11 : majeur sur cette tonique ; 12–23 : mineur.
    pub key: i32,
    pub bass_rhythm: usize,
    pub chord_rhythm: usize,
    pub seed: u64,
    pub variation: VariationParams,
    pub reset_every: u32,
}

/// Événements notés dans le journal `%TEMP%\harmogen.log`.
#[derive(Clone, Copy, Debug)]
pub enum LogEvent {
    CaptureStarted { bar: f64 },
    CaptureCancelled,
    Relearn,
    Playing,
}

pub enum Task {
    /// Nouvelle progression captée.
    Build { notes: Vec<TimedNote>, length: u32, settings: Settings },
    /// Réglages modifiés : nouveau modèle à partir de la même progression.
    Rebuild(Settings),
    /// Un passage a été consommé : en calculer un autre.
    Refill,
    Log(LogEvent),
}

pub struct Worker {
    notes: Vec<TimedNote>,
    length: u32,
    engine: Option<PassEngine>,
    ready: Arc<Mutex<VecDeque<Pattern>>>,
    /// Résumé lisible (progression, tonalité) pour l'interface à venir.
    pub summary: Arc<Mutex<String>>,
}

impl Worker {
    pub fn new(ready: Arc<Mutex<VecDeque<Pattern>>>, summary: Arc<Mutex<String>>) -> Self {
        Self { notes: Vec::new(), length: 0, engine: None, ready, summary }
    }

    pub fn handle(&mut self, task: Task) {
        match task {
            Task::Build { notes, length, settings } => {
                self.notes = notes;
                self.length = length;
                self.rebuild(settings);
            }
            Task::Rebuild(settings) => {
                if !self.notes.is_empty() {
                    self.rebuild(settings);
                }
            }
            Task::Refill => self.refill(),
            Task::Log(e) => log(&format!("{e:?}")),
        }
    }

    fn rebuild(&mut self, s: Settings) {
        let progression: Progression = segment(&self.notes, self.length);
        let analysis = detect_key(&self.notes, &progression);
        let scale = match s.key {
            k @ 0..=11 => Scale::new(k as u8, Mode::Major),
            k @ 12..=23 => Scale::new((k - 12) as u8, Mode::NaturalMinor),
            _ => analysis.best().unwrap_or(Scale::new(0, Mode::Major)),
        };
        let ctx = Context { progression: &progression, scale };
        let mut rng = Rng::new(s.seed);
        let base = match s.line {
            LineKind::Bass => bass::generate(&ctx, &bass::BassParams { preset: s.bass_rhythm, ..Default::default() }, &mut rng),
            LineKind::Chords => {
                chords::generate(&ctx, &chords::ChordParams { preset: s.chord_rhythm, ..Default::default() }, &mut rng)
            }
            LineKind::Melody => melody::generate(&ctx, &melody::MelodyParams::default(), &mut rng),
        };

        let symbols: Vec<String> = progression.segments.iter().map(|seg| seg.chord.symbol()).collect();
        let plausible: Vec<String> = analysis.plausible().iter().map(|k| k.name()).collect();
        let summary = format!(
            "{} mesures : {} | tonalité {}{} | alternatives : {} | {:?}, {} notes",
            self.length / 16,
            symbols.join(" "),
            scale.name(),
            if s.key >= 0 { " (imposée)" } else { "" },
            plausible.join(", "),
            s.line,
            base.notes.len()
        );
        log(&summary);
        if let Ok(mut sum) = self.summary.lock() {
            *sum = summary;
        }

        let mut engine =
            PassEngine::new(progression.clone(), scale, s.line, base, s.variation, s.seed.wrapping_add(1), 1);
        engine.reset_every = s.reset_every;
        self.engine = Some(engine);
        if let Ok(mut r) = self.ready.lock() {
            r.clear();
        }
        self.refill();
    }

    fn refill(&mut self) {
        let Some(engine) = self.engine.as_mut() else { return };
        if let Ok(mut r) = self.ready.lock() {
            while r.len() < READY_PASSES {
                r.push_back(engine.next_pass());
            }
        }
    }
}

/// Ajoute une ligne au journal (thread d'arrière-plan uniquement).
fn log(line: &str) {
    let path = std::env::temp_dir().join("harmogen.log");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            line: LineKind::Melody,
            bars: 4,
            key: -1,
            bass_rhythm: 0,
            chord_rhythm: 0,
            seed: 1,
            variation: VariationParams::default(),
            reset_every: 0,
        }
    }

    fn chords() -> Vec<TimedNote> {
        let mut n = Vec::new();
        for (s, pitches) in [(0u32, [48u8, 55, 63]), (16, [51, 58, 67]), (32, [46, 53, 62]), (48, [43, 50, 58])] {
            for p in pitches {
                n.push(TimedNote { start: s, end: s + 16, pitch: p, velocity: 80 });
            }
        }
        n
    }

    #[test]
    fn build_fills_the_ready_queue_and_refill_tops_it_up() {
        let ready = Arc::new(Mutex::new(VecDeque::new()));
        let mut w = Worker::new(ready.clone(), Arc::new(Mutex::new(String::new())));
        w.handle(Task::Build { notes: chords(), length: 64, settings: settings() });
        assert_eq!(ready.lock().unwrap().len(), READY_PASSES);
        let first = ready.lock().unwrap().pop_front().unwrap();
        assert_eq!(first.length, 64);
        w.handle(Task::Refill);
        assert_eq!(ready.lock().unwrap().len(), READY_PASSES);
    }

    #[test]
    fn rebuild_replaces_the_queue_with_the_new_model() {
        let ready = Arc::new(Mutex::new(VecDeque::new()));
        let mut w = Worker::new(ready.clone(), Arc::new(Mutex::new(String::new())));
        w.handle(Task::Build { notes: chords(), length: 64, settings: settings() });
        let melody = ready.lock().unwrap().front().cloned().unwrap();
        w.handle(Task::Rebuild(Settings { line: LineKind::Chords, ..settings() }));
        let chords = ready.lock().unwrap().front().cloned().unwrap();
        assert_ne!(melody, chords);
        assert!(chords.notes.len() > melody.notes.len());
    }
}
