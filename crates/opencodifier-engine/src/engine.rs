//! The decision engine: configuration plus the `decide` pipeline
//! (PLANNING.md §43, §44, §45).
//!
//! [`DecisionEngine::decide`] runs the canonical pipeline:
//!
//! ```text
//! validate -> canonicalize -> cache lookup
//!          -> rules -> candidate narrowing -> lexical scoring
//!          -> decision (Classifier) -> distribution
//!          -> confidence report -> policy cascade
//!          -> (verify?) -> DecisionResponse + trace + metrics
//! ```
//!
//! Every stage is deterministic and synchronous. The pipeline is expressed
//! as the configured [`DecisionGraph`], executed by the wave executor, so
//! the graph *is* the pipeline and the trace explains it.
//!
//! # Calibration (temporary)
//!
//! At V1 the calibrated confidence of an answer is its top probability —
//! identity calibration. That is explicitly temporary (PLANNING.md Rule 8,
//! §17): raw softmax output is not calibrated confidence, and a temperature
//! scaling layer arrives in Phase 9. `calibration_version` is part of every
//! cache key, so swapping calibration in later invalidates cached
//! decisions automatically.
//!
//! # Cancellation
//!
//! The engine owns one [`CancellationToken`], exposed by
//! [`DecisionEngine::cancellation_token`]. Cancelling it stops every
//! request this engine runs at the next node boundary; a node already
//! running is not preemptible.

use std::sync::Arc;
use std::time::Duration;

use opencodifier_core::{DecisionRequest, DecisionResponse, DecisionTrace, FactValue, TraceEntry};

use crate::cache::{CacheConfig, CacheKey, CacheKeyBuilder, DecisionCache, EngineIdentity};
use crate::classifier::Classifier;
use crate::clock::{CancellationToken, Clock, Deadline};
use crate::error::{EngineError, EngineResult};
use crate::executor::{Executor, GraphOutcome, RunReport};
use crate::graph::DecisionGraph;
use crate::rules::RuleEngine;

/// Everything the engine needs besides the request.
///
/// Construct with [`EngineConfig::new`] or
/// [`EngineConfig::with_default_pipeline`], then adjust with the `with_*`
/// builders.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Graph/model/calibration identity folded into every cache key.
    pub identity: EngineIdentity,
    /// The executed decision graph.
    pub graph: DecisionGraph,
    /// Deterministic rules, run before any scoring.
    pub rules: Arc<RuleEngine>,
    /// Maximum threads used per wave. `1` disables parallel execution.
    pub parallelism: usize,
    /// When `true` (the default), only deterministic evidence may remove
    /// candidates (PLANNING.md §45).
    pub safe_mode: bool,
    /// Opt-in lexical pruning: keep only the top `n` candidates by score.
    /// Ignored while `safe_mode` is on.
    pub lexical_prune_limit: Option<usize>,
    /// Cache sizing and expiry.
    pub cache: CacheConfig,
    /// Engine-side ceiling on request execution time; the effective
    /// deadline is the smaller of this and the request's own limit.
    pub max_execution_time: Duration,
}

impl EngineConfig {
    /// Configuration for `graph`, with defaults for everything else: no
    /// rules, safe mode on, no lexical pruning, parallel execution.
    #[must_use]
    pub fn new(graph: DecisionGraph) -> Self {
        Self {
            identity: EngineIdentity::default(),
            graph,
            rules: Arc::new(RuleEngine::default()),
            parallelism: available_parallelism(),
            safe_mode: true,
            lexical_prune_limit: None,
            cache: CacheConfig::default(),
            max_execution_time: opencodifier_core::Limits::default().max_execution_time,
        }
    }

    /// Configuration for the canonical V1 pipeline
    /// ([`DecisionGraph::default_pipeline`]).
    ///
    /// # Errors
    ///
    /// Propagates graph validation errors; the built-in pipeline is
    /// validated by construction, so this only fails if the graph
    /// invariant is ever broken.
    pub fn with_default_pipeline() -> EngineResult<Self> {
        Ok(Self::new(DecisionGraph::default_pipeline()?))
    }

