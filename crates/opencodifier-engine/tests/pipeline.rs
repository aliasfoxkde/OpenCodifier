//! End-to-end engine scenarios (PLANNING.md §43–§45).
//!
//! Each test drives the real pipeline through the public API and asserts
//! on the observable contract: outcomes, trace facts, metrics, and error
//! codes — never on internals.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{
    Cancelling, ClockAdvancing, choice_question, choice_request, choice_request_with, config,
    engine, engine_with, engine_with_clock, exclude_tag_rule, five_candidates, node,
    request_with_limit, request_with_policy, rules, state_with_fact, trace_fact, trace_int,
    two_way,
};
use opencodifier_core::{
    DecisionAnswer, DecisionOutcome, DecisionPolicy, DecisionQuestion, Distribution, FactValue,
    NodeId, OodMode, RequestMetadata, State,
};
use opencodifier_engine::{
    Action, CacheConfig, Classifier, Condition, DecisionEngine, DecisionGraph, EngineConfig,
    EngineError, EngineIdentity, LexicalClassifier, ManualClock, MockClassifier, NodeKind, Rule,
    SystemClock,
};

#[test]
fn independent_nodes_in_a_wave_run_in_parallel() {
    let engine = engine(Arc::new(MockClassifier::new("mock/test")), 4).unwrap();
    let (response, report) =
        engine.decide_with_report(&choice_request(&five_candidates())).unwrap();

    assert_eq!(response.answers().len(), 1);
    // The built-in pipeline has two waves with more than one runnable
    // node: {rule, cache} and {boolean, score, filter}.
    assert!(report.parallel_waves() >= 2, "waves: {:?}", report.waves());
    // `thread::scope` spawns one thread per node, so a parallel wave is
    // observable as more than one thread having executed a node.
    assert!(report.threads().len() >= 2, "threads: {:?}", report.threads());
}

#[test]
fn sequential_mode_never_spawns_a_thread() {
    let engine = engine(Arc::new(MockClassifier::new("mock/test")), 1).unwrap();
    let (_, report) = engine.decide_with_report(&choice_request(&five_candidates())).unwrap();
    assert_eq!(report.parallel_waves(), 0);
    assert_eq!(report.threads().len(), 1, "threads: {:?}", report.threads());
}

#[test]
fn dependencies_execute_before_dependents() {
    let graph = DecisionGraph::new(
        1,
        vec![
            node("normalize", NodeKind::Normalize, &[]),
            node("rule", NodeKind::Rule, &["normalize"]),
            node("filter", NodeKind::Filter, &["rule"]),
            node("lexical", NodeKind::Lexical, &["filter"]),
            node("choice", NodeKind::Choice, &["lexical"]),
            node("threshold", NodeKind::Threshold, &["choice"]).with_threshold(0.8),
            node("output", NodeKind::Output, &["threshold"]),
        ],
    )
    .unwrap();

    let engine = engine_with(EngineConfig::new(graph), Arc::new(MockClassifier::new("m"))).unwrap();
    let (response, report) =
        engine.decide_with_report(&choice_request(&five_candidates())).unwrap();
    assert_eq!(response.answers().len(), 1);

    // The chain forces one node per wave, in declaration order.
    let waves: Vec<Vec<String>> =
        report.waves().iter().map(|wave| wave.iter().map(ToString::to_string).collect()).collect();
    assert_eq!(
        waves,
        vec![
            vec!["normalize".to_owned()],
            vec!["rule".to_owned()],
            vec!["filter".to_owned()],
            vec!["lexical".to_owned()],
            vec!["choice".to_owned()],
            vec!["threshold".to_owned()],
            vec!["output".to_owned()],
        ]
    );

    // ...and the general property holds: no node runs before its inputs.
    let position = |id: &NodeId| {
        report.waves().iter().position(|wave| wave.contains(id)).unwrap_or(usize::MAX)
    };
    for wave in report.waves() {
        for id in wave {
            for dependency in &engine.config().graph.node(id).unwrap().depends_on {
                assert!(position(dependency) < position(id), "{dependency} must precede {id}");
            }
        }
    }
}

#[test]
fn an_expired_deadline_reports_a_timeout() {
    let clock = Arc::new(ManualClock::new());
    let classifier = Arc::new(ClockAdvancing {
        clock: Arc::clone(&clock),
        step: Duration::from_secs(5),
        delegate: MockClassifier::new("mock/test"),
    });
    let engine = engine_with_clock(config(1), clock, classifier).unwrap();
    let request = request_with_limit(&five_candidates(), Duration::from_secs(1));

    let error = engine.decide(&request).unwrap_err();
    assert_eq!(error.code(), "engine.timeout");
    match error {
        EngineError::Timeout { limit_ms, .. } => assert_eq!(limit_ms, 1_000),
        other => panic!("expected a timeout, got {other:?}"),
    }
}

