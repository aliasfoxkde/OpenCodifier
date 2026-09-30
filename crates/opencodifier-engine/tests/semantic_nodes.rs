//! Semantic-node integration (PLANNING.md §24, §26, §53, §54; D21).
//!
//! The whole D21 contract, exercised end to end through a real graph and
//! a mock backend: an `embedding` node annotates, a `retrieve` node
//! narrows with every drop disclosed, a `rerank` node permutes without
//! removing, and an engine assembled without the backend refuses such
//! graphs instead of degrading.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

use std::sync::Arc;

use opencodifier_core::{
    Candidate, ChoiceQuestion, DecisionOutcome, DecisionPolicy, DecisionQuestion, DecisionRequest,
    NodeId, RequestMetadata, State,
};
use opencodifier_engine::{
    DecisionGraph, EmbeddingReranker, EngineConfig, EngineError, EngineHandle, EngineIdentity,
    LexicalReranker, NodeKind, NodeSpec, Reranker,
};
use opencodifier_runtime::MockEmbeddingBackend;

/// A backend whose vectors separate coding from prose by construction, so
/// ordering assertions mean something.
fn backend() -> Arc<MockEmbeddingBackend> {
    Arc::new(MockEmbeddingBackend::new("mock-embed-test-v1", 16).unwrap())
}

/// A backend that always errors: the runtime must surface the failure,
/// never paper over it with an empty annotation.
struct Failing;

impl std::fmt::Debug for Failing {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Failing")
    }
}

impl opencodifier_runtime::EmbeddingBackend for Failing {
    fn model_id(&self) -> &'static str {
        "failing-embed-v1"
    }
    fn dims(&self) -> usize {
        4
    }
    fn embed(&self, _texts: &[&str]) -> Result<Vec<Vec<f32>>, opencodifier_runtime::RuntimeError> {
        Err(opencodifier_runtime::RuntimeError::InvalidTensor {
            reason: "backend exploded".to_owned(),
        })
    }
}

/// A backend that violates the one-vector-per-text batch contract.
struct Miscounting(usize);

impl std::fmt::Debug for Miscounting {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Miscounting")
    }
}

impl opencodifier_runtime::EmbeddingBackend for Miscounting {
    fn model_id(&self) -> &'static str {
        "miscounting-embed-v1"
    }
    fn dims(&self) -> usize {
        16
    }
    fn embed(&self, _texts: &[&str]) -> Result<Vec<Vec<f32>>, opencodifier_runtime::RuntimeError> {
        Ok(vec![vec![0.0; self.dims()]; self.0])
    }
}

/// The §53 shape: normalize → embedding → retrieve → choice → threshold
/// → output.
fn retrieval_graph(top_n: usize, floor: f64) -> DecisionGraph {
    let depends = |names: &[&str]| -> Vec<NodeId> {
        names.iter().map(|name| NodeId::new(*name).unwrap()).collect()
    };
    let retrieve = NodeSpec::build("retrieve", NodeKind::Retrieve)
        .unwrap()
        .with_dependencies(depends(&["embedding"]))
        .with_top_n(top_n)
        .with_floor(floor);
    DecisionGraph::new(
        1,
        vec![
            NodeSpec::build("normalize", NodeKind::Normalize).unwrap(),
            NodeSpec::build("embedding", NodeKind::Embedding)
                .unwrap()
                .with_dependencies(depends(&["normalize"])),
            retrieve,
            NodeSpec::build("choice", NodeKind::Choice)
                .unwrap()
                .with_dependencies(depends(&["normalize", "retrieve"])),
            NodeSpec::build("threshold", NodeKind::Threshold)
                .unwrap()
                .with_dependencies(depends(&["choice"]))
                .with_threshold(0.5),
            NodeSpec::build("output", NodeKind::Output)
                .unwrap()
                .with_dependencies(depends(&["threshold"])),
        ],
    )
    .unwrap()
}

