//! Decision answers: the typed, machine-actionable results (PLANNING.md §5).

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult, SUM_TOLERANCE, is_unit_interval};
use crate::ids::{CandidateId, QuestionId};

/// One probability entry in a distribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawDistributionEntry")]
pub struct DistributionEntry {
    /// Candidate id (choice) or level label (score).
    pub key: String,
    /// Calibrated probability in `[0, 1]`.
    pub probability: f64,
}

/// Deserialization mirror for [`DistributionEntry`]; conversion validates.
#[derive(Debug, Deserialize)]
struct RawDistributionEntry {
    key: String,
    probability: f64,
}

impl TryFrom<RawDistributionEntry> for DistributionEntry {
    type Error = CoreError;

    fn try_from(raw: RawDistributionEntry) -> CoreResult<Self> {
        if !is_unit_interval(raw.probability) {
            return Err(CoreError::InvalidProbability { value: raw.probability });
        }
        Ok(Self { key: raw.key, probability: raw.probability })
    }
}

/// A complete categorical distribution over candidates or score levels.
///
/// Order is preserved exactly as supplied: score levels are ordered, and
/// stable answer ordering keeps trace and cache output deterministic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawDistribution")]
pub struct Distribution {
    entries: Vec<DistributionEntry>,
}

/// Deserialization mirror for [`Distribution`]; conversion validates.
#[derive(Debug, Deserialize)]
struct RawDistribution {
    entries: Vec<DistributionEntry>,
}

impl TryFrom<RawDistribution> for Distribution {
    type Error = CoreError;

    fn try_from(raw: RawDistribution) -> CoreResult<Self> {
        Self::new(raw.entries)
    }
}

impl Distribution {
    /// Validates and constructs a distribution.
    ///
    /// Invariants: at least one entry, unique keys, every probability
    /// finite in `[0, 1]`, total mass equal to 1 within a `1e-6` sum
    /// tolerance.
    pub fn new(entries: Vec<DistributionEntry>) -> CoreResult<Self> {
        if entries.is_empty() {
            return Err(CoreError::EmptyField { field: "distribution entries" });
        }
        let mut seen = std::collections::HashSet::new();
        let mut sum = 0.0;
        for entry in &entries {
            if !seen.insert(entry.key.clone()) {
                return Err(CoreError::DuplicateCandidate { id: entry.key.clone() });
            }
            if !is_unit_interval(entry.probability) {
                return Err(CoreError::InvalidProbability { value: entry.probability });
            }
            sum += entry.probability;
        }
        if (sum - 1.0).abs() > SUM_TOLERANCE {
            return Err(CoreError::DistributionNotNormalized { sum, tolerance: SUM_TOLERANCE });
        }
        Ok(Self { entries })
    }

    /// Builds a distribution from `(key, probability)` pairs.
    pub fn from_pairs<K: Into<String>>(
        pairs: impl IntoIterator<Item = (K, f64)>,
    ) -> CoreResult<Self> {
        Self::new(
            pairs
                .into_iter()
                .map(|(key, probability)| DistributionEntry { key: key.into(), probability })
                .collect(),
        )
    }

    /// The entries in supplied order.
    pub fn entries(&self) -> &[DistributionEntry] {
        &self.entries
    }

    /// The probability of `key`, if present.
    pub fn probability_of(&self, key: &str) -> Option<f64> {
        self.entries.iter().find(|entry| entry.key == key).map(|entry| entry.probability)
    }

    /// The highest-probability entry.
    pub fn top(&self) -> &DistributionEntry {
        // Invariant: non-empty (checked in `new`), so unwrap is impossible.
        self.entries
            .iter()
            .max_by(|a, b| a.probability.total_cmp(&b.probability))
            .unwrap_or_else(|| &self.entries[0])
    }

    /// The difference between the top two probabilities.
    ///
    /// A small margin means a near-tie — the primary signal for triggering
    /// verification (PLANNING.md §19).
    pub fn margin(&self) -> f64 {
        let mut top = f64::NEG_INFINITY;
        let mut second = f64::NEG_INFINITY;
        for entry in &self.entries {
            if entry.probability > top {
                second = top;
                top = entry.probability;
            } else if entry.probability > second {
                second = entry.probability;
            }
        }
        if second.is_finite() { top - second } else { 1.0 }
    }

