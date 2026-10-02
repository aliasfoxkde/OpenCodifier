//! Per-node and per-kind confidence-gate policies — the escalation
//! ladder (PLANNING.md §24; the fusion study,
//! `benchmarks/decision-model/results/fusion-suite.md`).
//!
//! One graph may route different questions through different mechanisms
//! (exact rules, semantic narrowing, the decision model), and those
//! mechanisms' confidences are not commensurable: a relational proof is
//! p = 1.0 by construction, an embedding score carries no calibrated
//! probability at all, and the model rung carries a fitted temperature
//! (D15). A single request-level
//! [`DecisionPolicy`] therefore
//! cannot gate them all well. A [`LadderPolicy`] supplies optional
//! overrides — per node id, then per node kind — that the executor
//! consults at the existing confidence gate instead of the request
//! policy. No IR change, no wire change: the ladder is engine-internal
//! composition of an existing, validated type.
//!
//! The default ladder is empty: every question is gated by the request
//! policy exactly as before, traces and cache keys are unchanged, and
//! wire fixtures hold byte-for-byte. Overrides take effect only when a
//! caller configures them; the engine then decorates the model id with
//! `|ladder-v1@<id>` so cached decisions re-key (mirroring focused
//! extraction, D6).
//!
//! This is distinct from the graph's per-node `NodeSpec::threshold`
//! knob: that scalar is declared on the graph (and folded into node
//! fingerprinting) but is not consumed by the executor's gate; the
//! ladder is engine-configuration-level and carries a full policy.
//!
//! Named profiles ship as JSON documents ([`LadderProfile`]) under
//! `ladders/` in the repository root — `fusion-v1` is the measured
//! escalation profile of the fusion study,
//! `benchmarks/decision-model/results/fusion-suite.md` — and load
//! through the interfaces' `--ladder` flag. A profile is data, not
//! code: loading validates every policy and artifact before an engine
//! is built.

use std::collections::BTreeMap;

use opencodifier_core::DecisionPolicy;
use serde::Deserialize;

use crate::calibration::{Calibration, CalibrationArtifact, ProofAwareCalibration};
use crate::error::{EngineError, EngineResult};
use crate::graph::NodeKind;

/// Optional per-node / per-kind confidence-gate policy overrides.
///
/// Resolution order for a question decided by node `n` of kind `k`:
/// `per_node[n]`, then `per_kind[k]`, then the request policy.
///
/// A rung may also carry its own [`Calibration`] (D15): rung
/// confidences are not commensurable (an exact proof is p = 1.0 by
/// construction, a fitted model rung carries a temperature), so a
/// per-rung calibration resolves like the policy does and replaces the
/// engine-level calibration for that rung's questions. Artifact swaps
/// re-key through the ladder id — bump the id when a rung's calibration
/// changes, exactly like any other artifact change.
#[derive(Debug, Clone, Default)]
pub struct LadderPolicy {
    /// Ladder identity, folded into the cache key as `|ladder-v1@<id>`
    /// when the ladder is non-empty — changing the id re-keys every
    /// cached decision, exactly like a model swap. `"none"` is the empty
    /// default.
    pub id: String,
    /// Policy per deciding node id; wins over [`Self::per_kind`]. Keys
    /// are node ids as they appear in the graph.
    pub per_node: BTreeMap<String, DecisionPolicy>,
    /// Policy per deciding node kind, used when the deciding node has no
    /// [`Self::per_node`] entry.
    pub per_kind: BTreeMap<NodeKind, DecisionPolicy>,
    /// Calibration per rung, keyed by the same `node:<id>` /
    /// `kind:<name>` strings the trace records as `policy_source`.
    /// `None` from [`Self::resolve_calibration`] means the engine-level
    /// calibration applies, unchanged.
    pub calibrations: BTreeMap<String, std::sync::Arc<dyn Calibration>>,
}

