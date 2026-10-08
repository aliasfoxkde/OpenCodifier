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
/// c >= min_confidence      -> ACCEPT (unless an uncertainty gate trips)
/// c <  verify_below        -> run verifier
/// c <  abstain_below       -> ABSTAIN (or escalate, per caller)
/// in between               -> accept only after verification
/// ```
///
/// The uncertainty gates (PLANNING.md §19) demote an otherwise-accepted
/// decision to verification when the confidence report shows the input
/// was not cleanly decidable: entropy at or above `entropy_ceiling`, a
/// margin below `min_margin`, or an OOD score above `ood_ceiling`. Their
/// defaults disable them, so a policy behaves as before unless the
/// operator opts in.
///
/// The confidence gates must satisfy `abstain_below <= verify_below <=
/// min_confidence`, otherwise the cascade is contradictory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "RawDecisionPolicy", try_from = "RawDecisionPolicy")]
pub struct DecisionPolicy {
    min_confidence: f64,
    verify_below: f64,
    abstain_below: f64,
    risk: RiskLevel,
    /// Entropy (bits) at or above which a decision may not be accepted
    /// outright. `f64::INFINITY` disables the gate.
    entropy_ceiling: f64,
    /// Margin below which a decision may not be accepted outright.
    /// `0.0` disables the gate.
    min_margin: f64,
    /// OOD score above which a decision may not be accepted outright.
    /// `f64::INFINITY` disables the gate.
    ood_ceiling: f64,
    /// Which deterministic signal feeds the OOD channel (D28). The
    /// default measures answer entropy; `LexicalBand` opts into
    /// input-likeness evidence.
    ood_mode: OodMode,
    /// Boolean verdict boundary (RESEARCH §15.6 item 2): the question is
    /// answered true when `p_true >= boolean_threshold`, replacing the
    /// hard argmax (never hard-0.5 a Boolean). `None` — the default —
    /// keeps the argmax verdict byte-identically.
    boolean_threshold: Option<f64>,
    /// The elicited-abstain candidate (RESEARCH §15.6 item 3). `None` —
    /// the default — appends nothing and stays byte-identical.
    abstain_candidate: Option<AbstainCandidate>,
}

/// Deserialization mirror for [`DecisionPolicy`]; conversion validates.
/// The uncertainty gates default to disabled so older payloads (and the
/// byte-locked wire fixtures) deserialize unchanged, and a disabled gate
/// is omitted from serialization entirely: the canonical form of a
/// policy with the §19 gates off is byte-identical to the pre-§19 form,
/// so cache keys do not shift for engines that do not opt in.
#[derive(Debug, Deserialize, Serialize)]
struct RawDecisionPolicy {
    min_confidence: f64,
    verify_below: f64,
    abstain_below: f64,
    risk: RiskLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    entropy_ceiling: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min_margin: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ood_ceiling: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ood_mode: Option<OodMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    boolean_threshold: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    abstain_candidate: Option<AbstainCandidate>,
}

/// Which deterministic signal feeds the OOD channel (D28).
///
/// The OOD gate (`ood_ceiling`) demotes decisions whose OOD score runs
/// hot; the two modes choose *what is measured*, and are never fused:
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OodMode {
    /// Normalized answer entropy — the shape of the distribution only
    /// (the historical behavior, and the default).
    #[default]
    AnswerEntropy,
    /// Input-likeness: `1 − lexical_coverage` of the best-scoring
    /// document over the query's non-stop tokens. Rungs without lexical
    /// evidence fall back to answer entropy (D28).
    LexicalBand,
}

impl OodMode {
    /// `true` for [`OodMode::AnswerEntropy`] — drives the absent-on-default
    /// serialization that keeps canonical policy forms byte-stable.
    #[must_use]
    pub fn is_answer_entropy(self) -> bool {
        self == Self::AnswerEntropy
    }
}

/// The elicited-abstain candidate (RESEARCH §15.6 item 3): a synthetic
/// choice candidate appended to every choice question of a policy that
/// carries one, giving the deciding rung a way to elect "none of these"
/// instead of guessing. A decision that selects it becomes an
/// [`crate::response::DecisionOutcome::Abstain`], the answer is withheld
/// from the response, and the trace discloses both the configured
/// candidate and the elicitation. Abstain-when-elicted measured right in
/// 17 of 18 research cases — the rung knows when the state does not
/// decide the question, if it is given somewhere to say so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbstainCandidate {
    /// The synthetic candidate's id. The id *marks* the abstain candidate:
    /// if a declared candidate of the question already owns it, that
    /// candidate takes the abstain role and nothing is appended.
    id: String,
    /// How the candidate reads in the candidate list — the deciding rung
    /// scores it like any other option.
    description: String,
}

