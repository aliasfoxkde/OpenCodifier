//! The CLI's JSON projection of the engine's execution report.
//!
//! [`RunReport`] is instrumentation, not a wire format, and the engine
//! deliberately keeps it un-`serde`: the deterministic explainability
//! surface is the response's own trace. This module is the one place the
//! report becomes JSON, built field by field from its public accessors so
//! a change in the report's shape is a compile error here rather than a
//! silently different document.

use opencodifier_core::{DecisionOutcome, NodeId, QuestionId};
use opencodifier_engine::{LexicalScores, NarrowingOutcome, RunReport};
use serde_json::{Value, json};

/// Builds the execution-report document for `--trace`.
#[must_use]
pub(crate) fn execution_json(report: &RunReport) -> Value {
    json!({
        "waves": report.waves()
            .iter()
            .map(|wave| wave.iter().map(node_id).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        "parallel_waves": report.parallel_waves(),
        "threads": report.threads(),
        "cache_hit": report.cache_hit(),
        "cache_key": report.cache_key().map(opencodifier_engine::CacheKey::as_hex),
        "skipped": report.skipped().iter().map(node_id).collect::<Vec<_>>(),
        "narrowing": report.narrowing().iter().map(narrowing_json).collect::<Vec<_>>(),
        "lexical": report.lexical().iter().map(lexical_json).collect::<Vec<_>>(),
        "outcomes": report.outcomes()
            .iter()
            .map(|(question, outcome)| {
                json!({ "question": question.to_string(), "outcome": outcome_json(*outcome) })
            })
            .collect::<Vec<_>>(),
    })
}

/// A node id as a plain string.
fn node_id(node: &NodeId) -> String {
    node.to_string()
}

/// The `snake_case` wire name of an outcome.
fn outcome_json(outcome: DecisionOutcome) -> Value {
    serde_json::to_value(outcome).unwrap_or(Value::Null)
}

/// One question's candidate narrowing: what the deterministic stages
/// removed and why nothing else did.
fn narrowing_json(outcome: &NarrowingOutcome) -> Value {
    json!({
        "question": question_id(outcome.question_id()),
        "before": outcome.before(),
        "after": outcome.after(),
        "removed": outcome.removed().iter().map(ToString::to_string).collect::<Vec<_>>(),
        "surviving": outcome.surviving().iter().map(|c| c.id().to_string()).collect::<Vec<_>>(),
        "starved": outcome.is_starved(),
        "reduction_ratio": outcome.reduction_ratio(),
    })
}

/// One question's lexical scores over the surviving candidates.
fn lexical_json(scores: &LexicalScores) -> Value {
    json!({
        "question": question_id(scores.question_id()),
        "scores": scores.scores()
            .iter()
            .map(|(candidate, score)| json!({ "candidate": candidate.to_string(), "score": score }))
            .collect::<Vec<_>>(),
        "pruned": scores.pruned().iter().map(ToString::to_string).collect::<Vec<_>>(),
        "top_score": scores.top_score(),
    })
}

/// A question id as a plain string.
fn question_id(question: &QuestionId) -> String {
    question.to_string()
}
