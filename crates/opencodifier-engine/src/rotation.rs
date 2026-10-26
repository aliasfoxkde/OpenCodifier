//! Rotation readout: position-bias-corrected decision distributions (§31).
//!
//! A readout whose logits depend on *where* an option sits in the list
//! flips answers when the list is reordered — the board's own position
//! bias (footnote 18's by-pos spread 0.58–1.00 is a clean signature).
//! The `AnyJev` `L0` mechanism (RESEARCH.md §31) corrects it label-free:
//! decide the same question over the K cyclic permutations of the
//! candidate list, combine the K distributions in log space, and divide
//! out a label prior estimated from the same K reads (no labels).
//!
//! This module never touches the wire format or the executor: it is a
//! [`Classifier`] decorator, so the ladder, cache, calibration, and gate
//! machinery behave exactly as before — the decorator's `model_id()`
//! re-keys cached decisions mechanically, like any classifier swap.
//!
//! Determinism: the permutation set is fully determined by the
//! candidate count; the combination is plain arithmetic over the inner
//! classifier's deterministic distributions. Same inputs → same output,
//! the [`Classifier`] contract.

use std::sync::Arc;

use opencodifier_core::{Candidate, DecisionQuestion, Distribution, DistributionEntry};

use crate::classifier::Classifier;
use crate::error::{EngineError, EngineResult};

/// How the per-position reads are combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combine {
    /// Log-space mean over the K reads, after prior division. The §31
    /// default: one position's outlier bias cannot dominate.
    LogMean,
    /// Plain arithmetic mean over the K reads.
    ArithmeticMean,
}

/// A [`Classifier`] that decides over the cyclic rotations of a choice
/// question's candidate list and combines the reads. Boolean and score
/// questions have no option list to rotate; they pass through with a
/// single inner read (the decorator is a no-op for them).
///
/// The estimated label prior is the geometric mean of each candidate's
/// probability across rotations; dividing each read by it before
/// combining removes the systematic "this position scores high" bias
/// without any labelled data.
pub struct RotationReadout {
    inner: Arc<dyn Classifier>,
    combine: Combine,
    id: String,
}

impl RotationReadout {
    /// Wraps `inner`. `id` becomes the `model_id` suffix — pick a new
    /// one whenever the combination policy changes, so cached decisions
    /// re-key.
    pub fn new(inner: Arc<dyn Classifier>, id: impl Into<String>) -> Self {
        Self { inner, combine: Combine::LogMean, id: id.into() }
    }

    /// Sets the combination rule, consuming and returning `self`.
    #[must_use]
    pub fn with_combine(mut self, combine: Combine) -> Self {
        self.combine = combine;
        self
    }

    /// The cyclic rotations of `candidates` — K rotations for K
    /// candidates, rotation 0 the identity.
    fn rotations(candidates: &[Candidate]) -> Vec<Vec<Candidate>> {
        let k = candidates.len();
        (0..k)
            .map(|shift| candidates.iter().cycle().skip(shift).take(k).cloned().collect::<Vec<_>>())
            .collect()
    }
}

impl std::fmt::Debug for RotationReadout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RotationReadout")
            .field("inner", &self.inner)
            .field("combine", &self.combine)
            .field("id", &self.id)
            .finish()
    }
}

