//! Engine assembly and handle surfaces (PLANNING.md §43, §45, §64; D15,
//! D21, D25, D27).
//!
//! [`EngineConfig`] builders, the canonical graph they assemble, the
//! refusals that fire at assembly, and the [`EngineHandle`] facade the
//! interface crates drive: what each knob sets, which misconfigurations are
//! refused before anything runs, and the exact model-id decoration order
//! that keeps cached decisions keyed to whatever produced them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use common::node;
use opencodifier_core::{DecisionPolicy, DecisionRequest, RiskLevel, State};
use opencodifier_engine::{
    CacheConfig, Calibration, CalibrationArtifact, Classifier, DecisionEngine, EngineConfig,
    EngineError, EngineHandle, EngineIdentity, FocusPolicy, IdentityCalibration, LadderPolicy,
    MAX_BATCH, MockClassifier, NodeKind, Rung, SystemClock, TemperatureCalibration,
};

/// The canonical pipeline graph: the graph `with_default_pipeline` builds.
fn default_pipeline() -> opencodifier_engine::DecisionGraph {
    EngineConfig::with_default_pipeline().unwrap().graph
}

/// A choice request the mock answers uniformly: assembly-level tests only
/// need something the engine can run.
fn probe_request() -> DecisionRequest {
    common::choice_request_with(
        State::from_text("Summarize research across many sources"),
        &[("local-small", "small local model"), ("cloud-large", "cloud model")],
    )
}

/// A non-empty ladder over the choice node: the smallest ladder that
/// passes validation, since a non-empty ladder needs a distinct id.
fn ladder_with_id(id: &str) -> LadderPolicy {
    LadderPolicy {
        id: id.to_owned(),
        per_kind: BTreeMap::from([(
            NodeKind::Choice,
            DecisionPolicy::new(0.9, 0.7, 0.5, RiskLevel::Low).unwrap(),
        )]),
        ..LadderPolicy::default()
    }
}

#[test]
fn every_builder_sets_exactly_its_own_field() {
    let cache = CacheConfig { max_entries: 7, ttl: Duration::from_secs(9) };
    let artifact: CalibrationArtifact = common::calibration_artifact(2.0);
    let calibration = Arc::new(TemperatureCalibration::from_artifact(artifact).unwrap());
    let rung_policy = DecisionPolicy::new(0.9, 0.7, 0.5, RiskLevel::Low).unwrap();

    let built = EngineConfig::with_default_pipeline()
        .unwrap()
        // `model_id` is the one field `with_identity` cannot win: the live
        // classifier owns it, asserted in the decoration test below.
        .with_identity(EngineIdentity { graph_version: 42, ..EngineIdentity::builtin() })
        .with_parallelism(3)
        .with_safe_mode(false)
        .with_lexical_prune_limit(Some(2))
        .with_cache(cache)
        .with_max_execution_time(Duration::from_secs(11))
        .with_focus(Some(FocusPolicy::new(24)))
        .with_ladder(ladder_with_id("builders-v1"))
        .with_calibration(Arc::clone(&calibration) as Arc<dyn opencodifier_engine::Calibration>);

    assert_eq!(built.identity.graph_version, 42, "the identity fields a caller sets are kept");
    assert_eq!(built.identity.calibration_version, 0, "unset identity fields keep defaults");
    assert_eq!(built.calibration.version(), calibration.version(), "the calibration is adopted");
    assert_eq!(built.parallelism, 3);
    assert!(!built.safe_mode);
    assert_eq!(built.lexical_prune_limit, Some(2));
    assert_eq!(built.cache, cache);
    assert_eq!(built.max_execution_time, Duration::from_secs(11));
    assert_eq!(built.focus, Some(FocusPolicy::new(24)));
    assert_eq!(built.ladder.id, "builders-v1");
    assert_eq!(built.ladder.per_kind.len(), 1, "the ladder carries what it was given");
    assert_eq!(built.ladder.per_kind[&NodeKind::Choice], rung_policy);
    assert!(built.embedding.is_none(), "the engine stays zero-ML unless told otherwise");

    // The untouched defaults, pinned so a silent change to any of them is a
    // visible decision-runtime change rather than drift.
    let untouched = EngineConfig::with_default_pipeline().unwrap();
    assert!(untouched.parallelism >= 1, "some thread budget is always configured");
    assert!(untouched.safe_mode, "safe mode is the default posture (PLANNING.md §45)");
    assert_eq!(untouched.lexical_prune_limit, None, "lexical never prunes unless asked");
    assert_eq!(untouched.cache, CacheConfig::default());
    assert_eq!(
        untouched.max_execution_time,
        opencodifier_core::Limits::default().max_execution_time,
        "the engine ceiling is the IR default limit"
    );
    assert_eq!(untouched.focus, None, "no focused extraction by default");
    assert!(untouched.ladder.is_empty(), "an empty ladder is the default");
    assert_eq!(
        untouched.calibration.version(),
        IdentityCalibration.version(),
        "the identity calibration by default"
    );
    assert_eq!(
        untouched.identity,
        EngineIdentity::builtin(),
        "no hand-set identity unless one is supplied"
    );
}

