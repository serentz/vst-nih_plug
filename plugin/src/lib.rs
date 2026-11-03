//! Harmogen : instrument MIDI qui apprend une progression d'accords sur son entrée
//! et joue une mélodie, une ligne de basse ou des accords rythmés accordés dessus,
//! avec une variation à chaque passage.
//!
//! Cycle de vie (machine d'états) :
//! 1. **Attente** : le transport démarre → la capture commence au prochain début de mesure ;
//! 2. **Capture** : la progression est mise en mémoire pendant 4, 8 ou 16 mesures ;
//! 3. **Calcul** : analyse et génération en arrière-plan ;
//! 4. **Lecture** : passage 1 = modèle, puis une variation à chaque tour ; le passage
//!    suivant est toujours déjà calculé.
//!
//! Le paramètre « Réapprendre » relance une capture ; un changement de réglage
//! prend effet au début du tour suivant.

mod capture;
mod scheduler;
mod worker;

use capture::Capture;
use harmony_core::generate::rhythm::{BASS_PRESETS, CHORD_PRESETS};
use harmony_core::generate::variation::{LineKind, VariationParams};
use harmony_core::generate::Pattern;
use harmony_core::theory::NOTE_NAMES;
use harmony_core::STEPS_PER_BEAT;
use nih_plug::prelude::*;
use scheduler::{Event, Scheduler};
use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use worker::{LogEvent, Settings, Task, Worker};

#[derive(Enum, PartialEq, Eq, Clone, Copy, Debug)]
enum Line {
    #[name = "Mélodie"]
    Melody,
    #[name = "Basse"]
    Bass,
    #[name = "Accords"]
    Chords,
}

#[derive(Enum, PartialEq, Eq, Clone, Copy, Debug)]
enum Bars {
    #[name = "4 mesures"]
    Four,
    #[name = "8 mesures"]
    Eight,
    #[name = "16 mesures"]
    Sixteen,
}

impl Bars {
    fn count(self) -> u32 {
        match self {
            Bars::Four => 4,
            Bars::Eight => 8,
            Bars::Sixteen => 16,
        }
    }
}

fn key_name(v: i32) -> String {
    match v {
        0..=11 => format!("{} majeur", NOTE_NAMES[v as usize]),
        12..=23 => format!("{} mineur", NOTE_NAMES[(v - 12) as usize]),
        _ => "Auto".into(),
    }
}

