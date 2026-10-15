//! The decision layer: one trait and the two V1 implementations
//! (PLANNING.md §13, §43).
//!
//! [`Classifier`] is the seam every deciding component plugs into — the
//! lexical scorer today, the candidate-conditioned decision model in Phase
//! 7, a verifier in Phase 10. It returns a *distribution*, never a single
//! label, because confidence is computed downstream from the whole
//! distribution (PLANNING.md §18).
//!
//! # V1 implementations
//!
//! * [`MockClassifier`] — the plan's own §43 test stand-in. Scripted or
//!   uniform, fully deterministic. It is a **test and reference
//!   implementation**: it decides by table lookup, not by understanding,
//!   and must never be wired into a production path.
//! * [`LexicalClassifier`] — wraps the BM25 scorer. For boolean questions
//!   it scores the state text against the question's terms as an
//!   affirmative lexical hypothesis, with a zero-score null hypothesis on
//!   the other side, flipped by negation words in the question. This is a
//!   **lexical baseline**, honestly weak: no synonyms, no embeddings, no
//!   semantics. A real semantic model arrives with the ML runtime crate
//!   (Phases 6+).

use opencodifier_core::{
    BooleanQuestion, CandidateId, DecisionQuestion, Distribution, ScoreQuestion, State,
};
use std::collections::BTreeMap;

use crate::error::{EngineError, EngineResult};
use crate::lexical::{Bm25Index, bound_lexical_spread, negation_polarity, softmax};

/// Something that can decide one question about one state.
///
/// Implementations must be deterministic and side-effect free: the same
/// `(state, question)` pair must always produce the same distribution,
/// because decisions are cached under keys derived from exactly those
/// inputs.
pub trait Classifier: std::fmt::Debug + Send + Sync {
    /// Produces a normalized distribution over the question's answers.
    ///
    /// Keys are candidate ids for choice questions, `"true"`/`"false"` for
    /// boolean questions, and level labels for score questions.
    fn decide(
        &self,
        state: &State,
        question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError>;

    /// [`Classifier::decide`] plus the rung's optional lexical support
    /// signal: the coverage (0–1) of the best-scoring document over the
    /// query's informative tokens, `None` when the rung exposes no
    /// lexical evidence (D28). The `index` is the build-once handoff —
    /// an index the caller already built over the *identical* document
    /// set; implementations may ignore it and build their own (same
    /// documents, same scores, either way).
    fn decide_extended(
        &self,
        state: &State,
        question: &DecisionQuestion,
        index: Option<&Bm25Index>,
    ) -> Result<(Distribution, Option<f64>), EngineError> {
        let _ = index;
        Ok((self.decide(state, question)?, None))
    }

    /// The model identity folded into cache keys (PLANNING.md §64).
    ///
    /// Required, not defaulted: a classifier swap that keeps the same
    /// model id would serve stale cached decisions.
    fn model_id(&self) -> &str;
}

/// The keys a distribution over `question` must use.
///
/// Empty for a question kind this engine does not know: `DecisionQuestion`
/// is `#[non_exhaustive]`, so a future variant must degrade to "no answers
/// available" rather than fail to compile here.
fn answer_keys(question: &DecisionQuestion) -> Vec<String> {
    match question {
        DecisionQuestion::Choice(choice) => {
            choice.candidates().iter().map(|candidate| candidate.id().as_str().to_owned()).collect()
        }
        DecisionQuestion::Boolean(_) => vec!["true".to_owned(), "false".to_owned()],
        DecisionQuestion::Score(score) => {
            score.levels().iter().map(|level| level.label().to_owned()).collect()
        }
        // `DecisionQuestion` is `#[non_exhaustive]`: an unknown kind has no
        // answer keys, which `uniform` reports instead of dividing by zero.
        _ => Vec::new(),
    }
}

/// A scripted, deterministic test stand-in (PLANNING.md §43).
///
/// Decisions come from a lookup table keyed by question id; questions with
/// no entry get a uniform distribution, which is the "no opinion" answer.
/// Both paths are pure, so a graph can be exercised end to end before any
/// model exists.
///
/// This is deliberately **not** a production classifier: it has no notion
/// of the input text at all.
#[derive(Debug, Clone, Default)]
pub struct MockClassifier {
    model_id: String,
    scripted: BTreeMap<String, Distribution>,
}

impl MockClassifier {
    /// Builds a mock whose unscripted questions are answered uniformly.
    #[must_use]
    pub fn new(model_id: impl Into<String>) -> Self {
        Self { model_id: model_id.into(), scripted: BTreeMap::new() }
    }