impl PartialEq for LadderPolicy {
    /// Policy-shape equality: id and the two policy maps. Calibrations
    /// are compared by presence, not contents — calibration identity is
    /// pinned by each implementation's `version()` in the cache key and
    /// by the ladder id, not by structural comparison of fitted
    /// artifacts.
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.per_node == other.per_node
            && self.per_kind == other.per_kind
            && self.calibrations.len() == other.calibrations.len()
            && self
                .calibrations
                .keys()
                .zip(other.calibrations.keys())
                .all(|(left, right)| left == right)
    }
}

impl LadderPolicy {
    /// An empty ladder named `id` (`"none"` is the conventional name for
    /// the empty default).
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), ..Self::default() }
    }

    /// `true` when no overrides are configured — the engine then runs
    /// byte-identically to a single-policy engine.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.per_node.is_empty() && self.per_kind.is_empty() && self.calibrations.is_empty()
    }

    /// The override for a question decided by node id `node_id` of kind
    /// `kind`, most specific first, paired with the source to record in
    /// the trace (`node:<id>` / `kind:<name>`). `None` when the ladder
    /// has no override for this node — the request policy governs.
    #[must_use]
    pub fn resolve(&self, node_id: &str, kind: NodeKind) -> Option<(&DecisionPolicy, String)> {
        if let Some(policy) = self.per_node.get(node_id) {
            return Some((policy, format!("node:{node_id}")));
        }
        if let Some(policy) = self.per_kind.get(&kind) {
            return Some((policy, format!("kind:{}", kind.as_str())));
        }
        None
    }

    /// The calibration for a question decided by node id `node_id` of
    /// kind `kind`, resolved in the same order as [`Self::resolve`] and
    /// paired with its source string for the trace. `None` when the
    /// ladder has no calibration for this rung — the engine-level
    /// calibration governs, unchanged.
    #[must_use]
    pub fn resolve_calibration(
        &self,
        node_id: &str,
        kind: NodeKind,
    ) -> Option<(&dyn Calibration, String)> {
        if let Some(calibration) = self.calibrations.get(&format!("node:{node_id}")) {
            return Some((calibration.as_ref(), format!("node:{node_id}")));
        }
        if let Some(calibration) = self.calibrations.get(&format!("kind:{}", kind.as_str())) {
            return Some((calibration.as_ref(), format!("kind:{}", kind.as_str())));
        }
        None
    }

    /// Attaches a calibration to a rung key (`node:<id>` or
    /// `kind:<name>` — the same strings the trace records).
    #[must_use]
    pub fn with_calibration(
        mut self,
        rung: impl Into<String>,
        calibration: std::sync::Arc<dyn Calibration>,
    ) -> Self {
        self.calibrations.insert(rung.into(), calibration);
        self
    }

    /// Checks the ladder is internally consistent: a non-empty ladder
    /// must carry a real identity, because that identity is the only
    /// thing separating its cache keys from single-policy decisions
    /// (PLANNING.md §64 — all cache keys include the policy identity).
    ///
    /// # Errors
    ///
    /// A non-empty ladder whose `id` is empty or `"none"`.
    pub fn validate(&self) -> Result<(), String> {
        if !self.is_empty() && (self.id.is_empty() || self.id == "none") {
            return Err(format!(
                "a non-empty ladder needs a distinct `id` (got {:?}); the id is the \
                 cache-key discriminator for ladder-gated decisions",
                self.id
            ));
        }
        Ok(())
    }
}