#[test]
fn a_cancelled_token_reports_cancellation() {
    let cancelling = Arc::new(Cancelling::new());
    let slot = Arc::clone(&cancelling.token);
    let cancelling_engine =
        DecisionEngine::new(config(1), Arc::new(SystemClock), cancelling, None).unwrap();
    slot.set(cancelling_engine.cancellation_token()).expect("token set once");
    let request = choice_request(&five_candidates());

    let error = cancelling_engine.decide(&request).unwrap_err();
    assert_eq!(error.code(), "engine.cancelled");
    assert_eq!(error, EngineError::Cancelled);
}

#[test]
fn a_second_identical_decide_is_a_cache_hit() {
    let engine = engine(two_way(("local-small", 0.9), ("cloud-large", 0.1)), 2).unwrap();
    let request = choice_request(&[("local-small", "small local model"), ("cloud-large", "cloud")]);

    let (first, first_report) = engine.decide_with_report(&request).unwrap();
    assert!(!first_report.cache_hit());
    assert_eq!(engine.cache().len().unwrap(), 1);

    let (second, second_report) = engine.decide_with_report(&request).unwrap();
    assert!(second_report.cache_hit());
    assert!(second.metrics().cache_hit);
    assert_eq!(second.outcome(), first.outcome());
    assert_eq!(second.answers(), first.answers());
    // The hit is visible in the trace, ahead of the original execution
    // facts.
    assert_eq!(second.trace().entries()[0].node, "cache");
    assert_eq!(second.trace().entries()[0].detail.get("hit"), Some(&FactValue::Boolean(true)));

    // A reordered request hits the same entry: candidate order is not key
    // material.
    let reordered =
        choice_request(&[("cloud-large", "cloud"), ("local-small", "small local model")]);
    let (_, reordered_report) = engine.decide_with_report(&reordered).unwrap();
    assert!(reordered_report.cache_hit(), "candidate order must not change the key");

    // A different request is a miss: the key covers the whole request.
    let other = choice_request_with(
        State::from_text("translate this document instead"),
        &[("local-small", "small local model"), ("cloud-large", "cloud")],
    );
    let (_, other_report) = engine.decide_with_report(&other).unwrap();
    assert!(!other_report.cache_hit());
}

#[test]
fn rule_narrowing_counts_are_recorded() {
    let config = config(2).with_rules(rules(vec![exclude_tag_rule(
        "no cloud",
        "privacy",
        "local_only",
        "cloud",
    )]));
    let engine = engine_with(config, Arc::new(MockClassifier::new("mock/test"))).unwrap();
    let state = state_with_fact("privacy", "local_only");
    let request = choice_request_with(state, &five_candidates());

    let (response, report) = engine.decide_with_report(&request).unwrap();
    let narrowing = &report.narrowing()[0];
    assert_eq!(narrowing.before(), 5);
    assert_eq!(narrowing.after(), 3);
    assert_eq!(narrowing.removed().len(), 2);
    assert_eq!(narrowing.removed()[0].as_str(), "cloud-large");
    assert_eq!(narrowing.removed()[1].as_str(), "cloud-fast");
    assert!(!narrowing.is_starved());

    assert_eq!(response.metrics().candidates_in, 5);
    assert_eq!(response.metrics().candidates_out, 3);
    assert_eq!(trace_int(&response, "filter", "before"), Some(&FactValue::Integer(5)));
    assert_eq!(trace_int(&response, "filter", "after"), Some(&FactValue::Integer(3)));
    assert_eq!(trace_int(&response, "filter", "removed"), Some(&FactValue::Integer(2)));
    // The rule itself is traceable.
    assert_eq!(trace_int(&response, "rule", "excluded"), Some(&FactValue::Integer(2)));
}

#[test]
fn safe_mode_never_eliminates_on_lexical_score_alone() {
    let request = choice_request(&five_candidates());

    // Safe mode on, pruning requested anyway: scores may order, never
    // eliminate.
    let safe = config(2).with_safe_mode(true).with_lexical_prune_limit(Some(1));
    let safe_engine = engine_with(safe, Arc::new(MockClassifier::new("mock/test"))).unwrap();
    let (safe_response, safe_report) = safe_engine.decide_with_report(&request).unwrap();
    assert_eq!(safe_report.narrowing()[0].after(), 5, "safe mode must keep every candidate");
    assert_eq!(safe_report.lexical()[0].pruned(), []);
    assert_eq!(safe_response.metrics().candidates_out, 5);
    assert_eq!(trace_int(&safe_response, "lexical", "safe_mode"), Some(&FactValue::Boolean(true)));

    // The identical configuration with safe mode off: pruning happens, so
    // the gate is real rather than decorative.
    let unsafe_config = config(2).with_safe_mode(false).with_lexical_prune_limit(Some(1));
    let unsafe_engine =
        engine_with(unsafe_config, Arc::new(MockClassifier::new("mock/test"))).unwrap();
    let (unsafe_response, unsafe_report) = unsafe_engine.decide_with_report(&request).unwrap();
    assert_eq!(unsafe_report.narrowing()[0].after(), 5, "narrowing is rule-driven only");
    assert_eq!(unsafe_report.lexical()[0].pruned().len(), 4);
    assert_eq!(unsafe_response.metrics().candidates_out, 1);
}