    /// Sets the cache-key identity.
    #[must_use]
    pub fn with_identity(mut self, identity: EngineIdentity) -> Self {
        self.identity = identity;
        self
    }

    /// Sets the rule engine.
    #[must_use]
    pub fn with_rules(mut self, rules: Arc<RuleEngine>) -> Self {
        self.rules = rules;
        self
    }

    /// Sets the per-wave thread budget. `1` executes sequentially.
    #[must_use]
    pub fn with_parallelism(mut self, parallelism: usize) -> Self {
        self.parallelism = parallelism;
        self
    }

    /// Enables or disables safe mode.
    #[must_use]
    pub fn with_safe_mode(mut self, safe_mode: bool) -> Self {
        self.safe_mode = safe_mode;
        self
    }

    /// Sets the lexical prune limit (ignored in safe mode).
    #[must_use]
    pub fn with_lexical_prune_limit(mut self, limit: Option<usize>) -> Self {
        self.lexical_prune_limit = limit;
        self
    }

    /// Sets the cache policy.
    #[must_use]
    pub fn with_cache(mut self, cache: CacheConfig) -> Self {
        self.cache = cache;
        self
    }

    /// Sets the engine-side execution-time ceiling.
    #[must_use]
    pub fn with_max_execution_time(mut self, max_execution_time: Duration) -> Self {
        self.max_execution_time = max_execution_time;
        self
    }
}

/// A reasonable default thread budget.
fn available_parallelism() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

/// The deterministic decision engine.
///
/// One engine owns its cache, clock, classifier, and optional verifier.
/// Sharing one engine across threads is safe and expected: every component
/// is `Send + Sync`, and decide is side-effect free apart from the cache.
#[derive(Debug)]
pub struct DecisionEngine {
    config: EngineConfig,
    cache: DecisionCache,
    clock: Arc<dyn Clock>,
    classifier: Arc<dyn Classifier>,
    verifier: Option<Arc<dyn Classifier>>,
    token: Arc<CancellationToken>,
}

impl DecisionEngine {
    /// Builds and validates an engine.
    ///
    /// # Errors
    ///
    /// [`EngineError::InvalidConfig`] when parallelism is zero, the lexical
    /// prune limit is zero, or the graph exceeds the engine's node limit;
    /// [`EngineError::CacheMisconfigured`] when the cache policy is
    /// invalid.
    pub fn new(
        config: EngineConfig,
        clock: Arc<dyn Clock>,
        classifier: Arc<dyn Classifier>,
        verifier: Option<Arc<dyn Classifier>>,
    ) -> EngineResult<Self> {
        if config.parallelism == 0 {
            return Err(EngineError::InvalidConfig {
                reason: "parallelism must be at least 1".to_owned(),
            });
        }
        if config.lexical_prune_limit == Some(0) {
            return Err(EngineError::InvalidConfig {
                reason: "lexical_prune_limit must be at least 1 if set".to_owned(),
            });
        }
        let node_limit = opencodifier_core::Limits::default().max_graph_nodes;
        if config.graph.exceeds(node_limit) {
            return Err(EngineError::GraphTooLarge {
                nodes: config.graph.nodes().len(),
                limit: node_limit,
            });
        }
        let cache = DecisionCache::new(config.cache, Arc::clone(&clock))?;
        Ok(Self {
            config,
            cache,
            clock,
            classifier,
            verifier,
            token: Arc::new(CancellationToken::new()),
        })
    }

    /// The configuration this engine runs with.
    #[must_use]
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// The exact-decision cache, for inspection and warming.
    #[must_use]
    pub fn cache(&self) -> &DecisionCache {
        &self.cache
    }

    /// The engine's cancellation token.
    #[must_use]
    pub fn cancellation_token(&self) -> Arc<CancellationToken> {
        Arc::clone(&self.token)
    }

    /// Decides a request.
    ///
    /// # Errors
    ///
    /// IR errors from core ([`EngineError::Core`]), duplicate question ids,
    /// graphs too large for the request's limits, timeouts, and
    /// cancellation.
    pub fn decide(&self, request: &DecisionRequest) -> EngineResult<DecisionResponse> {
        self.decide_with_report(request).map(|(response, _)| response)
    }

