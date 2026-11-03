//! Outil en ligne de commande pour tester le moteur d'Harmogen hors d'Ableton.
//!
//! ```text
//! harmogen analyze <fichier.mid> [--bars N] [--min-pitch P] [--max-pitch P]
//! ```

use harmony_core::analysis::{detect_key, segment, TimedNote};
use harmony_core::theory::note_name;
use harmony_core::STEPS_PER_BEAT;
use midly::{MidiMessage, Smf, Timing, TrackEventKind};
use std::collections::HashMap;
use std::process::ExitCode;

const STEPS_PER_BAR: u32 = 4 * STEPS_PER_BEAT;

fn usage() -> ExitCode {
    eprintln!("usage : harmogen analyze <fichier.mid> [--bars N] [--min-pitch P] [--max-pitch P]");
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

/// Position façon Live : mesure.temps.double-croche, à partir de 1.
fn position(step: u32) -> String {
    format!(
        "{}.{}.{}",
        step / STEPS_PER_BAR + 1,
        step % STEPS_PER_BAR / STEPS_PER_BEAT + 1,
        step % STEPS_PER_BEAT + 1
    )
}

fn duration(steps: u32) -> String {
    let beats = steps as f32 / STEPS_PER_BEAT as f32;
    format!("{beats} temps")
}

fn analyze(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("fichier manquant")?;
    let mut bars: Option<u32> = None;
    let mut min_pitch = 0u8;
    let mut max_pitch = 127u8;
    let mut it = args[1..].iter();
    while let Some(flag) = it.next() {
        let value = it.next().ok_or(format!("valeur manquante pour {flag}"))?;
        let parse = |v: &str| v.parse::<u32>().map_err(|_| format!("{flag} : nombre attendu"));
        match flag.as_str() {
            "--bars" => bars = Some(parse(value)?),
            "--min-pitch" => min_pitch = parse(value)?.min(127) as u8,
            "--max-pitch" => max_pitch = parse(value)?.min(127) as u8,
            _ => return Err(format!("option inconnue : {flag}")),
        }
    }

    let notes: Vec<TimedNote> = load_notes(path)?
        .into_iter()
        .filter(|n| (min_pitch..=max_pitch).contains(&n.pitch))
        .collect();
    if notes.is_empty() {
        return Err("aucune note".into());
    }
    let last = notes.iter().map(|n| n.end).max().unwrap_or(0);
    let length = match bars {
        Some(b) => b * STEPS_PER_BAR,
        None => last.div_ceil(STEPS_PER_BAR) * STEPS_PER_BAR,
    };
    let lo = notes.iter().map(|n| n.pitch).min().unwrap();
    let hi = notes.iter().map(|n| n.pitch).max().unwrap();

    println!("{path}");
    println!(
        "  {} notes, {} mesures, {} – {}",
        notes.len(),
        length / STEPS_PER_BAR,
        note_name(lo),
        note_name(hi)
    );

    let progression = segment(&notes, length);
    println!("  Progression :");
    for s in &progression.segments {
        println!(
            "    {:<8} {:<16} ({})",
            position(s.start),
            s.chord.symbol(),
            duration(s.len())
        );
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("analyze") => match analyze(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("erreur : {e}");
                ExitCode::FAILURE
            }
        },
        _ => usage(),
    }
}