#[test]
fn a_verifier_that_agrees_upgrades_the_outcome_to_verified() {
    let primary = two_way(("local-small", 0.7), ("cloud-large", 0.3));
    let verifier = two_way(("local-small", 0.7), ("cloud-large", 0.3));
    let engine = DecisionEngine::new(
        config(1),
        Arc::new(SystemClock),
        primary,
        Some(verifier as Arc<dyn Classifier>),
    )
    .unwrap();
    let request = choice_request(&[("local-small", "small local model"), ("cloud-large", "cloud")]);

    let (response, report) = engine.decide_with_report(&request).unwrap();
    // 0.7 confidence is below min_confidence, so verification runs.
    assert_eq!(report.outcomes()[0].1, DecisionOutcome::Verified);
    assert_eq!(response.outcome(), DecisionOutcome::Verified);
    assert_eq!(response.confidence().verifier_agreement, Some(true));
    assert!(response.metrics().verification_triggered);
    assert_eq!(
        trace_int(&response, "threshold", "verifier"),
        Some(&FactValue::Text("agree".into()))
    );
}

#[test]
fn a_verifier_that_disagrees_abstains() {
    let primary = two_way(("local-small", 0.7), ("cloud-large", 0.3));
    let verifier = two_way(("cloud-large", 0.7), ("local-small", 0.3));
    let engine = DecisionEngine::new(
        config(1),
        Arc::new(SystemClock),
        primary,
        Some(verifier as Arc<dyn Classifier>),
    )
    .unwrap();
    let request = choice_request(&[("local-small", "small local model"), ("cloud-large", "cloud")]);

    let (response, report) = engine.decide_with_report(&request).unwrap();
    assert_eq!(report.outcomes()[0].1, DecisionOutcome::Abstain);
    assert_eq!(response.outcome(), DecisionOutcome::Abstain);
    assert_eq!(response.confidence().verifier_agreement, Some(false));
    assert!(response.metrics().verification_triggered);
    assert_eq!(
        trace_int(&response, "threshold", "verifier"),
        Some(&FactValue::Text("disagree".into()))
    );
}

#[test]
fn no_verifier_keeps_the_answer_marked_unverified() {
    let engine = engine(two_way(("local-small", 0.7), ("cloud-large", 0.3)), 1).unwrap();
    let request = choice_request(&[("local-small", "small local model"), ("cloud-large", "cloud")]);

    let (response, _) = engine.decide_with_report(&request).unwrap();
    assert_eq!(response.outcome(), DecisionOutcome::Verify);
    assert_eq!(response.confidence().verifier_agreement, None);
    assert!(!response.metrics().verification_triggered);
}

#[test]
fn low_confidence_abstains() {
    let low = Arc::new(
        MockClassifier::new("mock/low")
            .with_script(
                "model",
                vec![("local-small", 0.4), ("cloud-large", 0.35), ("local-tiny", 0.25)],
            )
            .unwrap(),
    );
    let engine = engine(low, 1).unwrap();
    let request = choice_request(&[
        ("local-small", "small local model"),
        ("cloud-large", "cloud model"),
        ("local-tiny", "tiny local model"),
    ]);

    let (response, report) = engine.decide_with_report(&request).unwrap();
    assert_eq!(response.outcome(), DecisionOutcome::Abstain);
    assert_eq!(report.outcomes()[0].1, DecisionOutcome::Abstain);
    assert!(response.confidence().calibrated_confidence < 0.5);
    assert!(!response.outcome().is_decisive(), "abstention is not decisive");
}

#[test]
fn a_starved_question_reports_no_valid_candidate() {
    let config = config(1).with_rules(rules(vec![Rule {
        name: Some("exclude everything".into()),
        when: Condition::FactExists { fact: "mode".into() },
        then: vec![
            Action::ExcludeCandidate { id: Some("local-small".into()), tag: None },
            Action::ExcludeCandidate { id: Some("cloud-large".into()), tag: None },
        ],
    }]));
    let engine = engine_with(config, Arc::new(MockClassifier::new("mock/test"))).unwrap();
    let request = common::choice_request_with(
        state_with_fact("mode", "strict"),
        &[("local-small", "small local model"), ("cloud-large", "cloud model")],
    );

    let (response, report) = engine.decide_with_report(&request).unwrap();
    assert_eq!(response.outcome(), DecisionOutcome::NoValidCandidate);
    assert!(report.narrowing()[0].is_starved());
    assert_eq!(report.narrowing()[0].after(), 0);
    assert_eq!(response.answers(), []);
}

