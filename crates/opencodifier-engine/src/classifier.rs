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
use crate::lexical::{Bm25Index, negation_polarity, softmax};

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
    #[must_use]
    pub fn new() -> Self {
        Self { model_id: "builtin-lexical-v1".to_owned() }
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
    /// distribution over candidate ids, in candidate order.
    fn decide_choice(
        query: &str,
        question: &opencodifier_core::ChoiceQuestion,
    ) -> EngineResult<Distribution> {
        let documents: Vec<&str> =
            question.candidates().iter().map(opencodifier_core::Candidate::description).collect();
        let scores = Bm25Index::new(documents).score_all(query);
        let probabilities = softmax(&scores);
        let pairs: Vec<(String, f64)> = question
            .candidates()
            .iter()
            .zip(probabilities)
            .map(|(candidate, probability)| (candidate.id().as_str().to_owned(), probability))
            .collect();
        Distribution::from_pairs(pairs).map_err(|error| EngineError::InvalidDistribution {
            question: question.id().to_string(),
            reason: error.to_string(),
        })
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
        let support = Bm25Index::new([question.text()]).score_all(state_text);
        let evidence = support.first().copied().unwrap_or(0.0);
        let mut scores = vec![evidence, 0.0];
        if negation_polarity(question.text()) < 0.0 {
            scores.reverse();
        }
        let probabilities = softmax(&scores);
        Distribution::from_pairs([("true", probabilities[0]), ("false", probabilities[1])]).map_err(
            |error| EngineError::InvalidDistribution {
                question: question.id().to_string(),
                reason: error.to_string(),
            },
        )
    }

    /// Score: BM25 over level labels, softmaxed into a distribution over
    /// labels. A state that mentions "expert" work leans towards the
    /// `expert` level and nothing else.
    fn decide_score(query: &str, question: &ScoreQuestion) -> EngineResult<Distribution> {
        let labels: Vec<&str> =
            question.levels().iter().map(opencodifier_core::ScoreLevel::label).collect();
        let scores = Bm25Index::new(labels).score_all(query);
        let probabilities = softmax(&scores);
        let pairs: Vec<(String, f64)> = question
            .levels()
            .iter()
            .zip(probabilities)
            .map(|(level, probability)| (level.label().to_owned(), probability))
            .collect();
        Distribution::from_pairs(pairs).map_err(|error| EngineError::InvalidDistribution {
            question: question.id().to_string(),
            reason: error.to_string(),
        })
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
        assert_eq!(classifier.model_id(), "builtin-lexical-v1");
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
}
