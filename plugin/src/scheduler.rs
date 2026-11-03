//! Lecture d'un passage calé sur la position de la chanson, au sample près.
//!
//! La boucle est ancrée sur le pas absolu où la capture a commencé : la position
//! dans le passage est `(pas - ancre) mod longueur`. À chaque retour au début de
//! la boucle, le passage suivant (déjà calculé) remplace le courant.

use harmony_core::generate::Pattern;

/// Nombre maximal de notes tenues en même temps.
pub const MAX_ACTIVE: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    On { offset: u32, pitch: u8, velocity: u8 },
    Off { offset: u32, pitch: u8 },
}

pub struct Scheduler {
    current: Pattern,
    anchor: i64,
    /// Notes en cours : (hauteur, pas absolu de fin).
    active: Vec<(u8, i64)>,
    /// Fin (en pas) du bloc précédent, pour détecter un saut de position.
    last_end: Option<f64>,
    /// Dernier début de boucle traité (pas relatif à l'ancre).
    last_wrap: Option<i64>,
    /// Le passage courant n'a pas encore été joué depuis son début.
    fresh: bool,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            current: Pattern::default(),
            anchor: 0,
            active: Vec::with_capacity(MAX_ACTIVE),
            last_end: None,
            last_wrap: None,
            fresh: false,
        }
    }
}

impl Scheduler {
    /// Commence la lecture de `pattern`, boucle ancrée au pas absolu `anchor`.
    pub fn start(&mut self, pattern: Pattern, anchor: i64) {
        self.current = pattern;
        self.anchor = anchor;
        self.last_wrap = None;
        self.fresh = true;
    }

    pub fn stop(&mut self) {
        self.current = Pattern::default();
        self.last_end = None;
    }

    /// Coupe toutes les notes en cours.
    pub fn all_off(&mut self, offset: u32, emit: &mut impl FnMut(Event)) {
        for (pitch, _) in self.active.drain(..) {
            emit(Event::Off { offset, pitch });
        }
    }

    /// Le transport s'est arrêté : plus aucune note ne doit sonner.
    pub fn transport_stopped(&mut self, emit: &mut impl FnMut(Event)) {
        self.all_off(0, emit);
        self.last_end = None;
    }

    /// Joue le bloc qui commence au pas `start` (fractionnaire) et dure `samples` échantillons.
    /// `next_pass` fournit le passage suivant à chaque début de boucle (`None` : on rejoue le courant).
    pub fn render(
        &mut self,
        start: f64,
        samples: u32,
        samples_per_step: f64,
        next_pass: &mut impl FnMut() -> Option<Pattern>,
        emit: &mut impl FnMut(Event),
    ) {
        if samples == 0 || samples_per_step <= 0.0 {
            return;
        }
        // Saut de position (boucle de Live, déplacement du curseur) : on repart proprement.
        if self.last_end.is_some_and(|e| (start - e).abs() > 0.5) {
            self.all_off(0, emit);
            self.last_wrap = None;
        }
        let end = start + samples as f64 / samples_per_step;
        self.last_end = Some(end);
        let length = self.current.length as i64;
        if length == 0 {
            return;
        }

        let mut step = start.ceil() as i64;
        while (step as f64) < end {
            let offset = (((step as f64 - start) * samples_per_step).round() as u32).min(samples - 1);
            let rel = step - self.anchor;
            let pos = rel.rem_euclid(length);

            if pos == 0 && self.last_wrap != Some(rel) {
                self.last_wrap = Some(rel);
                if self.fresh {
                    self.fresh = false;
                } else if let Some(next) = next_pass() {
                    self.current = next;
                }
            }

            // Fins de notes d'abord, puis attaques.
            let mut i = 0;
            while i < self.active.len() {
                if self.active[i].1 <= step {
                    let (pitch, _) = self.active.swap_remove(i);
                    emit(Event::Off { offset, pitch });
                } else {
                    i += 1;
                }
            }
            for n in self.current.notes.iter().filter(|n| n.start as i64 == pos) {
                if let Some(k) = self.active.iter().position(|a| a.0 == n.pitch) {
                    self.active.swap_remove(k);
                    emit(Event::Off { offset, pitch: n.pitch });
                }
                if self.active.len() < MAX_ACTIVE {
                    self.active.push((n.pitch, step + n.len.max(1) as i64));
                    emit(Event::On { offset, pitch: n.pitch, velocity: n.velocity });
                }
            }
            step += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmony_core::generate::GenNote;

    fn pattern(notes: &[(u32, u32, u8)], length: u32) -> Pattern {
        Pattern {
            length,
            notes: notes.iter().map(|&(start, len, pitch)| GenNote { start, len, pitch, velocity: 100 }).collect(),
        }
    }

    /// Joue `blocks` blocs de 512 échantillons (1 pas = 1000 échantillons).
    fn run(s: &mut Scheduler, from: f64, blocks: usize, passes: &mut Vec<Pattern>) -> Vec<(f64, Event)> {
        let mut out = Vec::new();
        let sps = 1000.0;
        for b in 0..blocks {
            let start = from + b as f64 * 512.0 / sps;
            let mut next = || if passes.is_empty() { None } else { Some(passes.remove(0)) };
            let mut emit = |e: Event| {
                let off = match e {
                    Event::On { offset, .. } | Event::Off { offset, .. } => offset,
                };
                out.push((start + off as f64 / sps, e));
            };
            s.render(start, 512, sps, &mut next, &mut emit);
        }
        out
    }

    #[test]
    fn notes_land_on_their_steps_sample_accurately() {
        let mut s = Scheduler::default();
        s.start(pattern(&[(0, 2, 60), (3, 1, 62)], 4), 8);
        let ev = run(&mut s, 8.0, 8, &mut vec![]);
        let times: Vec<(f64, bool, u8)> = ev
            .iter()
            .map(|(t, e)| match e {
                Event::On { pitch, .. } => (*t, true, *pitch),
                Event::Off { pitch, .. } => (*t, false, *pitch),
            })
            .collect();
        assert_eq!(times[0], (8.0, true, 60));
        assert_eq!(times[1], (10.0, false, 60));
        assert_eq!(times[2], (11.0, true, 62));
        assert_eq!(times[3], (12.0, false, 62));
    }

    #[test]
    fn next_pass_is_taken_at_each_loop_start_but_not_the_first() {
        let mut s = Scheduler::default();
        s.start(pattern(&[(0, 1, 60)], 4), 0);
        let mut passes = vec![pattern(&[(0, 1, 61)], 4), pattern(&[(0, 1, 62)], 4)];
        let ev = run(&mut s, 0.0, 23, &mut passes); // près de 12 pas = 3 boucles
        let ons: Vec<u8> = ev.iter().filter_map(|(_, e)| if let Event::On { pitch, .. } = e { Some(*pitch) } else { None }).collect();
        assert_eq!(ons, vec![60, 61, 62]);
    }

    #[test]
    fn position_jump_cuts_hanging_notes() {
        let mut s = Scheduler::default();
        s.start(pattern(&[(0, 4, 60)], 8), 0);
        run(&mut s, 0.0, 2, &mut vec![]);
        let ev = run(&mut s, 40.0, 1, &mut vec![]); // saut en avant
        assert_eq!(ev[0].1, Event::Off { offset: 0, pitch: 60 });
    }
}