#[test]
fn the_default_pipeline_wires_the_canonical_nodes() {
    let pipeline = default_pipeline();
    assert_eq!(pipeline.version(), 1);
    assert_eq!(pipeline.nodes().len(), 10, "the shipped pipeline is ten nodes");

    let kinds: BTreeMap<&str, NodeKind> =
        pipeline.nodes().iter().map(|spec| (spec.id.as_str(), spec.kind)).collect();
    for (id, kind) in [
        ("normalize", NodeKind::Normalize),
        ("rule", NodeKind::Rule),
        ("cache", NodeKind::Cache),
        ("boolean", NodeKind::Boolean),
        ("score", NodeKind::Score),
        ("filter", NodeKind::Filter),
        ("lexical", NodeKind::Lexical),
        ("choice", NodeKind::Choice),
        ("threshold", NodeKind::Threshold),
        ("output", NodeKind::Output),
    ] {
        assert_eq!(kinds.get(id), Some(&kind), "node {id} must exist with kind {kind}");
    }

    // The wiring, not just the parts: narrowing waits for the rules and the
    // cache, the gate reads every decided question, and the output waits
    // for the gate.
    let dependencies = |id: &str| -> Vec<&str> {
        let id = opencodifier_core::NodeId::new(id).unwrap();
        pipeline
            .node(&id)
            .unwrap()
            .depends_on
            .iter()
            .map(opencodifier_core::NodeId::as_str)
            .collect()
    };
    assert_eq!(dependencies("normalize"), Vec::<&str>::new());
    assert_eq!(dependencies("rule"), ["normalize"]);
    assert_eq!(dependencies("cache"), ["normalize"]);
    assert_eq!(dependencies("boolean"), ["rule"]);
    assert_eq!(dependencies("score"), ["rule"]);
    assert_eq!(dependencies("filter"), ["rule", "cache"], "narrowing waits for rules and cache");
    assert_eq!(dependencies("lexical"), ["filter"]);
    assert_eq!(dependencies("choice"), ["lexical"]);
    assert_eq!(
        dependencies("threshold"),
        ["choice", "boolean", "score"],
        "the gate reads every decided question"
    );
    assert_eq!(dependencies("output"), ["threshold"]);

    // There is exactly one output, and the shipped gate accepts at 0.80.
    assert_eq!(pipeline.output().id.as_str(), "output");
    let threshold =
        pipeline.node(&opencodifier_core::NodeId::new("threshold").unwrap()).unwrap().threshold;
    assert_eq!(threshold, Some(0.80));

    // Nodes are stored sorted by id, so every other accessor is
    // deterministic too.
    let ids: Vec<&str> = pipeline.nodes().iter().map(|spec| spec.id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "nodes are stored sorted by id");
    assert_eq!(pipeline.waves().map(|waves| waves.len()), Some(7), "ten nodes, seven waves");
}

#[test]
fn a_misconfigured_cache_is_refused_at_assembly() {
    let zero_capacity = EngineConfig::with_default_pipeline()
        .unwrap()
        .with_cache(CacheConfig { max_entries: 0, ttl: Duration::from_secs(5) });
    let zero_ttl = EngineConfig::with_default_pipeline()
        .unwrap()
        .with_cache(CacheConfig { max_entries: 4, ttl: Duration::ZERO });

    for (name, configuration) in [("zero capacity", zero_capacity), ("zero ttl", zero_ttl)] {
        // The engine refuses a cache policy it cannot honor rather than
        // deciding into it.
        let engine = DecisionEngine::new(
            configuration.clone(),
            Arc::new(SystemClock),
            Arc::new(MockClassifier::new("mock/cache")),
            None,
        )
        .unwrap_err();
        assert_eq!(engine.code(), "cache.miss_configured", "{name}");
        assert!(
            matches!(engine, EngineError::CacheMisconfigured { .. }),
            "{name}: expected CacheMisconfigured, got {engine:?}"
        );

        // The handle is the interface crates' assembly path, so the same
        // refusal reaches it unchanged.
        let handle =
            EngineHandle::new(configuration, Arc::new(MockClassifier::new("mock/cache")), None)
                .unwrap_err();
        assert_eq!(handle.code(), "cache.miss_configured", "{name}");
        let reason = handle.to_string();
        assert!(
            reason.contains("max_entries") || reason.contains("ttl"),
            "{name}: the reason names the offending field: {reason}"
        );
    }
}

