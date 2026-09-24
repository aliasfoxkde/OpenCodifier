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
    Cancelling, ClockAdvancing, choice_request, choice_request_with, config, engine, engine_with,
    engine_with_clock, exclude_tag_rule, five_candidates, node, request_with_limit, rules,
    state_with_fact, trace_int, two_way,
};
use opencodifier_core::{DecisionOutcome, FactValue, NodeId, RequestMetadata, State};
use opencodifier_engine::{
    Action, CacheConfig, Classifier, Condition, DecisionEngine, DecisionGraph, EngineConfig,
    EngineError, EngineIdentity, ManualClock, MockClassifier, NodeKind, Rule, SystemClock,
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
    assert!(safe_report.lexical()[0].pruned().is_empty());
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
    assert!(response.answers().is_empty());
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
    let classifier: Arc<dyn Classifier> = two_way(("local-small", 0.9), ("cloud-large", 0.1));
    let identity =
        |model_id: &str| EngineIdentity { model_id: model_id.to_owned(), ..Default::default() };

    let before =
        engine_with(config(1).with_identity(identity("lexical-v1")), Arc::clone(&classifier))
            .unwrap();
    let (first, first_report) = before.decide_with_report(&request).unwrap();
    assert!(!first_report.cache_hit());

    // Same request, different model id: a different key, so a miss — the
    // cached decision of the old model must not be served for the new one.
    let after = engine_with(config(1).with_identity(identity("lexical-v2")), classifier).unwrap();
    let (_, second_report) = after.decide_with_report(&request).unwrap();
    assert!(!second_report.cache_hit(), "a swapped model must not inherit cached decisions");
    assert_ne!(
        before.config().identity.model_id,
        after.config().identity.model_id,
        "the builder must be applied"
    );
    assert_eq!(first.outcome(), DecisionOutcome::Accept);
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
