//! The NLI verbalization verifier (PLAN unit 24j; RESEARCH §15.6.2
//! item 6).
//!
//! A cross-encoder entailment model is a **second opinion structurally
//! independent** of the pointer/logits readout family (§19): the state
//! is the premise, each candidate's verbalization is a hypothesis, and
//! the entailment probabilities — renormalized over candidates — are
//! the decision distribution. The arm of record is
//! `MoritzLaurer/deberta-v3-base-zeroshot-v2.0`, whose own repository
//! ships `onnx/model.onnx` (no conversion happens anywhere near this
//! tree); its head is 2-class, so the entailment probability is the
//! softmax mass of the `entailment` class and the only
//! renormalization is over candidates.
//!
//! The module splits the way `kai` does:
//!
//! * the **contract** — verbalizer templates, the pair rendering, and
//!   the entailment-to-distribution math — always compiles, over the
//!   [`NliScorer`] trait, and its tests run feature-free;
//! * the **serving scorer** — tokenizer plus ONNX graph through
//!   `opencodifier_runtime::nli::NliOnnxBackend` (feature-gated, so no
//!   doc link) — sits behind the
//!   crate's `nli` feature, and the compute-host parity runner
//!   (`benchmarks/validation/runner/nli-parity-rs`) reproduces the
//!   frozen [`FIXTURE`] against the real artifact (weights are never
//!   committed, D14).
//!
//! Honesty rules: every probability is computed from graph logits in
//! Rust (D7) — no mass is ever synthesized; a zero total entailment
//! mass is a refused distribution, not a uniform fallback; and the
//! verbalizer template is part of the model id, because changing the
//! template changes the distribution and must invalidate every cached
//! decision (D6).

use std::sync::Arc;

use opencodifier_core::{DecisionQuestion, Distribution, State};
use opencodifier_engine::classifier::Classifier;
use opencodifier_engine::error::{EngineError, EngineResult};
use serde::{Deserialize, Serialize};

use crate::error::ModelError;
use crate::render::{answers_of, question_text};

/// The frozen parity fixture: two internal-suite choice items with the
/// entailment probabilities and renormalized distributions the real
/// arm of record produces. Reproduced by the compute-host parity
/// runner (`nli-parity-rs`) against the locally fetched artifact.
pub const FIXTURE: &str = include_str!("nli/fixture.json");

/// The logits column the arm of record's 2-class head scores as
/// entailment (its `config.json` `id2label` maps `0` to
/// `entailment`). The serving scorer softmaxes both columns and reads
/// this one; a 3-class arm would also expose the neutral column, and
/// swapping to one is a deliberate contract change.
pub const ENTAIL_INDEX: usize = 0;

/// How a candidate's description becomes an NLI hypothesis.
///
/// The template is a measured lever, not a style choice: the §15.6.2
/// probe moved the same arm from below chance to 0.375 on the internal
/// slice by dropping the "The answer is" scaffold, so the template is
/// explicit, named, and cached (it is part of the model id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Verbalizer {
    /// The candidate description verbatim: `"deployment target alpha
    /// (region=…)."`. The probe's stronger template (t2).
    #[default]
    BareDescription,
    /// `"The answer is <description>."` — the classic pipeline
    /// scaffold, kept because §15.6 item 6's source literature uses
    /// it and the ladder may prefer it per node kind.
    TheAnswerIs,
}

impl Verbalizer {
    /// The hypothesis this template renders `description` into.
    #[must_use]
    pub fn hypothesis(&self, description: &str) -> String {
        match self {
            Self::BareDescription => format!("{description}."),
            Self::TheAnswerIs => format!("The answer is {description}."),
        }
    }

    /// The template's cache-key tag: part of [`model_id`](Classifier::model_id).
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::BareDescription => "bare",
            Self::TheAnswerIs => "answer-is",
        }
    }

    /// Parses a template tag (the inverse of [`Verbalizer::tag`]).
    ///
    /// # Errors
    ///
    /// [`ModelError::ContractViolation`] for a tag this module does
    /// not define.
    pub fn from_tag(tag: &str) -> Result<Self, ModelError> {
        match tag {
            "bare" => Ok(Self::BareDescription),
            "answer-is" => Ok(Self::TheAnswerIs),
            other => Err(ModelError::ContractViolation {
                reason: format!("unknown verbalizer tag {other:?}"),
            }),
        }
    }
}

