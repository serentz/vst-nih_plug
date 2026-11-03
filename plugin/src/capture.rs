//! Première passe : mise en mémoire de la progression jouée sur l'entrée MIDI.
//!
//! Les positions sont en pas absolus de la chanson (double croche, fractionnaires),
//! la capture couvre `[start, start + length)`.

use harmony_core::analysis::TimedNote;
use harmony_core::STEPS_PER_BEAT;

/// Tolérance pour une note jouée un peu avant le début de la capture (humanisation).
const EARLY_TOLERANCE: f64 = 1.0;

pub struct Capture {
    pub start: f64,
    pub length: u32,
    notes: Vec<TimedNote>,
    /// Début (pas relatif) et vélocité des notes tenues, par canal et hauteur.
    held: Box<[[Option<(f64, u8)>; 128]; 16]>,
}

impl Capture {
    pub fn new(start: f64, length: u32) -> Self {
        Self {
            start,
            length,
            notes: Vec::with_capacity(1024),
            held: Box::new([[None; 128]; 16]),
        }
    }

    pub fn end(&self) -> f64 {
        self.start + self.length as f64
    }

    pub fn note_on(&mut self, at: f64, channel: u8, pitch: u8, velocity: u8) {
        let rel = at - self.start;
        if rel < -EARLY_TOLERANCE || rel >= self.length as f64 {
            return;
        }
        self.close(at, channel, pitch);
        self.held[(channel & 15) as usize][(pitch & 127) as usize] = Some((rel.max(0.0), velocity));
    }

    pub fn note_off(&mut self, at: f64, channel: u8, pitch: u8) {
        self.close(at, channel, pitch);
    }

    fn close(&mut self, at: f64, channel: u8, pitch: u8) {
        if let Some((rel_start, velocity)) = self.held[(channel & 15) as usize][(pitch & 127) as usize].take() {
            let rel_end = (at - self.start).min(self.length as f64);
            if rel_end > rel_start && self.notes.len() < self.notes.capacity() {
                let spb = STEPS_PER_BEAT as f64;
                self.notes.push(TimedNote::from_beats(rel_start / spb, (rel_end - rel_start) / spb, pitch, velocity));
            }
        }
    }

    pub fn is_done(&self, now: f64) -> bool {
        now >= self.end()
    }

    /// Termine la capture : les notes encore tenues sont fermées à la fin de la boucle.
    pub fn finish(mut self) -> Vec<TimedNote> {
        let end = self.end();
        for ch in 0..16u8 {
            for pitch in 0..128u8 {
                if self.held[ch as usize][pitch as usize].is_some() {
                    self.close(end, ch, pitch);
                }
            }
        }
        self.notes.sort_by_key(|n| (n.start, n.pitch));
        self.notes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_relative_quantized_notes_and_closes_held_ones() {
        let mut c = Capture::new(64.0, 32);
        c.note_on(63.7, 0, 48, 90); // un peu en avance : ramenée au début
        c.note_on(64.0, 0, 55, 80);
        c.note_off(80.1, 0, 48);
        c.note_on(80.0, 0, 51, 70);
        c.note_on(100.0, 0, 60, 70); // hors capture
        assert!(!c.is_done(95.9));
        assert!(c.is_done(96.0));
        let notes = c.finish();
        let summary: Vec<(u32, u32, u8)> = notes.iter().map(|n| (n.start, n.end, n.pitch)).collect();
        assert_eq!(summary, vec![(0, 16, 48), (0, 32, 55), (16, 32, 51)]);
    }
}
