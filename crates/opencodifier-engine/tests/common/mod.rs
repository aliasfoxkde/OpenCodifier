//! Shared fixtures for the engine's integration tests.
//!
//! Every test here drives the real [`DecisionEngine`] — the same public
//! surface a caller uses — so the assertions cover the whole pipeline and
//! not a slice of it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]
// Each integration test binary compiles this module on its own, so a
// fixture another binary uses shows up as dead code here.
#![allow(dead_code)]

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use opencodifier_core::{
    BooleanQuestion, Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion, DecisionRequest,
    DecisionResponse, Distribution, FactValue, Limits, NodeId, RequestMetadata, ScoreLevel,
    ScoreQuestion, State,
};
use opencodifier_engine::{
    Action, CancellationToken, Classifier, Condition, DecisionEngine, DecisionGraph, EngineConfig,
    EngineError, EngineResult, ManualClock, MockClassifier, NodeKind, NodeSpec, Rule, RuleEngine,
    RuleSet, SystemClock,
};

/// A choice question over `candidates`, with descriptions that make BM25
/// scores easy to predict.
pub fn choice_question(id: &str, candidates: &[(&str, &str)]) -> DecisionQuestion {
    let candidates: Vec<Candidate> = candidates
        .iter()
        .map(|(candidate_id, description)| Candidate::new(*candidate_id, *description).unwrap())
        .collect();
    DecisionQuestion::Choice(
        ChoiceQuestion::new(id, "Which model should answer this request?", candidates).unwrap(),
    )
}

/// A one-question choice request.
pub fn choice_request(candidates: &[(&str, &str)]) -> DecisionRequest {
    choice_request_with(
        State::from_text("Summarize research across many sources and compare findings"),
        candidates,
    )
}

/// A one-question choice request with an explicit state.
pub fn choice_request_with(state: State, candidates: &[(&str, &str)]) -> DecisionRequest {
    request(state, vec![choice_question("model", candidates)], RequestMetadata::default())
}

/// A request with an explicit state, question set, and metadata.
pub fn request(
    state: State,
    questions: Vec<DecisionQuestion>,
    metadata: RequestMetadata,
) -> DecisionRequest {
    DecisionRequest::new(state, questions, DecisionPolicy::default(), metadata).unwrap()
}

/// A request whose own execution-time ceiling is `limit`.
pub fn request_with_limit(candidates: &[(&str, &str)], limit: Duration) -> DecisionRequest {
    let metadata = RequestMetadata {
        limits: Limits { max_execution_time: limit, ..Limits::default() },
        ..Default::default()
    };
    request(
        State::from_text("Summarize research across many sources"),
        vec![choice_question("model", candidates)],
        metadata,
    )
}

/// A state with one text fact.
pub fn state_with_fact(fact: &str, value: &str) -> State {
    State::from_text("Summarize research across many sources")
        .with_fact(fact, FactValue::Text(value.to_owned()))
}

/// The built-in pipeline configuration at a given parallelism.
pub fn config(parallelism: usize) -> EngineConfig {
    EngineConfig::with_default_pipeline().unwrap().with_parallelism(parallelism)
}

/// An engine with the built-in pipeline, no rules, and the given
/// classifier.
pub fn engine(classifier: Arc<dyn Classifier>, parallelism: usize) -> EngineResult<DecisionEngine> {
    engine_with(config(parallelism), classifier)
}

/// An engine over an explicit configuration.
pub fn engine_with(
    config: EngineConfig,
    classifier: Arc<dyn Classifier>,
) -> EngineResult<DecisionEngine> {
    DecisionEngine::new(config, Arc::new(SystemClock), classifier, None)
}

/// An engine over an explicit configuration, clock, and verifier.
pub fn engine_with_clock(
    config: EngineConfig,
    clock: Arc<ManualClock>,
    classifier: Arc<dyn Classifier>,
) -> EngineResult<DecisionEngine> {
    DecisionEngine::new(config, clock, classifier, None)
}

/// A rule that excludes candidates whose description or id mentions `tag`
/// when `fact` equals `value`.
pub fn exclude_tag_rule(name: &str, fact: &str, value: &str, tag: &str) -> Rule {
    Rule {
        name: Some(name.to_owned()),
        when: Condition::FactEquals {
            fact: fact.to_owned(),
            value: FactValue::Text(value.to_owned()),
        },
        then: vec![Action::ExcludeCandidate { id: None, tag: Some(tag.to_owned()) }],
    }
}

/// A rule engine holding `rules`.
pub fn rules(rules: Vec<Rule>) -> Arc<RuleEngine> {
    Arc::new(RuleEngine::new(RuleSet { rules }).unwrap())
}

/// Five candidates with distinct descriptions: three "local", two "cloud".
pub fn five_candidates() -> Vec<(&'static str, &'static str)> {
    vec![
        ("local-tiny", "tiny local model for short edits"),
        ("local-small", "small local coding model"),
        ("local-large", "large local model with long context"),
        ("cloud-large", "cloud research model with long context"),
        ("cloud-fast", "fast cloud model for short answers"),
    ]
}