/// The premise the verifier reads the state into: the full state text,
/// then the question, so the hypothesis is judged against what was
/// asked as well as what is known.
#[must_use]
pub fn premise(state: &State, question: &DecisionQuestion) -> String {
    format!("{}\nQuestion: {}", state.text(), question_text(question))
}

/// The `(answer key, hypothesis)` pairs `question` verbalizes into.
///
/// Choice and score questions get one hypothesis per declared answer;
/// a boolean gets one — the question text itself — because a 2-class
/// entailment head already answers "does the state support this
/// claim?", and the `false` mass is its complement. `None` for a kind
/// this engine does not know (`DecisionQuestion` is
/// `#[non_exhaustive]`).
#[must_use]
pub fn hypotheses(
    state: &State,
    question: &DecisionQuestion,
    template: Verbalizer,
) -> Option<Vec<(String, String)>> {
    let _ = state;
    match question {
        DecisionQuestion::Choice(choice) => Some(
            choice
                .candidates()
                .iter()
                .map(|candidate| {
                    (candidate.id().to_string(), template.hypothesis(candidate.description()))
                })
                .collect(),
        ),
        DecisionQuestion::Boolean(_) => {
            Some(vec![("true".to_owned(), question_text(question).to_owned())])
        }
        DecisionQuestion::Score(score) => Some(
            score
                .levels()
                .iter()
                .map(|level| (level.label().to_owned(), template.hypothesis(level.label())))
                .collect(),
        ),
        _ => None,
    }
}

/// Renormalizes raw entailment masses into the decision distribution.
///
/// This is the whole decision math (D7): one softmax inside the
/// scorer, then a mass-normalization over candidates here. A total
/// that is not finite and positive is refused — a uniform fallback
/// would fabricate evidence.
///
/// # Errors
///
/// [`ModelError::ContractViolation`] when the masses do not sum to a
/// normalizable total or the slice lengths disagree.
pub fn entailment_distribution(
    keys: &[String],
    masses: &[f64],
) -> Result<Distribution, ModelError> {
    if keys.len() != masses.len() {
        return Err(ModelError::ContractViolation {
            reason: format!(
                "{} answer keys but {} entailment masses — the scorer and the \
                 question disagree",
                keys.len(),
                masses.len()
            ),
        });
    }
    let total: f64 = masses.iter().sum();
    if !total.is_finite() || total <= 0.0 {
        return Err(ModelError::ContractViolation {
            reason: format!("entailment masses sum to {total}, not a normalizable total"),
        });
    }
    Distribution::from_pairs(
        keys.iter().zip(masses.iter()).map(|(key, mass)| (key.clone(), mass / total)),
    )
    .map_err(|error| ModelError::ContractViolation { reason: error.to_string() })
}

/// One `(premise, hypothesis) -> entailment mass` source.
///
/// The contract's seam: the serving scorer implements it over the ONNX
/// graph; tests implement it over scripted values. Implementations
/// must be deterministic — the decision cache is keyed on exactly
/// these inputs.
pub trait NliScorer: std::fmt::Debug + Send + Sync {
    /// Entailment mass for each hypothesis against the premise, in
    /// order. One mass per hypothesis, never a distribution — the
    /// normalization over candidates happens once, in
    /// [`entailment_distribution`].
    ///
    /// # Errors
    ///
    /// Implementation-defined ([`ModelError`]); a scorer must never
    /// panic on any input.
    fn entailment_probabilities(
        &self,
        premise: &str,
        hypotheses: &[String],
    ) -> Result<Vec<f64>, ModelError>;
}

/// The frozen fixture, deserialized: what the parity test walks.
#[derive(Debug, Deserialize, Serialize)]
struct Fixture {
    items: Vec<FixtureItem>,
}

