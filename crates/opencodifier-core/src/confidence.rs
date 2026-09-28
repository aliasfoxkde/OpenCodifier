//! Multi-dimensional confidence reporting (PLANNING.md §18).
//!
//! A single `confidence: 0.91` hides everything the verification cascade
//! needs. This module carries the full picture: raw top probability,
//! margin, entropy, the calibrated value, out-of-distribution signal, and
//! verifier agreement.

use serde::{Deserialize, Serialize};

use crate::answer::Distribution;
use crate::error::CoreResult;
use crate::policy::RiskLevel;

/// Calibrated, multi-dimensional confidence for one request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceReport {
    /// Probability of the winning answer before calibration.
    pub top_probability: f64,
    /// Gap between the top two probabilities (near-tie detector).
    pub margin: f64,
    /// Shannon entropy of the answer distribution, in bits.
    pub entropy: f64,
    /// Calibrated confidence — the only value policy gates should read.
    pub calibrated_confidence: f64,
    /// Out-of-distribution score in `[0, 1]`; higher means the input
    /// looks unlike anything the decision model was trained on.
    pub ood_score: f64,
    /// `Some(true)` when a verifier ran and agreed, `Some(false)` when it
    /// disagreed, `None` when no verifier ran.
    pub verifier_agreement: Option<bool>,
}

impl ConfidenceReport {
    /// Builds a report from a distribution, deriving the statistical
    /// fields and calibrating `top_probability` into
    /// `calibrated_confidence` via the supplied temperature-scaled
    /// mapping.
    ///
    /// `calibrated` is whatever the configured calibration produces —
    /// the raw top probability for the identity calibration (D15: an
    /// un-calibrated run must stay visible as such), a temperature-scaled
    /// value when a fitted artifact is loaded.
    ///
    /// `ood_score` is the caller's out-of-distribution evidence. The
    /// deterministic V1 signal is the distributional proxy (normalized
    /// entropy, computed by the engine); density-ratio and
    /// embedding-distance detectors replace it as the model rungs come
    /// online. It gates acceptance through `DecisionPolicy::ood_ceiling`.
    pub fn from_distribution(
        distribution: &Distribution,
        calibrated: f64,
        ood_score: f64,
        verifier_agreement: Option<bool>,
    ) -> CoreResult<Self> {
        let top = distribution.top();
        Ok(Self {
            top_probability: top.probability,
            margin: distribution.margin(),
            entropy: distribution.entropy(),
            calibrated_confidence: calibrated,
            ood_score,
            verifier_agreement,
        })
    }

