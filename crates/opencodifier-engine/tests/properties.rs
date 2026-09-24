//! Property tests for the engine's invariants (PLANNING.md §66).
//!
//! Three properties are load-bearing enough to be worth randomizing:
//!
//! 1. `normalized_request` makes cache keys candidate-order independent —
//!    otherwise the "same decision" would be cached twice and, worse, a
//!    reordered request would miss a decision it should hit.
//! 2. Cache keys are stable: identical requests hash identically, and any
//!    identity change (graph version, model, calibration, semver) changes
//!    the key.
//! 3. Graph validation rejects every randomly generated cycle — a cyclic
//!    graph must never reach the executor, whatever shape it has.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

use std::sync::Arc;

use opencodifier_core::{
    Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion, DecisionRequest, RequestMetadata,
    State,
};
use opencodifier_engine::{
    CacheKeyBuilder, DecisionGraph, EngineError, EngineIdentity, NodeKind, NodeSpec,
};
use proptest::prelude::*;

/// A request over `ids`, in the given order.
fn request_over(ids: &[String]) -> DecisionRequest {
    let candidates: Vec<Candidate> = ids
        .iter()
        .map(|id| Candidate::new(id.clone(), format!("description for {id}")).unwrap())
        .collect();
    let question = DecisionQuestion::Choice(
        ChoiceQuestion::new("model", "Which model should answer?", candidates).unwrap(),
    );
    DecisionRequest::new(
        State::from_text("a fixed state text"),
        vec![question],
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap()
}

/// Candidate ids drawn from a fixed pool, in a random order.
fn id_strategy() -> BoxedStrategy<Vec<String>> {
    prop::collection::vec("[a-z][a-z0-9_.-]{0,11}", 1..=6)
        .prop_map(|ids| {
            // Ids must be unique for a valid question; keep the first
            // occurrence of each.
            let mut seen = Vec::new();
            for id in ids {
                if !seen.contains(&id) {
                    seen.push(id);
                }
            }
            seen
        })
        .boxed()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Reordering candidates must not change the cache key.
    #[test]
    fn cache_keys_ignore_candidate_order(ids in id_strategy()) {
        let forward = request_over(&ids);

        let mut reversed = ids.clone();
        reversed.reverse();
        let backward = request_over(&reversed);

        let key = |request: &DecisionRequest| {
            CacheKeyBuilder::build(request, &EngineIdentity::builtin()).unwrap()
        };
        prop_assert_eq!(key(&forward), key(&backward));
    }

    /// The canonical form is a fixed point, and identical requests hash
    /// identically across calls.
    #[test]
    fn cache_keys_are_stable_and_normalized_requests_are_idempotent(ids in id_strategy()) {
        let request = request_over(&ids);
        let once = CacheKeyBuilder::normalized_request(&request);
        let twice = CacheKeyBuilder::normalized_request(&once);
        prop_assert_eq!(once, twice);

        let first = CacheKeyBuilder::build(&request, &EngineIdentity::builtin()).unwrap();
        let second = CacheKeyBuilder::build(&request, &EngineIdentity::builtin()).unwrap();
        prop_assert_eq!(first, second);
        prop_assert_eq!(first.as_hex().len(), 64);
    }

    /// Any identity change invalidates every cached decision.
    #[test]
    fn identity_changes_always_change_the_key(
        graph_version in 0_u64..,
        calibration_version in 0_u64..,
        model_id in "[a-z0-9-]{1,12}",
        engine_semver in "[0-9.]{1,8}",
    ) {
        let request = request_over(&["a".to_owned(), "b".to_owned()]);
        let identity = EngineIdentity { graph_version, model_id, calibration_version, engine_semver };
        let baseline = CacheKeyBuilder::build(&request, &identity).unwrap();

        let bumped_graph = EngineIdentity { graph_version: graph_version.wrapping_add(1), ..identity.clone() };
        let bumped_model = EngineIdentity { model_id: format!("{}-b", identity.model_id), ..identity.clone() };
        let bumped_calibration = EngineIdentity { calibration_version: calibration_version.wrapping_add(1), ..identity.clone() };
        let bumped_semver = EngineIdentity { engine_semver: format!("{}.1", identity.engine_semver), ..identity.clone() };

        for changed in [bumped_graph, bumped_model, bumped_calibration, bumped_semver] {
            prop_assert_ne!(baseline, CacheKeyBuilder::build(&request, &changed).unwrap());
        }
    }

    /// A randomly generated cycle is always rejected, whatever its shape.
    #[test]
    fn graph_validation_rejects_randomly_generated_cycles(
        seed in any::<u64>(),
        size in 2_usize..=8,
    ) {
        // A loop of `size` nodes, closed by making the first node depend on
        // the last. The output node hangs off the loop, so it is part of
        // the cycle too and cannot be rejected for any other reason.
        let prefix = format!("n{seed:08x}");
        let id_of = |index: usize| format!("{prefix}{index}");
        let mut nodes: Vec<NodeSpec> = (0..size)
            .map(|index| {
                let dependency = if index == 0 { size - 1 } else { index - 1 };
                NodeSpec::build(id_of(index), NodeKind::Rule)
                    .unwrap()
                    .with_dependencies(vec![opencodifier_core::NodeId::new(id_of(dependency)).unwrap()])
            })
            .collect();
        nodes.push(
            NodeSpec::build("output", NodeKind::Output)
                .unwrap()
                .with_dependencies(vec![opencodifier_core::NodeId::new(id_of(size - 1)).unwrap()]),
        );

        let error = DecisionGraph::new(1, nodes).unwrap_err();
        prop_assert!(matches!(error, EngineError::Cycle { .. }), "expected a cycle, got {error:?}");
    }
}

/// The identity of a request that reaches the cache is also invariant under
/// question reordering? No — questions are semantically ordered, so only
/// candidates are normalized. This test pins that decision.
#[test]
fn only_candidates_are_normalized() {
    let question = |id: &str| {
        DecisionQuestion::Choice(
            ChoiceQuestion::new(
                id,
                "Which model?",
                vec![
                    Candidate::new("a", "small local model").unwrap(),
                    Candidate::new("b", "large cloud model").unwrap(),
                ],
            )
            .unwrap(),
        )
    };
    let first = DecisionRequest::new(
        State::from_text("state"),
        vec![question("one"), question("two")],
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap();
    let second = DecisionRequest::new(
        State::from_text("state"),
        vec![question("two"), question("one")],
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap();

    let key = |request: &DecisionRequest| {
        CacheKeyBuilder::build(request, &EngineIdentity::builtin()).unwrap()
    };
    assert_ne!(key(&first), key(&second), "question order is semantic, not incidental");
}

/// A thread-safety smoke test: one engine shared across threads, as the
/// base binary does it.
#[test]
fn a_shared_engine_decides_concurrently() {
    let config = opencodifier_engine::EngineConfig::with_default_pipeline().unwrap();
    let engine = Arc::new(
        opencodifier_engine::DecisionEngine::new(
            config,
            Arc::new(opencodifier_engine::SystemClock),
            Arc::new(opencodifier_engine::MockClassifier::new("mock/shared")),
            None,
        )
        .unwrap(),
    );
    let request = Arc::new(
        DecisionRequest::new(
            State::from_text("summarize research"),
            vec![DecisionQuestion::Choice(
                ChoiceQuestion::new(
                    "model",
                    "Which model should answer?",
                    vec![
                        Candidate::new("local-small", "small local model").unwrap(),
                        Candidate::new("cloud-large", "large cloud model").unwrap(),
                    ],
                )
                .unwrap(),
            )],
            DecisionPolicy::default(),
            RequestMetadata::default(),
        )
        .unwrap(),
    );

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let engine = Arc::clone(&engine);
            let request = Arc::clone(&request);
            std::thread::spawn(move || engine.decide(&request).map(|response| response.outcome()))
        })
        .collect();
    let outcomes: Vec<_> =
        handles.into_iter().map(|handle| handle.join().unwrap().unwrap()).collect();
    assert!(
        outcomes.iter().all(|outcome| *outcome == outcomes[0]),
        "concurrent decides must agree: {outcomes:?}"
    );
}
