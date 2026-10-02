//! Topological wave executor (PLANNING.md §10).
//!
//! Responsibilities, in the order the spec lists them: topological
//! execution, dependency tracking, parallel independent nodes,
//! short-circuiting, conditional branches, result propagation, timeout,
//! cancellation, cache lookup, and trace generation.
//!
//! # Parallelism model
//!
//! The engine is synchronous — no async runtime. Parallelism is
//! `std::thread::scope` over the nodes of one wave: nodes in a wave are
//! independent by construction, so they may run concurrently, and their
//! results are merged in node-id order afterwards. Nodes never mutate
//! shared state directly; they report effects, and the coordinator folds
//! them in node-id order. That is what makes parallel execution
//! deterministic.
//!
//! # Preemption
//!
//! Deadline and cancellation are checked **before each node** (and before
//! each wave). A node that has started is not preemptible: it runs to
//! completion and its result is merged before the next check fires. A
//! synchronous executor cannot safely abandon work mid-flight, which is
//! why every node's work is bounded by construction (PLANNING.md §67).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use opencodifier_core::{
    Candidate, CandidateId, ChoiceQuestion, ConfidenceReport, DecisionAnswer, DecisionMetrics,
    DecisionOutcome, DecisionPolicy, DecisionQuestion, DecisionRequest, DecisionTrace,
    Distribution, FactValue, NodeId, QuestionId, State, TraceEntry,
};

use crate::cache::CacheKey;
use crate::classifier::{Classifier, LexicalClassifier};
use crate::clock::{CancellationToken, Clock, Deadline, Instant};
use crate::engine::EngineConfig;
use crate::error::{EngineError, EngineResult};
use crate::graph::{NodeKind, NodeSpec};
use crate::narrowing::{LexicalScores, NarrowingOutcome};
use crate::rerank::{EmbeddingReranker, LexicalReranker, Reranker, cosine};

/// Whether wave execution may spawn worker threads at all. Native targets
/// honor `EngineConfig::parallelism`; `wasm32` cannot spawn threads
/// (`std::thread::spawn` traps there), so wave execution is sequential by
/// construction and [`RunReport::parallel_waves`] stays zero — a compile
/// time fact stated once here, not a runtime surprise in a browser (D23).
#[cfg(not(target_arch = "wasm32"))]
const THREADS_AVAILABLE: bool = true;
#[cfg(target_arch = "wasm32")]
const THREADS_AVAILABLE: bool = false;

/// What one node produced, ready to be merged into the run state.
enum NodeOutput {
    /// Nothing to record.
    Inert,
    /// Rule effects: facts to apply, candidates to exclude or pin.
    Rules(crate::rules::RuleReport),
    /// Narrowing outcomes for choice questions.
    Filtered(Vec<NarrowingOutcome>),
    /// Lexical scores for choice questions.
    Scored(Vec<LexicalScores>),
    /// Distributions and answers for the questions this node decides,
    /// plus the run's focused-extraction counts.
    Decided(Vec<QuestionDecision>, crate::focus::FocusSummary),
    /// Outcomes after the confidence gate and verifier cascade, plus the
    /// decisions an escalation walk replaced (D27): a question answered
    /// by a later rung ships that rung's distribution, answer, and
    /// confidence, so the replacement rides back to the run state.
    Resolved(Vec<(QuestionId, DecisionOutcome, Option<bool>)>, Vec<QuestionDecision>),
    /// Whether a branch condition held.
    Branched(bool),
    /// Semantic (embedding) scores per choice question, best first —
    /// annotation only, consumed by `retrieve` (D21).
    Semantic(Vec<QuestionScores>),
    /// Candidates each choice question's `retrieve` node dropped, plus
    /// the score order of what it kept (D21).
    Retrieved(Vec<QuestionPrune>),
    /// The reranked candidate order per choice question (D21).
    Reranked(Vec<QuestionOrder>),
}

/// Semantic scores for one choice question, ordered by score descending
/// then candidate id ascending.
#[derive(Debug, Clone)]
pub(crate) struct QuestionScores {
    pub(crate) question: QuestionId,
    pub(crate) scores: Vec<(CandidateId, f64)>,
}

/// One `retrieve` node's effect on one choice question.
#[derive(Debug, Clone)]
pub(crate) struct QuestionPrune {
    pub(crate) question: QuestionId,
    /// Candidates dropped by the `floor/top_n` cut, each named (D21: never
    /// silent).
    pub(crate) dropped: Vec<CandidateId>,
    /// The kept candidates in score order — the question's new order.
    pub(crate) order: Vec<CandidateId>,
}

/// One `rerank` node's effect on one choice question.
#[derive(Debug, Clone)]
pub(crate) struct QuestionOrder {
    pub(crate) question: QuestionId,
    pub(crate) order: Vec<CandidateId>,
}

/// Everything one node execution produced.
struct NodeExecution {
    output: NodeOutput,
    entries: Vec<TraceEntry>,
    thread: String,
}

/// One question's answer, from the decision node until the response is
/// assembled.
#[derive(Debug, Clone)]
pub(crate) struct QuestionDecision {
    /// The (possibly narrowed) question that was decided.
    pub(crate) question: DecisionQuestion,
    /// Distribution over the surviving answers.
    pub(crate) distribution: Distribution,
    /// The answer as it appears in the response.
    pub(crate) answer: DecisionAnswer,
    /// Confidence derived from the distribution (identity calibration).
    pub(crate) report: ConfidenceReport,
    /// Outcome after the confidence gate and verifier cascade.
    pub(crate) outcome: DecisionOutcome,
    /// Whether the verifier ran for this question.
    pub(crate) verification_triggered: bool,
    /// The node that decided this question (id and kind), so the
    /// threshold node can re-apply the same rung's policy — ladder
    /// resolution must not depend on which graph path is re-traced.
    pub(crate) decided_by: (String, NodeKind),
}

/// Diagnostics about one execution, for callers that want more than the
/// response: which waves ran, whether they ran in parallel, what the cache
/// key was, and how narrowing reduced the candidate set.
///
/// Nothing here feeds back into the decision; it is instrumentation.
#[derive(Debug, Clone, Default)]
pub struct RunReport {
    pub(crate) waves: Vec<Vec<NodeId>>,
    pub(crate) parallel_waves: usize,
    pub(crate) threads: Vec<String>,
    pub(crate) cache_key: Option<CacheKey>,
    pub(crate) cache_hit: bool,
    pub(crate) skipped: Vec<NodeId>,
    pub(crate) narrowing: Vec<NarrowingOutcome>,
    pub(crate) lexical: Vec<LexicalScores>,
    /// Focused-extraction counts for the run: all zero when no focus
    /// policy is configured, so reports of unfocused engines are
    /// unchanged.
    pub(crate) focus: crate::focus::FocusSummary,
    pub(crate) outcomes: Vec<(QuestionId, DecisionOutcome)>,
}

impl RunReport {
    /// Topological waves the graph was decomposed into, in execution order.
    #[must_use]
    pub fn waves(&self) -> &[Vec<NodeId>] {
        &self.waves
    }

    /// Number of waves executed on more than one thread.
    #[must_use]
    pub fn parallel_waves(&self) -> usize {
        self.parallel_waves
    }

    /// Thread identifiers that executed at least one node, sorted.
    #[must_use]
    pub fn threads(&self) -> &[String] {
        &self.threads
    }

    /// The exact-decision cache key built for this request.
    #[must_use]
    pub fn cache_key(&self) -> Option<&CacheKey> {
        self.cache_key.as_ref()
    }

    /// Whether the response came from the cache.
    #[must_use]
    pub fn cache_hit(&self) -> bool {
        self.cache_hit
    }