#[test]
fn every_node_leaves_a_trace_entry() {
    let engine = engine(Arc::new(MockClassifier::new("mock/test")), 2).unwrap();
    let (response, _) = engine.decide_with_report(&choice_request(&five_candidates())).unwrap();

    let traced: std::collections::BTreeSet<&str> =
        response.trace().entries().iter().map(|entry| entry.node.as_str()).collect();
    for expected in
        ["normalize", "rule", "cache", "filter", "lexical", "choice", "threshold", "output"]
    {
        assert!(traced.contains(expected), "{expected} did not leave a trace entry");
    }
}

#[test]
fn graph_validation_rejects_malformed_graphs() {
    let ok = || {
        DecisionGraph::new(
            1,
            vec![
                node("normalize", NodeKind::Normalize, &[]),
                node("output", NodeKind::Output, &["normalize"]),
            ],
        )
        .unwrap()
    };

    // Cycle.
    let cyclic = DecisionGraph::new(
        1,
        vec![
            node("a", NodeKind::Normalize, &["b"]),
            node("b", NodeKind::Rule, &["a"]),
            node("output", NodeKind::Output, &["b"]),
        ],
    )
    .unwrap_err();
    assert_eq!(cyclic.code(), "graph.cycle");

    // Duplicate node id.
    let duplicated = DecisionGraph::new(
        1,
        vec![
            node("normalize", NodeKind::Normalize, &[]),
            node("normalize", NodeKind::Rule, &["normalize"]),
            node("output", NodeKind::Output, &["normalize"]),
        ],
    )
    .unwrap_err();
    assert_eq!(duplicated.code(), "graph.duplicate_node");

    // Unknown dependency.
    let unknown = DecisionGraph::new(
        1,
        vec![
            node("normalize", NodeKind::Normalize, &[]),
            node("rule", NodeKind::Rule, &["ghost"]),
            node("output", NodeKind::Output, &["rule"]),
        ],
    )
    .unwrap_err();
    assert_eq!(unknown.code(), "graph.unknown_dependency");

    // Two output nodes.
    let outputs = DecisionGraph::new(
        1,
        vec![
            node("normalize", NodeKind::Normalize, &[]),
            node("output", NodeKind::Output, &["normalize"]),
            node("output-2", NodeKind::Output, &["normalize"]),
        ],
    )
    .unwrap_err();
    assert_eq!(outputs.code(), "graph.multiple_outputs");

    // No output node at all.
    let missing =
        DecisionGraph::new(1, vec![node("normalize", NodeKind::Normalize, &[])]).unwrap_err();
    assert_eq!(missing.code(), "graph.missing_output");

    // A second entry node: a decision graph has exactly one.
    let roots = DecisionGraph::new(
        1,
        vec![
            node("normalize", NodeKind::Normalize, &[]),
            node("cache", NodeKind::Cache, &[]),
            node("output", NodeKind::Output, &["normalize"]),
        ],
    )
    .unwrap_err();
    assert_eq!(roots.code(), "graph.multiple_roots");

    // A threshold node that declares no threshold.
    let threshold = DecisionGraph::new(
        1,
        vec![
            node("normalize", NodeKind::Normalize, &[]),
            node("threshold", NodeKind::Threshold, &["normalize"]),
            node("output", NodeKind::Output, &["threshold"]),
        ],
    )
    .unwrap_err();
    assert_eq!(threshold.code(), "graph.invalid_node");

    // A well-formed graph still validates.
    assert_eq!(ok().nodes().len(), 2);
}

