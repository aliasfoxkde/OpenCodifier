//! The three canonical decision question types (PLANNING.md §5).

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};
use crate::ids::{CandidateId, QuestionId};

/// One possible answer to a choice question.
///
/// The candidate set is runtime-defined: callers supply it per request,
/// which is what makes candidate-conditioned scoring (rather than a
/// fixed-label classifier) the core ML shape of the system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawCandidate")]
pub struct Candidate {
    id: CandidateId,
    description: String,
}

/// Deserialization mirror for [`Candidate`]; conversion validates.
#[derive(Debug, Deserialize)]
struct RawCandidate {
    id: String,
    description: String,
}

impl TryFrom<RawCandidate> for Candidate {
    type Error = CoreError;

    fn try_from(raw: RawCandidate) -> CoreResult<Self> {
        Self::new(raw.id, raw.description)
    }
}

impl Candidate {
    /// Constructs a candidate, validating its id.
    ///
    /// The description may be empty but must be present; callers should
    /// still write meaningful descriptions — they are the primary signal
    /// the semantic layers score against.
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> CoreResult<Self> {
        Ok(Self { id: CandidateId::new(id)?, description: description.into() })
    }

    /// The candidate's identifier.
    pub fn id(&self) -> &CandidateId {
        &self.id
    }

    /// The candidate's human-written description.
    pub fn description(&self) -> &str {
        &self.description
    }
}

impl From<CandidateId> for Candidate {
    fn from(id: CandidateId) -> Self {
        Self { id, description: String::new() }
    }
}

/// Select one candidate from a runtime-defined set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawChoiceQuestion")]
pub struct ChoiceQuestion {
    id: QuestionId,
    text: String,
    candidates: Vec<Candidate>,
}

/// Deserialization mirror for [`ChoiceQuestion`]; conversion validates.
#[derive(Debug, Deserialize)]
struct RawChoiceQuestion {
    id: String,
    text: String,
    candidates: Vec<Candidate>,
}

impl TryFrom<RawChoiceQuestion> for ChoiceQuestion {
    type Error = CoreError;

    fn try_from(raw: RawChoiceQuestion) -> CoreResult<Self> {
        Self::new(raw.id, raw.text, raw.candidates)
    }
}

impl ChoiceQuestion {
    /// Constructs a choice question, enforcing non-empty unique candidates.
    pub fn new(
        id: impl Into<String>,
        text: impl Into<String>,
        candidates: Vec<Candidate>,
    ) -> CoreResult<Self> {
        let id = QuestionId::new(id)?;
        let text = text.into();
        if text.is_empty() {
            return Err(CoreError::EmptyField { field: "text" });
        }
        if candidates.is_empty() {
            return Err(CoreError::EmptyCandidates { question: id.to_string() });
        }
        let mut seen = std::collections::HashSet::new();
        for candidate in &candidates {
            if !seen.insert(candidate.id().clone()) {
                return Err(CoreError::DuplicateCandidate { id: candidate.id().to_string() });
            }
        }
        Ok(Self { id, text, candidates })
    }

    /// The question id.
    pub fn id(&self) -> &QuestionId {
        &self.id
    }

    /// The question text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The candidate set, in request order.
    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }
}

/// A yes/no question (Jev's `boolean`/`noul` primitive).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawBooleanQuestion")]
pub struct BooleanQuestion {
    id: QuestionId,
    text: String,
}

/// Deserialization mirror for [`BooleanQuestion`]; conversion validates.
#[derive(Debug, Deserialize)]
struct RawBooleanQuestion {
    id: String,
    text: String,
}

impl TryFrom<RawBooleanQuestion> for BooleanQuestion {
    type Error = CoreError;

    fn try_from(raw: RawBooleanQuestion) -> CoreResult<Self> {
        Self::new(raw.id, raw.text)
    }
}

impl BooleanQuestion {
    /// Constructs a boolean question with non-empty text.
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> CoreResult<Self> {
        let text = text.into();
        if text.is_empty() {
            return Err(CoreError::EmptyField { field: "text" });
        }
        Ok(Self { id: QuestionId::new(id)?, text })
    }

