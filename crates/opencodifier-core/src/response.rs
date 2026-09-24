//! The canonical response envelope: answers, outcome classification,
//! confidence, trace, and metrics.

use serde::{Deserialize, Serialize};

use crate::answer::DecisionAnswer;
use crate::confidence::ConfidenceReport;
use crate::error::{CoreError, CoreResult};
use crate::trace::{DecisionMetrics, DecisionTrace};

/// What the engine ultimately did with the request (PLANNING.md §21).
///
/// Abstention is a successful outcome, never an error: refusing to decide
/// under low confidence is the system working as designed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DecisionOutcome {
    /// Accepted directly; confidence cleared `min_confidence`.
    Accept,
    /// Answer produced after a verifier ran and agreed.
    Verified,
    /// Routed to a verifier by the confidence gate or risk policy.
    Verify,
    /// Refused: confidence below `abstain_below`.
    Abstain,
    /// Refused with a recommendation to invoke a larger model.
    Escalate,
    /// No candidate survived deterministic filtering.
    NoValidCandidate,
}

impl DecisionOutcome {
    /// `true` when this outcome carries a usable answer.
    pub fn is_decisive(self) -> bool {
        matches!(self, Self::Accept | Self::Verified)
    }
}

/// The full result of one decision request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawDecisionResponse")]
pub struct DecisionResponse {
    answers: Vec<DecisionAnswer>,
    outcome: DecisionOutcome,
    confidence: ConfidenceReport,
    trace: DecisionTrace,
    metrics: DecisionMetrics,
}

/// Deserialization mirror for [`DecisionResponse`]; conversion validates.
#[derive(Debug, Deserialize)]
struct RawDecisionResponse {
    answers: Vec<DecisionAnswer>,
    outcome: DecisionOutcome,
    confidence: ConfidenceReport,
    trace: DecisionTrace,
    metrics: DecisionMetrics,
}

impl TryFrom<RawDecisionResponse> for DecisionResponse {
    type Error = CoreError;

    fn try_from(raw: RawDecisionResponse) -> CoreResult<Self> {
        Self::new(raw.answers, raw.outcome, raw.confidence, raw.trace, raw.metrics)
    }
}

impl DecisionResponse {
    /// Constructs and validates a response.
    ///
    /// Invariant: decisive outcomes (`Accept`, `Verified`) must carry at
    /// least one answer; abstentions may legitimately carry none.
    pub fn new(
        answers: Vec<DecisionAnswer>,
        outcome: DecisionOutcome,
        confidence: ConfidenceReport,
        trace: DecisionTrace,
        metrics: DecisionMetrics,
    ) -> CoreResult<Self> {
        if outcome.is_decisive() && answers.is_empty() {
            return Err(CoreError::EmptyField {
                field: "answers (decisive outcome requires at least one)",
            });
        }
        for answer in &answers {
            answer.validate()?;
        }
        Ok(Self { answers, outcome, confidence, trace, metrics })
    }

    /// The per-question answers, in request order.
    pub fn answers(&self) -> &[DecisionAnswer] {
        &self.answers
    }

    /// The outcome classification.
    pub fn outcome(&self) -> DecisionOutcome {
        self.outcome
    }

    /// The confidence report.
    pub fn confidence(&self) -> &ConfidenceReport {
        &self.confidence
    }

    /// The execution trace.
    pub fn trace(&self) -> &DecisionTrace {
        &self.trace
    }

    /// The efficiency metrics.
    pub fn metrics(&self) -> &DecisionMetrics {
        &self.metrics
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use crate::answer::Distribution;
    use crate::ids::QuestionId;

    fn report() -> ConfidenceReport {
        ConfidenceReport {
            top_probability: 0.91,
            margin: 0.82,
            entropy: 0.2,
            calibrated_confidence: 0.91,
            ood_score: 0.02,
            verifier_agreement: None,
        }
    }

    fn answer() -> DecisionAnswer {
        DecisionAnswer::Choice {
            question_id: QuestionId::new("model").expect("valid"),
            choice: crate::ids::CandidateId::new("qwen").expect("valid"),
            distribution: Distribution::from_pairs([("qwen", 0.91), ("glm", 0.09)])
                .expect("normalized"),
            confidence: 0.91,
        }
    }

    #[test]
    fn decisive_outcome_requires_answers() {
        assert!(
            DecisionResponse::new(
                vec![answer()],
                DecisionOutcome::Accept,
                report(),
                DecisionTrace::new(),
                DecisionMetrics::default(),
            )
            .is_ok()
        );

        assert!(matches!(
            DecisionResponse::new(
                vec![],
                DecisionOutcome::Accept,
                report(),
                DecisionTrace::new(),
                DecisionMetrics::default(),
            ),
            Err(CoreError::EmptyField { .. })
        ));
    }

    #[test]
    fn abstention_may_carry_no_answers() {
        let response = DecisionResponse::new(
            vec![],
            DecisionOutcome::Abstain,
            report(),
            DecisionTrace::new(),
            DecisionMetrics::default(),
        )
        .expect("valid abstention");
        assert!(!response.outcome().is_decisive());
        assert!(response.answers().is_empty());
    }

    #[test]
    fn round_trips_through_json() {
        let response = DecisionResponse::new(
            vec![answer()],
            DecisionOutcome::Verified,
            report(),
            DecisionTrace::new(),
            DecisionMetrics::default(),
        )
        .expect("valid");
        let json = serde_json::to_string(&response).expect("serialize");
        let back: DecisionResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, response);
    }
}