fn percent(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

#[derive(Params)]
struct HarmogenParams {
    #[id = "line"]
    line: EnumParam<Line>,
    #[id = "bars"]
    bars: EnumParam<Bars>,
    #[id = "key"]
    key: IntParam,
    #[id = "bass_rhythm"]
    bass_rhythm: IntParam,
    #[id = "chord_rhythm"]
    chord_rhythm: IntParam,
    #[id = "seed"]
    seed: IntParam,
    #[id = "vary_silence"]
    vary_silence: FloatParam,
    #[id = "vary_pitch"]
    vary_pitch: FloatParam,
    #[id = "vary_velocity"]
    vary_velocity: FloatParam,
    #[id = "vary_length"]
    vary_length: FloatParam,
    #[id = "reset_every"]
    reset_every: IntParam,
    #[id = "relearn"]
    relearn: BoolParam,
    #[id = "thru"]
    thru: BoolParam,
}

impl Default for HarmogenParams {
    fn default() -> Self {
        let v = VariationParams::default();
        Self {
            line: EnumParam::new("Ligne", Line::Melody),
            bars: EnumParam::new("Longueur", Bars::Eight),
            key: IntParam::new("Tonalité", -1, IntRange::Linear { min: -1, max: 23 })
                .with_value_to_string(Arc::new(key_name)),
            bass_rhythm: IntParam::new("Rythme basse", 0, IntRange::Linear { min: 0, max: BASS_PRESETS.len() as i32 - 1 })
                .with_value_to_string(Arc::new(|v| BASS_PRESETS[v as usize % BASS_PRESETS.len()].name.to_string())),
            chord_rhythm: IntParam::new(
                "Rythme accords",
                0,
                IntRange::Linear { min: 0, max: CHORD_PRESETS.len() as i32 - 1 },
            )
            .with_value_to_string(Arc::new(|v| CHORD_PRESETS[v as usize % CHORD_PRESETS.len()].name.to_string())),
            seed: IntParam::new("Graine", 1, IntRange::Linear { min: 1, max: 999 }),
            vary_silence: percent("Var. silence", v.silence),
            vary_pitch: percent("Var. hauteur", v.pitch),
            vary_velocity: percent("Var. vélocité", v.velocity),
            vary_length: percent("Var. durée", v.length),
            reset_every: IntParam::new("Retour au modèle", 0, IntRange::Linear { min: 0, max: 16 })
                .with_value_to_string(Arc::new(|v| if v == 0 { "jamais".into() } else { format!("tous les {v} tours") })),
            relearn: BoolParam::new("Réapprendre", false),
            thru: BoolParam::new("Renvoyer l'entrée", false),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    /// Transport à l'arrêt ou capture pas encore commencée.
    Waiting,
    Capturing,
    /// Progression envoyée à l'arrière-plan, premier passage attendu.
    Building,
    Playing,
}

pub struct Harmogen {
    params: Arc<HarmogenParams>,
    state: State,
    capture: Option<Capture>,
    /// Pas absolu où la boucle commence (début de la capture).
    anchor: i64,
    scheduler: Scheduler,
    ready: Arc<Mutex<VecDeque<Pattern>>>,
    worker: Arc<Mutex<Worker>>,
    last_settings: Option<Settings>,
    previous_relearn: bool,
    /// Événements du bloc en cours (alloués une fois).
    events: Vec<Event>,
}

impl Default for Harmogen {
    fn default() -> Self {
        let ready = Arc::new(Mutex::new(VecDeque::with_capacity(8)));
        let summary = Arc::new(Mutex::new(String::new()));
        Self {
            params: Arc::new(HarmogenParams::default()),
            state: State::Waiting,
            capture: None,
            anchor: 0,
            scheduler: Scheduler::default(),
            worker: Arc::new(Mutex::new(Worker::new(ready.clone(), summary))),
            ready,
            last_settings: None,
            previous_relearn: false,
            events: Vec::with_capacity(1024),
        }
    }
}

impl Harmogen {
    fn settings(&self) -> Settings {
        let p = &self.params;
        Settings {
            line: match p.line.value() {
                Line::Melody => LineKind::Melody,
                Line::Bass => LineKind::Bass,
                Line::Chords => LineKind::Chords,
            },
            bars: p.bars.value().count(),
            key: p.key.value(),
            bass_rhythm: p.bass_rhythm.value() as usize,
            chord_rhythm: p.chord_rhythm.value() as usize,
            seed: p.seed.value() as u64,
            variation: VariationParams {
                silence: p.vary_silence.value(),
                pitch: p.vary_pitch.value(),
                velocity: p.vary_velocity.value(),
                length: p.vary_length.value(),
                ..VariationParams::default()
            },
            reset_every: p.reset_every.value() as u32,
        }
    }

    fn send_events(&mut self, context: &mut impl ProcessContext<Self>) {
        for e in self.events.drain(..) {
            let event = match e {
                Event::On { offset, pitch, velocity } => NoteEvent::NoteOn {
                    timing: offset,
                    voice_id: None,
                    channel: 0,
                    note: pitch,
                    velocity: velocity as f32 / 127.0,
                },
                Event::Off { offset, pitch } => NoteEvent::NoteOff {
                    timing: offset,
                    voice_id: None,
                    channel: 0,
                    note: pitch,
                    velocity: 0.0,
                },
            };
            context.send_event(event);
        }
    }
}

impl Plugin for Harmogen {
    const NAME: &'static str = "Harmogen";
    const VENDOR: &'static str = "Harmogen";
    const URL: &'static str = "https://github.com/serentz/vst-nih_plug";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // Instrument : pas d'entrée audio, sortie stéréo (silencieuse) ; tout passe par le MIDI.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::Basic;

    type SysExMessage = ();
    type BackgroundTask = Task;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let worker = self.worker.clone();
        Box::new(move |task| {
            if let Ok(mut w) = worker.lock() {
                w.handle(task);
            }
        })
    }

    fn reset(&mut self) {
        self.scheduler.transport_stopped(&mut |_| {});
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        for channel in buffer.as_slice() {
            channel.fill(0.0);
        }
        let samples = buffer.samples() as u32;
        let t = context.transport();
        let (playing, pos_beats, tempo, sample_rate) = (t.playing, t.pos_beats(), t.tempo, t.sample_rate);
        let bar_start = t.bar_start_pos_beats();
        let num = t.time_sig_numerator.unwrap_or(4).max(1) as f64;
        let den = t.time_sig_denominator.unwrap_or(4).max(1) as f64;
        let steps_per_bar = num * 4.0 / den * STEPS_PER_BEAT as f64;

        // Réapprendre (front montant) : nouvelle capture au prochain début de mesure.
        let relearn = self.params.relearn.value();
        if relearn && !self.previous_relearn {
            self.scheduler.all_off(0, &mut |e| self.events.push(e));
            self.scheduler.stop();
            self.capture = None;
            self.state = State::Waiting;
            context.execute_background(Task::Log(LogEvent::Relearn));
        }
        self.previous_relearn = relearn;

        // Réglages modifiés : nouveau modèle, joué à partir du tour suivant.
        let settings = self.settings();
        if self.last_settings != Some(settings) {
            if matches!(self.state, State::Building | State::Playing) {
                context.execute_background(Task::Rebuild(settings));
            }
            self.last_settings = Some(settings);
        }

        let timing = match (playing, pos_beats, tempo) {
            (true, Some(pos), Some(tempo)) if tempo > 0.0 => {
                let samples_per_step = sample_rate as f64 * 60.0 / (tempo * STEPS_PER_BEAT as f64);
                Some((pos * STEPS_PER_BEAT as f64, samples_per_step))
            }
            _ => None,
        };

        // Début de capture, avant de lire les notes du bloc (l'accord du premier temps en fait partie).
        if let (State::Waiting, Some((start, _))) = (self.state, timing) {
            // Lecture lancée pile sur une mesure : on capture tout de suite ; sinon, à la suivante.
            let bar = bar_start.map_or(start, |b| b * STEPS_PER_BEAT as f64);
            let capture_start = if start - bar < 1e-3 { bar } else { bar + steps_per_bar };
            let length = settings.bars * 16;
            self.capture = Some(Capture::new(capture_start, length));
            self.anchor = capture_start.round() as i64;
            self.state = State::Capturing;
            context.execute_background(Task::Log(LogEvent::CaptureStarted { bar: capture_start / steps_per_bar + 1.0 }));
        }

        // Entrée MIDI : capture (et renvoi éventuel).
        let thru = self.params.thru.value();
        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn { timing: offset, channel, note, velocity, .. } => {
                    if thru {
                        context.send_event(event);
                    }
                    if let (Some(cap), Some((start, sps))) = (self.capture.as_mut(), timing) {
                        cap.note_on(start + offset as f64 / sps, channel, note, (velocity * 127.0).round() as u8);
                    }
                }
                NoteEvent::NoteOff { timing: offset, channel, note, .. } => {
                    if thru {
                        context.send_event(event);
                    }
                    if let (Some(cap), Some((start, sps))) = (self.capture.as_mut(), timing) {
                        cap.note_off(start + offset as f64 / sps, channel, note);
                    }
                }
                _ => {}
            }
        }

        let Some((start, sps)) = timing else {
            // Transport arrêté : silence, et une capture interrompue est à recommencer.
            self.scheduler.transport_stopped(&mut |e| self.events.push(e));
            if self.state == State::Capturing {
                self.capture = None;
                self.state = State::Waiting;
                context.execute_background(Task::Log(LogEvent::CaptureCancelled));
            }
            self.send_events(context);
            return ProcessStatus::Normal;
        };
        let end = start + samples as f64 / sps;

        if self.state == State::Capturing && self.capture.as_ref().is_some_and(|c| c.is_done(end)) {
            let cap = self.capture.take().unwrap();
            let length = cap.length;
            context.execute_background(Task::Build { notes: cap.finish(), length, settings });
            self.state = State::Building;
        }

        if self.state == State::Building {
            if let Some(first) = self.ready.try_lock().ok().and_then(|mut r| r.pop_front()) {
                self.scheduler.start(first, self.anchor);
                self.state = State::Playing;
                context.execute_background(Task::Refill);
                context.execute_background(Task::Log(LogEvent::Playing));
            }
        }

        if self.state == State::Playing {
            let ready = &self.ready;
            let mut consumed = false;
            let mut next = || {
                let pass = ready.try_lock().ok().and_then(|mut r| r.pop_front());
                consumed |= pass.is_some();
                pass
            };
            let events = &mut self.events;
            self.scheduler.render(start, samples, sps, &mut next, &mut |e| events.push(e));
            if consumed {
                context.execute_background(Task::Refill);
            }
        }

        self.send_events(context);
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Harmogen {
    const VST3_CLASS_ID: [u8; 16] = *b"HarmogenPlugin01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Generator];
}

nih_export_vst3!(Harmogen);