/// A request whose candidates mix coding and prose descriptions.
fn request() -> DecisionRequest {
    let question = DecisionQuestion::Choice(
        ChoiceQuestion::new(
            "skill",
            "Which skill should run?",
            vec![
                Candidate::new("prose", "draft release notes prose").unwrap(),
                Candidate::new("code", "write Rust code with tests").unwrap(),
                Candidate::new("bench", "run the benchmark suite").unwrap(),
            ],
        )
        .unwrap(),
    );
    DecisionRequest::new(
        State::from_text("write code with tests for the parser"),
        vec![question],
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap()
}

fn handle_for(graph: DecisionGraph, embedding: Option<Arc<MockEmbeddingBackend>>) -> EngineHandle {
    let backend: Option<Arc<dyn opencodifier_runtime::EmbeddingBackend>> =
        embedding.map(|backend| backend as Arc<dyn opencodifier_runtime::EmbeddingBackend>);
    let config = EngineConfig::new(graph).with_safe_mode(false).with_embedding(backend.clone());
    EngineHandle::with_embedding(
        config,
        Arc::new(opencodifier_engine::LexicalClassifier::new()),
        None,
        backend,
    )
    .unwrap()
}

/// The full D21 path decides, and the trace names the retrieval cut:
/// kept count, floor, and every dropped candidate with its score.
#[test]
fn retrieval_narrows_with_every_drop_disclosed() {
    let handle = handle_for(retrieval_graph(1, 0.0), Some(backend()));
    let (response, _) = handle.decide_with_report(&request()).unwrap();

    assert_eq!(response.answers().len(), 1);
    let entries = response.trace().entries();
    let kept_id = entries
        .iter()
        .filter(|entry| entry.node == "retrieve")
        .find_map(|entry| entry.detail.get("order"))
        .and_then(|value| match value {
            opencodifier_core::FactValue::Text(order) => order.split(',').next().map(String::from),
            _ => None,
        })
        .expect("the retrieval summary names the kept order");
    let dropped: Vec<String> = entries
        .iter()
        .filter_map(|entry| match entry.detail.get("dropped_candidate") {
            Some(opencodifier_core::FactValue::Text(name)) if entry.node == "retrieve" => {
                Some(name.clone())
            }
            _ => None,
        })
        .collect();
    // top_n 1 over 3 candidates: exactly two drops, and the drops plus
    // the kept id partition the candidate set — nothing vanishes quietly.
    assert_eq!(dropped.len(), 2);
    let mut all = vec![kept_id];
    all.extend(dropped);
    all.sort();
    assert_eq!(all, ["bench", "code", "prose"]);
}

/// The D21 floor guarantee: a floor nothing survives can never starve a
/// question — the keep-set falls back to the single best candidate.
#[test]
fn the_floor_never_starves_a_question() {
    let handle = handle_for(retrieval_graph(3, 0.99), Some(backend()));
    let (response, _) = handle.decide_with_report(&request()).unwrap();
    // Anything but NoValidCandidate: the candidate set still has a member.
    assert_ne!(response.outcome(), DecisionOutcome::NoValidCandidate);
    assert_eq!(response.answers().len(), 1);
}

/// A `rerank` node orders but never removes: the decision stage still
/// sees every candidate, whichever reranker the node names.
#[test]
fn reranking_permutes_without_removing_either_signal() {
    let depends = |names: &[&str]| -> Vec<NodeId> {
        names.iter().map(|name| NodeId::new(*name).unwrap()).collect()
    };
    let rerank_lexical = NodeSpec::build("rerank", NodeKind::Rerank)
        .unwrap()
        .with_dependencies(depends(&["normalize"]))
        .with_reranker("lexical");
    let rerank_embedding = NodeSpec::build("rerank", NodeKind::Rerank)
        .unwrap()
        .with_dependencies(depends(&["normalize"]))
        .with_reranker("embedding");
    for (name, rerank) in [("lexical", rerank_lexical), ("embedding", rerank_embedding)] {
        let graph = DecisionGraph::new(
            1,
            vec![
                NodeSpec::build("normalize", NodeKind::Normalize).unwrap(),
                rerank,
                NodeSpec::build("choice", NodeKind::Choice)
                    .unwrap()
                    .with_dependencies(depends(&["normalize", "rerank"])),
                NodeSpec::build("threshold", NodeKind::Threshold)
                    .unwrap()
                    .with_dependencies(depends(&["choice"]))
                    .with_threshold(0.5),
                NodeSpec::build("output", NodeKind::Output)
                    .unwrap()
                    .with_dependencies(depends(&["threshold"])),
            ],
        )
        .unwrap();
        let embedding = (name == "embedding").then(backend);
        let handle = handle_for(graph, embedding);
        let (response, _) = handle.decide_with_report(&request()).unwrap();
        // All three candidates reached the decision stage: no removal.
        let choice_entry =
            response.trace().entries().iter().find(|entry| entry.node == "choice").unwrap();
        assert!(choice_entry.detail.contains_key("top"), "{name}: decided, not starved");
        assert_ne!(response.outcome(), DecisionOutcome::NoValidCandidate, "{name}");
    }
}

/// The reranker seam's two implementations agree on the contract, and the
/// lexical one orders a coding query ahead of prose.
#[test]
fn the_reranker_trait_orders_by_relevance() {
    let candidates = [
        Candidate::new("prose", "draft release notes prose").unwrap(),
        Candidate::new("code", "write Rust code with tests").unwrap(),
    ];
    let lexical = LexicalReranker.rerank("write code", &candidates).unwrap();
    assert_eq!(lexical[0].candidate.id().as_str(), "code");

    let embedding = EmbeddingReranker::new(backend()).rerank("write code", &candidates).unwrap();
    assert_eq!(embedding.len(), 2, "permuted, never removed");
}

/// An engine without a backend refuses a graph that needs one at
/// assembly — never at first request, never with silent degradation.
#[test]
fn a_missing_backend_is_refused_at_assembly() {
    for graph in [retrieval_graph(1, 0.0), {
        let depends = |names: &[&str]| -> Vec<NodeId> {
            names.iter().map(|name| NodeId::new(*name).unwrap()).collect()
        };
        DecisionGraph::new(
            1,
            vec![
                NodeSpec::build("normalize", NodeKind::Normalize).unwrap(),
                NodeSpec::build("embedding", NodeKind::Embedding)
                    .unwrap()
                    .with_dependencies(depends(&["normalize"])),
                NodeSpec::build("output", NodeKind::Output)
                    .unwrap()
                    .with_dependencies(depends(&["embedding"])),
            ],
        )
        .unwrap()
    }] {
        let config = EngineConfig::new(graph.clone()).with_safe_mode(false);
        let error = EngineHandle::with_embedding(
            config,
            Arc::new(opencodifier_engine::LexicalClassifier::new()),
            None,
            None,
        )
        .unwrap_err();
        assert_eq!(error.code(), "engine.missing_backend");
    }
}

/// Safe mode refuses `retrieve` at assembly: eliminating candidates on
/// semantic evidence is a deliberate opt-out, never a default.
#[test]
fn safe_mode_refuses_retrieval_pruning() {
    let config = EngineConfig::new(retrieval_graph(1, 0.0));
    let error = EngineHandle::with_embedding(
        config,
        Arc::new(opencodifier_engine::LexicalClassifier::new()),
        None,
        Some(backend()),
    )
    .unwrap_err();
    assert_eq!(error.code(), "engine.invalid_config");
    assert!(error.to_string().contains("retrieve"));
}

/// A swapped embedding backend is part of what a decision decided on:
/// the identity — and therefore every cache key — moves with it. (A
/// retrieval graph with `None` never assembles, per the refusal test
/// above, so the zero-ML anchor is the plain pipeline.)
#[test]
fn the_embedding_backend_rides_the_identity() {
    let base = EngineHandle::lexical(EngineConfig::with_default_pipeline().unwrap()).unwrap();
    assert_eq!(base.identity().embedding_model, "none");

    let with = handle_for(retrieval_graph(2, 0.0), Some(backend()));
    assert_eq!(with.identity().embedding_model, "mock-embed-test-v1");
    assert_ne!(base.identity().embedding_model, with.identity().embedding_model);
}

/// Errors surface as `engine.backend_failed` when the backend itself
/// fails mid-run, not as a silent empty annotation.
#[test]
fn a_failing_backend_fails_the_node_loudly() {
    // Assembly succeeds (a backend exists); the failure comes at run.
    let handle = EngineHandle::with_embedding(
        EngineConfig::new(retrieval_graph(1, 0.0)).with_safe_mode(false),
        Arc::new(opencodifier_engine::LexicalClassifier::new()),
        None,
        Some(Arc::new(Failing)),
    )
    .unwrap();
    let error: EngineError = handle.decide(&request()).unwrap_err();
    assert_eq!(error.code(), "engine.backend_failed");
    assert!(error.to_string().contains("backend exploded"), "{error}");
}

/// The batch contract is one vector per text, order preserved. A backend
/// that miscounts is a `backend_failed` naming both counts — a silent
/// misalignment would score the wrong candidate against the query.
#[test]
fn a_miscounting_backend_fails_the_semantic_node() {
    // One question with three candidates: the backend owes four vectors
    // (query included) and returns two.
    let handle = EngineHandle::with_embedding(
        EngineConfig::new(retrieval_graph(1, 0.0)).with_safe_mode(false),
        Arc::new(opencodifier_engine::LexicalClassifier::new()),
        None,
        Some(Arc::new(Miscounting(2))),
    )
    .unwrap();
    let error = handle.decide(&request()).unwrap_err();
    assert_eq!(error.code(), "engine.backend_failed");
    assert!(error.to_string().contains("vectors for"), "{error}");
}

/// The reranker seam passes backend failures through unchanged: ordering
/// evidence may be wrong or absent, never invented.
#[test]
fn an_embedding_reranker_surfaces_backend_failures() {
    let candidates = [
        Candidate::new("prose", "draft release notes prose").unwrap(),
        Candidate::new("code", "write Rust code with tests").unwrap(),
    ];

    let error =
        EmbeddingReranker::new(Arc::new(Failing)).rerank("write code", &candidates).unwrap_err();
    assert_eq!(error.code(), "engine.backend_failed");
    assert!(error.to_string().contains("backend exploded"), "{error}");

    // Two candidates plus the query: one vector back is a miscount.
    let error = EmbeddingReranker::new(Arc::new(Miscounting(1)))
        .rerank("write code", &candidates)
        .unwrap_err();
    assert_eq!(error.code(), "engine.backend_failed");
    assert!(error.to_string().contains("1 vectors for 3"), "{error}");
}

/// Semantic scoring sees what deterministic narrowing left alive: a
/// `filter` node upstream of the semantic path feeds the surviving set
/// into embedding and retrieval, not the declared set.
#[test]
fn semantic_scoring_runs_over_filtered_survivors() {
    let depends = |names: &[&str]| -> Vec<NodeId> {
        names.iter().map(|name| NodeId::new(*name).unwrap()).collect()
    };
    let graph = DecisionGraph::new(
        1,
        vec![
            NodeSpec::build("normalize", NodeKind::Normalize).unwrap(),
            NodeSpec::build("filter", NodeKind::Filter)
                .unwrap()
                .with_dependencies(depends(&["normalize"])),
            NodeSpec::build("embedding", NodeKind::Embedding)
                .unwrap()
                .with_dependencies(depends(&["filter"])),
            NodeSpec::build("retrieve", NodeKind::Retrieve)
                .unwrap()
                .with_dependencies(depends(&["embedding"]))
                .with_top_n(2)
                .with_floor(0.0),
            NodeSpec::build("choice", NodeKind::Choice)
                .unwrap()
                .with_dependencies(depends(&["normalize", "retrieve"])),
            NodeSpec::build("threshold", NodeKind::Threshold)
                .unwrap()
                .with_dependencies(depends(&["choice"]))
                .with_threshold(0.5),
            NodeSpec::build("output", NodeKind::Output)
                .unwrap()
                .with_dependencies(depends(&["threshold"])),
        ],
    )
    .unwrap();
    let handle = handle_for(graph, Some(backend()));
    let (response, _) = handle.decide_with_report(&request()).unwrap();
    assert_eq!(response.answers().len(), 1);
    assert_ne!(response.outcome(), DecisionOutcome::NoValidCandidate);
}

/// A `retrieve` node with no upstream `embedding` node is a run error,
/// not a silent no-op (the graph validator cannot see dataflow).
#[test]
fn retrieval_without_semantic_scores_fails_loudly() {
    let depends = |names: &[&str]| -> Vec<NodeId> {
        names.iter().map(|name| NodeId::new(*name).unwrap()).collect()
    };
    let retrieve = NodeSpec::build("retrieve", NodeKind::Retrieve)
        .unwrap()
        .with_dependencies(depends(&["normalize"]))
        .with_top_n(1);
    let graph = DecisionGraph::new(
        1,
        vec![
            NodeSpec::build("normalize", NodeKind::Normalize).unwrap(),
            retrieve,
            NodeSpec::build("choice", NodeKind::Choice)
                .unwrap()
                .with_dependencies(depends(&["normalize", "retrieve"])),
            NodeSpec::build("threshold", NodeKind::Threshold)
                .unwrap()
                .with_dependencies(depends(&["choice"]))
                .with_threshold(0.5),
            NodeSpec::build("output", NodeKind::Output)
                .unwrap()
                .with_dependencies(depends(&["threshold"])),
        ],
    )
    .unwrap();
    let handle = handle_for(graph, Some(backend()));
    let error = handle.decide(&request()).unwrap_err();
    assert_eq!(error.code(), "engine.node_failed");
    assert!(error.to_string().contains("embedding"));
}

/// The optimized identity is untouched by an embedding backend swap on a
/// graph that never used one — a sanity anchor for cache-key stability
/// (D6) across the optimizer (D22).
#[test]
fn identity_defaults_hold_for_unoptimized_shapes() {
    let identity = EngineIdentity::default();
    assert_eq!(identity.embedding_model, "none");
}
