//! Decision policy and request metadata: how much certainty a decision
//! must have, how risky the action is, and what resources one request may
//! consume (PLANNING.md §20, §63, §67).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// How consequential a decision is. Higher risk demands stronger
/// confidence or verification before the answer is accepted.
///
/// This is deterministic policy attached to the request — never inferred
/// from model output (PLANNING.md §20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RiskLevel {
    /// Formatting preferences and similar low-impact choices.
    #[default]
    Low,
    /// Model selection, tool selection.
    Medium,
    /// Source modification, data deletion.
    High,
    /// Production deployments and other irreversible actions.
    Critical,
}

/// Confidence gates controlling the outcome cascade.
///
/// Semantics, for a decision with calibrated confidence `c`:
///
/// ```text
/// c >= min_confidence      -> ACCEPT
/// c <  verify_below        -> run verifier
/// c <  abstain_below       -> ABSTAIN (or escalate, per caller)
/// in between               -> accept only after verification
/// ```
///
/// The gates must satisfy `abstain_below <= verify_below <=
/// min_confidence`, otherwise the cascade is contradictory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionPolicy {
    min_confidence: f64,
    verify_below: f64,
    abstain_below: f64,
    risk: RiskLevel,
}

impl DecisionPolicy {
    /// Gate defaults from PLANNING.md §63: accept ≥ 0.80, verify < 0.65,
    /// abstain < 0.50.
    pub const DEFAULTS: Self = Self {
        min_confidence: 0.80,
        verify_below: 0.65,
        abstain_below: 0.50,
        risk: RiskLevel::Low,
    };

    /// Validates and constructs a policy.
    pub fn new(
        min_confidence: f64,
        verify_below: f64,
        abstain_below: f64,
        risk: RiskLevel,
    ) -> CoreResult<Self> {
        for (name, value) in [
            ("min_confidence", min_confidence),
            ("verify_below", verify_below),
            ("abstain_below", abstain_below),
        ] {
            if !crate::error::is_unit_interval(value) {
                return Err(CoreError::InvalidPolicy {
                    reason: format!("{name} must be a finite value in [0, 1], got {value}"),
                });
            }
        }
        if !(abstain_below <= verify_below && verify_below <= min_confidence) {
            return Err(CoreError::InvalidPolicy {
                reason: format!(
                    "expected abstain_below <= verify_below <= min_confidence, \
                     got {abstain_below} > {verify_below} > {min_confidence}"
                ),
            });
        }
        Ok(Self { min_confidence, verify_below, abstain_below, risk })
    }

    /// Confidence at or above which a decision is accepted outright.
    pub fn min_confidence(&self) -> f64 {
        self.min_confidence
    }

    /// Confidence below which the verifier runs.
    pub fn verify_below(&self) -> f64 {
        self.verify_below
    }

    /// Confidence below which the engine abstains.
    pub fn abstain_below(&self) -> f64 {
        self.abstain_below
    }

    /// The risk classification of the action this decision feeds.
    pub fn risk(&self) -> RiskLevel {
        self.risk
    }
}

impl Default for DecisionPolicy {
    fn default() -> Self {
        Self::DEFAULTS
    }
}

/// Per-request resource ceilings (PLANNING.md §67). Conservative by
/// default; engines must enforce every field they can observe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// Maximum serialized input size accepted.
    pub max_input_bytes: usize,
    /// Maximum questions per request.
    pub max_questions: usize,
    /// Maximum candidates per choice question.
    pub max_candidates: usize,
    /// Maximum nodes in an executed graph.
    pub max_graph_nodes: usize,
    /// Maximum wall-clock time for one request.
    pub max_execution_time: Duration,
    /// Maximum results returned by a semantic retrieval stage.
    pub max_retrieval_results: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 1_048_576, // 1 MiB of state text
            max_questions: 32,
            max_candidates: 256,
            max_graph_nodes: 128,
            max_execution_time: Duration::from_secs(10),
            max_retrieval_results: 64,
        }
    }
}

/// Optional per-request metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RequestMetadata {
    /// Caller-supplied correlation id, echoed in responses when present.
    pub request_id: Option<String>,
    /// Resource ceilings for this request.
    pub limits: Limits,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    #[test]
    fn defaults_are_ordered_and_valid() {
        let policy = DecisionPolicy::default();
        assert_eq!(policy.min_confidence(), 0.80);
        assert_eq!(policy.verify_below(), 0.65);
        assert_eq!(policy.abstain_below(), 0.50);
        assert_eq!(policy.risk(), RiskLevel::Low);
    }

    #[test]
    fn rejects_out_of_range_gates() {
        assert!(DecisionPolicy::new(1.5, 0.65, 0.5, RiskLevel::Low).is_err());
        assert!(DecisionPolicy::new(f64::NAN, 0.65, 0.5, RiskLevel::Low).is_err());
        assert!(DecisionPolicy::new(-0.1, 0.65, 0.5, RiskLevel::Low).is_err());
    }

    #[test]
    fn rejects_contradictory_gate_ordering() {
        // min_confidence below verify_below is incoherent.
        assert!(DecisionPolicy::new(0.6, 0.65, 0.5, RiskLevel::High).is_err());
        // verify_below below abstain_below is incoherent.
        assert!(DecisionPolicy::new(0.9, 0.4, 0.5, RiskLevel::High).is_err());
    }

    #[test]
    fn round_trips_through_json() {
        let policy = DecisionPolicy::new(0.9, 0.7, 0.4, RiskLevel::Critical).expect("valid");
        let json = serde_json::to_string(&policy).expect("serialize");
        let back: DecisionPolicy = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, policy);
    }

    #[test]
    fn limits_default_conservatively() {
        let limits = Limits::default();
        assert_eq!(limits.max_questions, 32);
        assert_eq!(limits.max_candidates, 256);
        assert_eq!(limits.max_execution_time, Duration::from_secs(10));
    }
}
