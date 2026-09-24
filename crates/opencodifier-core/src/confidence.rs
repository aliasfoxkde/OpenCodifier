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
    /// Until a calibration layer exists, pass `identity` calibration;
    /// reports then clearly carry uncalibrated values (which policy must
    /// not treat as final confidence — PLANNING.md Rule 8).
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
    /// c >= min_confidence  -> ACCEPT for Low/Medium risk
    ///                      -> VERIFY for High/Critical risk (even
    ///                         confident answers are verified)
    /// otherwise            -> VERIFY
    /// ```
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
}
