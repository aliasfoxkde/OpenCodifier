//! The escalation ladder's arms (`src/ladder.rs`, `src/calibration.rs`)
//! pinned across the crate boundary (PLANNING.md §24, §64; D15, D25, D27).
//!
//! `tests/ladder.rs` drives the ladder through the engine and
//! `tests/rung_escalation.rs` walks the rung list; what is left unpinned
//! outside the crate is the composition surface those tests only reach
//! indirectly — resolution order and its source strings, equality as
//! composition, profile loading and its refusals, and the calibration
//! artifacts a rung carries. The calibration contract lives here because
//! the ladder is its only consumer: a fitted artifact is a rung's
//! confidence, and a refusal at load is a refused deployment.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::{
    boolean_question, choice_question, config, request_with_policy, score_question, trace_fact,
};
use opencodifier_core::{
    DecisionAnswer, DecisionOutcome, DecisionPolicy, Distribution, FactValue, RequestMetadata,
    RiskLevel, State,
};
use opencodifier_engine::calibration::question_class;
use opencodifier_engine::{
    Calibration, EngineError, IdentityCalibration, LadderPolicy, LadderProfile, MockClassifier,
    NodeKind, ProofAwareCalibration, RungCalibration, TemperatureCalibration,
};

/// A gate cascade accepting at `min_confidence`, with the verify band just
/// below it — the shape every rung policy in these tests uses.
fn gate(min_confidence: f64) -> DecisionPolicy {
    DecisionPolicy::new(min_confidence, min_confidence - 0.05, min_confidence / 2.0, RiskLevel::Low)
        .unwrap()
}

/// A flattening fit (T = 3): a raw 0.9 top lands near 0.675.
fn flattening() -> Arc<TemperatureCalibration> {
    Arc::new(TemperatureCalibration::from_artifact(common::calibration_artifact(3.0)).unwrap())
}

/// Parses a profile document, failing the test with the serde error.
fn profile(document: &str) -> LadderProfile {
    serde_json::from_str(document).unwrap_or_else(|error| panic!("profile: {error}"))
}

/// The calibrated-artifact body of a profile rung: `version` is the fit's
/// cache version, `default_temperature` the fit's flattening power.
fn rung_artifact(version: u64, default_temperature: f64) -> String {
    format!(
        r#"{{ "format_version": 1, "scheme": "temperature",
             "model_id": "test-model", "calibration_version": {version},
             "default_temperature": {default_temperature},
             "fit": {{ "items": 10, "ece_before": 0.2, "ece_after": 0.1,
                       "source": "test" }} }}"#
    )
}

#[test]
fn a_named_ladder_without_overrides_is_empty_and_governs_nothing() {
    // Emptiness is about overrides, not the name: a ladder with a distinct
    // id and no entry anywhere is still the do-nothing ladder, and it is
    // legal to build — the identity requirement bites only once something
    // is actually overridden.
    let ladder = LadderPolicy::new("idle-v1");
    assert!(ladder.is_empty());
    assert_eq!(ladder.resolve("choice", NodeKind::Choice), None);
    assert!(ladder.resolve_calibration("choice", NodeKind::Choice).is_none());
    ladder.validate().unwrap();
}

#[test]
fn policy_resolution_is_most_specific_first_and_names_its_source() {
    // The executor gates with the most specific override and records where
    // it came from; an unconfigured kind resolves to nothing at all, which
    // is what leaves the request policy governing unchanged.
    let ladder = LadderPolicy {
        id: "specificity-v1".to_owned(),
        per_node: BTreeMap::from([("choice".to_owned(), gate(0.95))]),
        per_kind: BTreeMap::from([(NodeKind::Choice, gate(0.60)), (NodeKind::Boolean, gate(0.70))]),
        calibrations: BTreeMap::new(),
    };

    let (node, source) = ladder.resolve("choice", NodeKind::Choice).unwrap();
    assert_eq!((node.min_confidence(), source.as_str()), (0.95, "node:choice"));

    let (kind, source) = ladder.resolve("choice_other", NodeKind::Choice).unwrap();
    assert_eq!((kind.min_confidence(), source.as_str()), (0.60, "kind:choice"));

    assert_eq!(
        ladder.resolve("score", NodeKind::Score),
        None,
        "a node id and kind with no entry resolve to nothing"
    );
}