/// A named, deserializable ladder document — the shipped form of a
/// [`LadderPolicy`].
///
/// The document maps one-to-one onto the policy's fields except the
/// rung calibrations, which a JSON document carries as fitted
/// [`CalibrationArtifact`]s (D15) rather than trait objects:
/// deserialization runs every artifact through
/// [`crate::calibration::TemperatureCalibration::from_artifact`], which
/// enforces the artifact's invariants, and `proof_aware` wraps the
/// rung's temperature in [`ProofAwareCalibration`] so exact proofs
/// arrive at their raw p = 1.0 while the delegated tail keeps the fit.
///
/// Policies deserialize through `DecisionPolicy`'s own validated
/// mirror, so an invalid gate relationship (`abstain_below >
/// verify_below`, …) is rejected at load, not at decide time.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LadderProfile {
    /// Ladder identity; becomes [`LadderPolicy::id`].
    pub id: String,
    /// Per-node-id rung policies.
    #[serde(default)]
    pub per_node: BTreeMap<String, DecisionPolicy>,
    /// Per-node-kind rung policies, keyed by the kinds' `snake_case`
    /// names (`rule`, `lexical`, `choice`, …).
    #[serde(default)]
    pub per_kind: BTreeMap<NodeKind, DecisionPolicy>,
    /// Per-rung calibration artifacts, keyed by the same `node:<id>` /
    /// `kind:<name>` strings the trace records.
    #[serde(default)]
    pub calibrations: BTreeMap<String, RungCalibration>,
}

/// One rung's fitted calibration: the D15 artifact plus the optional
/// proof-aware wrapper.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RungCalibration {
    /// The fitted temperatures (validated on load).
    pub artifact: CalibrationArtifact,
    /// Wrap the rung in [`ProofAwareCalibration`]: single-entry
    /// distributions (exact proofs) keep their raw p = 1.0 and only the
    /// delegated tail is tempered.
    #[serde(default)]
    pub proof_aware: bool,
}

