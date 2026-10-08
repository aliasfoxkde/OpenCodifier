//! Escalation-ladder wiring (PLANNING.md §24; the fusion study,
//! `benchmarks/decision-model/results/fusion-suite.md`).
//!
//! Per-node / per-kind confidence-gate overrides are consulted at the
//! threshold node instead of the request policy. The default ladder is
//! empty and must be byte-identical to no ladder at all — traces, cache
//! keys, and wire fixtures do not move. Overrides change the gate AND
//! the cache identity: the cache stores completed (gated) responses, so
//! the ladder id decorating the model id is what keeps a changed ladder
//! from serving a decision gated by the old one.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::{boolean_question, choice_question, config, engine_with, request_with_policy};
use opencodifier_core::{
    DecisionAnswer, DecisionOutcome, DecisionPolicy, FactValue, RequestMetadata, RiskLevel, State,
};
use opencodifier_engine::{
    CacheKeyBuilder, EngineConfig, EngineError, IdentityCalibration, LadderPolicy, LadderProfile,
    MockClassifier, NodeKind, TemperatureCalibration,
};

/// A request policy looser than the rung overrides below: accept at
/// 0.80, so a 0.90 scripted answer accepts unless a rung tightens it.
fn loose_request(
    questions: Vec<opencodifier_core::DecisionQuestion>,
) -> opencodifier_core::DecisionRequest {
    request_with_policy(
        State::from_text("Summarize research across many sources and compare findings"),
        questions,
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
}

/// A rung policy accepting only at `min_confidence`, verify band just
/// below it.
fn rung(min_confidence: f64) -> DecisionPolicy {
    DecisionPolicy::new(min_confidence, min_confidence - 0.05, min_confidence / 2.0, RiskLevel::Low)
        .unwrap()
}

/// A boolean-only ladder: one rung policy for every boolean node.
fn boolean_kind_ladder(id: &str, min_confidence: f64) -> LadderPolicy {
    LadderPolicy {
        id: id.to_owned(),
        per_kind: BTreeMap::from([(NodeKind::Boolean, rung(min_confidence))]),
        ..LadderPolicy::default()
    }
}

/// The first `policy_source` fact in the trace, if any rung recorded one.
fn policy_source(response: &opencodifier_core::DecisionResponse) -> Option<String> {
    response.trace().entries().iter().find_map(|entry| match entry.detail.get("policy_source") {
        Some(opencodifier_core::FactValue::Text(source)) => Some(source.clone()),
        _ => None,
    })
}

#[test]
fn an_empty_ladder_is_byte_identical_to_no_ladder() {
    let classifier = Arc::new(
        MockClassifier::new("mock/plain")
            .with_script("tools", vec![("true", 0.9), ("false", 0.1)])
            .unwrap(),
    );
    let request = loose_request(vec![boolean_question("tools", "Does this request need tools?")]);

    let plain = engine_with(config(1), classifier.clone()).unwrap();
    let laddered =
        engine_with(config(1).with_ladder(LadderPolicy::new("none")), classifier).unwrap();

    let (a, _) = plain.decide_with_report(&request).unwrap();
    let (b, _) = laddered.decide_with_report(&request).unwrap();
    assert_eq!(a, b, "an empty ladder must not move any byte of the response");
    assert_eq!(a.outcome(), DecisionOutcome::Accept);
    assert_eq!(policy_source(&a), None);
    // And the cache identity is undecorated.
    assert_eq!(laddered.config().identity.model_id, "mock/plain");
}

#[test]
fn a_per_kind_rung_tightens_that_kinds_gate() {
    let classifier = Arc::new(
        MockClassifier::new("mock/bools")
            .with_script("tools", vec![("true", 0.9), ("false", 0.1)])
            .unwrap(),
    );
    let request = loose_request(vec![boolean_question("tools", "Does this request need tools?")]);

    // Control: the request policy accepts 0.9.
    let control = engine_with(config(1), classifier.clone()).unwrap().decide(&request).unwrap();
    assert_eq!(control.outcome(), DecisionOutcome::Accept);
    assert_eq!(policy_source(&control), None);

    // The boolean rung demands 0.95, so the same distribution verifies —
    // and the trace names the rung whose gate fired.
    let laddered = engine_with(
        config(1).with_ladder(boolean_kind_ladder("bools-strict-v1", 0.95)),
        classifier,
    )
    .unwrap();
    let (response, report) = laddered.decide_with_report(&request).unwrap();
    assert_eq!(response.outcome(), DecisionOutcome::Verify);
    assert_eq!(report.outcomes()[0].1, DecisionOutcome::Verify);
    assert_eq!(policy_source(&response).as_deref(), Some("kind:boolean"));
    let threshold = common::trace_fact(&response, "threshold", "threshold");
    assert_eq!(threshold, Some(&opencodifier_core::FactValue::Float(0.95)));
    // The decorated identity rides the cache key.
    assert_eq!(laddered.config().identity.model_id, "mock/bools|ladder-v1@bools-strict-v1");
}

#[test]
fn a_per_node_rung_beats_the_per_kind_rung() {
    let classifier = Arc::new(
        MockClassifier::new("mock/rungs")
            .with_script("tools", vec![("true", 0.75), ("false", 0.25)])
            .unwrap(),
    );
    let request = loose_request(vec![boolean_question("tools", "Does this request need tools?")]);

    // per_kind[Boolean] would accept 0.75; the per-node entry (0.95) wins,
    // so the run verifies — and records the node source, not the kind.
    let ladder = LadderPolicy {
        id: "nodes-first-v1".to_owned(),
        per_node: BTreeMap::from([("boolean".to_owned(), rung(0.95))]),
        per_kind: BTreeMap::from([(NodeKind::Boolean, rung(0.70))]),
        ..LadderPolicy::default()
    };
    let engine = engine_with(config(1).with_ladder(ladder), classifier).unwrap();
    let (response, _) = engine.decide_with_report(&request).unwrap();
    assert_eq!(response.outcome(), DecisionOutcome::Verify);
    assert_eq!(policy_source(&response).as_deref(), Some("node:boolean"));
}

#[test]
fn an_accept_never_rung_never_accepts_on_probability() {
    // The embedding-rung profile from the fusion study: min_confidence
    // 1.0 means no probability clears the gate, whatever the model says.
    let classifier = Arc::new(
        MockClassifier::new("mock/choice")
            .with_script("model", vec![("local-small", 0.99), ("cloud-large", 0.01)])
            .unwrap(),
    );
    let request = loose_request(vec![choice_question(
        "model",
        &[("local-small", "small local model"), ("cloud-large", "cloud model")],
    )]);
    let ladder = LadderPolicy {
        id: "accept-never-v1".to_owned(),
        per_kind: BTreeMap::from([(NodeKind::Choice, rung(1.0))]),
        ..LadderPolicy::default()
    };
    let engine = engine_with(config(1).with_ladder(ladder), classifier).unwrap();
    let (response, _) = engine.decide_with_report(&request).unwrap();
    assert_eq!(response.outcome(), DecisionOutcome::Verify);
    assert_eq!(policy_source(&response).as_deref(), Some("kind:choice"));
}

#[test]
fn a_non_empty_ladder_without_identity_is_refused_at_assembly() {
    let ladder = LadderPolicy {
        id: "none".to_owned(),
        per_kind: BTreeMap::from([(NodeKind::Boolean, rung(0.95))]),
        ..LadderPolicy::default()
    };
    let classifier = Arc::new(MockClassifier::new("mock/any"));
    let error =
        engine_with(EngineConfig::with_default_pipeline().unwrap().with_ladder(ladder), classifier)
            .unwrap_err();
    assert!(matches!(error, EngineError::InvalidConfig { .. }), "{error:?}");
}

#[test]
fn a_changed_ladder_id_changes_the_cache_key() {
    let request = loose_request(vec![boolean_question("tools", "Does this request need tools?")]);
    let key_for = |id: &str| {
        let identity = EngineConfig::with_default_pipeline()
            .unwrap()
            .with_ladder(boolean_kind_ladder(id, 0.95));
        // The engine decorates model_id at construction; the same
        // decoration is what CacheKeyBuilder folds in.
        let engine = engine_with(identity, Arc::new(MockClassifier::new("mock/keys"))).unwrap();
        CacheKeyBuilder::build(&request, &engine.config().identity).unwrap()
    };
    assert_ne!(key_for("rungs-a-v1"), key_for("rungs-b-v1"));
}

#[test]
fn a_cache_hit_serves_the_same_gated_outcome() {
    // The cache stores completed (gated) responses; the ladder id in the
    // identity is what guarantees a changed ladder re-keys instead of
    // replaying an old gate. Within one engine the hit must therefore
    // carry the rung's outcome, not the request policy's.
    let classifier = Arc::new(
        MockClassifier::new("mock/cached")
            .with_script("tools", vec![("true", 0.9), ("false", 0.1)])
            .unwrap(),
    );
    let request = loose_request(vec![boolean_question("tools", "Does this request need tools?")]);
    let engine =
        engine_with(config(1).with_ladder(boolean_kind_ladder("rungs-v1", 0.95)), classifier)
            .unwrap();
    let (first, first_report) = engine.decide_with_report(&request).unwrap();
    let (second, second_report) = engine.decide_with_report(&request).unwrap();
    assert!(!first_report.cache_hit());
    assert!(second_report.cache_hit());
    // The hit replays the rung's gate (recorded in the cached response),
    // not the request policy's — and names the rung that decided it.
    // (Responses are not byte-equal by design: a hit prepends its own
    // `hit=true` trace entry.)
    assert_eq!(second.answers(), first.answers());
    assert_eq!(
        second.outcome(),
        DecisionOutcome::Verify,
        "the rung gate rides the cached response"
    );
    assert_eq!(policy_source(&second).as_deref(), Some("kind:boolean"));
}

/// Loads a shipped profile document from the repository root: the files
/// under `ladders/` must stay loadable by the engine, forever — they are
/// deployment artifacts, not illustrations.
fn shipped_ladder(file: &str) -> opencodifier_engine::LadderPolicy {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ladders").join(file);
    let document = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let profile: LadderProfile = serde_json::from_str(&document)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    profile.into_ladder().unwrap()
}

#[test]
fn the_shipped_fusion_profile_loads_and_names_its_rungs() {
    let ladder = shipped_ladder("fusion-v1.json");
    assert_eq!(ladder.id, "fusion-v1");
    // Proofs gate themselves; the classifier kinds accept on margin
    // alone (the gate cascade is demote-only, so min_confidence 0.0 +
    // the measured floor is the accept-on-margin shape).
    let rule = &ladder.per_kind[&NodeKind::Rule];
    assert_eq!(rule.min_confidence(), 1.0);
    for kind in [NodeKind::Choice, NodeKind::Boolean, NodeKind::Score] {
        let rung = &ladder.per_kind[&kind];
        assert_eq!(rung.min_confidence(), 0.0, "{kind:?}");
        assert_eq!(rung.min_margin(), 0.0183, "{kind:?}");
    }
}

#[test]
fn the_shipped_proofs_only_profile_accepts_only_proofs() {
    let ladder = shipped_ladder("proofs-only-v1.json");
    for kind in [NodeKind::Rule, NodeKind::Choice, NodeKind::Boolean, NodeKind::Score] {
        assert_eq!(ladder.per_kind[&kind].min_confidence(), 1.0, "{kind:?}");
    }
}

/// A flattening fit (T = 3): the raw 0.9 boolean top lands near 0.675.
fn flattening() -> Arc<TemperatureCalibration> {
    Arc::new(TemperatureCalibration::from_artifact(common::calibration_artifact(3.0)).unwrap())
}

#[test]
fn a_rung_calibration_replaces_the_engine_calibration() {
    // The engine-level fit flattens, so the raw 0.9 verdict calibrates to
    // ~0.675 and the 0.80 gate verifies. A `kind:boolean` rung carrying
    // the identity calibration restores raw confidence for that rung
    // only — replacement, not composition: if the engine fit still
    // applied underneath, 0.675 would verify regardless of the rung.
    let classifier = Arc::new(
        MockClassifier::new("mock/bools")
            .with_script("tools", vec![("true", 0.9), ("false", 0.1)])
            .unwrap(),
    );
    let request = loose_request(vec![boolean_question("tools", "Does this request need tools?")]);
    let flattened =
        engine_with(config(1).with_calibration(flattening()), classifier.clone()).unwrap();
    let (control, _) = flattened.decide_with_report(&request).unwrap();
    assert_eq!(control.outcome(), DecisionOutcome::Verify);
    assert_eq!(common::trace_fact(&control, "boolean", "calibration_source"), None);

    let laddered = engine_with(
        config(1).with_calibration(flattening()).with_ladder(
            LadderPolicy::new("rung-cal-v1")
                .with_calibration("kind:boolean", Arc::new(IdentityCalibration) as _),
        ),
        classifier,
    )
    .unwrap();
    let (response, _) = laddered.decide_with_report(&request).unwrap();
    assert_eq!(response.outcome(), DecisionOutcome::Accept);
    let calibrated = common::trace_fact(&response, "boolean", "calibrated");
    assert_eq!(calibrated, Some(&opencodifier_core::FactValue::Float(0.9)));
    // A calibration-only ladder is non-empty: its identity decorates the
    // model id, so swapping the rung artifact re-keys every cached
    // decision exactly like a model change.
    assert_eq!(laddered.config().identity.model_id, "mock/bools|ladder-v1@rung-cal-v1");
}

#[test]
fn the_rung_calibration_source_is_recorded_in_the_trace() {
    // Inverse of the test above: the rung carries the flattening fit and
    // the engine stays identity. The gate must read the rung's 0.675 and
    // the trace must name the rung that calibrated the answer.
    let classifier = Arc::new(
        MockClassifier::new("mock/bools")
            .with_script("tools", vec![("true", 0.9), ("false", 0.1)])
            .unwrap(),
    );
    let request = loose_request(vec![boolean_question("tools", "Does this request need tools?")]);
    let laddered = engine_with(
        config(1).with_ladder(
            LadderPolicy::new("rung-flat-v1").with_calibration("kind:boolean", flattening() as _),
        ),
        classifier,
    )
    .unwrap();
    let (response, _) = laddered.decide_with_report(&request).unwrap();
    assert_eq!(response.outcome(), DecisionOutcome::Verify);
    let Some(&opencodifier_core::FactValue::Float(calibrated)) =
        common::trace_fact(&response, "boolean", "calibrated")
    else {
        panic!("the boolean node must record its calibrated confidence");
    };
    assert!(
        (0.67..=0.68).contains(&calibrated),
        "T = 3 flattens a raw 0.9 to ~0.675, got {calibrated}"
    );
    assert_eq!(
        common::trace_fact(&response, "boolean", "calibration_source"),
        Some(&opencodifier_core::FactValue::Text("kind:boolean".to_owned()))
    );
}

#[test]
fn the_handle_assembles_with_a_ladder_and_propagates_refusals() {
    // `EngineHandle::with_ladder` is the interface-crate constructor —
    // CLI, HTTP, and MCP all assemble through it, so a real ladder must
    // decide here and an invalid one must be refused at assembly with
    // the ladder's own error, not at first decide.
    use opencodifier_engine::EngineHandle;

    let classifier = Arc::new(
        MockClassifier::new("mock/handle")
            .with_script("tools", vec![("true", 0.9), ("false", 0.1)])
            .unwrap(),
    );
    let request = loose_request(vec![boolean_question("tools", "Does this request need tools?")]);

    let engine = EngineHandle::with_ladder(
        config(1),
        classifier.clone(),
        None,
        boolean_kind_ladder("handle-strict-v1", 0.95),
    )
    .unwrap();
    let response = engine.decide(&request).unwrap();
    assert_eq!(response.outcome(), DecisionOutcome::Verify);
    assert_eq!(policy_source(&response).as_deref(), Some("kind:boolean"));
    assert_eq!(engine.identity().model_id, "mock/handle|ladder-v1@handle-strict-v1");

    // A non-empty ladder without a distinct id is refused here too —
    // the same `validate()` the engine-level constructor enforces.
    let error =
        EngineHandle::with_ladder(config(1), classifier, None, boolean_kind_ladder("none", 0.95))
            .unwrap_err();
    assert!(matches!(error, EngineError::InvalidConfig { .. }), "{error:?}");
}

/// A boolean rung carrying a fitted verdict boundary (RESEARCH §15.6
/// item 2), with gates that accept at `min_confidence` — the gate must
/// read the verdict actually given, not the argmax it overruled.
fn boolean_threshold_ladder(id: &str, threshold: f64, min_confidence: f64) -> LadderPolicy {
    let policy = DecisionPolicy::new(
        min_confidence,
        min_confidence / 2.0,
        min_confidence / 4.0,
        RiskLevel::Low,
    )
    .unwrap()
    .with_boolean_threshold(threshold)
    .unwrap();
    LadderPolicy {
        id: id.to_owned(),
        per_kind: BTreeMap::from([(NodeKind::Boolean, policy)]),
        ..LadderPolicy::default()
    }
}

/// The Boolean answer of a single-question response: `(value,
/// probability, confidence)`.
fn boolean_answer(response: &opencodifier_core::DecisionResponse) -> (bool, f64, f64) {
    match &response.answers()[0] {
        DecisionAnswer::Boolean { value, probability, confidence, .. } => {
            (*value, *probability, *confidence)
        }
        other => panic!("expected a boolean answer, got {other:?}"),
    }
}

#[test]
fn a_fitted_boolean_threshold_overrules_the_argmax_with_honest_mass() {
    // The research shape (UNFAIR-ToS): p_true 0.6 — the argmax says true,
    // the fitted boundary 0.86 says false. The verdict must follow the
    // boundary, and probability plus calibrated confidence must be the
    // verdict's own mass (0.4), never the overruled argmax's 0.6.
    let classifier = Arc::new(
        MockClassifier::new("mock/bools")
            .with_script("tools", vec![("true", 0.6), ("false", 0.4)])
            .unwrap(),
    );
    let request = loose_request(vec![boolean_question("tools", "Does this request need tools?")]);

    // Gates at 0.50: the argmax's 0.6 would accept outright, the
    // verdict's 0.4 must not — proving the gate read the answer given.
    let engine = engine_with(
        config(1).with_ladder(boolean_threshold_ladder("bools-fit-v1", 0.86, 0.50)),
        classifier,
    )
    .unwrap();
    let (response, _) = engine.decide_with_report(&request).unwrap();
    let (value, probability, confidence) = boolean_answer(&response);
    assert!(!value, "the boundary 0.86 answers false at p_true 0.6");
    assert!((probability - 0.4).abs() < 1e-9, "probability {probability}");
    assert!((confidence - 0.4).abs() < 1e-9, "confidence {confidence}");
    assert_eq!(response.outcome(), DecisionOutcome::Verify);
    // No hidden thresholds: the boundary is named, and the overruled
    // argmax is disclosed, in the deciding node's trace.
    let fact = common::trace_fact(&response, "boolean", "boolean_threshold");
    assert_eq!(fact, Some(&FactValue::Float(0.86)));
    let flipped = common::trace_fact(&response, "boolean", "boolean_flipped");
    assert_eq!(flipped, Some(&FactValue::Boolean(true)));
}

#[test]
fn a_threshold_flip_keeps_the_measured_distribution_in_the_answer() {
    // The model said true 0.6 / false 0.4; the boundary answers false.
    // The answer's distribution stays exactly what the model measured —
    // flipping a verdict never rewrites the evidence it came from.
    let classifier = Arc::new(
        MockClassifier::new("mock/bools")
            .with_script("tools", vec![("true", 0.6), ("false", 0.4)])
            .unwrap(),
    );
    let request = loose_request(vec![boolean_question("tools", "Does this request need tools?")]);
    let engine = engine_with(
        config(1).with_ladder(boolean_threshold_ladder("bools-fit-v1", 0.86, 0.50)),
        classifier,
    )
    .unwrap();
    let (response, _) = engine.decide_with_report(&request).unwrap();
    // The deciding node's facts still describe the model's measured
    // distribution: top stays the argmax (true @ 0.6) even though the
    // answer above is false @ 0.4.
    let top = common::trace_fact(&response, "boolean", "top");
    let probability = common::trace_fact(&response, "boolean", "probability");
    assert_eq!(top, Some(&FactValue::Text(String::from("true"))));
    assert_eq!(probability, Some(&FactValue::Float(0.6)));
}

#[test]
fn a_threshold_that_agrees_with_the_argmax_changes_no_decision() {
    // p_true 0.42, boundary 0.70: the verdict is false either way. The
    // boundary that agrees still discloses itself, but never flips
    // anything — answers, outcome, and confidence match the
    // same-gates ladder without the field.
    let classifier = Arc::new(
        MockClassifier::new("mock/bools")
            .with_script("tools", vec![("true", 0.42), ("false", 0.58)])
            .unwrap(),
    );
    let request = loose_request(vec![boolean_question("tools", "Does this request need tools?")]);

    let plain = engine_with(
        config(1).with_ladder(boolean_kind_ladder("bools-plain-v1", 0.70)),
        classifier.clone(),
    )
    .unwrap();
    let bounded = engine_with(
        config(1).with_ladder(boolean_threshold_ladder("bools-bound-v1", 0.70, 0.70)),
        classifier,
    )
    .unwrap();

    let (plain_response, _) = plain.decide_with_report(&request).unwrap();
    let (response, _) = bounded.decide_with_report(&request).unwrap();
    let (value, probability, confidence) = boolean_answer(&response);
    assert!(!value);
    assert!((probability - 0.58).abs() < 1e-9, "probability {probability}");
    assert!((confidence - 0.58).abs() < 1e-9, "confidence {confidence}");
    assert_eq!(response.outcome(), plain_response.outcome());
    let fact = common::trace_fact(&response, "boolean", "boolean_threshold");
    assert_eq!(fact, Some(&FactValue::Float(0.70)));
    let flipped = common::trace_fact(&response, "boolean", "boolean_flipped");
    assert_eq!(flipped, None, "an agreeing boundary must not claim a flip");
}

#[test]
fn without_a_boundary_the_boolean_verdict_stays_the_argmax() {
    // The default (None) posture: the same distribution the boundary
    // above flips to false decides true here, and no threshold fact
    // appears anywhere in the trace.
    let classifier = Arc::new(
        MockClassifier::new("mock/bools")
            .with_script("tools", vec![("true", 0.6), ("false", 0.4)])
            .unwrap(),
    );
    let request = loose_request(vec![boolean_question("tools", "Does this request need tools?")]);

    let engine =
        engine_with(config(1).with_ladder(boolean_kind_ladder("bools-plain-v1", 0.50)), classifier)
            .unwrap();
    let (response, _) = engine.decide_with_report(&request).unwrap();
    let (value, probability, confidence) = boolean_answer(&response);
    assert!(value);
    assert!((probability - 0.6).abs() < 1e-9, "probability {probability}");
    assert!((confidence - 0.6).abs() < 1e-9, "confidence {confidence}");
    for entry in response.trace().entries() {
        assert!(
            !entry.detail.contains_key("boolean_threshold"),
            "an unset boundary must not leak a trace fact"
        );
    }
}