    /// Adds a script entry, consuming and returning `self`.
    ///
    /// # Errors
    ///
    /// [`EngineError::Core`] when `pairs` do not form a normalized
    /// distribution.
    pub fn with_script<K: Into<String>>(
        mut self,
        question_id: K,
        pairs: Vec<(&str, f64)>,
    ) -> EngineResult<Self> {
        let distribution = Distribution::from_pairs(pairs)?;
        self.scripted.insert(question_id.into(), distribution);
        Ok(self)
    }

    /// Number of scripted questions.
    #[must_use]
    pub fn scripted(&self) -> usize {
        self.scripted.len()
    }

    fn uniform(question: &DecisionQuestion) -> EngineResult<Distribution> {
        let keys = answer_keys(question);
        if keys.is_empty() {
            return Err(EngineError::InvalidDistribution {
                question: question.id().to_string(),
                reason: "question kind has no answer keys".to_owned(),
            });
        }
        let mass = 1.0 / f64::from(u32::try_from(keys.len()).unwrap_or(u32::MAX));
        // Normalized by construction: `keys` are unique and the masses sum
        // to exactly 1.
        Ok(Distribution::from_pairs(keys.into_iter().map(|key| (key, mass)))?)
    }
}

impl Classifier for MockClassifier {
    fn decide(
        &self,
        _state: &State,
        question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError> {
        match self.scripted.get(question.id().as_str()) {
            Some(distribution) => Ok(distribution.clone()),
            None => Self::uniform(question),
        }
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }
}

/// The built-in lexical classifier: BM25 over candidate descriptions,
/// level labels, or boolean hypotheses.
///
/// Deterministic, allocation-light, and model-free — the "fast semantic
/// scoring" rung of the pipeline, which is to say: lexical, not semantic.
#[derive(Debug, Clone, Default)]
pub struct LexicalClassifier {
    model_id: String,
}

impl LexicalClassifier {
    /// Builds the classifier under its built-in model id.
    ///
    /// `v2` is the confidence-bounded lexical hypothesis (D35): identical
    /// predictions to `v1` — the bound preserves score order — with
    /// distributions strictly under 1.0. The bump re-keys every cached
    /// lexical decision, which is the point: cached `v1` confidences
    /// include saturated 1.0 values that exact-proof gates accepted as
    /// proofs.
    #[must_use]
    pub fn new() -> Self {
        Self { model_id: "builtin-lexical-v2".to_owned() }
    }

    /// Overrides the model id (bump it if the heuristic changes, so cached
    /// decisions are invalidated).
    #[must_use]
    pub fn with_model_id(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = model_id.into();
        self
    }

    /// The text lexical evidence is drawn from: the question plus the
    /// state text.
    #[must_use]
    pub fn query_for(state: &State, question: &DecisionQuestion) -> String {
        format!("{} {}", question.text(), state.text())
    }