    /// Applies the policy cascade to this report: which outcome does the
    /// current confidence justify?
    ///
    /// Cascade (PLANNING.md §19, §20):
    ///
    /// ```text
    /// c <  abstain_below   -> ABSTAIN
    /// c >= min_confidence  -> ACCEPT for Low/Medium risk, unless an
    ///                         uncertainty gate trips (entropy >= ceiling,
    ///                         margin < minimum, OOD > ceiling)
    ///                      -> VERIFY for High/Critical risk (even
    ///                         confident answers are verified)
    /// otherwise            -> VERIFY
    /// ```
    ///
    /// Every dimension of the report is read explicitly — calibrated
    /// confidence gates acceptance, the §19 uncertainty gates demote
    /// flat, near-tie, or out-of-distribution decisions to verification.
    pub fn outcome_for(
        &self,
        policy: &crate::policy::DecisionPolicy,
    ) -> crate::response::DecisionOutcome {
        use crate::response::DecisionOutcome;
        let confidence = self.calibrated_confidence;
        if confidence < policy.abstain_below() {
            DecisionOutcome::Abstain
        } else if confidence >= policy.min_confidence()
            && matches!(policy.risk(), RiskLevel::Low | RiskLevel::Medium)
            && !policy.uncertainty_gate_trips(self)
        {
            DecisionOutcome::Accept
        } else {
            DecisionOutcome::Verify
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use crate::policy::DecisionPolicy;

    #[test]
    fn derives_statistics_from_distribution() {
        let dist = Distribution::from_pairs([("a", 0.78), ("b", 0.17), ("c", 0.05)]).expect("ok");
        let report = ConfidenceReport::from_distribution(&dist, 0.78, 0.06, None).expect("ok");
        assert!((report.top_probability - 0.78).abs() < 1e-9);
        assert!((report.margin - 0.61).abs() < 1e-9);
        assert!(report.entropy > 0.9 && report.entropy < 1.1);
        assert_eq!(report.verifier_agreement, None);
    }

    #[test]
    fn cascade_respects_policy_gates() {
        let policy = DecisionPolicy::default();
        let confident = Distribution::from_pairs([("a", 1.0), ("b", 0.0)]).expect("ok");
        let report = ConfidenceReport::from_distribution(&confident, 0.91, 0.0, None).expect("ok");
        assert_eq!(report.outcome_for(&policy), crate::response::DecisionOutcome::Accept);

        let uncertain = Distribution::from_pairs([("a", 0.5), ("b", 0.5)]).expect("ok");
        let report = ConfidenceReport::from_distribution(&uncertain, 0.40, 0.0, None).expect("ok");
        assert_eq!(report.outcome_for(&policy), crate::response::DecisionOutcome::Abstain);

        let report = ConfidenceReport::from_distribution(&uncertain, 0.55, 0.0, None).expect("ok");
        assert_eq!(report.outcome_for(&policy), crate::response::DecisionOutcome::Verify);
    }

    #[test]
    fn critical_risk_requires_verification_even_when_confident() {
        let policy =
            DecisionPolicy::new(0.9, 0.7, 0.4, crate::policy::RiskLevel::Critical).expect("valid");
        // Confident answer: accepted outright under Low risk, but under
        // Critical risk it must still pass verification.
        let dist = Distribution::from_pairs([("a", 0.95), ("b", 0.05)]).expect("ok");
        let report = ConfidenceReport::from_distribution(&dist, 0.95, 0.0, None).expect("ok");
        assert_eq!(report.outcome_for(&policy), crate::response::DecisionOutcome::Verify);
        assert_eq!(
            report.outcome_for(&DecisionPolicy::default()),
            crate::response::DecisionOutcome::Accept
        );
    }

    #[test]
    fn uncertainty_gates_demote_flat_near_tie_and_ood_answers_to_verification() {
        use crate::response::DecisionOutcome;

        // Confident, well-separated, in-distribution: accepted.
        let dist = Distribution::from_pairs([("a", 0.93), ("b", 0.07)]).expect("ok");
        let report = ConfidenceReport::from_distribution(&dist, 0.93, 0.1, None).expect("ok");
        let base = DecisionPolicy::default();
        assert_eq!(report.outcome_for(&base), DecisionOutcome::Accept);

        // Entropy gate: the same confidence with a flat distribution is
        // demoted once the ceiling is set.
        let flat = Distribution::from_pairs([("a", 0.93), ("b", 0.07)]).expect("ok");
        let entropy_heavy =
            ConfidenceReport::from_distribution(&flat, 0.93, 0.1, None).expect("ok");
        let gated = base.clone().with_entropy_ceiling(1.0).expect("valid");
        // entropy ~0.39 bits: passes a 1.0-bit ceiling.
        assert_eq!(entropy_heavy.outcome_for(&gated), DecisionOutcome::Accept);
        let strict = base.clone().with_entropy_ceiling(0.2).expect("valid");
        assert_eq!(entropy_heavy.outcome_for(&strict), DecisionOutcome::Verify);

        // Margin gate: a near-tie cannot be accepted.
        let tie = Distribution::from_pairs([("a", 0.51), ("b", 0.49)]).expect("ok");
        let near_tie = ConfidenceReport::from_distribution(&tie, 0.93, 0.0, None).expect("ok");
        let margin_gate = base.clone().with_min_margin(0.1).expect("valid");
        assert_eq!(near_tie.outcome_for(&margin_gate), DecisionOutcome::Verify);

        // OOD gate: an out-of-distribution input cannot be accepted.
        let ood_gate = base.clone().with_ood_ceiling(0.5).expect("valid");
        let ood = ConfidenceReport::from_distribution(&dist, 0.93, 0.7, None).expect("ok");
        assert_eq!(ood.outcome_for(&ood_gate), DecisionOutcome::Verify);

        // Gates are disabled by default: the identical reports pass.
        assert_eq!(near_tie.outcome_for(&base), DecisionOutcome::Accept);
        assert_eq!(ood.outcome_for(&base), DecisionOutcome::Accept);
    }

    #[test]
    fn uncertainty_gates_are_visible_and_boundary_semantics_are_inclusive() {
        let base = DecisionPolicy::default();
        assert_eq!(base.entropy_ceiling(), f64::INFINITY);
        assert_eq!(base.min_margin(), 0.0);
        assert_eq!(base.ood_ceiling(), f64::INFINITY);

        let dist = Distribution::from_pairs([("a", 0.5), ("b", 0.5)]).expect("ok");
        let report = ConfidenceReport::from_distribution(&dist, 0.9, 0.0, None).expect("ok");
        // Entropy of a uniform two-answer distribution is exactly 1.0 bit;
        // the gate is `entropy >= ceiling`, so 1.0 trips a 1.0 ceiling.
        assert!((report.entropy - 1.0).abs() < 1e-12);
        let gated = base.clone().with_entropy_ceiling(1.0).expect("valid");
        assert!(gated.uncertainty_gate_trips(&report));
    }

    #[test]
    fn invalid_gate_values_are_rejected() {
        let base = DecisionPolicy::default();
        assert!(base.clone().with_entropy_ceiling(0.0).is_err());
        assert!(base.clone().with_entropy_ceiling(-1.0).is_err());
        assert!(base.clone().with_entropy_ceiling(f64::INFINITY).is_ok());
        assert!(base.clone().with_min_margin(-0.1).is_err());
        assert!(base.clone().with_min_margin(1.1).is_err());
        assert!(base.clone().with_min_margin(0.05).is_ok());
        assert!(base.clone().with_ood_ceiling(1.1).is_err());
        assert!(base.clone().with_ood_ceiling(f64::INFINITY).is_ok());
        assert!(base.with_ood_ceiling(0.5).is_ok());
    }

    #[test]
    fn legacy_policy_payloads_deserialize_with_gates_disabled() {
        let json = r#"{"min_confidence": 0.8, "verify_below": 0.65,
                       "abstain_below": 0.5, "risk": "low"}"#;
        let policy: DecisionPolicy = serde_json::from_str(json).expect("old payload");
        assert_eq!(policy, DecisionPolicy::default());

        let json = r#"{"min_confidence": 0.8, "verify_below": 0.65,
                       "abstain_below": 0.5, "risk": "low",
                       "entropy_ceiling": 0.8, "min_margin": 0.1,
                       "ood_ceiling": 0.4}"#;
        let policy: DecisionPolicy = serde_json::from_str(json).expect("new payload");
        assert_eq!(policy.entropy_ceiling(), 0.8);
        assert_eq!(policy.min_margin(), 0.1);
        assert_eq!(policy.ood_ceiling(), 0.4);
    }
}