    /// Focused-extraction counts for this run. All zero when the engine
    /// runs without a focus policy.
    #[must_use]
    pub fn focus(&self) -> crate::focus::FocusSummary {
        self.focus
    }

    /// Nodes skipped by branch short-circuiting, in execution order.
    #[must_use]
    pub fn skipped(&self) -> &[NodeId] {
        &self.skipped
    }

    /// Narrowing outcomes for choice questions, in question-id order.
    #[must_use]
    pub fn narrowing(&self) -> &[NarrowingOutcome] {
        &self.narrowing
    }

    /// Lexical scores for choice questions, in question-id order.
    #[must_use]
    pub fn lexical(&self) -> &[LexicalScores] {
        &self.lexical
    }

    /// Per-question final outcomes, in request order.
    #[must_use]
    pub fn outcomes(&self) -> &[(QuestionId, DecisionOutcome)] {
        &self.outcomes
    }
}

/// What the graph produced, before the engine turns it into a response.
pub(crate) struct GraphOutcome {
    pub(crate) answers: Vec<DecisionAnswer>,
    pub(crate) outcome: DecisionOutcome,
    pub(crate) report: ConfidenceReport,
    pub(crate) metrics: DecisionMetrics,
    pub(crate) trace: DecisionTrace,
    pub(crate) narrowing: Vec<NarrowingOutcome>,
    pub(crate) lexical: Vec<LexicalScores>,
    pub(crate) skipped: Vec<NodeId>,
    /// Focused-extraction counts for the run (all zero without a focus
    /// policy).
    pub(crate) focus: crate::focus::FocusSummary,
    pub(crate) waves: Vec<Vec<NodeId>>,
    pub(crate) parallel_waves: usize,
    pub(crate) threads: Vec<String>,
    pub(crate) outcomes: Vec<(QuestionId, DecisionOutcome)>,
}

/// Per-question resolution produced by the threshold node.
type Resolutions = Vec<(QuestionId, DecisionOutcome, Option<bool>)>;

/// One graph execution: the run state plus everything a node needs to read.
pub(crate) struct Executor<'a> {
    config: &'a EngineConfig,
    clock: &'a Arc<dyn Clock>,
    classifier: &'a Arc<dyn Classifier>,
    verifier: Option<&'a Arc<dyn Classifier>>,
    /// Ordered escalation rungs (D27) — consulted at the threshold node
    /// only while the previous rung's gate does not accept.
    fallbacks: &'a [crate::ladder::Rung],
    request: &'a DecisionRequest,
    key: CacheKey,
    deadline: Deadline,
    token: &'a CancellationToken,
    /// The run clock's reading when the run started, so elapsed time and
    /// the deadline are measured by the same clock.
    started: Instant,

    state: State,
    rules: Option<crate::rules::RuleReport>,
    narrowing: BTreeMap<QuestionId, NarrowingOutcome>,
    lexical: BTreeMap<QuestionId, LexicalScores>,
    /// Candidates removed by lexical pruning — only reachable with safe
    /// mode off (PLANNING.md §45).
    pruned_lexically: BTreeMap<QuestionId, Vec<CandidateId>>,
    /// Semantic scores per choice question (from `embedding` nodes),
    /// best first (D21).
    semantic: BTreeMap<QuestionId, Vec<(CandidateId, f64)>>,
    /// Candidates removed by `retrieve` nodes (D21) — every drop is
    /// trace-disclosed by the node that made it.
    pruned_semantically: BTreeMap<QuestionId, Vec<CandidateId>>,
    /// Candidate order per choice question, from `retrieve`/`rerank`
    /// nodes (D21). Candidates absent from the order keep their
    /// relative position after the ordered ones.
    reranked: BTreeMap<QuestionId, Vec<CandidateId>>,
    decisions: BTreeMap<QuestionId, QuestionDecision>,
    trace: DecisionTrace,
    skipped: BTreeSet<NodeId>,
    /// Focused-extraction counts accumulated across decision nodes.
    focus: crate::focus::FocusSummary,

    waves: Vec<Vec<NodeId>>,
    parallel_waves: usize,
    threads: BTreeSet<String>,
}

