//! Outil en ligne de commande pour tester le moteur d'Harmogen hors d'Ableton.
//!
//! ```text
//! harmogen analyze  <accords.mid> [options]
//! harmogen generate <accords.mid> [options] [--kind bass|chords|melody|all] [--seed S]
//!                   [--key Bb|Gm|...] [--bass-rhythm NOM] [--chord-rhythm NOM] [--out-dir DOSSIER]
//!                   [--passes N] [--vary-silence P] [--vary-pitch P] [--vary-velocity P]
//!                   [--vary-length P] [--reset-every N]
//! ```
//! Options communes : `--bars N`, `--min-pitch P`, `--max-pitch P`.

use harmony_core::analysis::{detect_key, segment, Progression, TimedNote};
use harmony_core::generate::rhythm::{preset_by_name, BASS_PRESETS, CHORD_PRESETS};
use harmony_core::generate::engine::PassEngine;
use harmony_core::generate::rng::Rng;
use harmony_core::generate::variation::{LineKind, VariationParams};
use harmony_core::generate::{bass, chords, melody, Context, GenNote, Pattern};
use harmony_core::theory::{note_name, Mode, Scale, NOTE_NAMES};
use harmony_core::STEPS_PER_BEAT;
use midly::num::{u15, u28, u4, u7};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const STEPS_PER_BAR: u32 = 4 * STEPS_PER_BEAT;
/// Résolution des fichiers écrits (ticks par noire).
const OUT_TPQ: u32 = 96;

fn usage() -> ExitCode {
    eprintln!(
        "usage :\n  harmogen analyze  <accords.mid> [--bars N] [--min-pitch P] [--max-pitch P]\n  \
         harmogen generate <accords.mid> [--bars N] [--kind bass|chords|melody|all] [--seed S]\n                    \
         [--key Bb|Gm|...] [--bass-rhythm NOM] [--chord-rhythm NOM] [--out-dir DOSSIER]\n                    \
         [--passes N] [--vary-silence P] [--vary-pitch P] [--vary-velocity P] [--vary-length P]\n                    \
         [--reset-every N]\n\n\
         rythmes de basse : {}\nrythmes d'accords : {}",
        BASS_PRESETS.iter().map(|p| p.name).collect::<Vec<_>>().join(", "),
        CHORD_PRESETS.iter().map(|p| p.name).collect::<Vec<_>>().join(", "),
    );
    ExitCode::from(2)
}

/// Lit toutes les notes d'un fichier MIDI (toutes pistes, tous canaux).
fn load_notes(path: &str) -> Result<Vec<TimedNote>, String> {
    let data = std::fs::read(path).map_err(|e| format!("{path} : {e}"))?;
    let smf = Smf::parse(&data).map_err(|e| format!("{path} : MIDI invalide ({e})"))?;
    let tpq = match smf.header.timing {
        Timing::Metrical(t) => t.as_int() as f64,
        Timing::Timecode(..) => return Err("timing SMPTE non géré".into()),
    };
    let mut notes = Vec::new();
    for track in &smf.tracks {
        let mut tick: u64 = 0;
        let mut on: HashMap<(u8, u8), (u64, u8)> = HashMap::new();
        for ev in track {
            tick += ev.delta.as_int() as u64;
            let TrackEventKind::Midi { channel, message } = ev.kind else { continue };
            let (key, vel, is_on) = match message {
                MidiMessage::NoteOn { key, vel } => (key.as_int(), vel.as_int(), vel.as_int() > 0),
                MidiMessage::NoteOff { key, vel } => (key.as_int(), vel.as_int(), false),
                _ => continue,
            };
            let id = (channel.as_int(), key);
            if is_on {
                on.insert(id, (tick, vel));
            } else if let Some((start, velocity)) = on.remove(&id) {
                notes.push(TimedNote::from_beats(
                    start as f64 / tpq,
                    (tick - start) as f64 / tpq,
                    key,
                    velocity,
                ));
            }
        }
    }
    notes.sort_by_key(|n| (n.start, n.pitch));
    Ok(notes)
}

