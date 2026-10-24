//! The canonical response envelope: answers, outcome classification,
//! confidence, trace, and metrics.

use serde::{Deserialize, Serialize};

use crate::answer::DecisionAnswer;
use crate::confidence::ConfidenceReport;
use crate::error::{CoreError, CoreResult};
use crate::escalation::EscalationReport;
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
    /// Refused with a recommendation to invoke a larger model
    /// (`ESCALATION_LANES_PLAN.md` §2.1): confidence landed in
    /// `escalate_below`'s band. Not decisive for the engine — the
    /// handoff document rides [`DecisionResponse::escalation`].
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
    /// The handoff document, present only when `outcome` is
    /// [`DecisionOutcome::Escalate`]; omitted from serialization when
    /// absent, so pre-band responses stay byte-identical (#189).
    #[serde(skip_serializing_if = "Option::is_none")]
    escalation: Option<EscalationReport>,
}

/// Deserialization mirror for [`DecisionResponse`]; conversion validates.
#[derive(Debug, Deserialize)]
struct RawDecisionResponse {
    answers: Vec<DecisionAnswer>,
    outcome: DecisionOutcome,
    confidence: ConfidenceReport,
    trace: DecisionTrace,
    metrics: DecisionMetrics,
    #[serde(default)]
    escalation: Option<EscalationReport>,
}

impl TryFrom<RawDecisionResponse> for DecisionResponse {
    type Error = CoreError;

    fn try_from(raw: RawDecisionResponse) -> CoreResult<Self> {
        let response = Self::new(raw.answers, raw.outcome, raw.confidence, raw.trace, raw.metrics)?;
        match raw.escalation {
            Some(report) => response.with_escalation(report),
            None => Ok(response),
        }
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
        Ok(Self { answers, outcome, confidence, trace, metrics, escalation: None })
    }

    /// Attaches the escalation report, validating both halves of the
    /// contract (`ESCALATION_LANES_PLAN.md` §2.2): the report itself must
    /// be well-formed, and only an [`DecisionOutcome::Escalate`] outcome
    /// may carry one — a report on any other outcome is a contradiction,
    /// not decoration.
    ///
    /// # Errors
    ///
    /// [`CoreError::InvalidEscalation`] when the report is malformed or
    /// the outcome is not `Escalate`.
    pub fn with_escalation(mut self, escalation: EscalationReport) -> CoreResult<Self> {
        escalation.validate()?;
        if self.outcome != DecisionOutcome::Escalate {
            return Err(CoreError::InvalidEscalation {
                reason: format!(
                    "report attached to outcome {outcome:?}, only Escalate carries one",
                    outcome = self.outcome
                ),
            });
        }
        self.escalation = Some(escalation);
        Ok(self)
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

    /// The escalation report, when the engine escalated with one.
    #[must_use]
    pub fn escalation(&self) -> Option<&EscalationReport> {
        self.escalation.as_ref()
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
        assert_eq!(response.answers(), []);
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

    fn escalation() -> crate::escalation::EscalationReport {
        use crate::escalation::{
            EngineProvenance, EscalationAction, EscalationGate, EscalationGateKind,
            EscalationPayload, EscalationQuestion, EscalationReason, StateSummary,
        };
        crate::escalation::EscalationReport {
            reason: EscalationReason::BelowConfidence,
            gate: EscalationGate {
                kind: EscalationGateKind::Confidence,
                threshold: 0.62,
                observed: 0.31,
            },
            requested_action: EscalationAction::AskTheHostModel,
            engine: EngineProvenance {
                model_id: "builtin-lexical-v2".to_owned(),
                graph_version: 7,
                calibration_version: 3,
                engine_semver: "0.6.1".to_owned(),
            },
            payload: EscalationPayload {
                question: EscalationQuestion {
                    kind: "boolean".to_owned(),
                    text: "Is the sky blue?".to_owned(),
                },
                state_summary: StateSummary { text_bytes: 41, fact_count: 3, image_count: 0 },
                candidates_ruled_in: vec![],
                candidates_ruled_out: vec![],
                partial_answer: None,
                trace: vec![],
                guidance: "Answer from the state above; say 'unknown' if the state does not \
                           settle it."
                    .to_owned(),
                truncated: false,
            },
        }
    }

    #[test]
    fn escalation_absent_keeps_pre_band_bytes() {
        // The #189 wire promise: without the band, no key, no drift.
        let response = DecisionResponse::new(
            vec![answer()],
            DecisionOutcome::Accept,
            report(),
            DecisionTrace::new(),
            DecisionMetrics::default(),
        )
        .expect("valid");
        let json = serde_json::to_string(&response).expect("serialize");
        assert!(!json.contains("escalation"), "absent-on-default: {json}");
        assert!(response.escalation().is_none());
    }

    #[test]
    fn escalation_round_trips_on_escalate() {
        let response = DecisionResponse::new(
            vec![],
            DecisionOutcome::Escalate,
            report(),
            DecisionTrace::new(),
            DecisionMetrics::default(),
        )
        .expect("a bare escalate is a valid response (the executor attaches the report)")
        .with_escalation(escalation())
        .expect("report attaches");
        assert!(response.escalation().is_some());

        let json = serde_json::to_string(&response).expect("serialize");
        assert!(json.contains(r#""outcome":"escalate""#), "{json}");
        assert!(json.contains(r#""escalation""#), "{json}");
        assert!(json.contains("below_confidence"), "{json}");
        let back: DecisionResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, response);
        assert_eq!(back.escalation(), Some(&escalation()));
    }

    #[test]
    fn escalation_refuses_any_other_outcome() {
        let response = DecisionResponse::new(
            vec![answer()],
            DecisionOutcome::Accept,
            report(),
            DecisionTrace::new(),
            DecisionMetrics::default(),
        )
        .expect("valid");
        let err = response.with_escalation(escalation()).expect_err("refused");
        assert!(matches!(err, CoreError::InvalidEscalation { .. }));
        assert_eq!(err.code(), "ir.invalid_escalation");
    }
}