impl<'a> Executor<'a> {
    /// Prepares a run: working state, deadline, and empty run state.
    ///
    /// Many arguments, one call site: every component the run needs is
    /// passed explicitly rather than threaded through a second config type.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        config: &'a EngineConfig,
        clock: &'a Arc<dyn Clock>,
        classifier: &'a Arc<dyn Classifier>,
        verifier: Option<&'a Arc<dyn Classifier>>,
        fallbacks: &'a [crate::ladder::Rung],
        request: &'a DecisionRequest,
        key: CacheKey,
        deadline: Deadline,
        token: &'a CancellationToken,
    ) -> Self {
        Self {
            config,
            clock,
            classifier,
            verifier,
            fallbacks,
            request,
            key,
            deadline,
            token,
            started: clock.now(),
            state: request.state().clone(),
            rules: None,
            narrowing: BTreeMap::new(),
            lexical: BTreeMap::new(),
            pruned_lexically: BTreeMap::new(),
            semantic: BTreeMap::new(),
            pruned_semantically: BTreeMap::new(),
            reranked: BTreeMap::new(),
            decisions: BTreeMap::new(),
            trace: DecisionTrace::new(),
            skipped: BTreeSet::new(),
            focus: crate::focus::FocusSummary::default(),
            waves: Vec::new(),
            parallel_waves: 0,
            threads: BTreeSet::new(),
        }
    }

    /// Executes the graph to completion.
    pub(crate) fn run(mut self) -> EngineResult<GraphOutcome> {
        let Some(waves) = self.config.graph.waves() else {
            return Err(EngineError::Cycle { node: self.config.graph.output().id.to_string() });
        };
        waves.clone_into(&mut self.waves);

        for wave in &waves {
            self.guard()?;
            if self.wave_is_skipped(wave) {
                continue;
            }
            if self.run_wave(wave)? {
                break;
            }
        }
        Ok(self.assemble())
    }

    /// The deadline and cancellation checks applied before every node.
    fn guard(&self) -> EngineResult<()> {
        if self.token.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        if self.deadline.expired(&**self.clock) {
            return Err(EngineError::Timeout {
                elapsed_ms: self.elapsed_ms(),
                limit_ms: u64::try_from(self.deadline.limit().as_millis()).unwrap_or(u64::MAX),
            });
        }
        Ok(())
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.clock.now().duration_since(self.started).as_millis()).unwrap_or(u64::MAX)
    }

    /// Records skip markers for the already-skipped nodes of `wave` and
    /// reports whether the whole wave is skipped.
    fn wave_is_skipped(&mut self, wave: &[NodeId]) -> bool {
        let skipped: Vec<NodeId> =
            wave.iter().filter(|id| self.skipped.contains(*id)).cloned().collect();
        for id in skipped {
            self.mark_skipped(&id);
        }
        wave.iter().all(|id| self.skipped.contains(id))
    }

    /// Marks a node, and everything downstream of it, as skipped.
    fn mark_skipped(&mut self, id: &NodeId) {
        if !self.skipped.insert(id.clone()) {
            return;
        }
        self.trace.push(TraceEntry::new(id.as_str(), [("skipped", FactValue::Boolean(true))]));
        let graph = &self.config.graph;
        for dependent in graph.dependents_of(id) {
            self.mark_skipped(&dependent.id.clone());
        }
    }

    /// Runs one wave, in parallel when it holds independent nodes.
    ///
    /// Returns `true` when execution should stop after this wave.
    fn run_wave(&mut self, wave: &[NodeId]) -> EngineResult<bool> {
        let runnable: Vec<&NodeSpec> = wave
            .iter()
            .filter_map(|id| self.config.graph.node(id))
            .filter(|spec| !self.skipped.contains(&spec.id))
            .collect();
        let parallel = runnable.len() > 1 && self.config.parallelism > 1 && THREADS_AVAILABLE;
        if parallel {
            self.parallel_waves += 1;
        }
        let executor = &*self;

        let mut results: Vec<(NodeId, EngineResult<NodeExecution>)> =
            Vec::with_capacity(runnable.len());
        if parallel {
            std::thread::scope(|scope| {
                let handles: Vec<_> = runnable
                    .iter()
                    .map(|spec| (spec.id.clone(), scope.spawn(move || executor.execute(spec))))
                    .collect();
                for (id, handle) in handles {
                    let joined = match handle.join() {
                        Ok(result) => result,
                        // Panics are banned by the workspace lint contract;
                        // if one escapes anyway, it must not take the whole
                        // decision down with it.
                        Err(_) => Err(EngineError::NodeFailed {
                            node: id.to_string(),
                            reason: "worker thread panicked".to_owned(),
                        }),
                    };
                    results.push((id, joined));
                }
            });
        } else {
            for spec in &runnable {
                let id = spec.id.clone();
                results.push((id, executor.execute(spec)));
            }
        }

        // Deterministic merge: node-id order, whichever thread finished
        // first.
        results.sort_by(|left, right| left.0.cmp(&right.0));
        let graph = &self.config.graph;
        let mut stop = false;
        for (id, result) in results {
            let Some(spec) = graph.node(&id) else { continue };
            let execution = result?;
            self.threads.insert(execution.thread);
            for entry in execution.entries {
                self.trace.push(entry);
            }
            self.merge(spec, execution.output);
            if spec.kind == NodeKind::Output {
                stop = true;
            }
        }
        Ok(stop)
    }

    /// Executes a single node against the immutable run snapshot.
    fn execute(&self, spec: &NodeSpec) -> EngineResult<NodeExecution> {
        // Last check before work starts: a running node is not preemptible.
        self.guard()?;
        let id = spec.id.as_str();
        let (output, entries) = match spec.kind {
            NodeKind::Normalize => (
                NodeOutput::Inert,
                vec![TraceEntry::new(
                    id,
                    [
                        ("state_bytes", FactValue::Integer(int(self.state.text().len()))),
                        ("facts", FactValue::Integer(int(self.state.facts().count()))),
                    ],
                )],
            ),
            NodeKind::Rule => {
                let report = self.config.rules.evaluate(&self.state, self.request.questions());
                let fired = report.fired().len();
                let facts_set = report.facts_set().len();
                let excluded: usize = report.exclusions().values().map(Vec::len).sum();
                let pinned: usize = report.pins().values().map(Vec::len).sum();
                (
                    NodeOutput::Rules(report),
                    vec![TraceEntry::new(
                        id,
                        [
                            ("fired", FactValue::Integer(int(fired))),
                            ("facts_set", FactValue::Integer(int(facts_set))),
                            ("excluded", FactValue::Integer(int(excluded))),
                            ("pinned", FactValue::Integer(int(pinned))),
                        ],
                    )],
                )
            }
            NodeKind::Cache => (
                NodeOutput::Inert,
                vec![TraceEntry::new(
                    id,
                    [
                        ("hit", FactValue::Boolean(false)),
                        ("key", FactValue::Text(self.key.as_hex())),
                    ],
                )],
            ),
            NodeKind::Filter => {
                let (outcomes, entries) = self.filter_candidates(id);
                (NodeOutput::Filtered(outcomes), entries)
            }
            NodeKind::Lexical => {
                let (scored, entries) = self.score_lexical(id);
                (NodeOutput::Scored(scored), entries)
            }
            NodeKind::Embedding => {
                let (scored, entries) = self.score_semantic(id)?;
                (NodeOutput::Semantic(scored), entries)
            }
            NodeKind::Retrieve => {
                let (prunes, entries) = self.retrieve(id, spec)?;
                (NodeOutput::Retrieved(prunes), entries)
            }
            NodeKind::Rerank => {
                let (orders, entries) = self.rerank_candidates(id, spec)?;
                (NodeOutput::Reranked(orders), entries)
            }
            NodeKind::Choice | NodeKind::Boolean | NodeKind::Score => {
                let (decided, focus, entries) = self.decide_questions(id, spec.kind)?;
                (NodeOutput::Decided(decided, focus), entries)
            }
            NodeKind::Threshold => {
                let (resolved, replacements, entries) = self.resolve_threshold(id)?;
                (NodeOutput::Resolved(resolved, replacements), entries)
            }
            NodeKind::Branch => {
                // A branch with no condition always fires.
                let fired = spec.when.as_ref().is_none_or(|when| when.matches(&self.state));
                (
                    NodeOutput::Branched(fired),
                    vec![TraceEntry::new(id, [("fired", FactValue::Boolean(fired))])],
                )
            }
            NodeKind::Output => (
                NodeOutput::Inert,
                vec![TraceEntry::new(id, [("reached", FactValue::Boolean(true))])],
            ),
        };
        Ok(NodeExecution { output, entries, thread: thread_name() })
    }

    /// Applies rule exclusions and pins to every choice question.
    fn filter_candidates(&self, id: &str) -> (Vec<NarrowingOutcome>, Vec<TraceEntry>) {
        let mut outcomes = Vec::new();
        let mut entries = Vec::new();
        for question in self.request.questions() {
            let DecisionQuestion::Choice(choice) = question else { continue };
            let (exclusions, pins) = self.selectors_for(choice.id());
            let outcome = NarrowingOutcome::apply(choice, exclusions, pins);
            entries.push(TraceEntry::new(
                id,
                [
                    ("question", FactValue::Text(choice.id().to_string())),
                    ("before", FactValue::Integer(int(outcome.before()))),
                    ("after", FactValue::Integer(int(outcome.after()))),
                    ("removed", FactValue::Integer(int(outcome.removed().len()))),
                ],
            ));
            outcomes.push(outcome);
        }
        (outcomes, entries)
    }

    /// Scores the surviving candidates lexically.
    ///
    /// Safe mode: lexical evidence may order candidates, never eliminate
    /// them (PLANNING.md §45). Pruning is opt-in and only reachable with
    /// safe mode off.
    fn score_lexical(&self, id: &str) -> (Vec<LexicalScores>, Vec<TraceEntry>) {
        let mut results = Vec::new();
        let mut entries = Vec::new();
        for question in self.request.questions() {
            let DecisionQuestion::Choice(choice) = question else { continue };
            let Some(outcome) = self.narrowing.get(choice.id()) else { continue };
            let Some(narrowed) = outcome.narrowed_question(choice) else { continue };
            let query = LexicalClassifier::query_for(&self.state, question);
            let scored = LexicalScores::score(&narrowed, outcome.surviving(), &query);
            let scored = match (self.config.safe_mode, self.config.lexical_prune_limit) {
                // Safe mode keeps every candidate; scores only order them.
                (true, _) | (false, None) => scored,
                (false, Some(keep)) => scored.prune(keep),
            };
            entries.push(TraceEntry::new(
                id,
                [
                    ("question", FactValue::Text(choice.id().to_string())),
                    ("scored", FactValue::Integer(int(scored.scores().len()))),
                    ("top_score", FactValue::Float(scored.top_score().unwrap_or(0.0))),
                    ("pruned", FactValue::Integer(int(scored.pruned().len()))),
                    ("safe_mode", FactValue::Boolean(self.config.safe_mode)),
                ],
            ));
            results.push(scored);
        }
        (results, entries)
    }

    /// Scores the surviving candidates by embedding cosine similarity
    /// (PLANNING.md §24, §53; D21).
    ///
    /// Annotation only: the scores never eliminate anything. The
    /// `retrieve` node that consumes them does the narrowing — and names
    /// every candidate it drops in the trace. Engines without a backend
    /// never reach here: assembly refuses graphs that need one (D21).
    fn score_semantic(&self, id: &str) -> EngineResult<(Vec<QuestionScores>, Vec<TraceEntry>)> {
        let Some(backend) = self.config.embedding.as_ref() else {
            return Err(EngineError::MissingBackend {
                node: id.to_owned(),
                backend: "embedding".to_owned(),
            });
        };
        let mut results = Vec::new();
        let mut entries = Vec::new();
        for question in self.request.questions() {
            let DecisionQuestion::Choice(choice) = question else { continue };
            let in_play = self.candidates_in_play(choice);
            let query = LexicalClassifier::query_for(&self.state, question);
            let mut texts: Vec<&str> = vec![query.as_str()];
            texts.extend(in_play.iter().map(Candidate::description));
            let vectors = backend.embed(&texts).map_err(|error| EngineError::BackendFailed {
                model_id: backend.model_id().to_owned(),
                reason: error.to_string(),
            })?;
            if vectors.len() != texts.len() {
                return Err(EngineError::BackendFailed {
                    model_id: backend.model_id().to_owned(),
                    reason: format!(
                        "backend returned {} vectors for {} texts",
                        vectors.len(),
                        texts.len()
                    ),
                });
            }
            let query_vector = &vectors[0];
            let mut scores: Vec<(CandidateId, f64)> = in_play
                .iter()
                .enumerate()
                .map(|(index, candidate)| {
                    (candidate.id().clone(), cosine(query_vector, &vectors[index + 1]))
                })
                .collect();
            scores.sort_by(|left, right| {
                right.1.total_cmp(&left.1).then_with(|| left.0.cmp(&right.0))
            });
            let top_score = scores.first().map_or(0.0, |(_, score)| *score);
            entries.push(TraceEntry::new(
                id,
                [
                    ("question", FactValue::Text(choice.id().to_string())),
                    ("model", FactValue::Text(backend.model_id().to_owned())),
                    ("dims", FactValue::Integer(int(vectors.first().map_or(0, Vec::len)))),
                    ("scored", FactValue::Integer(int(scores.len()))),
                    ("top_score", FactValue::Float(top_score)),
                ],
            ));
            results.push(QuestionScores { question: choice.id().clone(), scores });
        }
        Ok((results, entries))
    }

    /// Narrows each choice question to the semantically best candidates
    /// (PLANNING.md §53; D21).
    ///
    /// The two D21 invariants: the floor can never starve a question (an
    /// empty keep-set falls back to the single best candidate), and every
    /// dropped candidate is named with its score in the trace. Only
    /// candidates still surviving deterministic stages may be dropped.
    fn retrieve(
        &self,
        id: &str,
        spec: &NodeSpec,
    ) -> EngineResult<(Vec<QuestionPrune>, Vec<TraceEntry>)> {
        // The graph validator requires `top_n >= 1`; the default is dead
        // code for validated graphs but keeps the node total.
        let top_n = spec.top_n.unwrap_or(1);
        let floor = spec.floor.unwrap_or(0.0);
        let mut prunes = Vec::new();
        let mut entries = Vec::new();
        for question in self.request.questions() {
            let DecisionQuestion::Choice(choice) = question else { continue };
            let Some(scores) = self.semantic.get(choice.id()) else {
                return Err(EngineError::NodeFailed {
                    node: id.to_owned(),
                    reason: format!(
                        "no semantic scores for question `{}`: `retrieve` requires an upstream \
                         `embedding` node",
                        choice.id()
                    ),
                });
            };
            let surviving = self.surviving_ids(choice);
            let ranked: Vec<&(CandidateId, f64)> =
                scores.iter().filter(|(candidate, _)| surviving.contains(candidate)).collect();
            let above_floor: Vec<&(CandidateId, f64)> =
                ranked.iter().copied().filter(|(_, score)| *score >= floor).collect();
            let kept: Vec<&(CandidateId, f64)> = if above_floor.is_empty() {
                ranked.iter().take(1).copied().collect()
            } else {
                above_floor.into_iter().take(top_n).collect()
            };
            let kept_ids: BTreeSet<&CandidateId> =
                kept.iter().map(|(candidate, _)| candidate).collect();
            let dropped: Vec<&(CandidateId, f64)> = ranked
                .iter()
                .copied()
                .filter(|(candidate, _)| !kept_ids.contains(candidate))
                .collect();
            for (candidate, score) in &dropped {
                entries.push(TraceEntry::new(
                    id,
                    [
                        ("question", FactValue::Text(choice.id().to_string())),
                        ("dropped_candidate", FactValue::Text(candidate.to_string())),
                        ("score", FactValue::Float(*score)),
                        ("floor", FactValue::Float(floor)),
                    ],
                ));
            }
            entries.push(TraceEntry::new(
                id,
                [
                    ("question", FactValue::Text(choice.id().to_string())),
                    ("kept", FactValue::Integer(int(kept.len()))),
                    ("dropped", FactValue::Integer(int(dropped.len()))),
                    ("top_n", FactValue::Integer(int(top_n))),
                    ("floor", FactValue::Float(floor)),
                    ("order", FactValue::Text(order_text(kept.iter().map(|(c, _)| c.as_str())))),
                ],
            ));
            prunes.push(QuestionPrune {
                question: choice.id().clone(),
                dropped: dropped.iter().map(|(candidate, _)| (*candidate).clone()).collect(),
                order: kept.iter().map(|(candidate, _)| (*candidate).clone()).collect(),
            });
        }
        Ok((prunes, entries))
    }

    /// Reorders the surviving candidates with the node's named reranker
    /// (PLANNING.md §26, §54; D21).
    ///
    /// A reranker is a second opinion on ordering: it permutes, never
    /// removes, so the decision stage still sees every candidate
    /// deterministic narrowing left alive.
    fn rerank_candidates(
        &self,
        id: &str,
        spec: &NodeSpec,
    ) -> EngineResult<(Vec<QuestionOrder>, Vec<TraceEntry>)> {
        let reranker: Box<dyn Reranker> = match spec.reranker.as_deref() {
            Some("lexical") => Box::new(LexicalReranker),
            Some("embedding") => {
                let Some(backend) = self.config.embedding.as_ref() else {
                    return Err(EngineError::MissingBackend {
                        node: id.to_owned(),
                        backend: "embedding".to_owned(),
                    });
                };
                Box::new(EmbeddingReranker::new(Arc::clone(backend)))
            }
            // Both remaining arms are unreachable for a validated graph;
            // a total match keeps the node honest if validation changes.
            Some(other) => {
                return Err(EngineError::NodeFailed {
                    node: id.to_owned(),
                    reason: format!("unknown reranker `{other}`"),
                });
            }
            None => {
                return Err(EngineError::NodeFailed {
                    node: id.to_owned(),
                    reason: "rerank nodes must name a reranker".to_owned(),
                });
            }
        };
        let mut results = Vec::new();
        let mut entries = Vec::new();
        for question in self.request.questions() {
            let DecisionQuestion::Choice(choice) = question else { continue };
            let surviving: Vec<Candidate> = self.candidates_in_play(choice);
            if surviving.is_empty() {
                continue;
            }
            let query = LexicalClassifier::query_for(&self.state, question);
            let scored = reranker.rerank(query.as_str(), &surviving)?;
            let order: Vec<CandidateId> =
                scored.iter().map(|scored| scored.candidate.id().clone()).collect();
            entries.push(TraceEntry::new(
                id,
                [
                    ("question", FactValue::Text(choice.id().to_string())),
                    ("reranker", FactValue::Text(reranker.model_id().to_owned())),
                    ("count", FactValue::Integer(int(order.len()))),
                    (
                        "top",
                        FactValue::Text(
                            order
                                .first()
                                .map_or_else(String::new, std::string::ToString::to_string),
                        ),
                    ),
                    ("order", FactValue::Text(order_text(order.iter().map(CandidateId::as_str)))),
                ],
            ));
            results.push(QuestionOrder { question: choice.id().clone(), order });
        }
        Ok((results, entries))
    }

    /// The candidates of one choice question that are still in play: the
    /// rule-filtered survivors when a `filter` node ran, the full
    /// candidate list otherwise, always minus lexical and `retrieve`
    /// prunes.
    fn candidates_in_play(&self, choice: &ChoiceQuestion) -> Vec<Candidate> {
        let (pruned_lexically, pruned_semantically) = self.pruned_sets(choice.id());
        let base: Vec<Candidate> = match self.narrowing.get(choice.id()) {
            Some(outcome) => outcome.surviving().to_vec(),
            // No narrowing ran: every declared candidate is in play.
            None => choice.candidates().to_vec(),
        };
        base.into_iter()
            .filter(|candidate| {
                !pruned_lexically.contains(candidate.id())
                    && !pruned_semantically.contains(candidate.id())
            })
            .collect()
    }

    /// The candidate ids of one question still in play (see
    /// [`Executor::candidates_in_play`]). Ids only: the full-candidate
    /// clone in [`Executor::candidates_in_play`] is wasted work here, and
    /// the id list is the same length.
    fn surviving_ids(&self, choice: &ChoiceQuestion) -> BTreeSet<CandidateId> {
        let (pruned_lexically, pruned_semantically) = self.pruned_sets(choice.id());
        let base: &[Candidate] = match self.narrowing.get(choice.id()) {
            Some(outcome) => outcome.surviving(),
            None => choice.candidates(),
        };
        base.iter()
            .map(Candidate::id)
            .filter(|id| !pruned_lexically.contains(*id) && !pruned_semantically.contains(*id))
            .cloned()
            .collect()
    }

    /// The per-question pruned-id sets, built once for the survive-filter
    /// loops instead of probing the backing `Vec`s once per candidate
    /// (B2). Membership semantics are exactly the former `is_surviving`:
    /// a candidate survives when it appears in neither prune list.
    fn pruned_sets(&self, question: &QuestionId) -> (HashSet<&CandidateId>, HashSet<&CandidateId>) {
        (
            self.pruned_lexically.get(question).map_or(&[][..], Vec::as_slice).iter().collect(),
            self.pruned_semantically.get(question).map_or(&[][..], Vec::as_slice).iter().collect(),
        )
    }

    /// Asks the classifier about every question of this node's kind.
    fn decide_questions(
        &self,
        id: &str,
        kind: NodeKind,
    ) -> EngineResult<(Vec<QuestionDecision>, crate::focus::FocusSummary, Vec<TraceEntry>)> {
        let mut decided = Vec::new();
        let mut entries = Vec::new();
        let mut focus = crate::focus::FocusSummary::default();
        // Ladder resolution (PLANNING.md §24): the deciding node's own
        // gate wins over the request policy, so an escalation ladder can
        // hold exact rungs to p ≥ 1.0 while the model rung uses its
        // calibrated thresholds. `None` when no override applies —
        // byte-identical to the single-policy engine.
        let (policy, ladder_source) = self.config.ladder.resolve(id, kind).map_or_else(
            || (self.request.policy(), None),
            |(policy, source)| (policy, Some(source)),
        );
        // Per-rung calibration (D15/B4): a rung's confidences are not
        // commensurable with the engine default's, so a configured rung
        // calibration replaces it for this rung's questions. `None`
        // leaves the engine-level calibration — byte-identical.
        let ladder_calibration = self.config.ladder.resolve_calibration(id, kind);
        for question in self.request.questions() {
            if !Self::wants(kind, question) {
                continue;
            }
            let Some(narrowed) = self.prepare_question(question) else { continue };
            // Focused extraction (PLANNING.md §45): when configured and the
            // state exceeds the budget, the classifier reads a per-question
            // view; a weak view escalates to the full state before any
            // gate sees the answer.
            let view = self
                .config
                .focus
                .map(|policy| crate::focus::focus(&self.state, &narrowed, &policy));
            let (decide_state, extracted) = match &view {
                Some(view) => (&view.state, view.extracted),
                None => (&self.state, false),
            };
            let distribution = self.classifier.decide(decide_state, &narrowed)?;
            let (mut distribution, mut clipped) = Self::clip(&distribution, &narrowed)?;
            let mut escalated = false;
            if extracted && Self::escalation_warranted(policy, &distribution) {
                let full = self.classifier.decide(&self.state, &narrowed)?;
                let (full, full_clipped) = Self::clip(&full, &narrowed)?;
                distribution = full;
                clipped = full_clipped;
                escalated = true;
            }
            let (calibration, calibration_source) = ladder_calibration.as_ref().map_or_else(
                || (self.config.calibration.as_ref(), None),
                |(calibration, source)| (*calibration, Some(source.clone())),
            );
            let decision =
                Self::decide_question(&narrowed, distribution, policy, calibration, id, kind)?;
            let mut detail = vec![
                ("question", FactValue::Text(question.id().to_string())),
                ("top", FactValue::Text(decision.distribution.top().key.clone())),
                ("probability", FactValue::Float(decision.distribution.top().probability)),
                ("calibrated", FactValue::Float(decision.report.calibrated_confidence)),
                ("ood", FactValue::Float(decision.report.ood_score)),
                ("model", FactValue::Text(self.classifier.model_id().to_owned())),
                ("clipped", FactValue::Integer(int(clipped))),
            ];
            if let Some(source) = &ladder_source {
                detail.push(("policy_source", FactValue::Text(source.clone())));
            }
            if let Some(source) = calibration_source {
                detail.push(("calibration_source", FactValue::Text(source)));
            }
            if let Some(view) = &view {
                detail.push(("focus_engaged", FactValue::Boolean(view.extracted)));
                detail.push(("focus_kept", FactValue::Integer(int(view.kept_sentences))));
                detail.push(("focus_total", FactValue::Integer(int(view.total_sentences))));
                detail.push(("focus_tokens", FactValue::Integer(int(view.estimated_tokens))));
                detail.push(("focus_escalated", FactValue::Boolean(escalated)));
            }
            entries.push(TraceEntry::new(id, detail));
            // Counts are kept only for engines that focus: an unfocused run
            // must not grow a focus surface (the report accessor and the
            // JSON projection both key on `decided > 0`).
            if self.config.focus.is_some() {
                focus = focus.record(view.as_ref().is_some_and(|view| view.extracted), escalated);
            }
            decided.push(decision);
        }
        Ok((decided, focus, entries))
    }

    /// Whether a focused view's distribution is too weak to trust: below
    /// the policy's accept gate, or flat enough to trip the §19 entropy
    /// ceiling when one is set. Reverse escalation re-decides on the full
    /// state rather than shipping a degraded answer.
    fn escalation_warranted(
        policy: &opencodifier_core::DecisionPolicy,
        distribution: &Distribution,
    ) -> bool {
        distribution.top().probability < policy.min_confidence()
            || distribution.entropy() >= policy.entropy_ceiling()
    }

    /// Applies the confidence gate, the escalation walk (D27), and the
    /// verifier cascade to every decided question. Decisions a fallback
    /// rung answered are returned as replacements: the walk's final rung
    /// owns the question's distribution, answer, and confidence.
    fn resolve_threshold(
        &self,
        id: &str,
    ) -> EngineResult<(Resolutions, Vec<QuestionDecision>, Vec<TraceEntry>)> {
        let mut resolved = Vec::new();
        let mut replacements = Vec::new();
        let mut entries = Vec::new();
        for (question_id, decision) in &self.decisions {
            // The gate re-applies the deciding rung's policy: the cached
            // response was gated by whatever policy its deciding node
            // resolved, so the live gate must read the same rung to stay
            // consistent with it (PLANNING.md §24; a changed ladder
            // re-keys via the identity decoration, it does not re-gate
            // cached responses). No ladder override for this node → the
            // request policy, and the trace gains no `policy_source`
            // fact — byte-identical to the single-policy engine.
            let (node_id, kind) = &decision.decided_by;
            let (policy, ladder_source) = self.config.ladder.resolve(node_id, *kind).map_or_else(
                || (self.request.policy(), None),
                |(policy, source)| (policy, Some(source)),
            );
            // The escalation walk (D27): only a non-accepting gate fires
            // the next rung, so an accepted question runs exactly one
            // classifier. Each rung is gated by its own policy when it
            // carries one, else the node-resolved policy; its
            // calibration replaces the engine-level one for its own
            // distribution.
            let mut live = decision;
            let mut chain: Vec<String> = Vec::new();
            for rung in self.fallbacks {
                if live.report.outcome_for(policy) == DecisionOutcome::Accept {
                    break;
                }
                // A rung walk may not outrun the request's budget.
                self.guard()?;
                let raw = rung.classifier.decide(&self.state, &live.question)?;
                let (distribution, _) = Self::clip(&raw, &live.question)?;
                let calibration =
                    rung.calibration.as_deref().unwrap_or(self.config.calibration.as_ref());
                let rung_policy = rung.policy.as_ref().unwrap_or(policy);
                let escalated = Self::decide_question(
                    &live.question,
                    distribution,
                    rung_policy,
                    calibration,
                    node_id,
                    *kind,
                )?;
                chain.push(format!(
                    "{}(top={}, {})",
                    rung.classifier.model_id(),
                    escalated.distribution.top().key,
                    outcome_name(escalated.report.outcome_for(rung_policy))
                ));
                replacements.push(escalated);
                // `live` follows the freshest replacement: the walk's
                // final answer is the last rung's, and the verifier
                // cascade below must read that one.
                live = &replacements[replacements.len() - 1];
            }
            // `live` borrows `replacements`; the outcome and verifier
            // cascade below only read it.
            let outcome = live.report.outcome_for(policy);
            let (final_outcome, agreement, verifier) = if outcome == DecisionOutcome::Verify {
                match self.verifier {
                    Some(verifier) => {
                        let alternative = verifier.decide(&self.state, &live.question)?;
                        let agree = alternative.top().key == live.distribution.top().key;
                        if agree {
                            (DecisionOutcome::Verified, Some(true), "agree")
                        } else {
                            (DecisionOutcome::Abstain, Some(false), "disagree")
                        }
                    }
                    // No verifier configured: the answer stays marked
                    // unverified.
                    None => (DecisionOutcome::Verify, None, "none"),
                }
            } else {
                (outcome, None, "none")
            };
            let mut detail = vec![
                ("question", FactValue::Text(question_id.to_string())),
                ("threshold", FactValue::Float(policy.min_confidence())),
                (
                    "passed",
                    FactValue::Boolean(
                        live.report.calibrated_confidence >= policy.min_confidence(),
                    ),
                ),
                ("outcome", FactValue::Text(outcome_name(final_outcome))),
                ("verifier", FactValue::Text(verifier.to_owned())),
            ];
            // Which rung's gate fired — only when a ladder override
            // decided it, so default traces stay byte-identical.
            if let Some(source) = ladder_source {
                detail.push(("policy_source", FactValue::Text(source)));
            }
            // The escalation walk, only when it fired (D27): the chain
            // names each rung's model, answer, and its own gate outcome,
            // so the trace explains routing without exposing any model
            // reasoning.
            if !chain.is_empty() {
                detail.push(("rungs_fired", FactValue::Integer(int(chain.len()))));
                detail.push(("rung_chain", FactValue::Text(chain.join(" -> "))));
            }
            entries.push(TraceEntry::new(id, detail));
            resolved.push((question_id.clone(), final_outcome, agreement));
        }
        // Keep only questions the walk actually re-answered: a rung
        // firing for one question must not displace another's primary
        // decision. `replacements` accumulates one entry per fired rung,
        // and the last entry per question is the walk's final answer.
        let mut final_replacements: Vec<QuestionDecision> = Vec::new();
        {
            let mut last_index: BTreeMap<QuestionId, usize> = BTreeMap::new();
            for (index, replacement) in replacements.iter().enumerate() {
                last_index.insert(replacement.answer.question_id().clone(), index);
            }
            for index in last_index.into_values() {
                final_replacements.push(replacements[index].clone());
            }
        }
        Ok((resolved, final_replacements, entries))
    }

    /// `true` when this node decides questions of `question`'s kind.
    fn wants(kind: NodeKind, question: &DecisionQuestion) -> bool {
        matches!(
            (kind, question),
            (NodeKind::Choice, DecisionQuestion::Choice(_))
                | (NodeKind::Boolean, DecisionQuestion::Boolean(_))
                | (NodeKind::Score, DecisionQuestion::Score(_))
        )
    }

    /// Narrows a choice question to its surviving candidates.
    ///
    /// `None` for questions that are not choices and for questions whose
    /// candidates were all eliminated, which the assembler reports as
    /// `NoValidCandidate` instead of failing the run. Survivors are the
    /// rule-filtered set minus lexical prunes minus `retrieve` prunes,
    /// ordered by any `retrieve`/`rerank` node that ran (D21).
    fn prepare_question(&self, question: &DecisionQuestion) -> Option<DecisionQuestion> {
        let DecisionQuestion::Choice(choice) = question else { return Some(question.clone()) };
        // The narrowed shape when a `filter` node ran; the full question
        // otherwise — a semantic graph may narrow only through `retrieve`.
        let narrowed = match self.narrowing.get(choice.id()) {
            Some(outcome) => {
                if outcome.is_starved() {
                    return None;
                }
                outcome.narrowed_question(choice)?
            }
            None => choice.clone(),
        };
        let (pruned_lexically, pruned_semantically) = self.pruned_sets(choice.id());
        let mut surviving: Vec<Candidate> = narrowed
            .candidates()
            .iter()
            .filter(|candidate| {
                !pruned_lexically.contains(candidate.id())
                    && !pruned_semantically.contains(candidate.id())
            })
            .cloned()
            .collect();
        if surviving.is_empty() {
            // Pruning eliminated everything; that is starvation, reported
            // rather than papered over. (`retrieve` keeps top-1, so only
            // the lexical prune knob can get here.)
            return None;
        }
        if let Some(order) = self.reranked.get(choice.id()) {
            // Rank map instead of `position()` inside the sort comparator;
            // `or_insert` keeps the first occurrence, matching the former
            // linear scan exactly (B2).
            let mut rank: HashMap<&CandidateId, usize> = HashMap::with_capacity(order.len());
            for (position, kept) in order.iter().enumerate() {
                rank.entry(kept).or_insert(position);
            }
            surviving
                .sort_by_key(|candidate| rank.get(candidate.id()).copied().unwrap_or(usize::MAX));
        }
        let rebuilt = ChoiceQuestion::new(choice.id().as_str(), narrowed.text(), surviving).ok()?;
        Some(DecisionQuestion::Choice(rebuilt))
    }

    /// Rule-produced exclusion and pin lists for one choice question.
    fn selectors_for(&self, id: &QuestionId) -> (&[CandidateId], &[CandidateId]) {
        match self.rules.as_ref() {
            Some(report) => (report.exclusions_for(id), report.pins_for(id)),
            None => (&[], &[]),
        }
    }

    /// Restricts a classifier distribution to answers that still exist.
    ///
    /// A classifier may name a candidate that deterministic narrowing
    /// removed. The surviving candidate set is the source of truth, so such
    /// keys are dropped and the remainder renormalized; the count is
    /// reported in the trace rather than hidden.
    #[allow(clippy::cast_precision_loss)] // candidate counts are tiny
    fn clip(
        distribution: &Distribution,
        question: &DecisionQuestion,
    ) -> EngineResult<(Distribution, usize)> {
        let allowed: BTreeSet<String> = match question {
            DecisionQuestion::Choice(choice) => choice
                .candidates()
                .iter()
                .map(|candidate| candidate.id().as_str().to_owned())
                .collect(),
            DecisionQuestion::Boolean(_) => {
                ["true".to_owned(), "false".to_owned()].into_iter().collect()
            }
            DecisionQuestion::Score(score) => {
                score.levels().iter().map(|level| level.label().to_owned()).collect()
            }
            // `#[non_exhaustive]`: an unknown question kind has no answer
            // set to clip against, so nothing is allowed.
            _ => BTreeSet::new(),
        };
        let kept: Vec<(String, f64)> = distribution
            .entries()
            .iter()
            .filter(|entry| allowed.contains(&entry.key))
            .map(|entry| (entry.key.clone(), entry.probability))
            .collect();
        let clipped = distribution.entries().len().saturating_sub(kept.len());
        if kept.is_empty() {
            return Err(EngineError::InvalidDistribution {
                question: question.id().to_string(),
                reason: "no key overlaps the surviving answer set".to_owned(),
            });
        }
        let total: f64 = kept.iter().map(|(_, probability)| probability).sum();
        let normalized: Vec<(String, f64)> = if !total.is_finite() || total <= 0.0 {
            let mass = 1.0 / kept.len() as f64; // allowed above
            kept.iter().map(|(key, _)| (key.clone(), mass)).collect()
        } else {
            kept.iter().map(|(key, probability)| (key.clone(), probability / total)).collect()
        };
        let normalized = Distribution::from_pairs(normalized).map_err(|error| {
            EngineError::InvalidDistribution {
                question: question.id().to_string(),
                reason: error.to_string(),
            }
        })?;
        Ok((normalized, clipped))
    }

    /// Turns a normalized distribution into an answer and a confidence
    /// report.
    ///
    /// The confidence path is explicit (PLANNING.md §18, §73): the
    /// configured [`Calibration`](crate::calibration::Calibration) maps the
    /// raw top probability to calibrated confidence, and the OOD channel
    /// carries the deterministic distributional signal (normalized entropy
    /// — the only input-unlikeness evidence available before a trained
    /// density model exists; see [`distributional_ood`]). Policy reads the
    /// dimensions separately in `ConfidenceReport::outcome_for`; nothing
    /// is fused silently.
    fn decide_question(
        question: &DecisionQuestion,
        distribution: Distribution,
        policy: &DecisionPolicy,
        calibration: &dyn crate::calibration::Calibration,
        decided_by: &str,
        decided_kind: NodeKind,
    ) -> EngineResult<QuestionDecision> {
        let top = distribution.top().clone();
        let ood = distributional_ood(&distribution);
        let calibrated =
            calibration.calibrate(crate::calibration::question_class(question), &distribution);
        let report = ConfidenceReport::from_distribution(&distribution, calibrated, ood, None)?;
        let outcome = ConfidenceReport::outcome_for(&report, policy);
        let answer = match question {
            DecisionQuestion::Choice(choice) => DecisionAnswer::Choice {
                question_id: choice.id().clone(),
                choice: CandidateId::new(&top.key)?,
                distribution: distribution.clone(),
                confidence: report.calibrated_confidence,
            },
            DecisionQuestion::Boolean(boolean) => DecisionAnswer::Boolean {
                question_id: boolean.id().clone(),
                value: top.key == "true",
                probability: top.probability,
                confidence: report.calibrated_confidence,
            },
            DecisionQuestion::Score(score) => {
                let expected = Self::expected_value(&distribution);
                DecisionAnswer::Score {
                    question_id: score.id().clone(),
                    expected,
                    level: score.level_for(expected).label().to_owned(),
                    distribution: distribution.clone(),
                    confidence: report.calibrated_confidence,
                }
            }
            // `DecisionQuestion` is `#[non_exhaustive]`: an unknown kind has
            // no answer shape, so it is declined rather than guessed at.
            _ => {
                return Err(EngineError::InvalidDistribution {
                    question: question.id().to_string(),
                    reason: "unsupported question kind".to_owned(),
                });
            }
        };
        Ok(QuestionDecision {
            question: question.clone(),
            distribution,
            answer,
            report,
            outcome,
            verification_triggered: false,
            decided_by: (decided_by.to_owned(), decided_kind),
        })
    }

    /// Expected value of a distribution over positional weights.
    #[allow(clippy::cast_precision_loss)]
    fn expected_value(distribution: &Distribution) -> f64 {
        distribution
            .entries()
            .iter()
            .enumerate()
            .map(|(index, entry)| entry.probability * index as f64)
            .sum()
    }

    /// Folds one node's effects into the run state.
    fn merge(&mut self, spec: &NodeSpec, output: NodeOutput) {
        match output {
            NodeOutput::Inert => {}
            NodeOutput::Rules(report) => {
                report.apply_to(&mut self.state);
                self.rules = Some(report);
            }
            NodeOutput::Filtered(outcomes) => {
                for outcome in outcomes {
                    self.narrowing.insert(outcome.question_id().clone(), outcome);
                }
            }
            NodeOutput::Scored(scores) => {
                for scores in scores {
                    let question_id = scores.question_id().clone();
                    let pruned = scores.pruned().to_vec();
                    self.lexical.insert(question_id.clone(), scores);
                    if !pruned.is_empty() {
                        self.pruned_lexically.entry(question_id).or_default().extend(pruned);
                    }
                }
            }
            NodeOutput::Semantic(scores) => {
                for scored in scores {
                    self.semantic.insert(scored.question, scored.scores);
                }
            }
            NodeOutput::Retrieved(prunes) => {
                for prune in prunes {
                    if !prune.dropped.is_empty() {
                        self.pruned_semantically
                            .entry(prune.question.clone())
                            .or_default()
                            .extend(prune.dropped);
                    }
                    self.reranked.insert(prune.question, prune.order);
                }
            }
            NodeOutput::Reranked(orders) => {
                for order in orders {
                    self.reranked.insert(order.question, order.order);
                }
            }
            NodeOutput::Decided(decided, focus) => {
                for decision in decided {
                    self.decisions.insert(decision.answer.question_id().clone(), decision);
                }
                self.focus.decided += focus.decided;
                self.focus.engaged += focus.engaged;
                self.focus.escalated += focus.escalated;
            }
            NodeOutput::Resolved(resolved, replacements) => {
                // A replaced decision is inserted whole: the escalation
                // walk's final rung owns the question's distribution,
                // answer, and confidence (D27), and the outcome patch
                // below must land on that decision, not the primary's.
                for replacement in replacements {
                    self.decisions.insert(replacement.answer.question_id().clone(), replacement);
                }
                for (id, outcome, agreement) in resolved {
                    if let Some(decision) = self.decisions.get_mut(&id) {
                        decision.outcome = outcome;
                        let mut report = decision.report.clone();
                        report.verifier_agreement = agreement;
                        decision.report = report;
                        decision.verification_triggered = agreement.is_some();
                    }
                }
            }
            NodeOutput::Branched(fired) => {
                if !fired {
                    let graph = &self.config.graph;
                    let dependents: Vec<NodeId> =
                        graph.dependents_of(&spec.id).into_iter().map(|d| d.id.clone()).collect();
                    for dependent in dependents {
                        self.mark_skipped(&dependent);
                    }
                }
            }
        }
    }

    /// Assembles the answer set, outcome, confidence, metrics, and report.
    fn assemble(mut self) -> GraphOutcome {
        let narrowing: Vec<NarrowingOutcome> = self.narrowing.values().cloned().collect();
        let lexical: Vec<LexicalScores> = self.lexical.values().cloned().collect();
        let skipped: Vec<NodeId> = self.skipped.iter().cloned().collect();

        let mut answers = Vec::new();
        let mut outcomes: Vec<(QuestionId, DecisionOutcome)> = Vec::new();
        for question in self.request.questions() {
            match self.decisions.get(question.id()) {
                Some(decision) => {
                    answers.push(decision.answer.clone());
                    outcomes.push((question.id().clone(), decision.outcome));
                }
                // The node that would have answered this question was
                // skipped, so the engine declines rather than guessing.
                None => outcomes.push((question.id().clone(), DecisionOutcome::Abstain)),
            }
        }

        // A starved question overrides everything: no candidate survived
        // deterministic filtering.
        let starved = narrowing.iter().any(NarrowingOutcome::is_starved);
        let outcome =
            if starved { DecisionOutcome::NoValidCandidate } else { self.conservative(&outcomes) };

        let candidates_in = narrowing.iter().map(NarrowingOutcome::before).max().unwrap_or(0);
        let narrowed_out = narrowing.iter().map(NarrowingOutcome::after).max().unwrap_or(0);
        let lexically_pruned = self.pruned_lexically.values().map(Vec::len).max().unwrap_or(0);
        let semantically_pruned =
            self.pruned_semantically.values().map(Vec::len).max().unwrap_or(0);
        let candidates_out =
            narrowed_out.saturating_sub(lexically_pruned).saturating_sub(semantically_pruned);
        let metrics = DecisionMetrics {
            candidates_in,
            candidates_out,
            cache_hit: false,
            verification_triggered: self.decisions.values().any(|d| d.verification_triggered),
        };

        GraphOutcome {
            answers,
            outcome,
            report: self.confidence_for(outcome, &outcomes),
            metrics,
            trace: std::mem::take(&mut self.trace),
            narrowing,
            lexical,
            skipped,
            focus: self.focus,
            waves: std::mem::take(&mut self.waves),
            parallel_waves: self.parallel_waves,
            threads: self.threads.into_iter().collect(),
            outcomes,
        }
    }

    /// The most conservative per-question outcome; ties break on the lowest
    /// calibrated confidence, then on question id.
    fn conservative(&self, outcomes: &[(QuestionId, DecisionOutcome)]) -> DecisionOutcome {
        outcomes
            .iter()
            .max_by(|left, right| {
                severity(left.1)
                    .cmp(&severity(right.1))
                    .then_with(|| {
                        self.confidence_of(&right.0).total_cmp(&self.confidence_of(&left.0))
                    })
                    .then_with(|| left.0.cmp(&right.0))
            })
            .map_or(DecisionOutcome::Abstain, |(_, outcome)| *outcome)
    }

    fn confidence_of(&self, id: &QuestionId) -> f64 {
        self.decisions.get(id).map_or(1.0, |decision| decision.report.calibrated_confidence)
    }

    /// The confidence report the response carries: the report of the
    /// question that drove the final outcome.
    fn confidence_for(
        &self,
        outcome: DecisionOutcome,
        outcomes: &[(QuestionId, DecisionOutcome)],
    ) -> ConfidenceReport {
        let driver =
            outcomes.iter().filter(|(_, candidate)| *candidate == outcome).min_by(|left, right| {
                self.confidence_of(&left.0)
                    .total_cmp(&self.confidence_of(&right.0))
                    .then_with(|| left.0.cmp(&right.0))
            });
        match driver.and_then(|(id, _)| self.decisions.get(id)) {
            Some(decision) => decision.report.clone(),
            None => ConfidenceReport {
                top_probability: 0.0,
                margin: 0.0,
                entropy: 0.0,
                calibrated_confidence: 0.0,
                ood_score: 0.0,
                verifier_agreement: None,
            },
        }
    }
}