    /// The question id.
    pub fn id(&self) -> &QuestionId {
        &self.id
    }

    /// The question text.
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// One ordered level of a score question.
///
/// Levels are ordered by position; the expected value of a score is
/// computed against position weights (0, 1, 2, ...). Explicit weights can
/// be added later without breaking the wire format, so V1 keeps the
/// simpler invariant: position is meaning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawScoreLevel")]
pub struct ScoreLevel {
    label: String,
}

/// Deserialization mirror for [`ScoreLevel`]; conversion validates.
#[derive(Debug, Deserialize)]
struct RawScoreLevel {
    label: String,
}

impl TryFrom<RawScoreLevel> for ScoreLevel {
    type Error = CoreError;

    fn try_from(raw: RawScoreLevel) -> CoreResult<Self> {
        Self::new(raw.label)
    }
}

impl ScoreLevel {
    /// Constructs a level with a non-empty label.
    pub fn new(label: impl Into<String>) -> CoreResult<Self> {
        let label = label.into();
        if label.is_empty() {
            return Err(CoreError::EmptyField { field: "label" });
        }
        Ok(Self { label })
    }

    /// The level label, e.g. `"difficult"`.
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// An ordered-severity question returning a distribution over levels plus
/// an expected value (e.g. task difficulty).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawScoreQuestion")]
pub struct ScoreQuestion {
    id: QuestionId,
    text: String,
    levels: Vec<ScoreLevel>,
}

/// Deserialization mirror for [`ScoreQuestion`]; conversion validates.
#[derive(Debug, Deserialize)]
struct RawScoreQuestion {
    id: String,
    text: String,
    levels: Vec<ScoreLevel>,
}

impl TryFrom<RawScoreQuestion> for ScoreQuestion {
    type Error = CoreError;

    fn try_from(raw: RawScoreQuestion) -> CoreResult<Self> {
        Self::new(raw.id, raw.text, raw.levels)
    }
}

impl ScoreQuestion {
    /// Constructs a score question with at least two uniquely-labeled,
    /// ordered levels.
    pub fn new(
        id: impl Into<String>,
        text: impl Into<String>,
        levels: Vec<ScoreLevel>,
    ) -> CoreResult<Self> {
        let id = QuestionId::new(id)?;
        let text = text.into();
        if text.is_empty() {
            return Err(CoreError::EmptyField { field: "text" });
        }
        if levels.len() < 2 {
            return Err(CoreError::TooFewLevels { question: id.to_string(), count: levels.len() });
        }
        let mut seen = std::collections::HashSet::new();
        for level in &levels {
            if !seen.insert(level.label.clone()) {
                return Err(CoreError::DuplicateLevel { label: level.label.clone() });
            }
        }
        Ok(Self { id, text, levels })
    }

    /// The question id.
    pub fn id(&self) -> &QuestionId {
        &self.id
    }

    /// The question text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The ordered level list.
    pub fn levels(&self) -> &[ScoreLevel] {
        &self.levels
    }

    /// Positional weights used for the expected value: `0, 1, 2, ...`.
    ///
    /// Level counts are tiny (single digits in practice), so the
    /// usize→f64 conversion is exact.
    #[allow(clippy::cast_precision_loss)]
    pub fn weights(&self) -> Vec<f64> {
        (0..self.levels.len()).map(|index| index as f64).collect()
    }

    /// The level an expected value falls into (floor, clamped to range).
    ///
    /// An expected value of `3.8` on `trivial..expert` maps to
    /// `"difficult"` (index 3), matching the worked example in
    /// PLANNING.md §5.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn level_for(&self, expected: f64) -> &ScoreLevel {
        let index = if expected.is_finite() && expected >= 0.0 {
            let floor = expected.floor() as usize;
            floor.min(self.levels.len() - 1)
        } else {
            0
        };
        &self.levels[index]
    }
}

/// Any decidable question (PLANNING.md §6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum DecisionQuestion {
    /// Select one candidate.
    Choice(ChoiceQuestion),
    /// Yes/no with probability.
    Boolean(BooleanQuestion),
    /// Ordered levels with expected value.
    Score(ScoreQuestion),
}