impl AbstainCandidate {
    /// Validates and constructs the synthetic candidate.
    ///
    /// # Errors
    ///
    /// [`CoreError::EmptyField`] when the id is empty or `"none"` (the
    /// D25 reserved id), or the description is empty.
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> CoreResult<Self> {
        let id = id.into();
        let description = description.into();
        if id.is_empty() || id == "none" {
            return Err(CoreError::EmptyField {
                field: "abstain candidate id (empty or the reserved \"none\")",
            });
        }
        if description.is_empty() {
            return Err(CoreError::EmptyField { field: "abstain candidate description" });
        }
        Ok(Self { id, description })
    }

    /// The synthetic candidate's id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// How the candidate reads in the candidate list.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
}

impl From<DecisionPolicy> for RawDecisionPolicy {
    fn from(policy: DecisionPolicy) -> Self {
        // Disabled gates serialize as absent, keeping the canonical form
        // byte-stable for engines that have not opted into the §19 gates
        // (cache keys do not shift; wire fixtures stay locked).
        Self {
            min_confidence: policy.min_confidence,
            verify_below: policy.verify_below,
            abstain_below: policy.abstain_below,
            risk: policy.risk,
            entropy_ceiling: (policy.entropy_ceiling != f64::INFINITY)
                .then_some(policy.entropy_ceiling),
            min_margin: (policy.min_margin != 0.0).then_some(policy.min_margin),
            ood_ceiling: (policy.ood_ceiling != f64::INFINITY).then_some(policy.ood_ceiling),
            ood_mode: (!policy.ood_mode.is_answer_entropy()).then_some(policy.ood_mode),
            boolean_threshold: policy.boolean_threshold,
            abstain_candidate: policy.abstain_candidate,
        }
    }
}

impl TryFrom<RawDecisionPolicy> for DecisionPolicy {
    type Error = CoreError;

    fn try_from(raw: RawDecisionPolicy) -> CoreResult<Self> {
        let policy = Self::new(raw.min_confidence, raw.verify_below, raw.abstain_below, raw.risk)?;
        let policy = match raw.entropy_ceiling {
            Some(ceiling) => policy.with_entropy_ceiling(ceiling)?,
            None => policy,
        };
        let policy = match raw.min_margin {
            Some(margin) => policy.with_min_margin(margin)?,
            None => policy,
        };
        let policy = match raw.ood_ceiling {
            Some(ceiling) => policy.with_ood_ceiling(ceiling)?,
            None => policy,
        };
        let policy = match raw.boolean_threshold {
            Some(threshold) => policy.with_boolean_threshold(threshold)?,
            None => policy,
        };
        let policy = match raw.abstain_candidate {
            Some(candidate) => policy.with_abstain_candidate(candidate)?,
            None => policy,
        };
        Ok(policy.with_ood_mode(raw.ood_mode.unwrap_or_default()))
    }
}

impl DecisionPolicy {
    /// Gate defaults from PLANNING.md §63: accept ≥ 0.80, verify < 0.65,
    /// abstain < 0.50. The uncertainty gates default to disabled
    /// (entropy ceiling and OOD ceiling infinite, minimum margin zero).
    pub const DEFAULTS: Self = Self {
        min_confidence: 0.80,
        verify_below: 0.65,
        abstain_below: 0.50,
        risk: RiskLevel::Low,
        entropy_ceiling: f64::INFINITY,
        min_margin: 0.0,
        ood_ceiling: f64::INFINITY,
        ood_mode: OodMode::AnswerEntropy,
        boolean_threshold: None,
        abstain_candidate: None,
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
        Ok(Self {
            min_confidence,
            verify_below,
            abstain_below,
            risk,
            entropy_ceiling: f64::INFINITY,
            min_margin: 0.0,
            ood_ceiling: f64::INFINITY,
            ood_mode: OodMode::AnswerEntropy,
            boolean_threshold: None,
            abstain_candidate: None,
        })
    }