#[test]
fn a_serialized_graph_round_trips_and_is_revalidated() {
    let graph = opencodifier_engine::DecisionGraph::default_pipeline().unwrap();
    let json = serde_json::to_string(&graph).unwrap();
    let reparsed: DecisionGraph = serde_json::from_str(&json).unwrap();
    assert_eq!(reparsed, graph);

    // Tampering with the wire format is caught by validation, not by
    // deserialization.
    let tampered = |node: &str, dependency: &str| {
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let target = value["nodes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["id"] == serde_json::json!(node))
            .unwrap();
        target["depends_on"] = serde_json::json!([dependency]);
        serde_json::from_value::<DecisionGraph>(value).map_err(|error| error.to_string())
    };
    // The wire format stores nodes sorted by id, so the tamper has to name
    // its target rather than lean on an array position.
    assert!(
        tampered("normalize", "output").unwrap_err().contains("cycle"),
        "the entry node depending on the output is a cycle"
    );
    assert!(
        tampered("normalize", "ghost").unwrap_err().contains("unknown node"),
        "a dependency on nothing is rejected"
    );
}

#[test]
fn a_branch_that_does_not_fire_skips_its_dependents() {
    // branch -> lexical -> choice -> threshold -> output, with the branch
    // conditioned on a fact the request does not carry.
    let graph = DecisionGraph::new(
        1,
        vec![
            node("normalize", NodeKind::Normalize, &[]),
            node("branch", NodeKind::Branch, &["normalize"]).with_condition(
                Condition::FactEquals {
                    fact: "mode".into(),
                    value: FactValue::Text("never".into()),
                },
            ),
            node("lexical", NodeKind::Lexical, &["branch"]),
            node("choice", NodeKind::Choice, &["lexical"]),
            node("threshold", NodeKind::Threshold, &["choice"]).with_threshold(0.8),
            node("output", NodeKind::Output, &["threshold"]),
        ],
    )
    .unwrap();
    let engine = engine_with(
        EngineConfig::new(graph).with_parallelism(1),
        Arc::new(MockClassifier::new("mock/test")),
    )
    .unwrap();

    let metadata = RequestMetadata::default();
    let question = common::choice_question(
        "model",
        &[("local-small", "small local model"), ("cloud-large", "cloud model")],
    );
    let request = common::request(State::from_text("summarize research"), vec![question], metadata);

    let (response, report) = engine.decide_with_report(&request).unwrap();
    assert!(report.skipped().iter().any(|id| id.as_str() == "lexical"), "{:?}", report.skipped());
    assert!(response.answers().is_empty(), "a skipped decision node answers nothing");
    assert_eq!(response.outcome(), DecisionOutcome::Abstain);
}

#[test]
fn a_changed_identity_invalidates_cached_decisions() {
    let request = choice_request(&[("local-small", "small local model"), ("cloud-large", "cloud")]);
    let before_classifier: Arc<dyn Classifier> = Arc::new(
        MockClassifier::new("mock/model-v1")
            .with_script("model", vec![("local-small", 0.9), ("cloud-large", 0.1)])
            .unwrap(),
    );
    let after_classifier: Arc<dyn Classifier> = Arc::new(
        MockClassifier::new("mock/model-v2")
            .with_script("model", vec![("local-small", 0.9), ("cloud-large", 0.1)])
            .unwrap(),
    );

    let before = engine_with(config(1), Arc::clone(&before_classifier)).unwrap();
    let (first, first_report) = before.decide_with_report(&request).unwrap();
    assert!(!first_report.cache_hit());
    // The live classifier, not the hand-set identity, names the key.
    assert_eq!(before.config().identity.model_id, "mock/model-v1");

    // Same request, different classifier id: a different key, so a miss —
    // the cached decision of the old model must not be served for the
    // new one. Swapping the classifier is all it takes; no caller has to
    // remember to bump anything.
    let after = engine_with(config(1), after_classifier).unwrap();
    let (_, second_report) = after.decide_with_report(&request).unwrap();
    assert!(!second_report.cache_hit(), "a swapped model must not inherit cached decisions");
    assert_eq!(after.config().identity.model_id, "mock/model-v2");
    assert_eq!(first.outcome(), DecisionOutcome::Accept);
}

#[test]
fn the_classifiers_id_overrides_a_hand_set_identity_model_id() {
    let request = choice_request(&[("local-small", "small local model"), ("cloud-large", "cloud")]);
    let classifier: Arc<dyn Classifier> = two_way(("local-small", 0.9), ("cloud-large", 0.1));
    // Even a deliberate lie in the identity cannot make a `mock/two-way`
    // decision cache under another model's name (PLANNING.md §64).
    let lying = config(1)
        .with_identity(EngineIdentity { model_id: "lexical-v9".into(), ..Default::default() });
    let engine = engine_with(lying, classifier).unwrap();
    assert_eq!(engine.config().identity.model_id, "mock/two-way");
    let (_, report) = engine.decide_with_report(&request).unwrap();
    assert!(!report.cache_hit());
}

#[test]
fn a_calibration_artifact_changes_confidence_and_invalidates_cached_decisions() {
    let request = choice_request(&[("local-small", "small local model"), ("cloud-large", "cloud")]);
    let classifier: Arc<dyn Classifier> = two_way(("local-small", 0.9), ("cloud-large", 0.1));

    // Identity calibration (the default): calibrated confidence is the
    // raw top probability (D15), and a replay is a cache hit.
    let raw = engine_with(config(1), Arc::clone(&classifier)).unwrap();
    let (first, _) = raw.decide_with_report(&request).unwrap();
    assert_eq!(first.confidence().calibrated_confidence, 0.9);
    let (_, replay) = raw.decide_with_report(&request).unwrap();
    assert!(replay.cache_hit());

    // A fitted temperature artifact: flatter confidence, and a miss —
    // the key now carries the artifact's calibration_version.
    let flat_artifact = r#"{
        "format_version": 1, "scheme": "temperature",
        "model_id": "two-way-test", "calibration_version": 3,
        "default_temperature": 2.0, "temperatures": {},
        "fit": {"items": 120, "ece_before": 0.1, "ece_after": 0.02,
                "source": "test fixture"}
    }"#;
    let flat = engine_with(
        config(1).with_calibration(Arc::new(
            opencodifier_engine::TemperatureCalibration::from_json(flat_artifact).unwrap(),
        )),
        classifier,
    )
    .unwrap();
    let (calibrated, miss) = flat.decide_with_report(&request).unwrap();
    assert!(!miss.cache_hit(), "a loaded artifact must invalidate cached raw decisions");
    assert!(
        calibrated.confidence().calibrated_confidence < 0.9,
        "T=2 must flatten 0.9, got {}",
        calibrated.confidence().calibrated_confidence
    );
    let (_, hit) = flat.decide_with_report(&request).unwrap();
    assert!(hit.cache_hit());

    // A refitted artifact (different version and temperature): another
    // miss — cache keys distinguish every calibration the engine can
    // load (D6), and T=1 restores the raw value.
    let sharp_artifact = flat_artifact.replace('3', "4").replace("2.0", "1.0");
    let sharp = engine_with(
        config(1).with_calibration(Arc::new(
            opencodifier_engine::TemperatureCalibration::from_json(&sharp_artifact).unwrap(),
        )),
        two_way(("local-small", 0.9), ("cloud-large", 0.1)),
    )
    .unwrap();
    let (restored, refresh) = sharp.decide_with_report(&request).unwrap();
    assert!(!refresh.cache_hit(), "a refitted artifact must produce a fresh key");
    assert_eq!(restored.confidence().calibrated_confidence, 0.9);
}