    /// Shannon entropy of the distribution in bits: `-Σ p·log2(p)`.
    ///
    /// 0 bits = fully certain; `log2(n)` bits = uniformly uncertain.
    pub fn entropy(&self) -> f64 {
        let mut entropy = 0.0;
        for entry in &self.entries {
            if entry.probability > 0.0 {
                entropy -= entry.probability * entry.probability.log2();
            }
        }
        entropy
    }
}

/// The answer to one question (PLANNING.md §5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", try_from = "RawDecisionAnswer")]
#[non_exhaustive]
pub enum DecisionAnswer {
    /// Selected candidate with the full probability distribution.
    Choice {
        /// Id of the question this answers.
        question_id: QuestionId,
        /// The selected candidate.
        choice: CandidateId,
        /// Full distribution over candidates.
        distribution: Distribution,
        /// Calibrated confidence for this answer.
        confidence: f64,
    },
    /// Boolean value with probability and confidence.
    Boolean {
        /// Id of the question this answers.
        question_id: QuestionId,
        /// The decided value.
        value: bool,
        /// Probability that `value` is correct.
        probability: f64,
        /// Calibrated confidence for this answer.
        confidence: f64,
    },
    /// Expected score with full level distribution.
    Score {
        /// Id of the question this answers.
        question_id: QuestionId,
        /// Expected value over positional weights.
        expected: f64,
        /// The level the expected value falls into.
        level: String,
        /// Full distribution over levels.
        distribution: Distribution,
        /// Calibrated confidence for this answer.
        confidence: f64,
    },
}

/// Deserialization mirror for [`DecisionAnswer`]; conversion validates.
///
/// The variant shapes match [`DecisionAnswer`] exactly so the wire format
/// is unchanged; every field type already validates itself, and
/// [`DecisionAnswer::validate`] enforces the answer-level invariants.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RawDecisionAnswer {
    /// Mirrors [`DecisionAnswer::Choice`].
    Choice {
        /// Id of the question this answers.
        question_id: QuestionId,
        /// The selected candidate.
        choice: CandidateId,
        /// Full distribution over candidates.
        distribution: Distribution,
        /// Calibrated confidence for this answer.
        confidence: f64,
    },
    /// Mirrors [`DecisionAnswer::Boolean`].
    Boolean {
        /// Id of the question this answers.
        question_id: QuestionId,
        /// The decided value.
        value: bool,
        /// Probability that `value` is correct.
        probability: f64,
        /// Calibrated confidence for this answer.
        confidence: f64,
    },
    /// Mirrors [`DecisionAnswer::Score`].
    Score {
        /// Id of the question this answers.
        question_id: QuestionId,
        /// Expected value over positional weights.
        expected: f64,
        /// The level the expected value falls into.
        level: String,
        /// Full distribution over levels.
        distribution: Distribution,
        /// Calibrated confidence for this answer.
        confidence: f64,
    },
}

impl TryFrom<RawDecisionAnswer> for DecisionAnswer {
    type Error = CoreError;

    fn try_from(raw: RawDecisionAnswer) -> CoreResult<Self> {
        let answer = match raw {
            RawDecisionAnswer::Choice { question_id, choice, distribution, confidence } => {
                Self::Choice { question_id, choice, distribution, confidence }
            }
            RawDecisionAnswer::Boolean { question_id, value, probability, confidence } => {
                Self::Boolean { question_id, value, probability, confidence }
            }
            RawDecisionAnswer::Score { question_id, expected, level, distribution, confidence } => {
                Self::Score { question_id, expected, level, distribution, confidence }
            }
        };
        answer.validate()?;
        Ok(answer)
    }
}

impl DecisionAnswer {
    /// The id of the question this answer responds to.
    pub fn question_id(&self) -> &QuestionId {
        match self {
            Self::Choice { question_id, .. }
            | Self::Boolean { question_id, .. }
            | Self::Score { question_id, .. } => question_id,
        }
    }