#[test]
fn the_identity_decorations_compose_in_documented_order() {
    let ladder = ladder_with_id("decorators-v1");
    let classifier: Arc<dyn Classifier> = Arc::new(MockClassifier::new("mock/all"));

    // Hand-set identities are not trusted for the model component: the live
    // classifier is the source of truth (PLANNING.md §64).
    let hand_set = EngineConfig::with_default_pipeline()
        .unwrap()
        .with_identity(EngineIdentity {
            model_id: "caller-asserted".to_owned(),
            graph_version: 42,
            ..EngineIdentity::builtin()
        })
        .with_parallelism(1);
    let plain =
        DecisionEngine::new(hand_set.clone(), Arc::new(SystemClock), Arc::clone(&classifier), None)
            .unwrap();
    assert_eq!(plain.config().identity.model_id, "mock/all", "the classifier's id wins");
    assert!(!plain.config().identity.model_id.contains("caller-asserted"));
    assert_eq!(
        plain.config().identity.graph_version,
        42,
        "non-model identity fields are the caller's to set"
    );
    assert_eq!(plain.config().identity.embedding_model, "none", "no backend, no model");

    // Focus, ladder, and rungs decorate the same id, in that order, so a
    // cached decision's key names everything that produced it.
    let decorated = DecisionEngine::new_with_rungs(
        hand_set
            .clone()
            .with_focus(Some(FocusPolicy::new(24)))
            .with_ladder(ladder)
            .with_max_execution_time(Duration::from_secs(30)),
        Arc::new(SystemClock),
        Arc::clone(&classifier),
        None,
        vec![Rung::new(Arc::new(MockClassifier::new("mock/fallback")))],
    )
    .unwrap();
    assert_eq!(
        decorated.config().identity.model_id,
        "mock/all|focused-v1@24|ladder-v1@decorators-v1|rungs-v1@mock/fallback@0",
        "focus, then ladder, then rungs, each separated by `|`"
    );

    // Each decoration alone produces its own documented suffix, so a run
    // with any one of them cannot be served a decision cached without it.
    let with_focus = DecisionEngine::new(
        hand_set.clone().with_focus(Some(FocusPolicy::new(24))),
        Arc::new(SystemClock),
        Arc::clone(&classifier),
        None,
    )
    .unwrap();
    assert_eq!(with_focus.config().identity.model_id, "mock/all|focused-v1@24");
    let with_ladder = DecisionEngine::new(
        hand_set.clone().with_ladder(ladder_with_id("decorators-v1")),
        Arc::new(SystemClock),
        Arc::clone(&classifier),
        None,
    )
    .unwrap();
    assert_eq!(with_ladder.config().identity.model_id, "mock/all|ladder-v1@decorators-v1");
    let with_rungs = DecisionEngine::new_with_rungs(
        hand_set,
        Arc::new(SystemClock),
        Arc::clone(&classifier),
        None,
        vec![Rung::new(Arc::new(MockClassifier::new("mock/fallback")))],
    )
    .unwrap();
    assert_eq!(with_rungs.config().identity.model_id, "mock/all|rungs-v1@mock/fallback@0");
}

#[test]
fn the_handle_surfaces_the_graph_identity_and_health() {
    let handle = EngineHandle::lexical(EngineConfig::with_default_pipeline().unwrap()).unwrap();
    let request = probe_request();

    // The handle decides through the same engine the interfaces share.
    let response = handle.decide(&request).unwrap();
    assert_eq!(response.answers().len(), 1);

    // `graph()` hands back the validated DAG decisions run on, not a
    // summary of it: interfaces introspect this (MCP `codify_graph`).
    assert_eq!(handle.graph().nodes().len(), 10);
    assert_eq!(handle.graph().output().id.as_str(), "output");
    assert_eq!(handle.graph(), &default_pipeline(), "the shipped pipeline, verbatim");

    // Identity and health agree with the configuration underneath.
    let identity = handle.identity();
    assert_eq!(identity.model_id, "relational-v1|builtin-lexical-v2");
    assert_eq!(identity.graph_version, EngineIdentity::builtin().graph_version);
    assert_eq!(identity.embedding_model, "none");
    let health = handle.health();
    assert_eq!(health.identity, identity, "health reports the same identity");
    assert_eq!(health.nodes, 10);
    assert!(health.parallelism >= 1);
    assert!(health.cache_enabled, "the default cache is budgeted");
    assert_eq!(health, handle.health(), "health is a pure snapshot");

    // The batch ceiling is one constant the engine owns, so HTTP and MCP
    // cannot drift on what a batch is.
    assert_eq!(MAX_BATCH, 16);
}

#[test]
fn the_handle_validates_a_graph_without_running_it() {
    // A valid graph round-trips through the one validation path.
    EngineHandle::validate_graph(&default_pipeline()).unwrap();

    // Re-deriving from the spec keeps validation honest: the same nodes
    // validate again, so a deserialized graph is judged by what it is.
    let rebuilt = common::graph(vec![
        node("normalize", NodeKind::Normalize, &[]),
        node("rule", NodeKind::Rule, &["normalize"]),
        node("output", NodeKind::Output, &["rule"]),
    ]);
    EngineHandle::validate_graph(&rebuilt).unwrap();

    // And a graph that cannot exist is refused with the graph error, not an
    // assembly error.
    let cycle = opencodifier_engine::DecisionGraph::new(
        1,
        vec![
            node("a", NodeKind::Rule, &["b"]),
            node("b", NodeKind::Rule, &["a"]),
            node("output", NodeKind::Output, &["a"]),
        ],
    );
    assert_eq!(cycle.unwrap_err().code(), "graph.cycle");
}