#[test]
fn calibration_resolution_is_independent_of_policy_resolution() {
    // The two maps key the same rung strings but resolve separately: a rung
    // may carry a calibration with no gate of its own, and a gate with no
    // calibration of its own. Neither implies the other.
    let policy_only = LadderPolicy {
        id: "policy-only-v1".to_owned(),
        per_kind: BTreeMap::from([(NodeKind::Choice, gate(0.90))]),
        ..LadderPolicy::default()
    };
    let (policy, source) = policy_only.resolve("choice", NodeKind::Choice).unwrap();
    assert_eq!((policy.min_confidence(), source.as_str()), (0.90, "kind:choice"));
    assert!(
        policy_only.resolve_calibration("choice", NodeKind::Choice).is_none(),
        "a gate override is not a calibration override"
    );

    let calibration_only = LadderPolicy::new("calibration-only-v1")
        .with_calibration("node:choice", Arc::new(IdentityCalibration) as _);
    assert!(
        calibration_only.resolve("choice", NodeKind::Choice).is_none(),
        "a calibration override is not a gate override"
    );
    let (calibration, source) =
        calibration_only.resolve_calibration("choice", NodeKind::Choice).unwrap();
    assert_eq!(source, "node:choice");
    // The handed-back trait object is a real callable, not a marker.
    let hedged = Distribution::from_pairs([("a", 0.70), ("b", 0.30)]).unwrap();
    assert_eq!(calibration.calibrate("choice", &hedged), 0.70, "the identity answers raw");
}

#[test]
fn ladder_equality_is_the_whole_composition_not_just_the_gates() {
    // Equality is the composition, not the numbers: the same 0.90 gate
    // reached through `per_kind` and through `per_node` is a different
    // ladder, because it gates a different set of questions and the trace
    // names a different source. A rung key in the calibration map is part
    // of the composition too.
    let kind_gated = || LadderPolicy {
        id: "eq-v1".to_owned(),
        per_kind: BTreeMap::from([(NodeKind::Choice, gate(0.90))]),
        ..LadderPolicy::default()
    };
    assert_eq!(kind_gated(), kind_gated(), "the same composition is the same ladder");

    let node_gated = LadderPolicy {
        id: "eq-v1".to_owned(),
        per_node: BTreeMap::from([("choice".to_owned(), gate(0.90))]),
        ..LadderPolicy::default()
    };
    assert_ne!(kind_gated(), node_gated);

    let calibrated = LadderPolicy::new("eq-v1")
        .with_calibration("kind:choice", Arc::new(IdentityCalibration) as _);
    assert_ne!(kind_gated(), calibrated, "a rung calibration changes the composition");
}