impl LadderProfile {
    /// Builds the engine-level ladder: validates the identity, fits
    /// every calibration artifact, and refuses an empty profile.
    ///
    /// # Errors
    ///
    /// [`EngineError::InvalidConfig`] when the profile carries no
    /// override at all, fails [`LadderPolicy::validate`], or contains
    /// an artifact that `TemperatureCalibration::from_artifact`
    /// rejects.
    pub fn into_ladder(self) -> EngineResult<LadderPolicy> {
        let mut ladder = LadderPolicy {
            id: self.id,
            per_node: self.per_node,
            per_kind: self.per_kind,
            calibrations: BTreeMap::new(),
        };
        if ladder.is_empty() {
            return Err(EngineError::InvalidConfig {
                reason: "ladder profile configures no override; run without --ladder \
                         so the request policy governs"
                    .to_owned(),
            });
        }
        ladder.validate().map_err(|reason| EngineError::InvalidConfig { reason })?;
        for (rung, calibration) in self.calibrations {
            let temperature =
                crate::calibration::TemperatureCalibration::from_artifact(calibration.artifact)
                    .map_err(|error| EngineError::InvalidConfig {
                        reason: format!("rung {rung:?}: {error}"),
                    })?;
            let calibrated: std::sync::Arc<dyn Calibration> = if calibration.proof_aware {
                std::sync::Arc::new(ProofAwareCalibration::new(std::sync::Arc::new(temperature)))
            } else {
                std::sync::Arc::new(temperature)
            };
            ladder.calibrations.insert(rung, calibrated);
        }
        Ok(ladder)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use super::*;
    use std::sync::Arc;

    use opencodifier_core::{Distribution, RiskLevel};

    use crate::calibration::IdentityCalibration;

    fn policy(min_confidence: f64) -> DecisionPolicy {
        DecisionPolicy::new(
            min_confidence,
            min_confidence - 0.1,
            min_confidence / 2.0,
            RiskLevel::Low,
        )
        .unwrap()
    }

    #[test]
    fn empty_ladder_resolves_nothing() {
        let ladder = LadderPolicy::new("none");
        assert!(ladder.is_empty());
        assert_eq!(ladder.resolve("decide", NodeKind::Choice), None);
        ladder.validate().unwrap();
    }

    #[test]
    fn per_kind_override_resolves_with_kind_source() {
        let ladder = LadderPolicy {
            id: "test-v1".into(),
            per_kind: BTreeMap::from([(NodeKind::Choice, policy(0.9))]),
            ..LadderPolicy::default()
        };
        let (resolved, source) = ladder.resolve("q_decide", NodeKind::Choice).unwrap();
        assert_eq!(resolved.min_confidence(), 0.9);
        assert_eq!(source, "kind:choice");
        // A different kind is untouched.
        assert_eq!(ladder.resolve("q_decide", NodeKind::Boolean), None);
    }

    #[test]
    fn per_node_beats_per_kind() {
        let ladder = LadderPolicy {
            id: "test-v1".into(),
            per_node: BTreeMap::from([("decide_choice".into(), policy(0.7))]),
            per_kind: BTreeMap::from([(NodeKind::Choice, policy(0.95))]),
            calibrations: BTreeMap::new(),
        };
        let (resolved, source) = ladder.resolve("decide_choice", NodeKind::Choice).unwrap();
        assert_eq!(resolved.min_confidence(), 0.7);
        assert_eq!(source, "node:decide_choice");
        // Another choice node falls through to the kind entry.
        let (resolved, source) = ladder.resolve("decide_other", NodeKind::Choice).unwrap();
        assert_eq!(resolved.min_confidence(), 0.95);
        assert_eq!(source, "kind:choice");
    }

    #[test]
    fn non_empty_ladder_requires_identity() {
        let ladder = LadderPolicy {
            per_kind: BTreeMap::from([(NodeKind::Boolean, policy(0.9))]),
            ..LadderPolicy::new("none")
        };
        assert!(ladder.validate().is_err());
        let unnamed = LadderPolicy { per_kind: ladder.per_kind.clone(), ..LadderPolicy::new("") };
        assert!(unnamed.validate().is_err());
    }

    #[test]
    fn empty_ladder_accepts_the_none_identity() {
        assert!(LadderPolicy::new("none").validate().is_ok());
    }

    #[test]
    fn calibration_only_ladder_is_not_empty_and_needs_identity() {
        // Calibrations alone change confidence values, so they alone make
        // a ladder non-empty: the validate() identity requirement must
        // fire for them too, or an artifact swap could silently re-gate
        // decisions under single-policy cache keys.
        let ladder = LadderPolicy::new("none")
            .with_calibration("kind:choice", Arc::new(IdentityCalibration));
        assert!(!ladder.is_empty());
        assert!(ladder.validate().is_err());
    }

    #[test]
    fn rung_calibration_resolves_node_over_kind_with_source() {
        let ladder = LadderPolicy::new("rungs-v1")
            .with_calibration("node:decide_choice", Arc::new(IdentityCalibration))
            .with_calibration("kind:choice", Arc::new(SharpCalibration));

        let (node_calibration, source) =
            ladder.resolve_calibration("decide_choice", NodeKind::Choice).unwrap();
        assert_eq!(source, "node:decide_choice");
        assert_eq!(node_calibration.version(), 0, "IdentityCalibration");

        let (kind_calibration, source) =
            ladder.resolve_calibration("decide_other", NodeKind::Choice).unwrap();
        assert_eq!(source, "kind:choice");
        assert_eq!(kind_calibration.version(), 5, "SharpCalibration");
        // Resolution hands back a real callable: SharpCalibration answers
        // with the raw top probability.
        let hedged = Distribution::from_pairs([("a", 0.7), ("b", 0.3)]).unwrap();
        assert_eq!(kind_calibration.calibrate("choice", &hedged), 0.7);

        // Boolean has no rung entry: the engine-level calibration governs.
        assert!(ladder.resolve_calibration("decide", NodeKind::Boolean).is_none());
    }

    #[test]
    fn ladder_equality_compares_shape_and_rung_keys() {
        // Equality is the policy shape plus the rung-key set: calibration
        // identity rides the ladder id in the cache key, never a
        // structural comparison of fitted artifacts.
        let base = || {
            LadderPolicy::new("eq-v1")
                .with_calibration("kind:choice", Arc::new(IdentityCalibration))
        };
        assert_eq!(base(), base());
        // A different id is a different ladder — it re-keys the cache.
        assert_ne!(base(), LadderPolicy { id: "eq-v2".into(), ..base() });
        // A different gate value is a different ladder.
        assert_ne!(
            base(),
            LadderPolicy {
                per_kind: BTreeMap::from([(NodeKind::Choice, policy(0.9))]),
                ..LadderPolicy::new("eq-v1")
            }
        );
        // Same rung keys under different fits stay equal; a different key
        // set (or no calibrations at all) does not.
        let rescaled =
            LadderPolicy::new("eq-v1").with_calibration("kind:choice", Arc::new(SharpCalibration));
        assert_eq!(base(), rescaled);
        assert_ne!(
            base(),
            LadderPolicy::new("eq-v1")
                .with_calibration("kind:boolean", Arc::new(IdentityCalibration))
        );
        assert_ne!(base(), LadderPolicy::new("eq-v1"));
    }

    /// A named-version calibration distinguishing it from identity in
    /// the resolution test above.
    #[derive(Debug)]
    struct SharpCalibration;

    impl Calibration for SharpCalibration {
        fn calibrate(&self, _class: &str, distribution: &Distribution) -> f64 {
            distribution.top().probability
        }

        fn version(&self) -> u64 {
            5
        }
    }

    /// The JSON a shipped profile document uses — `min_confidence 0.0`
    /// with a margin floor is the fusion study's accept-on-margin rung.
    const FUSION_PROFILE_JSON: &str = r#"{
        "id": "fusion-v1",
        "per_kind": {
            "rule": {
                "min_confidence": 1.0, "verify_below": 0.95,
                "abstain_below": 0.5, "risk": "low"
            },
            "choice": {
                "min_confidence": 0.0, "verify_below": 0.0,
                "abstain_below": 0.0, "risk": "low", "min_margin": 0.0183
            }
        }
    }"#;

