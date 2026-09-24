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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use opencodifier_core::{
    Candidate, CandidateId, ChoiceQuestion, ConfidenceReport, DecisionAnswer, DecisionMetrics,
    DecisionOutcome, DecisionPolicy, DecisionQuestion, DecisionRequest, DecisionTrace,
    Distribution, FactValue, NodeId, QuestionId, State, TraceEntry,
};

use crate::cache::CacheKey;
use crate::classifier::{Classifier, LexicalClassifier};
use crate::clock::{CancellationToken, Clock, Deadline};
use crate::engine::EngineConfig;
use crate::error::{EngineError, EngineResult};
use crate::graph::{NodeKind, NodeSpec};
use crate::narrowing::{LexicalScores, NarrowingOutcome};

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
    /// Distributions and answers for the questions this node decides.
    Decided(Vec<QuestionDecision>),
    /// Outcomes after the confidence gate and verifier cascade.
    Resolved(Vec<(QuestionId, DecisionOutcome, Option<bool>)>),
    /// Whether a branch condition held.
    Branched(bool),
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
    decisions: BTreeMap<QuestionId, QuestionDecision>,
    trace: DecisionTrace,
    skipped: BTreeSet<NodeId>,

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
            decisions: BTreeMap::new(),
            trace: DecisionTrace::new(),
            skipped: BTreeSet::new(),
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
        let parallel = runnable.len() > 1 && self.config.parallelism > 1;
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
            NodeKind::Choice | NodeKind::Boolean | NodeKind::Score => {
                let (decided, entries) = self.decide_questions(id, spec.kind)?;
                (NodeOutput::Decided(decided), entries)
            }
            NodeKind::Threshold => {
                let (resolved, entries) = self.resolve_threshold(id)?;
                (NodeOutput::Resolved(resolved), entries)
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

    /// Asks the classifier about every question of this node's kind.
    fn decide_questions(
        &self,
        id: &str,
        kind: NodeKind,
    ) -> EngineResult<(Vec<QuestionDecision>, Vec<TraceEntry>)> {
        let mut decided = Vec::new();
        let mut entries = Vec::new();
        for question in self.request.questions() {
            if !Self::wants(kind, question) {
                continue;
            }
            let Some(narrowed) = self.prepare_question(question) else { continue };
            let distribution = self.classifier.decide(&self.state, &narrowed)?;
            let (distribution, clipped) = Self::clip(&distribution, &narrowed)?;
            let decision = Self::decide_question(&narrowed, distribution, self.request.policy())?;
            entries.push(TraceEntry::new(
                id,
                [
                    ("question", FactValue::Text(question.id().to_string())),
                    ("top", FactValue::Text(decision.distribution.top().key.clone())),
                    ("probability", FactValue::Float(decision.distribution.top().probability)),
                    ("model", FactValue::Text(self.classifier.model_id().to_owned())),
                    ("clipped", FactValue::Integer(int(clipped))),
                ],
            ));
            decided.push(decision);
        }
        Ok((decided, entries))
    }

    /// Applies the confidence gate and the verifier cascade to every
    /// decided question.
    fn resolve_threshold(&self, id: &str) -> EngineResult<(Resolutions, Vec<TraceEntry>)> {
        let policy = self.request.policy();
        let mut resolved = Vec::new();
        let mut entries = Vec::new();
        for (question_id, decision) in &self.decisions {
            let outcome = decision.report.outcome_for(policy);
            let (final_outcome, agreement, verifier) = if outcome == DecisionOutcome::Verify {
                match self.verifier {
                    Some(verifier) => {
                        let alternative = verifier.decide(&self.state, &decision.question)?;
                        let agree = alternative.top().key == decision.distribution.top().key;
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
            entries.push(TraceEntry::new(
                id,
                [
                    ("question", FactValue::Text(question_id.to_string())),
                    ("threshold", FactValue::Float(policy.min_confidence())),
                    (
                        "passed",
                        FactValue::Boolean(
                            decision.report.calibrated_confidence >= policy.min_confidence(),
                        ),
                    ),
                    ("outcome", FactValue::Text(outcome_name(final_outcome))),
                    ("verifier", FactValue::Text(verifier.to_owned())),
                ],
            ));
            resolved.push((question_id.clone(), final_outcome, agreement));
        }
        Ok((resolved, entries))
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
    /// `NoValidCandidate` instead of failing the run.
    fn prepare_question(&self, question: &DecisionQuestion) -> Option<DecisionQuestion> {
        let DecisionQuestion::Choice(choice) = question else { return Some(question.clone()) };
        let outcome = self.narrowing.get(choice.id())?;
        if outcome.is_starved() {
            return None;
        }
        let narrowed = outcome.narrowed_question(choice)?;
        let pruned = match self.pruned_lexically.get(choice.id()) {
            Some(pruned) => pruned.as_slice(),
            None => &[],
        };
        if pruned.is_empty() {
            return Some(DecisionQuestion::Choice(narrowed));
        }
        let surviving: Vec<Candidate> = narrowed
            .candidates()
            .iter()
            .filter(|candidate| !pruned.contains(candidate.id()))
            .cloned()
            .collect();
        if surviving.is_empty() {
            // Lexical pruning eliminated everything; that is starvation,
            // reported rather than papered over.
            return None;
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
    /// Calibration at V1 is the identity map: `calibrated_confidence` is
    /// the top probability. That is temporary by design (PLANNING.md Rule 8,
    /// §17) — raw softmax output is not calibrated confidence, and the
    /// calibration layer arrives in Phase 9.
    fn decide_question(
        question: &DecisionQuestion,
        distribution: Distribution,
        policy: &DecisionPolicy,
    ) -> EngineResult<QuestionDecision> {
        let top = distribution.top().clone();
        let report =
            ConfidenceReport::from_distribution(&distribution, top.probability, 0.0, None)?;
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
            NodeOutput::Decided(decided) => {
                for decision in decided {
                    self.decisions.insert(decision.answer.question_id().clone(), decision);
                }
            }
            NodeOutput::Resolved(resolved) => {
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
        let candidates_out = narrowed_out.saturating_sub(lexically_pruned);
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

/// The name of the thread a node ran on, recorded in the run report.
fn thread_name() -> String {
    format!("{:?}", std::thread::current().id())
}