    /// Decides a request and returns execution diagnostics alongside the
    /// response.
    ///
    /// # Errors
    ///
    /// See [`DecisionEngine::decide`].
    pub fn decide_with_report(
        &self,
        request: &DecisionRequest,
    ) -> EngineResult<(DecisionResponse, RunReport)> {
        self.validate(request)?;
        let canonical = CacheKeyBuilder::normalized_request(request);
        let key = CacheKeyBuilder::build(&canonical, &self.config.identity)?;
        let timeout =
            request.metadata().limits.max_execution_time.min(self.config.max_execution_time);
        let deadline = Deadline::after(self.clock.as_ref(), timeout);

        if let Some(cached) = self.cache.get(&key) {
            let response = Self::cached_response(&cached, key);
            let report = RunReport {
                waves: self.config.graph.waves().unwrap_or_default(),
                cache_key: Some(key),
                cache_hit: true,
                outcomes: per_question_outcomes(request, cached.outcome()),
                ..RunReport::default()
            };
            return Ok((response, report));
        }

        let executor = Executor::new(
            &self.config,
            &self.clock,
            &self.classifier,
            self.verifier.as_ref(),
            request,
            key,
            deadline,
            &self.token,
        );
        let outcome: GraphOutcome = executor.run()?;
        let response = DecisionResponse::new(
            outcome.answers,
            outcome.outcome,
            outcome.report,
            outcome.trace,
            outcome.metrics,
        )?;
        let report = RunReport {
            waves: outcome.waves,
            parallel_waves: outcome.parallel_waves,
            threads: outcome.threads,
            cache_key: Some(key),
            cache_hit: false,
            skipped: outcome.skipped,
            narrowing: outcome.narrowing,
            lexical: outcome.lexical,
            outcomes: outcome.outcomes,
        };
        self.cache.insert(key, response.clone());
        Ok((response, report))
    }

    /// Request-level checks the IR does not make.
    fn validate(&self, request: &DecisionRequest) -> EngineResult<()> {
        let mut seen = std::collections::BTreeSet::new();
        for question in request.questions() {
            if !seen.insert(question.id().clone()) {
                return Err(EngineError::DuplicateQuestion { question: question.id().to_string() });
            }
        }
        let limits = &request.metadata().limits;
        if self.config.graph.exceeds(limits.max_graph_nodes) {
            return Err(EngineError::GraphTooLarge {
                nodes: self.config.graph.nodes().len(),
                limit: limits.max_graph_nodes,
            });
        }
        Ok(())
    }

    /// Rebuilds a cached response as a cache hit: same answers, metrics
    /// relabelled, and a trace that records the hit in front of the
    /// original execution facts.
    fn cached_response(cached: &DecisionResponse, key: CacheKey) -> DecisionResponse {
        let mut metrics = *cached.metrics();
        metrics.cache_hit = true;
        let mut trace = DecisionTrace::new();
        trace.push(TraceEntry::new(
            "cache",
            [("hit", FactValue::Boolean(true)), ("key", FactValue::Text(key.as_hex()))],
        ));
        for entry in cached.trace().entries() {
            trace.push(entry.clone());
        }
        match DecisionResponse::new(
            cached.answers().to_vec(),
            cached.outcome(),
            cached.confidence().clone(),
            trace,
            metrics,
        ) {
            Ok(response) => response,
            // Unreachable in practice: the cached response was validated
            // when it was first built, and rebuilding it with the same
            // answers and an extra trace entry cannot break an invariant.
            // Degrade to the cached response unchanged rather than fail a
            // decision that already succeeded once.
            Err(_) => cached.clone(),
        }
    }
}

/// Per-question outcomes for the run report.
fn per_question_outcomes(
    request: &DecisionRequest,
    outcome: opencodifier_core::DecisionOutcome,
) -> Vec<(opencodifier_core::QuestionId, opencodifier_core::DecisionOutcome)> {
    request.questions().iter().map(|question| (question.id().clone(), outcome)).collect()
}