    #[test]
    fn a_profile_document_builds_the_named_ladder() {
        let profile: LadderProfile = serde_json::from_str(FUSION_PROFILE_JSON).unwrap();
        let ladder = profile.into_ladder().unwrap();
        assert_eq!(ladder.id, "fusion-v1");
        assert_eq!(ladder.per_kind.len(), 2);
        // The accept-on-margin rung: probability gates never demote, the
        // §19 margin floor is the only gate that can.
        let choice = &ladder.per_kind[&NodeKind::Choice];
        assert_eq!(choice.min_confidence(), 0.0);
        assert_eq!(choice.min_margin(), 0.0183);
        ladder.validate().unwrap();
    }

    #[test]
    fn an_empty_profile_is_refused() {
        let ladder: LadderProfile = serde_json::from_str(r#"{ "id": "empty-v1" }"#).unwrap();
        let error = ladder.into_ladder().unwrap_err();
        assert!(matches!(error, EngineError::InvalidConfig { .. }), "{error:?}");
    }

    #[test]
    fn a_profile_artifact_fits_into_the_rung() {
        let document = r#"{
            "id": "cal-v1",
            "per_kind": {
                "boolean": {
                    "min_confidence": 0.9, "verify_below": 0.7,
                    "abstain_below": 0.4, "risk": "low"
                }
            },
            "calibrations": {
                "kind:boolean": {
                    "artifact": {
                        "format_version": 1, "scheme": "temperature",
                        "model_id": "test-model", "calibration_version": 3,
                        "default_temperature": 2.0,
                        "fit": { "items": 10, "ece_before": 0.2,
                                 "ece_after": 0.1, "source": "test" }
                    },
                    "proof_aware": true
                }
            }
        }"#;
        let profile: LadderProfile = serde_json::from_str(document).unwrap();
        let ladder = profile.into_ladder().unwrap();
        let (calibration, source) =
            ladder.resolve_calibration("any_bool", NodeKind::Boolean).unwrap();
        assert_eq!(source, "kind:boolean");
        assert_eq!(calibration.version(), 3);
        // The wrapper: a proof passes at raw 1.0 even under T = 2.
        let proof = Distribution::from_pairs([("proved", 1.0)]).unwrap();
        assert_eq!(calibration.calibrate("boolean", &proof), 1.0);
    }