#[derive(Debug, Deserialize, Serialize)]
struct FixtureItem {
    premise: String,
    rows: Vec<FixtureRow>,
}

#[derive(Debug, Deserialize, Serialize)]
struct FixtureRow {
    hypothesis: String,
    entailment: f64,
}

/// Reproduces the frozen fixture against a scorer.
///
/// The parity gate for the whole verbalization path — rendering,
/// tokenization, graph execution, softmax, class pick — against the
/// distributions the real arm of record produced when the fixture was
/// frozen (2026-10-08, fp32 `onnx/model.onnx`).
///
/// # Errors
///
/// [`ModelError::ContractViolation`] on the first row that drifts
/// beyond `1e-4` (the fixture freezes 6 decimal places; the tolerance
/// absorbs the f32 graph path).
pub fn verify_fixture(scorer: &dyn NliScorer) -> Result<usize, ModelError> {
    let fixture: Fixture =
        serde_json::from_str(FIXTURE).map_err(|error| ModelError::ContractViolation {
            reason: format!("the frozen fixture is unreadable: {error}"),
        })?;
    for item in &fixture.items {
        let hypotheses: Vec<String> = item.rows.iter().map(|row| row.hypothesis.clone()).collect();
        let masses = scorer.entailment_probabilities(&item.premise, &hypotheses)?;
        for (row, mass) in item.rows.iter().zip(masses.iter()) {
            if (row.entailment - mass).abs() > 1e-4 {
                return Err(ModelError::ContractViolation {
                    reason: format!(
                        "parity drift on {:?}: fixture {:.6}, scorer {:.6}",
                        row.hypothesis, row.entailment, mass
                    ),
                });
            }
        }
    }
    Ok(fixture.items.iter().map(|item| item.rows.len()).sum())
}

/// The NLI verifier as an engine [`Classifier`].
///
/// Construct it around any [`NliScorer`] (the serving scorer behind
/// the `nli` feature, or a scripted stand-in in tests).
#[derive(Debug)]
pub struct NliVerifier {
    scorer: Arc<dyn NliScorer>,
    template: Verbalizer,
    model_id: String,
}

impl NliVerifier {
    /// Builds a verifier over `scorer`.
    ///
    /// The model id folds the scorer's own identity and the verbalizer
    /// tag: either changing the graph or the template changes the
    /// distribution, so either must invalidate cached decisions (D6).
    #[must_use]
    pub fn new(scorer: Arc<dyn NliScorer>, model_prefix: &str, template: Verbalizer) -> Self {
        Self { scorer, template, model_id: format!("{model_prefix}:{}", template.tag()) }
    }

    /// The frozen fixture's parity verdict against this verifier's
    /// scorer (see [`verify_fixture`]).
    ///
    /// # Errors
    ///
    /// Whatever [`verify_fixture`] reports.
    pub fn verify_parity(&self) -> Result<usize, ModelError> {
        verify_fixture(self.scorer.as_ref())
    }
}

impl Classifier for NliVerifier {
    fn decide(&self, state: &State, question: &DecisionQuestion) -> EngineResult<Distribution> {
        let Some(keys) = answers_of(question) else {
            return Err(EngineError::ClassifierFailed {
                model_id: self.model_id.clone(),
                reason: "unsupported question kind: the NLI verifier defines choice, \
                         boolean, and score readouts only"
                    .to_owned(),
            });
        };
        let Some(pairs) = hypotheses(state, question, self.template) else {
            return Err(EngineError::ClassifierFailed {
                model_id: self.model_id.clone(),
                reason: "unsupported question kind: no verbalization exists".to_owned(),
            });
        };
        // A boolean verbalizes to one hypothesis; its `false` mass is
        // the complement, keeping both declared keys in the
        // distribution without a second graph pass.
        let masses: Vec<f64> = if pairs.len() == keys.len() {
            self.scorer
                .entailment_probabilities(&premise(state, question), &hypotheses_only(&pairs))
                .map_err(|error| EngineError::ClassifierFailed {
                    model_id: self.model_id.clone(),
                    reason: error.to_string(),
                })?
        } else {
            let single = self
                .scorer
                .entailment_probabilities(&premise(state, question), &hypotheses_only(&pairs))
                .map_err(|error| EngineError::ClassifierFailed {
                    model_id: self.model_id.clone(),
                    reason: error.to_string(),
                })?;
            match (single.first(), keys.as_slice()) {
                (Some(entailed), keys)
                    if keys.len() == 2 && keys[0] == "true" && keys[1] == "false" =>
                {
                    vec![*entailed, 1.0 - entailed]
                }
                _ => {
                    return Err(EngineError::ClassifierFailed {
                        model_id: self.model_id.clone(),
                        reason: format!(
                            "boolean verbalization produced {} masses for the \
                             true/false pair",
                            single.len()
                        ),
                    });
                }
            }
        };
        entailment_distribution(&keys, &masses).map_err(|error| EngineError::ClassifierFailed {
            model_id: self.model_id.clone(),
            reason: error.to_string(),
        })
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }
}

