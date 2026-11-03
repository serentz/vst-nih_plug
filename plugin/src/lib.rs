//! Phase 0 — prototype de faisabilité.
//!
//! Vérifie dans Ableton Live :
//! 1. qu'un instrument VST3 écrit en Rust reçoit le MIDI de sa piste ;
//! 2. que sa sortie MIDI (notes transposées) peut être routée vers une autre piste ;
//! 3. que l'hôte fournit tempo, position, mesure, signature et boucle.
//!
//! L'état du transport est écrit toutes les 250 ms dans `%TEMP%\harmogen_spike.log`
//! par un thread séparé (jamais d'E/S sur le thread audio).

use nih_plug::prelude::*;
use std::fs::OpenOptions;
use std::io::Write;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

const NO_VALUE: f64 = f64::NAN;

/// État partagé entre le thread audio (écriture) et le thread de log (lecture).
#[derive(Default)]
struct Status {
    playing: AtomicBool,
    tempo: AtomicU64,
    pos_beats: AtomicU64,
    bar_start_beats: AtomicU64,
    bar_number: AtomicI32,
    time_sig: AtomicU32,
    loop_start: AtomicU64,
    loop_end: AtomicU64,
    sample_rate: AtomicU32,
    buffer_size: AtomicU32,
    notes_in: AtomicU32,
    notes_out: AtomicU32,
    last_note_in: AtomicI32,
    last_note_timing: AtomicU32,
    last_note_out: AtomicI32,
}

fn store_f64(a: &AtomicU64, v: Option<f64>) {
    a.store(v.unwrap_or(NO_VALUE).to_bits(), Ordering::Relaxed);
}

fn load_f64(a: &AtomicU64) -> f64 {
    f64::from_bits(a.load(Ordering::Relaxed))
}

/// Note émise à chaque temps en mode `Pulse` (C5).
const PULSE_NOTE: u8 = 72;

#[derive(Enum, PartialEq, Clone, Copy)]
enum Mode {
    /// Renvoie les notes reçues, transposées.
    Echo,
    /// Ignore l'entrée et joue C5 sur chaque temps : signal impossible à confondre
    /// avec le clip source, pour vérifier le routage de la sortie MIDI.
    Pulse,
}

#[derive(Params)]
struct SpikeParams {
    #[id = "mode"]
    mode: EnumParam<Mode>,
    #[id = "transpose"]
    transpose: IntParam,
}

impl Default for SpikeParams {
    fn default() -> Self {
        Self {
            mode: EnumParam::new("Mode", Mode::Pulse),
            transpose: IntParam::new("Transpose", 12, IntRange::Linear { min: -24, max: 24 })
                .with_unit(" st"),
        }
    }
}

pub struct Harmogen {
    params: Arc<SpikeParams>,
    status: Arc<Status>,
    stop_logger: Arc<AtomicBool>,
    logger: Option<JoinHandle<()>>,
    /// Note de sortie actuellement jouée pour chaque (canal, note d'entrée), afin que
    /// le note-off corresponde même si la transposition change pendant la note.
    active: [[Option<u8>; 128]; 16],
    /// Nombre d'échantillons restant avant le note-off de la note `Pulse` en cours.
    pulse_off_in: Option<u32>,
    /// Dernier temps ayant déclenché un pulse, pour ne jamais le jouer deux fois.
    last_pulse_beat: i64,
}

impl Default for Harmogen {
    fn default() -> Self {
        let status = Status::default();
        for a in [
            &status.tempo,
            &status.pos_beats,
            &status.bar_start_beats,
            &status.loop_start,
            &status.loop_end,
        ] {
            store_f64(a, None);
        }
        status.last_note_in.store(-1, Ordering::Relaxed);
        status.last_note_out.store(-1, Ordering::Relaxed);

        Self {
            params: Arc::new(SpikeParams::default()),
            status: Arc::new(status),
            stop_logger: Arc::new(AtomicBool::new(false)),
            logger: None,
            active: [[None; 128]; 16],
            pulse_off_in: None,
            last_pulse_beat: i64::MIN,
        }
    }
}

impl Harmogen {
    fn spawn_logger(&mut self) {
        if self.logger.is_some() {
            return;
        }
        let status = self.status.clone();
        let stop = self.stop_logger.clone();
        self.logger = Some(std::thread::spawn(move || {
            let path = std::env::temp_dir().join("harmogen_spike.log");
            let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) else {
                return;
            };
            let _ = writeln!(file, "=== harmogen spike démarré ===");
            let mut last = String::new();
            while !stop.load(Ordering::Relaxed) {
                let s = &status;
                let ts = s.time_sig.load(Ordering::Relaxed);
                let line = format!(
                    "playing={} tempo={:.2} pos={:.3} bar_start={:.3} bar={} sig={}/{} loop=[{:.2}..{:.2}] sr={} buf={} in={} out={} last_in={} @{} last_out={}",
                    s.playing.load(Ordering::Relaxed),
                    load_f64(&s.tempo),
                    load_f64(&s.pos_beats),
                    load_f64(&s.bar_start_beats),
                    s.bar_number.load(Ordering::Relaxed),
                    ts >> 16,
                    ts & 0xffff,
                    load_f64(&s.loop_start),
                    load_f64(&s.loop_end),
                    s.sample_rate.load(Ordering::Relaxed),
                    s.buffer_size.load(Ordering::Relaxed),
                    s.notes_in.load(Ordering::Relaxed),
                    s.notes_out.load(Ordering::Relaxed),
                    s.last_note_in.load(Ordering::Relaxed),
                    s.last_note_timing.load(Ordering::Relaxed),
                    s.last_note_out.load(Ordering::Relaxed),
                );
                if line != last {
                    let _ = writeln!(file, "{line}");
                    last = line;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }));
    }
}

impl Drop for Harmogen {
    fn drop(&mut self) {
        self.stop_logger.store(true, Ordering::Relaxed);
        if let Some(h) = self.logger.take() {
            let _ = h.join();
        }
    }
}

impl Plugin for Harmogen {
    const NAME: &'static str = "Harmogen Spike";
    const VENDOR: &'static str = "Harmogen";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // Instrument : pas d'entrée audio, sortie stéréo (silencieuse).
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::Basic;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        _layout: &AudioIOLayout,
        config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.status
            .sample_rate
            .store(config.sample_rate as u32, Ordering::Relaxed);
        self.status
            .buffer_size
            .store(config.max_buffer_size, Ordering::Relaxed);
        self.spawn_logger();
        true
    }

