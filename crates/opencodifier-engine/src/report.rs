//! The engine's JSON projection of its execution report.
//!
//! [`RunReport`] is instrumentation, not a wire format, and the engine
//! deliberately keeps it un-`serde`: the deterministic explainability
//! surface is the response's own trace. This module is the one place the
//! report becomes JSON, built field by field from its public accessors so
//! a change in the report's shape is a compile error here rather than a
//! silently different document. Every interface that explains an execution
//! (CLI `--trace`, MCP `codify_explain`) projects through this function,
//! so no two surfaces can drift apart on what an execution looked like.

use opencodifier_core::{DecisionOutcome, NodeId, QuestionId};
use serde_json::{Value, json};

use crate::executor::RunReport;
use crate::narrowing::{LexicalScores, NarrowingOutcome};

/// Builds the execution-report document for trace-style output.
#[must_use]
pub fn execution_json(report: &RunReport) -> Value {
    let mut document = json!({
        "waves": report.waves()
            .iter()
            .map(|wave| wave.iter().map(node_id).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        "parallel_waves": report.parallel_waves(),
        "threads": report.threads(),
        "cache_hit": report.cache_hit(),
        "cache_key": report.cache_key().map(crate::CacheKey::as_hex),
        "skipped": report.skipped().iter().map(node_id).collect::<Vec<_>>(),
        "narrowing": report.narrowing().iter().map(narrowing_json).collect::<Vec<_>>(),
        "lexical": report.lexical().iter().map(lexical_json).collect::<Vec<_>>(),
        "outcomes": report.outcomes()
            .iter()
            .map(|(question, outcome)| {
                json!({ "question": question.to_string(), "outcome": outcome_json(*outcome) })
            })
            .collect::<Vec<_>>(),
    });
    // Focus counts appear only for engines that focus, so the projection
    // of an unfocused run is byte-identical to its pre-focus form.
    let focus = report.focus();
    if focus.decided > 0
        && let Some(object) = document.as_object_mut()
    {
        object.insert(
            "focus".to_owned(),
            json!({
                "decided": focus.decided,
                "engaged": focus.engaged,
                "escalated": focus.escalated,
            }),
        );
    }
    document
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use crate::EngineHandle;
    use crate::engine::EngineConfig;
    use opencodifier_core::{Candidate, ChoiceQuestion, DecisionQuestion, RequestMetadata, State};

    /// A minimal choice request the default pipeline can decide.
    fn choice_request() -> opencodifier_core::DecisionRequest {
        let question = DecisionQuestion::Choice(
            ChoiceQuestion::new(
                "model",
                "which model should serve this request?",
                vec![
                    Candidate::new("local-qwen", "fast general coding").unwrap(),
                    Candidate::new("local-glm", "deep reasoning specialist").unwrap(),
                ],
            )
            .unwrap(),
        );
        opencodifier_core::DecisionRequest::new(
            State::from_text("prefer the smallest model that fits in 8 GiB"),
            vec![question],
            opencodifier_core::DecisionPolicy::default(),
            RequestMetadata::default(),
        )
        .unwrap()
    }

    #[test]
    fn projection_carries_the_whole_report_shape() {
        let handle = EngineHandle::lexical(EngineConfig::with_default_pipeline().unwrap()).unwrap();
        let (_, report) = handle.decide_with_report(&choice_request()).unwrap();

        let document = execution_json(&report);
        for key in [
            "waves",
            "parallel_waves",
            "threads",
            "cache_hit",
            "cache_key",
            "skipped",
            "narrowing",
            "lexical",
            "outcomes",
        ] {
            assert!(document.get(key).is_some(), "missing `{key}`");
        }
        // Every executed node appears in wave order, and the question's
        // outcome is reported once.
        assert!(!document["waves"].as_array().unwrap().is_empty());
        assert_eq!(document["outcomes"].as_array().unwrap().len(), 1);
    }
}