/// Deterministic distributional OOD proxy: the Shannon entropy of the
/// surviving distribution normalized by its maximum, `H / log2 k` in
/// `[0, 1]` (entropy is carried in bits, so the maximum is `log2 k`).
/// A near-uniform distribution over the surviving answers is the only
/// input-unlikeness signal the engine can produce without a trained
/// density model — it says "this input gave the scorer nothing to
/// separate the candidates with", which is exactly the flat-output
/// symptom an OOD detector exists to catch. The density-ratio and
/// embedding-distance detectors arrive with the model rungs (D2) and
/// will replace this proxy; the channel and its policy gate
/// (`ood_ceiling`) stay as they are.
///
/// Zero for degenerate distributions (fewer than two surviving entries).
#[allow(clippy::cast_precision_loss)] // answer-set sizes are tiny
fn distributional_ood(distribution: &Distribution) -> f64 {
    let k = distribution.entries().len();
    if k < 2 {
        return 0.0;
    }
    let max_entropy = (k as f64).log2();
    if max_entropy <= 0.0 {
        return 0.0;
    }
    (distribution.entropy() / max_entropy).clamp(0.0, 1.0)
}

/// Severity ordering for combining per-question outcomes: a request is only
/// as decisive as its least decisive question.
fn severity(outcome: DecisionOutcome) -> u8 {
    match outcome {
        DecisionOutcome::Accept => 1,
        DecisionOutcome::Verified => 2,
        DecisionOutcome::Verify => 3,
        DecisionOutcome::Escalate | DecisionOutcome::Abstain => 4,
        // `NoValidCandidate` — and any outcome added to this
        // `#[non_exhaustive]` enum later — is treated as the most
        // conservative ranking until it is classified.
        _ => 5,
    }
}

/// The canonical name of an outcome, for traces.
fn outcome_name(outcome: DecisionOutcome) -> String {
    match outcome {
        DecisionOutcome::Accept => "accept".to_owned(),
        DecisionOutcome::Verified => "verified".to_owned(),
        DecisionOutcome::Verify => "verify".to_owned(),
        DecisionOutcome::Abstain => "abstain".to_owned(),
        DecisionOutcome::Escalate => "escalate".to_owned(),
        DecisionOutcome::NoValidCandidate => "no_valid_candidate".to_owned(),
        // `#[non_exhaustive]`: traces stay readable for future outcomes.
        _ => "unknown".to_owned(),
    }
}

/// `usize` → `i64` for trace facts; candidate counts and byte lengths never
/// approach `i64::MAX`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_possible_wrap)]
fn int(value: usize) -> i64 {
    value as i64
}

/// The readable candidate-order fact: ids joined by `,` in kept order.
fn order_text<'a>(ids: impl Iterator<Item = &'a str>) -> String {
    ids.collect::<Vec<_>>().join(",")
}

/// The name of the thread a node ran on, recorded in the run report.
fn thread_name() -> String {
    format!("{:?}", std::thread::current().id())
}