    fn reset(&mut self) {
        self.active = [[None; 128]; 16];
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let t = context.transport();
        let s = &self.status;
        s.playing.store(t.playing, Ordering::Relaxed);
        store_f64(&s.tempo, t.tempo);
        store_f64(&s.pos_beats, t.pos_beats());
        store_f64(&s.bar_start_beats, t.bar_start_pos_beats());
        s.bar_number
            .store(t.bar_number().unwrap_or(-1), Ordering::Relaxed);
        let num = t.time_sig_numerator.unwrap_or(0) as u32;
        let den = t.time_sig_denominator.unwrap_or(0) as u32;
        s.time_sig.store((num << 16) | den, Ordering::Relaxed);
        let lr = t.loop_range_beats();
        store_f64(&s.loop_start, lr.map(|r| r.0));
        store_f64(&s.loop_end, lr.map(|r| r.1));
        let (playing, pos_beats, tempo, sample_rate) =
            (t.playing, t.pos_beats(), t.tempo, t.sample_rate);

        let mode = self.params.mode.value();
        let transpose = self.params.transpose.value();
        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn {
                    timing,
                    voice_id,
                    channel,
                    note,
                    velocity,
                } => {
                    s.notes_in.fetch_add(1, Ordering::Relaxed);
                    s.last_note_in.store(note as i32, Ordering::Relaxed);
                    s.last_note_timing.store(timing, Ordering::Relaxed);
                    let out = note as i32 + transpose;
                    if mode == Mode::Echo && (0..128).contains(&out) {
                        let out = out as u8;
                        self.active[channel as usize][note as usize] = Some(out);
                        context.send_event(NoteEvent::NoteOn {
                            timing,
                            voice_id,
                            channel,
                            note: out,
                            velocity,
                        });
                        s.notes_out.fetch_add(1, Ordering::Relaxed);
                        s.last_note_out.store(out as i32, Ordering::Relaxed);
                    }
                }
                NoteEvent::NoteOff {
                    timing,
                    voice_id,
                    channel,
                    note,
                    velocity,
                } => {
                    if let Some(out) = self.active[channel as usize][note as usize].take() {
                        context.send_event(NoteEvent::NoteOff {
                            timing,
                            voice_id,
                            channel,
                            note: out,
                            velocity,
                        });
                    }
                }
                _ => {}
            }
        }

        // Mode Pulse : un C5 d'un quart de temps sur chaque temps.
        let n = buffer.samples() as u32;
        if let Some(remaining) = self.pulse_off_in {
            if !playing {
                send_pulse(context, 0, false);
                self.pulse_off_in = None;
            } else if remaining < n {
                send_pulse(context, remaining, false);
                self.pulse_off_in = None;
            } else {
                self.pulse_off_in = Some(remaining - n);
            }
        }
        if mode == Mode::Pulse && playing {
            if let (Some(pos), Some(tempo)) = (pos_beats, tempo) {
                let samples_per_beat = sample_rate as f64 * 60.0 / tempo;
                let next_beat = pos.ceil();
                let offset = ((next_beat - pos) * samples_per_beat) as u32;
                if offset < n && next_beat as i64 != self.last_pulse_beat {
                    self.last_pulse_beat = next_beat as i64;
                    send_pulse(context, offset, true);
                    s.notes_out.fetch_add(1, Ordering::Relaxed);
                    s.last_note_out.store(PULSE_NOTE as i32, Ordering::Relaxed);
                    let off_at = offset + (samples_per_beat / 4.0) as u32;
                    if off_at < n {
                        send_pulse(context, off_at, false);
                    } else {
                        self.pulse_off_in = Some(off_at - n);
                    }
                }
            }
        }
        if !playing {
            self.last_pulse_beat = i64::MIN;
        }

        for channel in buffer.as_slice() {
            channel.fill(0.0);
        }
        ProcessStatus::Normal
    }
}

fn send_pulse(context: &mut impl ProcessContext<Harmogen>, timing: u32, on: bool) {
    let event = if on {
        NoteEvent::NoteOn {
            timing,
            voice_id: None,
            channel: 0,
            note: PULSE_NOTE,
            velocity: 0.8,
        }
    } else {
        NoteEvent::NoteOff {
            timing,
            voice_id: None,
            channel: 0,
            note: PULSE_NOTE,
            velocity: 0.0,
        }
    };
    context.send_event(event);
}

impl Vst3Plugin for Harmogen {
    const VST3_CLASS_ID: [u8; 16] = *b"HarmogenSpike001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Generator];
}

nih_export_vst3!(Harmogen);
