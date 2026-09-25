//! The shared engine facade for every interface crate (PLAN Phases 8–10).
//!
//! CLI, HTTP, and MCP all assemble and drive the runtime through
//! [`EngineHandle`]; no endpoint reimplements pipeline wiring. The handle
//! owns an assembled [`DecisionEngine`] and exposes exactly the operations
//! an interface legitimately needs: decide (with or without the execution
//! report), graph validation, and a health snapshot for `healthz`-style
//! probes. It stays synchronous (D5) — interface crates add their own
//! async shells around it.

use std::sync::Arc;

use opencodifier_core::{DecisionRequest, DecisionResponse};

use crate::cache::EngineIdentity;
use crate::classifier::{Classifier, LexicalClassifier};
use crate::clock::SystemClock;
use crate::engine::{DecisionEngine, EngineConfig};
use crate::error::EngineResult;
use crate::executor::RunReport;

/// A ready-to-serve decision runtime.
#[derive(Debug)]
pub struct EngineHandle {
    engine: DecisionEngine,
}

impl EngineHandle {
    /// Assembles the engine from `config` (graph, cache, limits,
    /// identity) plus the semantic layers: `classifier` decides,
    /// `verifier` is the optional confidence-gated second opinion.
    ///
    /// # Errors
    ///
    /// Propagates [`DecisionEngine::new`] — invalid parallelism, prune
    /// limit, oversized graph, or misconfigured cache.
    pub fn new(
        config: EngineConfig,
        classifier: Arc<dyn Classifier>,
        verifier: Option<Arc<dyn Classifier>>,
    ) -> EngineResult<Self> {
        Ok(Self {
            engine: DecisionEngine::new(config, Arc::new(SystemClock), classifier, verifier)?,
        })
    }

    /// The fully deterministic, zero-ML engine: built-in lexical
    /// classifier only. This is the base binary's default posture —
    /// useful with no model files anywhere on the machine.
    ///
    /// # Errors
    ///
    /// Propagates [`DecisionEngine::new`].
    pub fn lexical(config: EngineConfig) -> EngineResult<Self> {
        Self::new(config, Arc::new(LexicalClassifier::new()), None)
    }

    /// Decides a request through the full pipeline.
    ///
    /// # Errors
    ///
    /// Propagates [`DecisionEngine::decide`].
    pub fn decide(&self, request: &DecisionRequest) -> EngineResult<DecisionResponse> {
        self.engine.decide(request)
    }

    /// Decides and also returns the deterministic execution report
    /// (waves, cache hit, narrowing outcomes) interfaces surface as the
    /// explainability trace.
    ///
    /// # Errors
    ///
    /// Propagates [`DecisionEngine::decide_with_report`].
    pub fn decide_with_report(
        &self,
        request: &DecisionRequest,
    ) -> EngineResult<(DecisionResponse, RunReport)> {
        self.engine.decide_with_report(request)
    }

    /// Validates a graph without running it: construction itself enforces
    /// the DAG contract (ids, edges, single output, cycles, node limit).
    /// Interfaces call this after deserializing a graph from the wire.
    ///
    /// # Errors
    ///
    /// Whatever [`DecisionGraph::new`](crate::graph::DecisionGraph::new)
    /// rejects.
    pub fn validate_graph(graph: &crate::graph::DecisionGraph) -> EngineResult<()> {
        // `DecisionGraph::new` already validated; re-deriving from the
        // spec keeps this the one validation path, never a weaker copy.
        crate::graph::DecisionGraph::new(graph.version(), graph.nodes().to_vec()).map(|_| ())
    }

    /// The engine identity decisions are cached under.
    #[must_use]
    pub fn identity(&self) -> EngineIdentity {
        self.engine.config().identity.clone()
    }

    /// The health snapshot behind `healthz`-style probes: liveness here
    /// is "the engine assembled and can answer", not a stub `ok` string.
    #[must_use]
    pub fn health(&self) -> EngineHealth {
        let config = self.engine.config();
        EngineHealth {
            identity: config.identity.clone(),
            nodes: config.graph.nodes().len(),
            parallelism: config.parallelism,
            cache_enabled: config.cache.max_entries > 0,
        }
    }
}

/// Liveness and identity facts about an assembled engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineHealth {
    /// Identity decisions are cached under.
    pub identity: EngineIdentity,
    /// Nodes in the active decision graph.
    pub nodes: usize,
    /// Configured executor parallelism.
    pub parallelism: usize,
    /// Whether the exact-decision cache is budgeted at all.
    pub cache_enabled: bool,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use opencodifier_core::{
        Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion, RequestMetadata, State,
    };
    use std::sync::Arc as StdArc;

    fn choice_request() -> DecisionRequest {
        let question = DecisionQuestion::Choice(
            ChoiceQuestion::new(
                "model",
                "Which model should run deep reasoning?",
                vec![
                    Candidate::new("local-qwen", "fast general coding").unwrap(),
                    Candidate::new("local-glm", "deep reasoning specialist").unwrap(),
                ],
            )
            .unwrap(),
        );
        DecisionRequest::new(
            State::from_text("deep reasoning over a large proof"),
            vec![question],
            DecisionPolicy::default(),
            RequestMetadata::default(),
        )
        .unwrap()
    }

    #[test]
    fn lexical_handle_decides_and_reports_health() {
        let handle = EngineHandle::lexical(EngineConfig::with_default_pipeline().unwrap()).unwrap();
        let response = handle.decide(&choice_request()).unwrap();
        assert_eq!(response.answers().len(), 1);

        let health = handle.health();
        assert_eq!(health.identity, EngineIdentity::builtin());
        assert!(health.nodes > 0);
        assert!(health.parallelism >= 1);
        assert!(health.cache_enabled);

        let (response_with_report, report) = handle.decide_with_report(&choice_request()).unwrap();
        assert_eq!(response_with_report.answers().len(), 1);
        assert!(!report.waves().is_empty());
        assert_eq!(handle.identity().model_id, "builtin-lexical-v1");
    }

    #[test]
    fn validate_graph_accepts_the_default_and_rejects_broken() {
        let good = EngineConfig::with_default_pipeline().unwrap().graph;
        EngineHandle::validate_graph(&good).unwrap();

        let cycle = crate::graph::DecisionGraph::new(
            1,
            vec![
                crate::graph::NodeSpec::build("a", crate::graph::NodeKind::Rule)
                    .unwrap()
                    .with_dependencies(vec![opencodifier_core::NodeId::new("b").unwrap()]),
                crate::graph::NodeSpec::build("b", crate::graph::NodeKind::Rule)
                    .unwrap()
                    .with_dependencies(vec![opencodifier_core::NodeId::new("a").unwrap()]),
            ],
        )
        .unwrap_err();
        assert_eq!(cycle.code(), "graph.cycle");
    }

    #[test]
    fn handle_is_shareable_across_threads() {
        let handle = StdArc::new(
            EngineHandle::lexical(EngineConfig::with_default_pipeline().unwrap()).unwrap(),
        );
        let request = choice_request();
        let mut join_handles = Vec::new();
        for _ in 0..4 {
            let thread_handle = StdArc::clone(&handle);
            let thread_request = request.clone();
            join_handles.push(std::thread::spawn(move || {
                thread_handle.decide(&thread_request).unwrap().answers().len()
            }));
        }
        for join_handle in join_handles {
            assert_eq!(join_handle.join().unwrap(), 1);
        }
    }
}