#[test]
fn the_ood_channel_is_live_and_gates_acceptance() {
    // two_way(0.9, 0.1): entropy ≈ 0.325 nats over two surviving answers,
    // so the distributional OOD proxy is ≈ 0.469. With the ceiling at
    // 0.3, the confident answer (0.9 ≥ 0.8 min_confidence) may not be
    // accepted outright — it must route to verification (PLANNING §19).
    let candidates: Vec<(&str, &str)> =
        vec![("local-small", "small local model"), ("cloud-large", "cloud")];
    let classifier: Arc<dyn Classifier> = two_way(("local-small", 0.9), ("cloud-large", 0.1));

    let (open, _) = engine_with(config(1), Arc::clone(&classifier))
        .unwrap()
        .decide_with_report(&choice_request(&candidates))
        .unwrap();
    let ood = open.confidence().ood_score;
    assert!((ood - 0.469).abs() < 0.01, "the OOD proxy must be live, got {ood}");
    assert_eq!(open.outcome(), DecisionOutcome::Accept);
    assert!(
        trace_fact(&open, "choice", "ood").is_some(),
        "the OOD channel must be observable in the trace"
    );

    let policy = DecisionPolicy::default().with_ood_ceiling(0.3).unwrap();
    let request = request_with_policy(
        State::from_text("Summarize research across many sources and compare findings"),
        vec![choice_question("model", &candidates)],
        policy,
        RequestMetadata::default(),
    );
    let (gated, _) =
        engine_with(config(1), classifier).unwrap().decide_with_report(&request).unwrap();
    assert_eq!(gated.outcome(), DecisionOutcome::Verify, "OOD above the ceiling must verify");
}