impl Classifier for RotationReadout {
    fn decide(
        &self,
        state: &opencodifier_core::State,
        question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError> {
        let DecisionQuestion::Choice(choice) = question else {
            // No option list to rotate: a single pass-through read.
            return self.inner.decide(state, question);
        };
        let candidates = choice.candidates();
        if candidates.len() < 2 {
            return self.inner.decide(state, question);
        }

        let reads: Vec<Distribution> = Self::rotations(candidates)
            .iter()
            .map(|rotated| {
                let rotated_choice = opencodifier_core::ChoiceQuestion::new(
                    choice.id().as_str(),
                    choice.text(),
                    rotated.clone(),
                )
                .map_err(|e| EngineError::InvalidConfig {
                    reason: format!("rotation re-render refused: {e}"),
                })?;
                self.inner.decide(state, &DecisionQuestion::Choice(rotated_choice))
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Label prior per candidate: geometric mean of its probability
        // across the K reads. Pure arithmetic on the reads themselves —
        // no labels, no state mutation.
        let k = reads.len();
        #[allow(clippy::cast_precision_loss)]
        let kf = k as f64;
        let mut prior = std::collections::BTreeMap::<String, f64>::new();
        for read in &reads {
            for entry in read.entries() {
                *prior.entry(entry.key.clone()).or_insert(0.0) += entry.probability.ln();
            }
        }
        for v in prior.values_mut() {
            *v = (*v / kf).exp();
        }

        // Divide out the prior, then combine the K corrected reads.
        let mut combined = std::collections::BTreeMap::<String, f64>::new();
        for read in &reads {
            for entry in read.entries() {
                let base = prior.get(&entry.key).copied().unwrap_or(1.0);
                let corrected =
                    if base > f64::EPSILON { entry.probability / base } else { entry.probability };
                *combined.entry(entry.key.clone()).or_insert(0.0) += match self.combine {
                    Combine::ArithmeticMean => corrected,
                    Combine::LogMean => corrected.ln(),
                };
            }
        }
        let scale = match self.combine {
            Combine::ArithmeticMean => kf,
            Combine::LogMean => {
                for v in combined.values_mut() {
                    *v = (*v / kf).exp();
                }
                // log-mean outputs are not yet normalized; normalize below.
                1.0
            }
        };

        let mut entries: Vec<DistributionEntry> = combined
            .into_iter()
            .map(|(key, v)| {
                let p = if scale > 1.0 { v / scale } else { v };
                DistributionEntry { key, probability: p }
            })
            .collect();
        normalize(&mut entries)?;
        Distribution::new(entries).map_err(|e| EngineError::InvalidConfig {
            reason: format!("rotation combination produced an invalid distribution: {e}"),
        })
    }

    fn decide_extended(
        &self,
        state: &opencodifier_core::State,
        question: &DecisionQuestion,
        index: Option<&crate::lexical::Bm25Index>,
    ) -> Result<(Distribution, Option<f64>), EngineError> {
        let _ = index;
        Ok((self.decide(state, question)?, None))
    }

    fn model_id(&self) -> &str {
        &self.id
    }
}

/// Renormalizes to sum 1 within the core's tolerance, guarding against
/// an all-zero correction.
fn normalize(entries: &mut [DistributionEntry]) -> EngineResult<()> {
    let sum: f64 = entries.iter().map(|e| e.probability).sum();
    if sum <= 0.0 || !sum.is_finite() {
        return Err(EngineError::InvalidConfig {
            reason: "rotation combination collapsed to zero mass".into(),
        });
    }
    for e in entries.iter_mut() {
        e.probability /= sum;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::RotationReadout;
    use crate::classifier::{Classifier, MockClassifier};
    use opencodifier_core::{
        Candidate, ChoiceQuestion, DecisionQuestion, Distribution, DistributionEntry, State,
    };
    use std::sync::Arc;

    /// A test classifier that is deliberately position-biased: it always
    /// answers with the FIRST candidate in the list it is shown, at the
    /// same confidence. An unbiased readout must defeat this.
    #[derive(Debug)]
    struct FirstBiased;

    impl Classifier for FirstBiased {
        fn decide(
            &self,
            _state: &State,
            question: &DecisionQuestion,
        ) -> Result<Distribution, crate::error::EngineError> {
            let DecisionQuestion::Choice(choice) = question else {
                return Err(crate::error::EngineError::InvalidConfig {
                    reason: "first-biased mock only answers choice".into(),
                });
            };
            #[allow(clippy::cast_precision_loss)]
            let denom = choice.candidates().len() as f64 - 1.0;
            let mut entries: Vec<DistributionEntry> = choice
                .candidates()
                .iter()
                .enumerate()
                .map(|(i, c)| DistributionEntry {
                    key: c.id().as_str().to_owned(),
                    probability: if i == 0 { 0.9 } else { 0.1 / denom },
                })
                .collect();
            let sum: f64 = entries.iter().map(|e| e.probability).sum();
            for e in &mut entries {
                e.probability /= sum;
            }
            Ok(Distribution::new(entries).unwrap())
        }

        fn model_id(&self) -> &'static str {
            "test/first-biased"
        }
    }

    fn three_candidates() -> Vec<Candidate> {
        vec![
            Candidate::new("a", "alpha").unwrap(),
            Candidate::new("b", "beta").unwrap(),
            Candidate::new("c", "gamma").unwrap(),
        ]
    }

    fn choice_question(candidates: Vec<Candidate>) -> DecisionQuestion {
        DecisionQuestion::Choice(ChoiceQuestion::new("q1", "which one", candidates).unwrap())
    }

    #[test]
    fn boolean_and_score_questions_pass_through() {
        let inner = MockClassifier::new("mock/inner");
        let rot = RotationReadout::new(Arc::new(inner), "rot/test");
        let bool_q = DecisionQuestion::Boolean(
            opencodifier_core::BooleanQuestion::new("b1", "is it so").unwrap(),
        );
        let d = rot.decide(&State::from_text("s"), &bool_q).unwrap();
        assert!((d.entries()[0].probability - 0.5).abs() < 1e-9);
    }

    #[test]
    fn single_candidate_passes_through() {
        let inner = MockClassifier::new("mock/inner");
        let rot = RotationReadout::new(Arc::new(inner), "rot/test");
        let q = choice_question(vec![Candidate::new("only", "the one").unwrap()]);
        let d = rot.decide(&State::from_text("s"), &q).unwrap();
        assert!((d.entries()[0].probability - 1.0).abs() < 1e-9);
    }

    #[test]
    fn uniform_inner_stays_uniform() {
        let inner = MockClassifier::new("mock/inner");
        let rot = RotationReadout::new(Arc::new(inner), "rot/test");
        let q = choice_question(three_candidates());
        let d = rot.decide(&State::from_text("s"), &q).unwrap();
        for e in d.entries() {
            assert!((e.probability - 1.0 / 3.0).abs() < 1e-9, "{e:?}");
        }
    }

    #[test]
    fn position_bias_is_corrected() {
        // The biased inner always picks the first-shown candidate. The
        // rotation readout shows every candidate at every position
        // exactly once, so the corrected distribution must be uniform —
        // the bias is divided out by the label prior.
        let rot = RotationReadout::new(Arc::new(FirstBiased), "rot/bias-test");
        let q = choice_question(three_candidates());
        let d = rot.decide(&State::from_text("s"), &q).unwrap();
        for e in d.entries() {
            assert!((e.probability - 1.0 / 3.0).abs() < 1e-6, "bias survived: {e:?}");
        }
    }

    #[test]
    fn deterministic_across_repeats() {
        let rot = RotationReadout::new(Arc::new(FirstBiased), "rot/det");
        let q = choice_question(three_candidates());
        let s = State::from_text("same state");
        let a = rot.decide(&s, &q).unwrap();
        let b = rot.decide(&s, &q).unwrap();
        for (x, y) in a.entries().iter().zip(b.entries()) {
            assert_eq!(x.key, y.key);
            assert_eq!(x.probability, y.probability);
        }
    }

    #[test]
    fn model_id_carries_the_configuration() {
        let rot = RotationReadout::new(Arc::new(MockClassifier::new("mock/i")), "rot/x1");
        assert_eq!(rot.model_id(), "rot/x1");
    }
}