/// Écrit une boucle générée dans un fichier MIDI à une piste.
fn save_pattern(pattern: &Pattern, path: &Path) -> Result<(), String> {
    let ticks = OUT_TPQ / STEPS_PER_BEAT;
    // (tick, 0 = note-off avant 1 = note-on au même tick, hauteur, vélocité)
    let mut events: Vec<(u32, u8, u8, u8)> = Vec::new();
    for n in &pattern.notes {
        events.push((n.start * ticks, 1, n.pitch, n.velocity));
        events.push(((n.start + n.len) * ticks, 0, n.pitch, 0));
    }
    events.sort_unstable();
    let mut track: Vec<TrackEvent> = vec![TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::TimeSignature(4, 2, 24, 8)),
    }];
    let mut last = 0;
    for (tick, on, pitch, vel) in events {
        let message = if on == 1 {
            MidiMessage::NoteOn { key: u7::new(pitch), vel: u7::new(vel) }
        } else {
            MidiMessage::NoteOff { key: u7::new(pitch), vel: u7::new(0) }
        };
        track.push(TrackEvent {
            delta: u28::new(tick - last),
            kind: TrackEventKind::Midi { channel: u4::new(0), message },
        });
        last = tick;
    }
    let end = pattern.length * ticks;
    track.push(TrackEvent {
        delta: u28::new(end.saturating_sub(last)),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    let smf = Smf {
        header: Header::new(Format::SingleTrack, Timing::Metrical(u15::new(OUT_TPQ as u16))),
        tracks: vec![track],
    };
    smf.save(path).map_err(|e| format!("{} : {e}", path.display()))
}

/// Position façon Live : mesure.temps.double-croche, à partir de 1.
fn position(step: u32) -> String {
    format!(
        "{}.{}.{}",
        step / STEPS_PER_BAR + 1,
        step % STEPS_PER_BAR / STEPS_PER_BEAT + 1,
        step % STEPS_PER_BEAT + 1
    )
}

/// Tonalité écrite `Bb`, `A#`, `Gm`, `Bbm`…
fn parse_key(s: &str) -> Option<Scale> {
    let (name, mode) = match s.strip_suffix('m') {
        Some(n) => (n, Mode::NaturalMinor),
        None => (s, Mode::Major),
    };
    let mut chars = name.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let base = NOTE_NAMES.iter().position(|n| n.len() == 1 && n.starts_with(letter))? as i32;
    let shift = match chars.as_str() {
        "" => 0,
        "#" => 1,
        "b" => -1,
        _ => return None,
    };
    Some(Scale::new((base + shift).rem_euclid(12) as u8, mode))
}

struct Options {
    input: String,
    bars: Option<u32>,
    min_pitch: u8,
    max_pitch: u8,
    kind: String,
    seed: u64,
    key: Option<Scale>,
    bass_rhythm: usize,
    chord_rhythm: usize,
    out_dir: PathBuf,
    passes: u32,
    variation: VariationParams,
    reset_every: u32,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        input: args.first().ok_or("fichier manquant")?.clone(),
        bars: None,
        min_pitch: 0,
        max_pitch: 127,
        kind: "all".into(),
        seed: 1,
        key: None,
        bass_rhythm: 0,
        chord_rhythm: 0,
        out_dir: PathBuf::from("."),
        passes: 1,
        variation: VariationParams::default(),
        reset_every: 0,
    };
    let mut it = args[1..].iter();
    while let Some(flag) = it.next() {
        let value = it.next().ok_or(format!("valeur manquante pour {flag}"))?;
        let num = |v: &str| v.parse::<u64>().map_err(|_| format!("{flag} : nombre attendu"));
        let prob = |v: &str| {
            v.parse::<f32>()
                .ok()
                .filter(|x| (0.0..=1.0).contains(x))
                .ok_or(format!("{flag} : probabilité entre 0 et 1 attendue"))
        };
        match flag.as_str() {
            "--bars" => o.bars = Some(num(value)? as u32),
            "--min-pitch" => o.min_pitch = num(value)?.min(127) as u8,
            "--max-pitch" => o.max_pitch = num(value)?.min(127) as u8,
            "--kind" => o.kind = value.clone(),
            "--seed" => o.seed = num(value)?,
            "--key" => o.key = Some(parse_key(value).ok_or(format!("tonalité inconnue : {value}"))?),
            "--bass-rhythm" => {
                o.bass_rhythm = preset_by_name(BASS_PRESETS, value).ok_or(format!("rythme de basse inconnu : {value}"))?
            }
            "--chord-rhythm" => {
                o.chord_rhythm =
                    preset_by_name(CHORD_PRESETS, value).ok_or(format!("rythme d'accords inconnu : {value}"))?
            }
            "--out-dir" => o.out_dir = PathBuf::from(value),
            "--passes" => o.passes = (num(value)? as u32).max(1),
            "--vary-silence" => o.variation.silence = prob(value)?,
            "--vary-pitch" => o.variation.pitch = prob(value)?,
            "--vary-velocity" => o.variation.velocity = prob(value)?,
            "--vary-length" => o.variation.length = prob(value)?,
            "--reset-every" => o.reset_every = num(value)? as u32,
            _ => return Err(format!("option inconnue : {flag}")),
        }
    }
    Ok(o)
}

/// Charge le fichier, découpe la progression et estime la tonalité.
fn load_progression(o: &Options) -> Result<(Vec<TimedNote>, Progression, Scale, bool), String> {
    let notes: Vec<TimedNote> = load_notes(&o.input)?
        .into_iter()
        .filter(|n| (o.min_pitch..=o.max_pitch).contains(&n.pitch))
        .collect();
    if notes.is_empty() {
        return Err("aucune note".into());
    }
    let last = notes.iter().map(|n| n.end).max().unwrap_or(0);
    let length = match o.bars {
        Some(b) => b * STEPS_PER_BAR,
        None => last.div_ceil(STEPS_PER_BAR) * STEPS_PER_BAR,
    };
    let progression = segment(&notes, length);
    let key = detect_key(&notes, &progression);
    let (scale, forced) = match o.key {
        Some(k) => (k, true),
        None => (key.best().ok_or("tonalité introuvable")?, false),
    };
    Ok((notes, progression, scale, forced))
}