#[test]
fn a_profile_without_overrides_is_refused_with_a_stable_code() {
    // Loading a profile that gates nothing is a refused deployment, not a
    // silent no-op: the caller asked for a ladder and would get none.
    let error = profile(r#"{ "id": "empty-v1" }"#).into_ladder().unwrap_err();
    assert!(matches!(error, EngineError::InvalidConfig { .. }), "{error:?}");
    assert_eq!(error.code(), "engine.invalid_config");
    assert!(error.to_string().contains("configures no override"), "{error}");
}

#[test]
fn a_non_empty_profile_still_needs_a_distinct_identity() {
    // The ladder id is the cache-key discriminator (PLANNING.md §64): the
    // empty default's name — and no name at all — may not gate anything,
    // or ladder-gated decisions would be served under single-policy keys.
    let rung = r#"{ "min_confidence": 0.95, "verify_below": 0.90,
                    "abstain_below": 0.45, "risk": "low" }"#;
    for identity in ["", "none"] {
        let document = format!(r#"{{ "id": "{identity}", "per_kind": {{ "choice": {rung} }} }}"#);
        let error = profile(&document).into_ladder().unwrap_err();
        assert_eq!(error.code(), "engine.invalid_config");
        assert!(error.to_string().contains("distinct `id`"), "{error}");
    }
}

#[test]
fn a_profile_builds_node_keyed_rung_calibrations() {
    // `kind:` keys are the usual shape and `node:` keys the specific one;
    // both go through the same fit-and-wrap path, and a node key applies to
    // that node only — no other node of the same kind inherits it.
    let document = format!(
        r#"{{
            "id": "node-cal-v1",
            "per_node": {{
                "choice": {{ "min_confidence": 0.95, "verify_below": 0.90,
                            "abstain_below": 0.45, "risk": "low" }}
            }},
            "calibrations": {{
                "node:choice": {{ "artifact": {artifact}, "proof_aware": true }}
            }}
        }}"#,
        artifact = rung_artifact(3, 3.0),
    );
    let ladder = profile(&document).into_ladder().unwrap();
    assert_eq!(ladder.id, "node-cal-v1");

    let (policy, policy_source) = ladder.resolve("choice", NodeKind::Choice).unwrap();
    assert_eq!((policy.min_confidence(), policy_source.as_str()), (0.95, "node:choice"));

    let (calibration, calibration_source) =
        ladder.resolve_calibration("choice", NodeKind::Choice).unwrap();
    assert_eq!(calibration_source, "node:choice");
    assert_eq!(calibration.version(), 3, "the artifact's version rides the cache key");

    // Proof-aware: a single-entry distribution is an exact proof and keeps
    // its raw p = 1.0 even under T = 3, while the delegated tail flattens.
    let proof = Distribution::from_pairs([("proved", 1.0)]).unwrap();
    assert_eq!(calibration.calibrate("choice", &proof), 1.0);
    let hedged = Distribution::from_pairs([("a", 0.9), ("b", 0.1)]).unwrap();
    let calibrated = calibration.calibrate("choice", &hedged);
    assert!((0.67..=0.68).contains(&calibrated), "T = 3 flattens 0.9 to ~0.675, got {calibrated}");

    assert!(
        ladder.resolve_calibration("boolean", NodeKind::Boolean).is_none(),
        "another node of another kind inherits nothing"
    );
}

#[test]
fn a_profile_rung_that_fails_to_fit_names_the_rung() {
    // `calibration_version 0` is reserved for "no calibration", so the fit
    // is refused at load — and the error must name the offending rung
    // instead of dropping it silently.
    let document = format!(
        r#"{{
            "id": "bad-cal-v1",
            "per_node": {{ "choice": {{ "min_confidence": 0.95, "verify_below": 0.90,
                                        "abstain_below": 0.45, "risk": "low" }} }},
            "calibrations": {{ "node:choice": {{ "artifact": {artifact} }} }}
        }}"#,
        artifact = rung_artifact(0, 3.0),
    );
    let error = profile(&document).into_ladder().unwrap_err();
    assert_eq!(error.code(), "engine.invalid_config");
    let message = error.to_string();
    assert!(message.contains("node:choice"), "{message}");
    assert!(message.contains("calibration_version"), "{message}");
}