/// A scripted classifier with caller-chosen lexical coverage: the
/// distribution is held fixed while the input-likeness evidence varies,
/// isolating the lexical-band OOD channel from distribution shape (D28).
#[derive(Debug)]
struct CoveredClassifier {
    pairs: Vec<(&'static str, f64)>,
    coverage: f64,
}

impl Classifier for CoveredClassifier {
    fn decide(
        &self,
        _state: &State,
        _question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError> {
        Distribution::from_pairs(self.pairs.clone()).map_err(EngineError::from)
    }

    fn decide_extended(
        &self,
        state: &State,
        question: &DecisionQuestion,
        _index: Option<&opencodifier_engine::Bm25Index>,
    ) -> Result<(Distribution, Option<f64>), EngineError> {
        Ok((self.decide(state, question)?, Some(self.coverage)))
    }

    fn model_id(&self) -> &'static str {
        "covered/test"
    }
}

#[test]
fn the_lexical_band_mode_swaps_the_ood_signal_not_the_distribution() {
    // Same sharp distribution both times — 0.9 clears the accept gate in
    // distribution terms. Only the coverage evidence differs: fully
    // grounded input → OOD exactly 0.0 → accept; weakly grounded input →
    // OOD 0.8 > the 0.5 ceiling → verify (PLANNING §19, D28).
    let candidates: Vec<(&str, &str)> =
        vec![("local-small", "small local model"), ("cloud-large", "cloud")];
    let policy = DecisionPolicy::default()
        .with_ood_mode(OodMode::LexicalBand)
        .with_ood_ceiling(0.5)
        .unwrap();
    let request = request_with_policy(
        State::from_text("Summarize research across many sources and compare findings"),
        vec![choice_question("model", &candidates)],
        policy,
        RequestMetadata::default(),
    );

    let (grounded, _) = engine_with(
        config(1),
        Arc::new(CoveredClassifier {
            pairs: vec![("local-small", 0.9), ("cloud-large", 0.1)],
            coverage: 1.0,
        }),
    )
    .unwrap()
    .decide_with_report(&request)
    .unwrap();
    assert_eq!(grounded.confidence().ood_score, 0.0, "full coverage is zero OOD");
    assert_eq!(grounded.outcome(), DecisionOutcome::Accept);

    let (ungrounded, _) = engine_with(
        config(1),
        Arc::new(CoveredClassifier {
            pairs: vec![("local-small", 0.9), ("cloud-large", 0.1)],
            coverage: 0.2,
        }),
    )
    .unwrap()
    .decide_with_report(&request)
    .unwrap();
    assert!((ungrounded.confidence().ood_score - 0.8).abs() < 1e-12);
    assert_eq!(ungrounded.outcome(), DecisionOutcome::Verify);
}

#[test]
fn the_ood_mode_is_policy_selected_and_defaults_to_entropy() {
    // Byte-identical default (D28): with the default mode the coverage
    // evidence never reaches the gate — the same 0.9/0.1 distribution
    // reports the distributional proxy (≈ 0.469) whether the rung saw a
    // grounded or ungrounded input.
    let candidates: Vec<(&str, &str)> =
        vec![("local-small", "small local model"), ("cloud-large", "cloud")];
    let request = request_with_policy(
        State::from_text("Summarize research across many sources and compare findings"),
        vec![choice_question("model", &candidates)],
        DecisionPolicy::default().with_ood_ceiling(0.5).unwrap(),
        RequestMetadata::default(),
    );
    for coverage in [1.0, 0.2] {
        let (response, _) = engine_with(
            config(1),
            Arc::new(CoveredClassifier {
                pairs: vec![("local-small", 0.9), ("cloud-large", 0.1)],
                coverage,
            }),
        )
        .unwrap()
        .decide_with_report(&request)
        .unwrap();
        assert!(
            (response.confidence().ood_score - 0.469).abs() < 0.01,
            "coverage {coverage} must not leak into the default OOD signal"
        );
        assert_eq!(response.outcome(), DecisionOutcome::Accept);
    }

    // Fallback (D28): a rung that exposes no lexical evidence decides in
    // lexical-band mode exactly as it would in entropy mode — the
    // MockClassifier returns no support, so the entropy signal stands in.
    let request = request_with_policy(
        State::from_text("Summarize research across many sources and compare findings"),
        vec![choice_question("model", &candidates)],
        DecisionPolicy::default()
            .with_ood_mode(OodMode::LexicalBand)
            .with_ood_ceiling(0.3)
            .unwrap(),
        RequestMetadata::default(),
    );
    let (fallback, _) = engine_with(config(1), two_way(("local-small", 0.9), ("cloud-large", 0.1)))
        .unwrap()
        .decide_with_report(&request)
        .unwrap();
    assert!(
        (fallback.confidence().ood_score - 0.469).abs() < 0.01,
        "no-evidence rungs must fall back to the entropy signal"
    );
    assert_eq!(fallback.outcome(), DecisionOutcome::Verify);
}

/// Wraps a mock and counts how often the executor handed over a prebuilt
/// BM25 index — the observable of the build-once handoff (#80).
#[derive(Debug)]
struct IndexProbe {
    inner: MockClassifier,
    handed: std::sync::atomic::AtomicUsize,
}

impl IndexProbe {
    fn wrapped() -> Arc<Self> {
        Arc::new(Self {
            inner: MockClassifier::new("mock/probe"),
            handed: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    fn handed(&self) -> usize {
        self.handed.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Classifier for IndexProbe {
    fn decide(
        &self,
        state: &State,
        question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError> {
        self.inner.decide(state, question)
    }

    fn decide_extended(
        &self,
        state: &State,
        question: &DecisionQuestion,
        index: Option<&opencodifier_engine::Bm25Index>,
    ) -> Result<(Distribution, Option<f64>), EngineError> {
        if index.is_some() {
            self.handed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        self.inner.decide_extended(state, question, index)
    }

    fn model_id(&self) -> &'static str {
        "probe/test"
    }
}

#[test]
fn the_executor_hands_the_lexical_index_to_the_deciding_classifier_once() {
    // filter -> lexical -> choice: the lexical node builds the index, and
    // the choice node must reuse it instead of rebuilding (#80).
    let candidates: Vec<(&str, &str)> =
        vec![("local-small", "small local model"), ("cloud-large", "cloud model")];
    let graph = DecisionGraph::new(
        1,
        vec![
            node("normalize", NodeKind::Normalize, &[]),
            node("rule", NodeKind::Rule, &["normalize"]),
            node("filter", NodeKind::Filter, &["rule"]),
            node("lexical", NodeKind::Lexical, &["filter"]),
            node("choice", NodeKind::Choice, &["lexical"]),
            node("output", NodeKind::Output, &["choice"]),
        ],
    )
    .unwrap();
    let probe = IndexProbe::wrapped();
    let engine =
        engine_with(EngineConfig::new(graph), Arc::clone(&probe) as Arc<dyn Classifier>).unwrap();
    let request = choice_request(&candidates);
    let (response, _) = engine.decide_with_report(&request).unwrap();
    assert_eq!(response.answers().len(), 1);
    assert_eq!(probe.handed(), 1, "the choice node must receive the lexical node's index");

    // Without a lexical node there is nothing to hand over: the classifier
    // builds its own and the decision is unchanged.
    let graph = DecisionGraph::new(
        1,
        vec![
            node("normalize", NodeKind::Normalize, &[]),
            node("rule", NodeKind::Rule, &["normalize"]),
            node("filter", NodeKind::Filter, &["rule"]),
            node("choice", NodeKind::Choice, &["filter"]),
            node("output", NodeKind::Output, &["choice"]),
        ],
    )
    .unwrap();
    let bare = IndexProbe::wrapped();
    let engine =
        engine_with(EngineConfig::new(graph), Arc::clone(&bare) as Arc<dyn Classifier>).unwrap();
    engine.decide_with_report(&request).unwrap();
    assert_eq!(bare.handed(), 0, "no lexical node means no index to hand over");
}

#[test]
fn the_index_handoff_produces_the_distribution_the_classifier_makes_alone() {
    // The mispairing hazard of a stale index: scores paired with the wrong
    // candidates. With filter -> lexical -> choice, the handed index and a
    // locally built one cover identical documents in identical order, so
    // the engine's distribution must equal the classifier's own over the
    // same narrowed candidate list — to the bit.
    let candidates: Vec<(&str, &str)> =
        vec![("local-small", "summarize local research sources"), ("cloud-large", "cloud")];
    let graph = DecisionGraph::new(
        1,
        vec![
            node("normalize", NodeKind::Normalize, &[]),
            node("rule", NodeKind::Rule, &["normalize"]),
            node("filter", NodeKind::Filter, &["rule"]),
            node("lexical", NodeKind::Lexical, &["filter"]),
            node("choice", NodeKind::Choice, &["lexical"]),
            node("output", NodeKind::Output, &["choice"]),
        ],
    )
    .unwrap();
    let engine = engine_with(EngineConfig::new(graph), Arc::new(LexicalClassifier::new())).unwrap();
    let (response, _) = engine.decide_with_report(&choice_request(&candidates)).unwrap();
    let DecisionAnswer::Choice { distribution, .. } = &response.answers()[0] else {
        panic!("choice answer");
    };

    let state = State::from_text("Summarize research across many sources and compare findings");
    let question = choice_question("model", &candidates);
    let direct = LexicalClassifier::new().decide(&state, &question).unwrap();
    assert_eq!(*distribution, direct, "the handoff must not shift a single bit");
}

#[test]
fn the_cache_policy_is_applied_to_the_engine() {
    let policy = CacheConfig { max_entries: 1, ttl: Duration::from_secs(60) };
    let engine = engine_with(
        config(1).with_cache(policy),
        Arc::new(MockClassifier::new("mock/bounded-cache")),
    )
    .unwrap();
    assert_eq!(*engine.cache().config(), policy, "the cache policy must be applied");

    let alpha = choice_request(&[("local-small", "small local model")]);
    let beta = choice_request(&[("cloud-large", "cloud model")]);
    engine.decide(&alpha).unwrap();
    engine.decide(&beta).unwrap();
    assert_eq!(engine.cache().len().unwrap(), 1, "the LRU bound must hold");

    // The survivor is the most recent decision, and the evicted one is
    // recomputed rather than served stale.
    let (_, hit) = engine.decide_with_report(&beta).unwrap();
    assert!(hit.cache_hit());
    let (_, miss) = engine.decide_with_report(&alpha).unwrap();
    assert!(!miss.cache_hit());
}

#[test]
fn the_engine_time_ceiling_binds_before_the_request_ceiling() {
    let clock = Arc::new(ManualClock::new());
    let classifier = Arc::new(ClockAdvancing::new(Arc::clone(&clock), Duration::from_secs(5)));
    // The request allows ten seconds; the engine allows one. The effective
    // deadline is the smaller of the two.
    let engine = engine_with_clock(
        config(1).with_max_execution_time(Duration::from_secs(1)),
        clock,
        classifier,
    )
    .unwrap();
    let request = request_with_limit(&five_candidates(), Duration::from_secs(10));

    match engine.decide(&request).unwrap_err() {
        EngineError::Timeout { limit_ms, .. } => assert_eq!(limit_ms, 1_000),
        other => panic!("expected a timeout, got {other:?}"),
    }
}