/// The value of an integer fact in a trace entry, if present.
pub fn trace_int<'a>(
    response: &'a DecisionResponse,
    node: &str,
    key: &str,
) -> Option<&'a FactValue> {
    response
        .trace()
        .entries()
        .iter()
        .find(|entry| entry.node == node)
        .and_then(|entry| entry.detail.get(key))
}

/// A boolean question, for requests that mix question kinds.
pub fn boolean_question(id: &str, text: &str) -> DecisionQuestion {
    DecisionQuestion::Boolean(BooleanQuestion::new(id, text).unwrap())
}

/// A score question over ordered level labels.
pub fn score_question(id: &str, levels: &[&str]) -> DecisionQuestion {
    let levels: Vec<ScoreLevel> =
        levels.iter().map(|label| ScoreLevel::new(*label).unwrap()).collect();
    DecisionQuestion::Score(
        ScoreQuestion::new(id, "How demanding is this request?", levels).unwrap(),
    )
}

/// A request over `questions`, with default policy and metadata.
pub fn question_request(questions: Vec<DecisionQuestion>) -> DecisionRequest {
    request(
        State::from_text("Summarize research across many sources and compare findings"),
        questions,
        RequestMetadata::default(),
    )
}

/// A scripted distribution over two candidates for the `model` question.
pub fn two_way(first: (&str, f64), second: (&str, f64)) -> Arc<MockClassifier> {
    Arc::new(MockClassifier::new("mock/two-way").with_script("model", vec![first, second]).unwrap())
}

/// A validated node spec with `dependencies`.
pub fn node(id: &str, kind: NodeKind, dependencies: &[&str]) -> NodeSpec {
    let dependencies: Vec<NodeId> =
        dependencies.iter().map(|dependency| NodeId::new(*dependency).unwrap()).collect();
    if dependencies.is_empty() {
        NodeSpec::build(id, kind).unwrap()
    } else {
        NodeSpec::build(id, kind).unwrap().with_dependencies(dependencies)
    }
}

/// A validated graph over `nodes`.
pub fn graph(nodes: Vec<NodeSpec>) -> DecisionGraph {
    DecisionGraph::new(1, nodes).unwrap()
}

/// A linear graph of `length` rule nodes finished by an output node: the
/// smallest way to make a graph that is valid but too large.
pub fn long_chain(length: usize) -> DecisionGraph {
    let mut nodes: Vec<NodeSpec> = (0..length)
        .map(|index| {
            let id = format!("n{index}");
            match index {
                0 => node(&id, NodeKind::Rule, &[]),
                _ => node(&id, NodeKind::Rule, &[&format!("n{}", index - 1)]),
            }
        })
        .collect();
    nodes.push(node("output", NodeKind::Output, &[&format!("n{}", length - 1)]));
    graph(nodes)
}

/// A classifier that moves the run clock forward on every decision, so a
/// deadline can be reached deterministically without real waiting.
#[derive(Debug)]
pub struct ClockAdvancing {
    /// The clock the classifier advances.
    pub clock: Arc<ManualClock>,
    /// How far each decision moves the clock.
    pub step: Duration,
    /// Where the distribution actually comes from.
    pub delegate: MockClassifier,
}

impl ClockAdvancing {
    /// Builds a classifier that charges `step` of manual time per decision.
    pub fn new(clock: Arc<ManualClock>, step: Duration) -> Self {
        Self { clock, step, delegate: MockClassifier::new("mock/clock-advancing") }
    }
}

impl Classifier for ClockAdvancing {
    fn decide(
        &self,
        state: &State,
        question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError> {
        self.clock.advance(self.step);
        self.delegate.decide(state, question)
    }

    fn model_id(&self) -> &'static str {
        "test/clock-advancing"
    }
}

/// A classifier that cancels the run at its first decision.
///
/// The engine owns its token and hands it out after construction, so the
/// classifier reads it from a slot the test fills in between.
#[derive(Debug)]
pub struct Cancelling {
    /// The slot the test fills with the engine's own token.
    pub token: Arc<OnceLock<Arc<CancellationToken>>>,
    /// Where the distribution actually comes from.
    pub delegate: MockClassifier,
}

impl Cancelling {
    /// Builds a classifier whose token slot is still empty.
    pub fn new() -> Self {
        Self { token: Arc::new(OnceLock::new()), delegate: MockClassifier::new("mock/cancelling") }
    }
}

impl Default for Cancelling {
    fn default() -> Self {
        Self::new()
    }
}

impl Classifier for Cancelling {
    fn decide(
        &self,
        state: &State,
        question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError> {
        if let Some(token) = self.token.get() {
            token.cancel();
        }
        self.delegate.decide(state, question)
    }

    fn model_id(&self) -> &'static str {
        "test/cancelling"
    }
}