    /// The calibrated confidence attached to this answer.
    pub fn confidence(&self) -> f64 {
        match self {
            Self::Choice { confidence, .. }
            | Self::Boolean { confidence, .. }
            | Self::Score { confidence, .. } => *confidence,
        }
    }

    /// Validates answer-level invariants (confidence in `[0, 1]`).
    pub fn validate(&self) -> CoreResult<()> {
        let confidence = self.confidence();
        if !is_unit_interval(confidence) {
            return Err(CoreError::InvalidProbability { value: confidence });
        }
        match self {
            Self::Boolean { probability, .. } if !is_unit_interval(*probability) => {
                Err(CoreError::InvalidProbability { value: *probability })
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    fn sample() -> Distribution {
        Distribution::from_pairs([("a", 0.78), ("b", 0.17), ("c", 0.05)]).expect("normalized")
    }

    #[test]
    fn distribution_computes_top_margin_entropy() {
        let dist = sample();
        assert_eq!(dist.top().key, "a");
        assert!((dist.margin() - 0.61).abs() < 1e-9);
        assert!(dist.entropy() > 0.9 && dist.entropy() < 1.1);
        assert_eq!(dist.probability_of("b"), Some(0.17));
    }

    #[test]
    fn uniform_distribution_has_max_entropy() {
        let dist = Distribution::from_pairs([("a", 0.25), ("b", 0.25), ("c", 0.25), ("d", 0.25)])
            .expect("normalized");
        assert!((dist.entropy() - 2.0).abs() < 1e-9, "{}", dist.entropy());
        assert!((dist.margin() - 0.0).abs() < 1e-9);
    }

    #[test]
    fn certain_distribution_has_zero_entropy() {
        let dist = Distribution::from_pairs([("a", 1.0), ("b", 0.0)]).expect("normalized");
        assert!(dist.entropy().abs() < 1e-12);
    }

    #[test]
    fn rejects_unnormalized_or_invalid_distributions() {
        assert!(Distribution::from_pairs([("a", 0.5), ("b", 0.4)]).is_err());
        assert!(Distribution::from_pairs([("a", 1.2), ("b", -0.2)]).is_err());
        assert!(Distribution::from_pairs([("a", f64::NAN), ("b", 0.5)]).is_err());
        let dupe = Distribution::from_pairs([("a", 0.5), ("a", 0.5)]);
        assert!(dupe.is_err());
        let empty: Vec<(String, f64)> = Vec::new();
        assert!(Distribution::from_pairs(empty).is_err());
    }

    #[test]
    fn answer_validates_confidence() {
        let answer = DecisionAnswer::Choice {
            question_id: QuestionId::new("model").expect("valid"),
            choice: CandidateId::new("qwen").expect("valid"),
            distribution: sample(),
            confidence: 0.78,
        };
        assert!(answer.validate().is_ok());
        assert!((answer.confidence() - 0.78).abs() < 1e-9);

        let bad = DecisionAnswer::Choice {
            confidence: 1.5,
            question_id: QuestionId::new("model").expect("valid"),
            choice: CandidateId::new("qwen").expect("valid"),
            distribution: sample(),
        };
        assert!(matches!(bad.validate(), Err(CoreError::InvalidProbability { value: 1.5 })));
    }

    #[test]
    fn answer_round_trips_through_json() {
        let answer = DecisionAnswer::Score {
            question_id: QuestionId::new("difficulty").expect("valid"),
            expected: 3.8,
            level: "difficult".into(),
            distribution: Distribution::from_pairs([
                ("trivial", 0.01),
                ("easy", 0.04),
                ("moderate", 0.17),
                ("difficult", 0.61),
                ("expert", 0.17),
            ])
            .expect("normalized"),
            confidence: 0.71,
        };
        let json = serde_json::to_string(&answer).expect("serialize");
        assert!(json.contains(r#""type":"score""#));
        let back: DecisionAnswer = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, answer);
    }
}