    #[test]
    fn a_profile_artifact_that_fails_to_fit_names_the_rung() {
        // `calibration_version 0` is reserved for "no calibration", so
        // the artifact is refused at load — and the config error must
        // name the offending rung instead of dropping it silently.
        let document = r#"{
            "id": "bad-cal-v1",
            "per_kind": {
                "boolean": {
                    "min_confidence": 0.9, "verify_below": 0.7,
                    "abstain_below": 0.4, "risk": "low"
                }
            },
            "calibrations": {
                "kind:boolean": {
                    "artifact": {
                        "format_version": 1, "scheme": "temperature",
                        "model_id": "test-model", "calibration_version": 0,
                        "default_temperature": 2.0,
                        "fit": { "items": 10, "ece_before": 0.2,
                                 "ece_after": 0.1, "source": "test" }
                    },
                    "proof_aware": true
                }
            }
        }"#;
        let profile: LadderProfile = serde_json::from_str(document).unwrap();
        let error = profile.into_ladder().unwrap_err();
        assert!(matches!(error, EngineError::InvalidConfig { .. }), "{error:?}");
        assert!(error.to_string().contains("kind:boolean"), "{error}");
    }

    #[test]
    fn a_plain_rung_calibration_applies_the_fit_without_the_proof_wrapper() {
        // Omitting `proof_aware` ships the bare temperature: every
        // distribution of the rung is tempered. (For a normalized
        // single-entry proof the temperature is the identity anyway, so
        // the wrapper buys the bimodal split without changing that case.)
        let document = r#"{
            "id": "plain-cal-v1",
            "per_kind": {
                "choice": {
                    "min_confidence": 0.9, "verify_below": 0.7,
                    "abstain_below": 0.4, "risk": "low"
                }
            },
            "calibrations": {
                "kind:choice": {
                    "artifact": {
                        "format_version": 1, "scheme": "temperature",
                        "model_id": "test-model", "calibration_version": 4,
                        "default_temperature": 2.0,
                        "fit": { "items": 10, "ece_before": 0.2,
                                 "ece_after": 0.1, "source": "test" }
                    }
                }
            }
        }"#;
        let profile: LadderProfile = serde_json::from_str(document).unwrap();
        let ladder = profile.into_ladder().unwrap();
        let (calibration, source) = ladder.resolve_calibration("any", NodeKind::Choice).unwrap();
        assert_eq!(source, "kind:choice");
        assert_eq!(calibration.version(), 4);
        // Plain temperature semantics on the delegated tail: T = 2
        // flattens a raw 0.9 top to ~0.75 (p^(1/2) rescaled).
        let hedged = Distribution::from_pairs([("a", 0.9), ("b", 0.1)]).unwrap();
        let calibrated = calibration.calibrate("choice", &hedged);
        assert!(
            (0.74..=0.76).contains(&calibrated),
            "T = 2 flattens a raw 0.9 to ~0.75, got {calibrated}"
        );
    }

    #[test]
    fn a_profile_rejects_unknown_fields_and_bad_policies() {
        // Unknown fields stay denied — a typo'd profile is refused, not
        // half-applied.
        let typo: Result<LadderProfile, _> =
            serde_json::from_str(r#"{ "id": "x", "per_kindd": {} }"#);
        assert!(typo.is_err());
        // DecisionPolicy's own validated mirror rejects an inverted
        // cascade at load.
        let inverted: Result<LadderProfile, _> = serde_json::from_str(
            r#"{ "id": "x", "per_kind": { "rule": {
                "min_confidence": 0.5, "verify_below": 0.9,
                "abstain_below": 0.1, "risk": "low" } } }"#,
        );
        assert!(inverted.is_err());
    }
}