    /// Sets the Boolean verdict boundary (RESEARCH §15.6 item 2): the
    /// question is answered true when `p_true >= threshold` instead of
    /// at the hard argmax. Absent (the default), the argmax governs and
    /// canonical serializations are byte-identical to the pre-field
    /// form. The value must lie strictly inside `(0, 1)`: an endpoint
    /// would collapse every Boolean to one verdict.
    ///
    /// # Errors
    ///
    /// [`CoreError::InvalidPolicy`] unless the threshold is a finite
    /// value strictly between 0 and 1.
    pub fn with_boolean_threshold(mut self, threshold: f64) -> CoreResult<Self> {
        if !threshold.is_finite() || !(0.0..1.0).contains(&threshold) {
            return Err(CoreError::InvalidPolicy {
                reason: format!(
                    "boolean_threshold must be a finite value strictly inside (0, 1), \
                     got {threshold}"
                ),
            });
        }
        self.boolean_threshold = Some(threshold);
        Ok(self)
    }

    /// The Boolean verdict boundary, when one is configured.
    #[must_use]
    pub fn boolean_threshold(&self) -> Option<f64> {
        self.boolean_threshold
    }

    /// Appends the elicited-abstain candidate (RESEARCH §15.6 item 3) to
    /// every choice question this policy decides. Absent (the default),
    /// nothing is appended and canonical serializations are byte-identical
    /// to the pre-field form.
    ///
    /// # Errors
    ///
    /// [`CoreError::EmptyField`] when the candidate fails
    /// [`AbstainCandidate::new`]'s validation (re-checked here because the
    /// builder is also the deserialization path).
    pub fn with_abstain_candidate(mut self, candidate: AbstainCandidate) -> CoreResult<Self> {
        AbstainCandidate::new(candidate.id.clone(), candidate.description.clone())?;
        self.abstain_candidate = Some(candidate);
        Ok(self)
    }

    /// The elicited-abstain candidate, when one is configured.
    #[must_use]
    pub fn abstain_candidate(&self) -> Option<&AbstainCandidate> {
        self.abstain_candidate.as_ref()
    }

    /// Selects which deterministic signal feeds the OOD channel (D28).
    /// The default [`OodMode::AnswerEntropy`] is the historical behavior;
    /// [`OodMode::LexicalBand`] opts into input-likeness evidence. Both
    /// are deterministic, and the mode is orthogonal to the gate
    /// relationship validated by [`Self::new`].
    #[must_use]
    pub fn with_ood_mode(mut self, mode: OodMode) -> Self {
        self.ood_mode = mode;
        self
    }

    /// Sets the entropy ceiling (PLANNING.md §19): a decision whose
    /// distribution entropy reaches this many bits may not be accepted
    /// outright and is routed to verification instead. `f64::INFINITY`
    /// (the default) disables the gate.
    ///
    /// # Errors
    ///
    /// [`CoreError::InvalidPolicy`] unless the ceiling is positive (or
    /// exactly `f64::INFINITY`, which disables the gate).
    pub fn with_entropy_ceiling(mut self, ceiling: f64) -> CoreResult<Self> {
        if (!ceiling.is_finite() || ceiling <= 0.0) && ceiling != f64::INFINITY {
            return Err(CoreError::InvalidPolicy {
                reason: format!(
                    "entropy_ceiling must be a positive number (or infinity to disable), \
                     got {ceiling}"
                ),
            });
        }
        self.entropy_ceiling = ceiling;
        Ok(self)
    }

    /// Sets the minimum margin (PLANNING.md §19): a decision whose
    /// top-two probability gap falls below this may not be accepted
    /// outright. `0.0` (the default) disables the gate.
    ///
    /// # Errors
    ///
    /// [`CoreError::InvalidPolicy`] unless the margin is a finite value
    /// in `[0, 1]`.
    pub fn with_min_margin(mut self, margin: f64) -> CoreResult<Self> {
        if !crate::error::is_unit_interval(margin) {
            return Err(CoreError::InvalidPolicy {
                reason: format!("min_margin must be a finite value in [0, 1], got {margin}"),
            });
        }
        self.min_margin = margin;
        Ok(self)
    }