    /// Choice: BM25 over candidate descriptions, softmaxed into a
    /// distribution over candidate ids, in candidate order. Also returns
    /// the best document's lexical coverage (D28).
    fn decide_choice_with_support(
        query: &str,
        question: &opencodifier_core::ChoiceQuestion,
        index: Option<&Bm25Index>,
    ) -> EngineResult<(Distribution, f64)> {
        let documents: Vec<&str> =
            question.candidates().iter().map(opencodifier_core::Candidate::description).collect();
        // Build-once handoff (D28/#80): a caller-provided index over the
        // identical candidate set replaces the local build; anything else
        // is ignored in favor of the correct local one.
        let local;
        let index = match index {
            Some(prebuilt) if prebuilt.len() == documents.len() => prebuilt,
            _ => {
                local = Bm25Index::new(documents);
                &local
            }
        };
        // The spread bound (D35) runs before the softmax: a long state
        // scores candidate descriptions through a term-frequency product
        // that grows with state length, and an unbounded softmax converts
        // that into exactly 1.0 — a saturated lexical distribution that
        // exact-proof gates then treat as a proof.
        let scores = bound_lexical_spread(index.score_all(query));
        let best = scores
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map_or(0, |(position, _)| position);
        let coverage = index.coverage(best, query);
        let probabilities = softmax(&scores);
        let pairs: Vec<(String, f64)> = question
            .candidates()
            .iter()
            .zip(probabilities)
            .map(|(candidate, probability)| (candidate.id().as_str().to_owned(), probability))
            .collect();
        let distribution =
            Distribution::from_pairs(pairs).map_err(|error| EngineError::InvalidDistribution {
                question: question.id().to_string(),
                reason: error.to_string(),
            })?;
        Ok((distribution, coverage))
    }

    /// Choice: BM25 over candidate descriptions, softmaxed into a
    /// distribution over candidate ids, in candidate order.
    fn decide_choice(
        query: &str,
        question: &opencodifier_core::ChoiceQuestion,
    ) -> EngineResult<Distribution> {
        Ok(Self::decide_choice_with_support(query, question, None)?.0)
    }

    /// Boolean: the affirmative hypothesis is "the state text supports the
    /// question's terms"; the negative hypothesis is the zero-score null.
    ///
    /// The query is the state text only. The question's own terms are the
    /// document being scored, so feeding them back into the query would
    /// make every question support itself.
    ///
    /// Negation words in the *question* flip the polarity, so "does this
    /// request not need tools?" reads inverted. Both moves are crude and
    /// documented as such: this is a lexical baseline that will frequently
    /// abstain by returning a near-uniform distribution.
    fn decide_boolean(state_text: &str, question: &BooleanQuestion) -> EngineResult<Distribution> {
        Ok(Self::decide_boolean_with_support(state_text, question)?.0)
    }

    /// [`Self::decide_boolean`] plus the question document's coverage of
    /// the state text's informative tokens (D28). The polarity flip does
    /// not affect coverage: it measures grounding, not direction.
    fn decide_boolean_with_support(
        state_text: &str,
        question: &BooleanQuestion,
    ) -> EngineResult<(Distribution, f64)> {
        let index = Bm25Index::new([question.text()]);
        let support = index.score_all(state_text);
        let coverage = index.coverage(0, state_text);
        let evidence = support.first().copied().unwrap_or(0.0);
        // The affirmative hypothesis is an unbounded BM25 score against a
        // fixed zero null — exactly the shape that saturates on long
        // states (D35). Bounded before the flip: the bound is per-element
        // given the same max, so polarity does not affect it.
        let mut scores = bound_lexical_spread(vec![evidence, 0.0]);
        if negation_polarity(question.text()) < 0.0 {
            scores.reverse();
        }
        let probabilities = softmax(&scores);
        let distribution =
            Distribution::from_pairs([("true", probabilities[0]), ("false", probabilities[1])])
                .map_err(|error| EngineError::InvalidDistribution {
                    question: question.id().to_string(),
                    reason: error.to_string(),
                })?;
        Ok((distribution, coverage))
    }

    /// Score: BM25 over level labels, softmaxed into a distribution over
    /// labels. A state that mentions "expert" work leans towards the
    /// `expert` level and nothing else.
    fn decide_score(query: &str, question: &ScoreQuestion) -> EngineResult<Distribution> {
        Ok(Self::decide_score_with_support(query, question)?.0)
    }