fn analyze(args: &[String]) -> Result<(), String> {
    let o = parse_options(args)?;
    let (notes, progression, _, _) = load_progression(&o)?;
    let lo = notes.iter().map(|n| n.pitch).min().unwrap();
    let hi = notes.iter().map(|n| n.pitch).max().unwrap();
    println!("{}", o.input);
    println!(
        "  {} notes, {} mesures, {} – {}",
        notes.len(),
        progression.length / STEPS_PER_BAR,
        note_name(lo),
        note_name(hi)
    );
    println!("  Progression :");
    for s in &progression.segments {
        let beats = s.len() as f32 / STEPS_PER_BEAT as f32;
        println!("    {:<8} {:<16} ({beats} temps)", position(s.start), s.chord.symbol());
    }
    let key = detect_key(&notes, &progression);
    println!("  Tonalité :");
    for k in key.candidates.iter().take(3) {
        println!("    {:<22} score {:.3}", k.scale.name(), k.score);
    }
    if key.is_ambiguous() {
        let names: Vec<String> = key.plausible().iter().map(|s| s.name()).collect();
        println!("    ⚠ aussi plausibles : {} — à confirmer dans l'interface", names.join(", "));
    }
    Ok(())
}

fn generate(args: &[String]) -> Result<(), String> {
    let o = parse_options(args)?;
    let (_, progression, scale, forced) = load_progression(&o)?;
    let ctx = Context { progression: &progression, scale };
    let symbols: Vec<String> = progression.segments.iter().map(|s| s.chord.symbol()).collect();
    println!(
        "{} mesures : {}\ntonalité : {}{}",
        progression.length / STEPS_PER_BAR,
        symbols.join(" "),
        scale.name(),
        if forced { " (imposée)" } else { " (détectée)" }
    );

    let stem = Path::new(&o.input)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "harmogen".into());
    std::fs::create_dir_all(&o.out_dir).map_err(|e| format!("{} : {e}", o.out_dir.display()))?;

    let kinds: &[&str] = match o.kind.as_str() {
        "all" => &["bass", "chords", "melody"],
        "bass" => &["bass"],
        "chords" => &["chords"],
        "melody" => &["melody"],
        k => return Err(format!("--kind inconnu : {k}")),
    };
    for &kind in kinds {
        // Une graine par ligne, pour que changer l'une ne modifie pas les autres.
        let mut rng = Rng::new(o.seed.wrapping_mul(31).wrapping_add(kind.len() as u64));
        let (pattern, detail) = match kind {
            "bass" => {
                let p = bass::BassParams { preset: o.bass_rhythm, ..Default::default() };
                (bass::generate(&ctx, &p, &mut rng), BASS_PRESETS[p.preset].name)
            }
            "chords" => {
                let p = chords::ChordParams { preset: o.chord_rhythm, ..Default::default() };
                (chords::generate(&ctx, &p, &mut rng), CHORD_PRESETS[p.preset].name)
            }
            _ => (melody::generate(&ctx, &melody::MelodyParams::default(), &mut rng), "A–A′"),
        };
        let line_kind = match kind {
            "bass" => LineKind::Bass,
            "chords" => LineKind::Chords,
            _ => LineKind::Melody,
        };
        // Passage 1 = modèle, puis une variation du précédent à chaque passage.
        let mut engine = PassEngine::new(
            progression.clone(),
            scale,
            line_kind,
            pattern,
            o.variation,
            o.seed.wrapping_mul(7919).wrapping_add(kind.len() as u64),
            2,
        );
        engine.reset_every = o.reset_every;
        let mut all = Pattern { length: 0, notes: Vec::new() };
        let mut counts = Vec::new();
        for _ in 0..o.passes {
            let pass = engine.next_pass();
            counts.push(pass.notes.len().to_string());
            let offset = all.length;
            all.notes.extend(pass.notes.iter().map(|n| GenNote { start: n.start + offset, ..*n }));
            all.length += pass.length;
        }
        let suffix = if o.passes > 1 { format!("_x{}", o.passes) } else { String::new() };
        let path = o.out_dir.join(format!("{stem}_{kind}_s{}{suffix}.mid", o.seed));
        save_pattern(&all, &path)?;
        println!("  {kind:<7} notes par passage {}  ({detail})  → {}", counts.join("/"), path.display());
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("analyze") => analyze(&args[1..]),
        Some("generate") => generate(&args[1..]),
        _ => return usage(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erreur : {e}");
            ExitCode::FAILURE
        }
    }
}