#[test]
fn a_rung_calibration_document_defaults_to_the_plain_temperature() {
    // `proof_aware` is opt-in: a rung document that omits it ships the bare
    // temperature, and an unknown field is refused rather than half-applied.
    let artifact = rung_artifact(2, 2.0);
    let plain: RungCalibration =
        serde_json::from_str(&format!(r#"{{ "artifact": {artifact} }}"#)).unwrap();
    assert!(!plain.proof_aware, "the default is the bare temperature");
    assert!(plain.artifact.temperatures.is_empty(), "no per-class table in this fit");
    assert_eq!(plain.artifact.fit.items, 10);

    let wrapped: RungCalibration =
        serde_json::from_str(&format!(r#"{{ "artifact": {artifact}, "proof_aware": true }}"#))
            .unwrap();
    assert!(wrapped.proof_aware);

    let typo: Result<RungCalibration, _> =
        serde_json::from_str(&format!(r#"{{ "artifact": {artifact}, "proof_awaree": true }}"#));
    assert!(typo.is_err(), "an unknown field is refused");
}

#[test]
fn an_artifact_is_refused_on_every_invalid_header_field() {
    // A calibration artifact is trusted the moment it is loaded, so every
    // header invariant is enforced with a message naming the field that
    // broke: format version, scheme, cache version, and the model the fit
    // describes.
    let base = common::calibration_artifact(2.0);
    TemperatureCalibration::from_artifact(base.clone()).unwrap();

    let mut wrong_version = base.clone();
    wrong_version.format_version = 2;
    let error = TemperatureCalibration::from_artifact(wrong_version).unwrap_err();
    assert_eq!(error.code(), "calibration.invalid");
    assert!(error.to_string().contains("format_version"), "{error}");

    let mut wrong_scheme = base.clone();
    wrong_scheme.scheme = "isotonic".to_owned();
    let error = TemperatureCalibration::from_artifact(wrong_scheme).unwrap_err();
    assert_eq!(error.code(), "calibration.invalid");
    assert!(error.to_string().contains("scheme"), "{error}");

    let mut unversioned = base.clone();
    unversioned.calibration_version = 0;
    let error = TemperatureCalibration::from_artifact(unversioned).unwrap_err();
    assert_eq!(error.code(), "calibration.invalid");
    assert!(error.to_string().contains("calibration_version"), "{error}");

    let mut anonymous = base;
    anonymous.model_id = "   ".to_owned();
    let error = TemperatureCalibration::from_artifact(anonymous).unwrap_err();
    assert_eq!(error.code(), "calibration.invalid");
    assert!(error.to_string().contains("model_id"), "{error}");
}

#[test]
fn an_artifact_temperature_must_be_finite_and_positive_everywhere() {
    // The default and every per-class entry: one unusable temperature
    // anywhere refuses the whole artifact, and the message names the class
    // it was fitted for.
    let error =
        TemperatureCalibration::from_artifact(common::calibration_artifact(0.0)).unwrap_err();
    assert!(error.to_string().contains("<default>"), "{error}");

    for (class, temperature) in [("choice", f64::NAN), ("score", -1.0), ("boolean", f64::INFINITY)]
    {
        let mut artifact = common::calibration_artifact(2.0);
        artifact.temperatures.insert(class.to_owned(), temperature);
        let error = TemperatureCalibration::from_artifact(artifact).unwrap_err();
        assert_eq!(error.code(), "calibration.invalid", "{class}");
        assert!(error.to_string().contains(class), "{error}");
    }
}

#[test]
fn a_fitted_artifact_keeps_its_table_its_default_and_its_provenance() {
    // The per-class table wins over the default, the default covers every
    // class the fit never saw, and the validated artifact is readable back
    // field for field — logs and wire payloads cite the real fit.
    let mut artifact = common::calibration_artifact(2.07);
    artifact.temperatures.insert("choice".to_owned(), 1.91);
    artifact.calibration_version = 4;
    let calibration = TemperatureCalibration::from_artifact(artifact).unwrap();

    assert_eq!(calibration.temperature_for("choice"), 1.91, "the class entry wins");
    assert_eq!(calibration.temperature_for("boolean"), 2.07, "the default covers the rest");
    assert_eq!(calibration.temperature_for("score"), 2.07);
    assert_eq!(calibration.temperature_for("a class the fit never saw"), 2.07);
    assert_eq!(calibration.version(), 4, "the cache version comes from the artifact");

    let shown = calibration.artifact();
    assert_eq!(shown.format_version, 1);
    assert_eq!(shown.scheme, "temperature");
    assert_eq!(shown.model_id, "test-model");
    assert_eq!(shown.default_temperature, 2.07);
    assert_eq!(shown.temperatures, BTreeMap::from([("choice".to_owned(), 1.91)]));
    assert_eq!(shown.fit.items, 120);
    assert_eq!(shown.fit.ece_before, 0.5);
    assert_eq!(shown.fit.ece_after, 0.1);
    assert_eq!(shown.fit.source, "test");
}

#[test]
fn the_proof_aware_wrapper_spares_proofs_and_tempers_the_tail() {
    // The bimodal split (CALIBRATION finding 2): a single-entry
    // distribution is an exact proof no temperature may move, the delegated
    // tail is the inner fit unchanged, and the cache version stays the
    // inner one — the ladder id is what re-keys a wrapper change.
    let inner =
        Arc::new(TemperatureCalibration::from_artifact(common::calibration_artifact(2.0)).unwrap());
    let wrapped = ProofAwareCalibration::new(Arc::clone(&inner) as _);

    let proof = Distribution::from_pairs([("proved", 1.0)]).unwrap();
    assert_eq!(wrapped.calibrate("choice", &proof), 1.0);

    let hedged = Distribution::from_pairs([("a", 0.9), ("b", 0.1)]).unwrap();
    assert_eq!(wrapped.calibrate("choice", &hedged), inner.calibrate("choice", &hedged));
    assert!(wrapped.calibrate("choice", &hedged) < 0.9, "T = 2 must flatten the tail");

    assert_eq!(wrapped.version(), inner.version());
}

#[test]
fn an_uncalibrated_rung_reports_raw_confidence_at_version_zero() {
    // D15: un-calibrated runs stay visible as such — the raw top
    // probability under a calibration whose cache version is 0.
    let calibration = IdentityCalibration;
    let hedged = Distribution::from_pairs([("a", 0.78), ("b", 0.22)]).unwrap();
    assert_eq!(calibration.calibrate("choice", &hedged), 0.78);
    assert_eq!(calibration.calibrate("a class with no fit", &hedged), 0.78);
    assert_eq!(calibration.version(), 0, "version 0 means no calibration");

    // The class a fit keys on is the question kind's canonical name.
    assert_eq!(question_class(&choice_question("model", &[("a", "x"), ("b", "y")])), "choice");
    assert_eq!(question_class(&boolean_question("tools", "tools?")), "boolean");
    assert_eq!(question_class(&score_question("difficulty", &["low", "high"])), "score");
}

#[test]
fn a_node_keyed_rung_calibration_gates_only_that_nodes_answers() {
    // The engine consumes `resolve_calibration` per deciding node: the
    // choice node's fit applies to choice questions only, the boolean node
    // keeps the engine calibration, and the trace names the rung that
    // calibrated each answer.
    let classifier = Arc::new(
        MockClassifier::new("mock/mixed")
            .with_script("model", vec![("cloud-large", 0.9), ("local-small", 0.1)])
            .unwrap()
            .with_script("tools", vec![("true", 0.9), ("false", 0.1)])
            .unwrap(),
    );
    let request = request_with_policy(
        State::from_text("Summarize research across many sources and compare findings"),
        vec![
            choice_question("model", &[("cloud-large", "cloud model"), ("local-small", "local")]),
            boolean_question("tools", "Does this request need tools?"),
        ],
        DecisionPolicy::default(),
        RequestMetadata::default(),
    );

    let ladder =
        LadderPolicy::new("node-cal-v1").with_calibration("node:choice", flattening() as _);
    let engine = common::engine_with(config(1).with_ladder(ladder), classifier).unwrap();
    let (response, _) = engine.decide_with_report(&request).unwrap();
    assert_eq!(response.outcome(), DecisionOutcome::Verify, "the flattened choice verifies");

    for answer in response.answers() {
        match answer {
            DecisionAnswer::Choice { question_id, confidence, .. } => {
                assert_eq!(question_id.as_str(), "model");
                assert!(
                    (0.67..=0.68).contains(confidence),
                    "T = 3 flattens the raw 0.9, got {confidence}"
                );
            }
            DecisionAnswer::Boolean { question_id, confidence, .. } => {
                assert_eq!(question_id.as_str(), "tools");
                assert_eq!(*confidence, 0.9, "an unconfigured node keeps the engine calibration");
            }
            other => panic!("unexpected answer shape: {other:?}"),
        }
    }

    assert_eq!(
        trace_fact(&response, "choice", "calibration_source"),
        Some(&FactValue::Text("node:choice".to_owned())),
        "the trace names the rung that calibrated the choice"
    );
    assert_eq!(
        trace_fact(&response, "boolean", "calibration_source"),
        None,
        "an unconfigured node records no calibration source"
    );
    // The calibration-only ladder is non-empty, so its identity decorates
    // the cache key: swapping the rung artifact re-keys every decision.
    assert_eq!(engine.config().identity.model_id, "mock/mixed|ladder-v1@node-cal-v1");
}