impl DecisionQuestion {
    /// The question id, whichever shape this is.
    pub fn id(&self) -> &QuestionId {
        match self {
            Self::Choice(question) => question.id(),
            Self::Boolean(question) => question.id(),
            Self::Score(question) => question.id(),
        }
    }

    /// The question text, whichever shape this is.
    pub fn text(&self) -> &str {
        match self {
            Self::Choice(question) => question.text(),
            Self::Boolean(question) => question.text(),
            Self::Score(question) => question.text(),
        }
    }
}

impl From<ChoiceQuestion> for DecisionQuestion {
    fn from(question: ChoiceQuestion) -> Self {
        Self::Choice(question)
    }
}

impl From<BooleanQuestion> for DecisionQuestion {
    fn from(question: BooleanQuestion) -> Self {
        Self::Boolean(question)
    }
}

impl From<ScoreQuestion> for DecisionQuestion {
    fn from(question: ScoreQuestion) -> Self {
        Self::Score(question)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    #[test]
    fn choice_round_trips_through_json() {
        let question = ChoiceQuestion::new(
            "model",
            "Which model should handle this request?",
            vec![
                Candidate::new("qwen", "General coding and reasoning").expect("valid"),
                Candidate::new("glm", "Complex reasoning").expect("valid"),
            ],
        )
        .expect("valid");
        let wrapped = DecisionQuestion::from(question);
        let json = serde_json::to_string(&wrapped).expect("serialize");
        assert!(json.contains(r#""type":"choice""#));
        let back: DecisionQuestion = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, wrapped);
    }

    #[test]
    fn choice_rejects_empty_and_duplicate_candidates() {
        assert!(matches!(
            ChoiceQuestion::new("model", "pick", vec![]),
            Err(CoreError::EmptyCandidates { .. })
        ));
        let dupes = vec![
            Candidate::new("qwen", "one").expect("valid"),
            Candidate::new("qwen", "two").expect("valid"),
        ];
        assert!(matches!(
            ChoiceQuestion::new("model", "pick", dupes),
            Err(CoreError::DuplicateCandidate { id }) if id == "qwen"
        ));
    }

    #[test]
    fn boolean_round_trips_and_rejects_empty_text() {
        let question = BooleanQuestion::new("needs_tools", "Does this need tools?").expect("valid");
        let json = serde_json::to_string(&question).expect("serialize");
        let back: BooleanQuestion = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, question);
        assert!(BooleanQuestion::new("x", "").is_err());
    }

    #[test]
    fn score_rejects_invalid_level_sets() {
        let one_level = vec![ScoreLevel::new("easy").expect("valid")];
        assert!(matches!(
            ScoreQuestion::new("difficulty", "how hard?", one_level),
            Err(CoreError::TooFewLevels { count: 1, .. })
        ));
        let dupes =
            vec![ScoreLevel::new("easy").expect("valid"), ScoreLevel::new("easy").expect("valid")];
        assert!(matches!(
            ScoreQuestion::new("difficulty", "how hard?", dupes),
            Err(CoreError::DuplicateLevel { label }) if label == "easy"
        ));
    }

    #[test]
    fn score_expected_value_maps_to_floor_level() {
        let question = ScoreQuestion::new(
            "difficulty",
            "How difficult is this request?",
            ["trivial", "easy", "moderate", "difficult", "expert"]
                .iter()
                .map(|label| ScoreLevel::new(*label).expect("valid"))
                .collect(),
        )
        .expect("valid");

        assert_eq!(question.weights(), vec![0.0, 1.0, 2.0, 3.0, 4.0]);
        assert_eq!(question.level_for(3.8).label(), "difficult");
        assert_eq!(question.level_for(0.0).label(), "trivial");
        // Clamped at both ends.
        assert_eq!(question.level_for(-1.0).label(), "trivial");
        assert_eq!(question.level_for(99.0).label(), "expert");
    }
}