    /// Sets the OOD ceiling (PLANNING.md §19): a decision whose
    /// out-of-distribution score exceeds this may not be accepted
    /// outright. `f64::INFINITY` (the default) disables the gate.
    ///
    /// # Errors
    ///
    /// [`CoreError::InvalidPolicy`] unless the ceiling is a finite value
    /// in `[0, 1]` or exactly `f64::INFINITY` (disabled).
    pub fn with_ood_ceiling(mut self, ceiling: f64) -> CoreResult<Self> {
        if ceiling != f64::INFINITY && !crate::error::is_unit_interval(ceiling) {
            return Err(CoreError::InvalidPolicy {
                reason: format!(
                    "ood_ceiling must be a finite value in [0, 1] (or infinity to disable), \
                     got {ceiling}"
                ),
            });
        }
        self.ood_ceiling = ceiling;
        Ok(self)
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

    /// Entropy (bits) at or above which outright acceptance is refused.
    pub fn entropy_ceiling(&self) -> f64 {
        self.entropy_ceiling
    }

    /// Margin below which outright acceptance is refused.
    pub fn min_margin(&self) -> f64 {
        self.min_margin
    }

    /// OOD score above which outright acceptance is refused.
    pub fn ood_ceiling(&self) -> f64 {
        self.ood_ceiling
    }

    /// The mode selecting what the OOD channel measures (D28).
    #[must_use]
    pub fn ood_mode(&self) -> OodMode {
        self.ood_mode
    }

    /// Whether the §19 uncertainty gates would demote this report to
    /// verification: flat distribution (entropy), near-tie (margin), or
    /// out-of-distribution input.
    #[must_use]
    pub fn uncertainty_gate_trips(&self, report: &crate::confidence::ConfidenceReport) -> bool {
        report.entropy >= self.entropy_ceiling
            || report.margin < self.min_margin
            || report.ood_score > self.ood_ceiling
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
            // 120 s: the model rung's measured CPU tail, not a guess — a
            // 15 KB long-policy state over a 5-candidate parallel readout
            // takes ~22 s, so the earlier 10 s ceiling turned long-document
            // escalations into `schema.limit_exceeded` rejections (found by
            // the JevBench ladder runs, 2026-10-05). Still a hard ceiling:
            // a request may declare anything at or under it, never above.
            max_execution_time: Duration::from_secs(120),
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
    fn abstain_candidate_serializes_as_absent_on_the_default() {
        // Same absent-on-default discipline as the §19 gates and the
        // Boolean boundary: a policy without elicitation is byte-identical
        // to the pre-field form, so cache keys do not shift.
        let default_json = serde_json::to_string(&DecisionPolicy::default()).expect("serialize");
        assert!(
            !default_json.contains("abstain_candidate"),
            "the default must serialize as absent, got {default_json}"
        );
        let opted = DecisionPolicy::default()
            .with_abstain_candidate(
                AbstainCandidate::new("abstain", "None of the listed candidates").unwrap(),
            )
            .unwrap();
        let opted_json = serde_json::to_string(&opted).expect("serialize");
        assert!(opted_json.contains("\"abstain_candidate\":{\"id\":\"abstain\""), "{opted_json}");
        let restored: DecisionPolicy = serde_json::from_str(&opted_json).expect("deserialize");
        assert_eq!(restored, opted);
        let restored_default: DecisionPolicy =
            serde_json::from_str(&default_json).expect("deserialize");
        assert!(restored_default.abstain_candidate().is_none());
    }

    #[test]
    fn abstain_candidate_construction_refuses_empty_and_reserved_ids() {
        assert!(AbstainCandidate::new("", "anything").is_err());
        assert!(AbstainCandidate::new("none", "anything").is_err());
        assert!(AbstainCandidate::new("abstain", "").is_err());
        // And the builder re-checks, because it is the deserialization
        // path: a struct that bypassed `new` must not install either.
        let forged = AbstainCandidate { id: String::new(), description: "x".to_owned() };
        assert!(DecisionPolicy::default().with_abstain_candidate(forged).is_err());
    }

    #[test]
    fn ood_mode_serializes_as_absent_on_the_default() {
        // D28: the canonical form of a policy in the default mode is
        // byte-identical to the pre-D28 form, so cache keys do not shift
        // for engines that never opt in.
        let default_json = serde_json::to_string(&DecisionPolicy::default()).expect("serialize");
        assert!(
            !default_json.contains("ood_mode"),
            "the default mode must serialize as absent, got {default_json}"
        );
        let opted = DecisionPolicy::default().with_ood_mode(OodMode::LexicalBand);
        let opted_json = serde_json::to_string(&opted).expect("serialize");
        assert!(opted_json.contains("\"ood_mode\":\"lexical_band\""), "{opted_json}");
        let restored: DecisionPolicy = serde_json::from_str(&opted_json).expect("deserialize");
        assert_eq!(restored, opted);
        // The default round-trips into the default mode.
        let restored_default: DecisionPolicy =
            serde_json::from_str(&default_json).expect("deserialize");
        assert_eq!(restored_default.ood_mode(), OodMode::AnswerEntropy);
    }

    #[test]
    fn limits_default_conservatively() {
        let limits = Limits::default();
        assert_eq!(limits.max_questions, 32);
        assert_eq!(limits.max_candidates, 256);
        assert_eq!(limits.max_execution_time, Duration::from_secs(120));
    }
}