    /// [`Self::decide_score`] plus the best label's coverage (D28).
    fn decide_score_with_support(
        query: &str,
        question: &ScoreQuestion,
    ) -> EngineResult<(Distribution, f64)> {
        let labels: Vec<&str> =
            question.levels().iter().map(opencodifier_core::ScoreLevel::label).collect();
        let index = Bm25Index::new(labels);
        let scores = bound_lexical_spread(index.score_all(query));
        let best = scores
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map_or(0, |(position, _)| position);
        let coverage = index.coverage(best, query);
        let probabilities = softmax(&scores);
        let pairs: Vec<(String, f64)> = question
            .levels()
            .iter()
            .zip(probabilities)
            .map(|(level, probability)| (level.label().to_owned(), probability))
            .collect();
        let distribution =
            Distribution::from_pairs(pairs).map_err(|error| EngineError::InvalidDistribution {
                question: question.id().to_string(),
                reason: error.to_string(),
            })?;
        Ok((distribution, coverage))
    }
}

impl Classifier for LexicalClassifier {
    fn decide(
        &self,
        state: &State,
        question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError> {
        let query = Self::query_for(state, question);
        match question {
            DecisionQuestion::Choice(choice) => Self::decide_choice(&query, choice),
            DecisionQuestion::Boolean(boolean) => Self::decide_boolean(state.text(), boolean),
            DecisionQuestion::Score(score) => Self::decide_score(&query, score),
            // `#[non_exhaustive]`: an unknown question kind is declined
            // rather than guessed at.
            _ => Err(EngineError::InvalidDistribution {
                question: question.id().to_string(),
                reason: "unsupported question kind".to_owned(),
            }),
        }
    }

    fn decide_extended(
        &self,
        state: &State,
        question: &DecisionQuestion,
        index: Option<&Bm25Index>,
    ) -> Result<(Distribution, Option<f64>), EngineError> {
        let query = Self::query_for(state, question);
        let (distribution, coverage) = match question {
            DecisionQuestion::Choice(choice) => {
                Self::decide_choice_with_support(&query, choice, index)?
            }
            DecisionQuestion::Boolean(boolean) => {
                Self::decide_boolean_with_support(state.text(), boolean)?
            }
            DecisionQuestion::Score(score) => Self::decide_score_with_support(&query, score)?,
            // `#[non_exhaustive]`: an unknown question kind is declined
            // rather than guessed at.
            _ => Err(EngineError::InvalidDistribution {
                question: question.id().to_string(),
                reason: "unsupported question kind".to_owned(),
            })?,
        };
        Ok((distribution, Some(coverage)))
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }
}

/// The candidate id a boolean distribution's winning key maps to.
///
/// Booleans are not candidates, so this helper exists for callers that
/// want to compare a boolean answer with a choice answer (verification
/// agreement, for instance).
#[must_use]
pub fn boolean_key(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// Maps a distribution key back to a candidate id.
pub fn candidate_id(key: &str) -> EngineResult<CandidateId> {
    CandidateId::new(key).map_err(EngineError::from)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use opencodifier_core::{
        BooleanQuestion, Candidate, ChoiceQuestion, QuestionId, ScoreLevel, ScoreQuestion,
    };

    fn choice_question() -> DecisionQuestion {
        DecisionQuestion::Choice(
            ChoiceQuestion::new(
                "model",
                "Which model should answer?",
                vec![
                    Candidate::new("local-small", "small local coding model").unwrap(),
                    Candidate::new("cloud-large", "long context cloud research model").unwrap(),
                ],
            )
            .unwrap(),
        )
    }

    #[test]
    fn mock_returns_scripted_distributions() {
        let mock = MockClassifier::new("mock/test")
            .with_script("model", vec![("cloud-large", 0.9), ("local-small", 0.1)])
            .unwrap();
        let distribution = mock.decide(&State::from_text("anything"), &choice_question()).unwrap();
        assert_eq!(distribution.top().key, "cloud-large");
        assert!((distribution.probability_of("cloud-large").unwrap() - 0.9).abs() < 1e-12);
        assert_eq!(mock.model_id(), "mock/test");
        assert_eq!(mock.scripted(), 1);
    }

    #[test]
    fn mock_falls_back_to_uniform() {
        let mock = MockClassifier::new("mock/test");
        let distribution = mock.decide(&State::from_text("anything"), &choice_question()).unwrap();
        assert!((distribution.top().probability - 0.5).abs() < 1e-12);
        assert_eq!(distribution.entries().len(), 2);
    }

    #[test]
    fn mock_answers_boolean_and_score_questions_uniformly() {
        let mock = MockClassifier::new("mock/test");
        let boolean = DecisionQuestion::Boolean(
            BooleanQuestion::new("tools", "Does this request need tools?").unwrap(),
        );
        let distribution = mock.decide(&State::from_text("x"), &boolean).unwrap();
        assert_eq!(distribution.entries().len(), 2);
        assert!((distribution.probability_of("true").unwrap() - 0.5).abs() < 1e-12);

        let score = DecisionQuestion::Score(
            ScoreQuestion::new(
                "difficulty",
                "How hard is this?",
                vec![
                    ScoreLevel::new("easy").unwrap(),
                    ScoreLevel::new("hard").unwrap(),
                    ScoreLevel::new("expert").unwrap(),
                ],
            )
            .unwrap(),
        );
        let distribution = mock.decide(&State::from_text("x"), &score).unwrap();
        assert!((distribution.top().probability - 1.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn mock_is_deterministic_and_rejects_bad_scripts() {
        let mock = MockClassifier::new("mock/test")
            .with_script("model", vec![("local-small", 0.7), ("cloud-large", 0.3)])
            .unwrap();
        let first = mock.decide(&State::from_text("x"), &choice_question()).unwrap();
        let second = mock.decide(&State::from_text("x"), &choice_question()).unwrap();
        assert_eq!(first, second);
        assert!(MockClassifier::new("m").with_script("model", vec![("a", 0.5)]).is_err());
    }

    #[test]
    fn lexical_prefers_lexically_supported_candidates() {
        let classifier = LexicalClassifier::new();
        let state = State::from_text("summarize this research paper across many sources");
        let distribution = classifier.decide(&state, &choice_question()).unwrap();
        let sum: f64 = distribution.entries().iter().map(|entry| entry.probability).sum();
        assert!((sum - 1.0).abs() < 1e-9);
        assert!(
            distribution.probability_of("cloud-large").unwrap()
                > distribution.probability_of("local-small").unwrap()
        );
    }

    #[test]
    fn lexical_is_deterministic_and_reports_its_model_id() {
        let classifier = LexicalClassifier::new();
        let state = State::from_text("refactor the parser");
        let first = classifier.decide(&state, &choice_question()).unwrap();
        let second = classifier.decide(&state, &choice_question()).unwrap();
        assert_eq!(first, second);
        assert_eq!(classifier.model_id(), "builtin-lexical-v2");
        assert_eq!(
            LexicalClassifier::default().with_model_id("lexical-v2").model_id(),
            "lexical-v2"
        );
    }

    #[test]
    fn lexical_boolean_reads_support_and_negation() {
        let classifier = LexicalClassifier::new();
        let question = DecisionQuestion::Boolean(
            BooleanQuestion::new("research", "research sources summarization?").unwrap(),
        );
        let supporting = State::from_text("summarize research from many sources");
        let affirmative = classifier.decide(&supporting, &question).unwrap();
        assert!(affirmative.probability_of("true").unwrap() > 0.5, "{affirmative:?}");

        let unrelated = State::from_text("unrelated gibberish zzz");
        let neutral = classifier.decide(&unrelated, &question).unwrap();
        assert!((neutral.probability_of("true").unwrap() - 0.5).abs() < 1e-9);

        let negated = DecisionQuestion::Boolean(
            BooleanQuestion::new("research", "not never without research?").unwrap(),
        );
        let flipped = classifier.decide(&supporting, &negated).unwrap();
        assert!(flipped.probability_of("true").unwrap() < 0.5, "{flipped:?}");
    }

    #[test]
    fn lexical_boolean_never_saturates_on_long_states() {
        // The #115 misfire class: a long state repeating the question's
        // terms drove the affirmative BM25 score past ~37, where
        // softmax([evidence, 0]) returns exactly 1.0 in `f64` — a
        // confidence no lexical overlap can certify (D35).
        let classifier = LexicalClassifier::new();
        let question = DecisionQuestion::Boolean(
            BooleanQuestion::new(
                "policy",
                "dispute valid raised representment compelling evidence cardholder credit?",
            )
            .unwrap(),
        );
        let repeated = State::from_text(
            "dispute valid raised representment compelling evidence cardholder credit ".repeat(40),
        );
        let distribution = classifier.decide(&repeated, &question).unwrap();
        let p_true = distribution.probability_of("true").unwrap();
        assert!(p_true < 1.0, "saturated: {distribution:?}");
        assert!(p_true <= 0.99, "above the lexical cap: {distribution:?}");
        // The bound preserves order: the prediction is unchanged.
        assert!(p_true > distribution.probability_of("false").unwrap());
    }

    #[test]
    fn lexical_boolean_negation_flip_holds_under_the_confidence_bound() {
        // The recorded failure: rubric text with an odd negator count
        // flipped the polarity of a saturated distribution — "no" at
        // exactly 1.0 (D35). The flip survives the bound; the false
        // certainty does not.
        let classifier = LexicalClassifier::new();
        let question = DecisionQuestion::Boolean(
            BooleanQuestion::new("policy", "the dispute does not contain compelling evidence?")
                .unwrap(),
        );
        let repeated = State::from_text(
            "dispute compelling evidence credit retained policy rules ".repeat(40),
        );
        let distribution = classifier.decide(&repeated, &question).unwrap();
        assert!(distribution.probability_of("false").unwrap() > 0.5, "{distribution:?}");
        assert!(distribution.top().probability < 1.0, "saturated: {distribution:?}");
    }

    #[test]
    fn lexical_choice_never_saturates_on_long_states() {
        // Choice saturation (#115): score gaps between candidate
        // descriptions grow with state length; unbounded, the winner hit
        // exactly 1.0 and exact-proof gates accepted it (D35).
        let classifier = LexicalClassifier::new();
        let question = DecisionQuestion::Choice(
            ChoiceQuestion::new(
                "disposition",
                "Which disposition applies?",
                vec![
                    Candidate::new("erase_all", "erase all personal data no retention").unwrap(),
                    Candidate::new("retain_records", "retain transaction records restricted")
                        .unwrap(),
                ],
            )
            .unwrap(),
        );
        let repeated = State::from_text(
            "erase personal data retention records restricted transaction ".repeat(60),
        );
        let distribution = classifier.decide(&repeated, &question).unwrap();
        assert!(distribution.top().probability < 1.0, "saturated: {distribution:?}");
        assert!(distribution.top().probability <= 0.99, "above the cap: {distribution:?}");
    }

    #[test]
    fn lexical_score_leans_towards_mentioned_levels() {
        let classifier = LexicalClassifier::new();
        let score = DecisionQuestion::Score(
            ScoreQuestion::new(
                "difficulty",
                "How difficult is this task?",
                vec![
                    ScoreLevel::new("trivial").unwrap(),
                    ScoreLevel::new("complex").unwrap(),
                    ScoreLevel::new("expert").unwrap(),
                ],
            )
            .unwrap(),
        );
        let state = State::from_text("an expert migration of the schema with complex constraints");
        let distribution = classifier.decide(&state, &score).unwrap();
        let trivial = distribution.probability_of("trivial").unwrap();
        let expert = distribution.probability_of("expert").unwrap();
        assert!(expert > trivial, "{distribution:?}");
    }

    #[test]
    fn helper_functions_map_keys() {
        assert_eq!(boolean_key(true), "true");
        assert_eq!(boolean_key(false), "false");
        assert_eq!(candidate_id("local-small").unwrap(), CandidateId::new("local-small").unwrap());
        assert!(candidate_id("bad id").is_err());
        assert_eq!(
            LexicalClassifier::query_for(&State::from_text("state"), &choice_question()),
            "Which model should answer? state"
        );
        assert_eq!(
            answer_keys(&DecisionQuestion::Boolean(BooleanQuestion::new("b", "t").unwrap())),
            vec!["true".to_owned(), "false".to_owned()]
        );
        assert_eq!(
            answer_keys(&choice_question()),
            vec!["local-small".to_owned(), "cloud-large".to_owned()]
        );
        assert_eq!(
            QuestionId::new("model").unwrap().to_string(),
            "model",
            "question ids stay stable"
        );
    }

    #[test]
    fn lexical_decide_extended_reports_coverage() {
        let classifier = LexicalClassifier::new();
        let state = State::from_text("summarize this research paper across many sources");
        let (distribution, support) =
            classifier.decide_extended(&state, &choice_question(), None).unwrap();
        let coverage = support.expect("lexical rung always exposes coverage");
        assert!((0.0..=1.0).contains(&coverage));
        // The distribution side must equal plain `decide` bit for bit.
        assert_eq!(distribution, classifier.decide(&state, &choice_question()).unwrap());
    }

    #[test]
    fn lexical_decide_extended_uses_the_prebuilt_index() {
        // The same documents the question carries, built by a caller
        // (#80's build-once handoff): scores and coverage must be
        // identical to the locally-built index.
        let classifier = LexicalClassifier::new();
        let state = State::from_text("summarize research");
        let question = choice_question();
        let DecisionQuestion::Choice(choice) = &question else {
            panic!("choice question");
        };
        let documents: Vec<&str> = choice.candidates().iter().map(Candidate::description).collect();
        let prebuilt = Bm25Index::new(documents);
        let (with_index, index_support) =
            classifier.decide_extended(&state, &question, Some(&prebuilt)).unwrap();
        let (without, local_support) = classifier.decide_extended(&state, &question, None).unwrap();
        assert_eq!(with_index, without);
        assert_eq!(index_support, local_support);
        // A mismatched prebuilt index (wrong document count) is ignored in
        // favor of the correct local build.
        let wrong = Bm25Index::new(["unrelated"]);
        let (mismatched, _) = classifier.decide_extended(&state, &question, Some(&wrong)).unwrap();
        assert_eq!(mismatched, without);
    }

    #[test]
    fn mock_decide_extended_defaults_to_no_support() {
        let mock = MockClassifier::new("mock/test");
        let (_, support) =
            mock.decide_extended(&State::from_text("x"), &choice_question(), None).unwrap();
        assert_eq!(support, None, "rungs without lexical evidence expose none");
    }

    #[test]
    fn lexical_boolean_and_score_extended_report_coverage() {
        let classifier = LexicalClassifier::new();
        let boolean = DecisionQuestion::Boolean(
            BooleanQuestion::new("research", "research sources summarization?").unwrap(),
        );
        let (_, boolean_support) = classifier
            .decide_extended(&State::from_text("summarize research sources"), &boolean, None)
            .unwrap();
        assert!(boolean_support.is_some_and(|coverage| coverage > 0.0));

        let score = DecisionQuestion::Score(
            ScoreQuestion::new(
                "difficulty",
                "How hard is this?",
                vec![ScoreLevel::new("easy").unwrap(), ScoreLevel::new("expert").unwrap()],
            )
            .unwrap(),
        );
        let (_, score_support) =
            classifier.decide_extended(&State::from_text("expert work"), &score, None).unwrap();
        assert!(score_support.is_some());
    }
}