fn hypotheses_only(pairs: &[(String, String)]) -> Vec<String> {
    pairs.iter().map(|(_, hypothesis)| hypothesis.clone()).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use std::sync::Arc;

    use opencodifier_core::{
        BooleanQuestion, Candidate, ChoiceQuestion, ScoreLevel, ScoreQuestion,
    };

    use super::*;

    /// A scripted scorer: one canned mass per hypothesis, in order.
    #[derive(Debug)]
    struct Scripted(Vec<f64>);

    impl NliScorer for Scripted {
        fn entailment_probabilities(
            &self,
            _premise: &str,
            hypotheses: &[String],
        ) -> Result<Vec<f64>, ModelError> {
            if self.0.len() != hypotheses.len() {
                return Err(ModelError::ContractViolation {
                    reason: format!("scripted {} for {}", self.0.len(), hypotheses.len()),
                });
            }
            Ok(self.0.clone())
        }
    }

    /// A scripted scorer: the same mass for every hypothesis.
    #[derive(Debug)]
    struct Arbitrary(f64);

    impl NliScorer for Arbitrary {
        fn entailment_probabilities(
            &self,
            _premise: &str,
            hypotheses: &[String],
        ) -> Result<Vec<f64>, ModelError> {
            Ok(vec![self.0; hypotheses.len()])
        }
    }

    fn choice() -> ChoiceQuestion {
        ChoiceQuestion::new(
            "q1",
            "which target?",
            vec![
                Candidate::new("a", "alpha cluster").expect("valid id"),
                Candidate::new("b", "beta cluster").expect("valid id"),
            ],
        )
        .expect("valid question")
    }

    #[test]
    fn templates_render_and_round_trip_through_tags() {
        assert_eq!(Verbalizer::default(), Verbalizer::BareDescription);
        assert_eq!(Verbalizer::BareDescription.hypothesis("alpha"), "alpha.");
        assert_eq!(Verbalizer::TheAnswerIs.hypothesis("alpha"), "The answer is alpha.");
        assert_eq!(Verbalizer::BareDescription.tag(), "bare");
        assert_eq!(Verbalizer::TheAnswerIs.tag(), "answer-is");
        assert_eq!(Verbalizer::from_tag("bare"), Ok(Verbalizer::BareDescription));
        assert_eq!(Verbalizer::from_tag("answer-is"), Ok(Verbalizer::TheAnswerIs));
        assert!(Verbalizer::from_tag("strict").is_err());
    }

    #[test]
    fn renormalization_preserves_order_and_sums_to_one() {
        let keys = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let distribution = entailment_distribution(&keys, &[0.2, 0.1, 0.1]).expect("normalizable");
        let mass = |key: &str| distribution.probability_of(key).expect("present");
        assert_eq!(mass("a"), 0.5);
        assert_eq!(mass("b"), 0.25);
        assert_eq!(mass("c"), 0.25);
        let total: f64 = distribution.entries().iter().map(|e| e.probability).sum();
        assert!((total - 1.0).abs() < 1e-12);
    }

    #[test]
    fn renormalization_refuses_degenerate_mass() {
        let keys = vec!["a".to_owned(), "b".to_owned()];
        let zero = entailment_distribution(&keys, &[0.0, 0.0]).unwrap_err();
        assert!(zero.to_string().contains("not a normalizable total"), "{zero}");
        let mismatch = entailment_distribution(&keys, &[0.5]).unwrap_err();
        assert!(mismatch.to_string().contains("disagree"), "{mismatch}");
    }

    #[test]
    fn decide_ranks_candidates_by_entailment_mass() {
        let state = State::from_text("the cluster is in alpha region");
        let verifier = NliVerifier::new(
            Arc::new(Scripted(vec![0.4, 0.1])),
            "nli-zeroshot:model.onnx",
            Verbalizer::BareDescription,
        );
        let distribution =
            verifier.decide(&state, &DecisionQuestion::Choice(choice())).expect("decides");
        assert_eq!(distribution.top().key, "a");
        assert!((distribution.probability_of("a").unwrap() - 0.8).abs() < 1e-12);
        assert_eq!(
            verifier.model_id(),
            "nli-zeroshot:model.onnx:bare",
            "template and graph identity are both in the cache key (D6)"
        );
    }

    #[test]
    fn boolean_complements_one_pass_into_two_keys() {
        let question = BooleanQuestion::new("b1", "is the cluster reachable?").expect("valid");
        let state = State::from_text("the cluster answers on port 443");
        let verifier = NliVerifier::new(
            Arc::new(Scripted(vec![0.7])),
            "nli-zeroshot:model.onnx",
            Verbalizer::BareDescription,
        );
        let distribution =
            verifier.decide(&state, &DecisionQuestion::Boolean(question)).expect("decides");
        assert!((distribution.probability_of("true").unwrap() - 0.7).abs() < 1e-12);
        assert!((distribution.probability_of("false").unwrap() - 0.3).abs() < 1e-12);
    }

    #[test]
    fn score_levels_verbalize_their_labels() {
        let question = ScoreQuestion::new(
            "s1",
            "how hard?",
            vec![ScoreLevel::new("low").expect("valid"), ScoreLevel::new("high").expect("valid")],
        )
        .expect("valid");
        let state = State::from_text("one task, two known levels");
        let verifier = NliVerifier::new(
            Arc::new(Scripted(vec![0.3, 0.9])),
            "nli-zeroshot:model.onnx",
            Verbalizer::TheAnswerIs,
        );
        let distribution =
            verifier.decide(&state, &DecisionQuestion::Score(question)).expect("decides");
        assert_eq!(distribution.top().key, "high");
        assert_eq!(
            verifier.model_id(),
            "nli-zeroshot:model.onnx:answer-is",
            "the template tag changes the id and invalidates cached decisions (D6)"
        );
    }

    #[test]
    fn fixture_is_parseable_and_declares_its_arm() {
        let fixture: serde_json::Value = serde_json::from_str(FIXTURE).expect("valid json");
        assert_eq!(fixture["entail_index"], 0);
        assert_eq!(fixture["template"], "bare_description");
        let rows = fixture["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item["rows"].as_array().expect("rows").len())
            .sum::<usize>();
        assert!(rows >= 10, "the fixture must pin enough rows to catch drift, got {rows}");
    }

    #[test]
    fn scripted_drift_fails_the_parity_gate() {
        // Returns 0.5 for every hypothesis — never what the frozen
        // rows say — so the parity gate must reject it on row one.
        let scorer = Arbitrary(0.5);
        let error = verify_fixture(&scorer).unwrap_err();
        assert!(error.to_string().contains("parity drift"), "{error}");
    }
}

#[cfg(feature = "nli")]
pub mod serving;

// The live parity run — the serving scorer reproducing the frozen
// fixture through the real graph — is a standalone compute-host
// runner (`benchmarks/validation/runner/nli-parity-rs`), not a
// `cargo test` case: ORT's load-dynamic teardown does not survive the
// test harness's exit (measured: all tests pass, harness then dies on
// SIGSEGV). The runners exist for the kai and julia contracts for the
// same reason.
