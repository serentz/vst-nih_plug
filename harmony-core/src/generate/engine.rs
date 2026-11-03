//! Enchaînement des passages, avec rendus calculés à l'avance.
//!
//! Le premier passage joue le modèle ; chaque passage suivant est une variation du
//! précédent. `lookahead` rendus (1 ou 2) sont toujours prêts dans la file, pour que
//! le thread audio n'ait jamais à attendre un calcul : il prend le suivant, et le
//! moteur en calcule un nouveau en arrière-plan.

use super::rng::Rng;
use super::variation::{vary, LineKind, VariationParams};
use super::{Context, Pattern};
use crate::analysis::Progression;
use crate::theory::Scale;
use std::collections::VecDeque;

pub struct PassEngine {
    progression: Progression,
    scale: Scale,
    kind: LineKind,
    pub params: VariationParams,
    /// Revenir au modèle tous les N passages (0 = jamais).
    pub reset_every: u32,
    base: Pattern,
    /// Dernier rendu mis en file (point de départ de la prochaine variation).
    tail: Pattern,
    queue: VecDeque<Pattern>,
    lookahead: usize,
    /// Numéro du prochain passage à calculer (1 = modèle).
    next_index: u32,
    rng: Rng,
}

impl PassEngine {
    pub fn new(
        progression: Progression,
        scale: Scale,
        kind: LineKind,
        base: Pattern,
        params: VariationParams,
        seed: u64,
        lookahead: usize,
    ) -> Self {
        let mut engine = Self {
            progression,
            scale,
            kind,
            params,
            reset_every: 0,
            tail: base.clone(),
            base: base.clone(),
            queue: VecDeque::from([base]),
            lookahead: lookahead.clamp(1, 2),
            next_index: 2,
            rng: Rng::new(seed),
        };
        engine.refill();
        engine
    }

    /// Calcule les rendus manquants pour avoir `1 + lookahead` passages prêts.
    fn refill(&mut self) {
        while self.queue.len() < 1 + self.lookahead {
            let next = if self.reset_every > 0 && (self.next_index - 1) % self.reset_every == 0 {
                self.base.clone()
            } else {
                let ctx = Context { progression: &self.progression, scale: self.scale };
                vary(&self.tail, &self.base, &ctx, self.kind, &self.params, &mut self.rng)
            };
            self.tail = next.clone();
            self.queue.push_back(next);
            self.next_index += 1;
        }
    }

    /// Passage à jouer maintenant ; le suivant est déjà prêt.
    pub fn next_pass(&mut self) -> Pattern {
        let pass = self.queue.pop_front().expect("file jamais vide");
        self.refill();
        pass
    }

    /// Rendus déjà calculés, du prochain au plus lointain.
    pub fn ready(&self) -> impl Iterator<Item = &Pattern> {
        self.queue.iter()
    }

    /// Repart du modèle au prochain passage calculé (bouton « revenir au modèle »).
    pub fn back_to_model(&mut self) {
        self.queue.truncate(1);
        self.tail = self.base.clone();
        self.queue.push_back(self.base.clone());
        self.refill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::melody;
    use crate::generate::test_support::progression;

    fn engine(lookahead: usize) -> (PassEngine, Pattern) {
        let (prog, scale) = progression(8);
        let ctx = Context { progression: &prog, scale };
        let base = melody::generate(&ctx, &Default::default(), &mut Rng::new(1));
        let e = PassEngine::new(prog.clone(), scale, LineKind::Melody, base.clone(), Default::default(), 9, lookahead);
        (e, base)
    }

    #[test]
    fn first_pass_is_the_model_and_lookahead_is_ready() {
        let (mut e, base) = engine(2);
        assert_eq!(e.ready().count(), 3);
        assert_eq!(e.next_pass(), base);
        assert_eq!(e.ready().count(), 3);
        assert_ne!(e.next_pass(), base);
    }

    #[test]
    fn reset_every_returns_to_the_model() {
        let (mut e, base) = engine(1);
        e.reset_every = 4;
        let passes: Vec<Pattern> = (0..9).map(|_| e.next_pass()).collect();
        // Passages 1, 5 et 9 : le modèle ; entre les deux, des variations.
        assert_eq!(passes[0], base);
        assert_ne!(passes[1], base);
        assert_eq!(passes[4], base);
        assert_eq!(passes[8], base);
    }

    #[test]
    fn back_to_model_restarts_from_the_model() {
        let (mut e, base) = engine(2);
        for _ in 0..5 {
            e.next_pass();
        }
        e.back_to_model();
        e.next_pass(); // passage déjà prêt au moment du clic
        assert_eq!(e.next_pass(), base);
    }
}
