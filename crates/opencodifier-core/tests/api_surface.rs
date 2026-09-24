//! Public-surface coverage: error codes, conversions, accessors, and
//! iteration — the contract other crates build on.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

use opencodifier_core::{
    BooleanQuestion, Candidate, CandidateId, CoreError, DecisionPolicy, DecisionQuestion,
    DecisionRequest, DecisionTrace, Distribution, FactValue, Limits, NodeId, QuestionId,
    RequestMetadata, ScoreLevel, ScoreQuestion, State, TraceEntry,
};

#[test]
fn error_policy_ctor_sets_reason() {
    let error = CoreError::policy("gates inverted");
    assert!(matches!(error, CoreError::InvalidPolicy { .. }));
    assert_eq!(error.to_string(), "invalid decision policy: gates inverted");
}

#[test]
fn every_error_variant_reports_its_stable_code() {
    let cases: Vec<(CoreError, &str)> = vec![
        (CoreError::EmptyField { field: "text" }, "ir.empty_field"),
        (CoreError::EmptyCandidates { question: "q".into() }, "ir.empty_candidates"),
        (CoreError::DuplicateCandidate { id: "c".into() }, "ir.duplicate_candidate"),
        (CoreError::TooFewLevels { question: "q".into(), count: 1 }, "ir.too_few_levels"),
        (CoreError::DuplicateLevel { label: "l".into() }, "ir.duplicate_level"),
        (CoreError::policy("x"), "ir.invalid_policy"),
        (CoreError::InvalidId { value: String::new(), reason: "empty".into() }, "ir.invalid_id"),
        (CoreError::InvalidProbability { value: 1.5 }, "ir.invalid_probability"),
        (
            CoreError::DistributionNotNormalized { sum: 0.9, tolerance: 1e-6 },
            "ir.distribution_not_normalized",
        ),
        (CoreError::EmptyQuestions, "ir.empty_questions"),
    ];
    for (error, code) in cases {
        assert_eq!(error.code(), code, "code drift for {code}");
    }
}

#[test]
fn identifiers_expose_str_reference() {
    let candidate = CandidateId::new("qwen").expect("valid");
    assert_eq!(candidate.as_ref(), "qwen");
    let question = QuestionId::new("model").expect("valid");
    assert_eq!(question.as_ref(), "model");
    let node = NodeId::new("rules").expect("valid");
    assert_eq!(node.as_ref(), "rules");
}

#[test]
fn score_question_rejects_empty_text() {
    let levels =
        vec![ScoreLevel::new("easy").expect("valid"), ScoreLevel::new("hard").expect("valid")];
    assert!(matches!(
        ScoreQuestion::new("difficulty", "", levels),
        Err(CoreError::EmptyField { field }) if field == "text"
    ));
}

#[test]
fn boolean_and_score_questions_convert_into_decision_question() {
    let boolean = BooleanQuestion::new("needs_tools", "Does this need tools?").expect("valid");
    assert!(matches!(DecisionQuestion::from(boolean), DecisionQuestion::Boolean(_)));

    let score = ScoreQuestion::new(
        "difficulty",
        "How hard?",
        vec![ScoreLevel::new("easy").expect("valid"), ScoreLevel::new("hard").expect("valid")],
    )
    .expect("valid");
    assert!(matches!(DecisionQuestion::from(score), DecisionQuestion::Score(_)));
}

#[test]
fn request_rejects_more_questions_than_the_limit() {
    let limit = Limits::default().max_questions;
    let questions: Vec<DecisionQuestion> = (0..=limit)
        .map(|index| {
            let id = format!("q{index}");
            DecisionQuestion::Boolean(BooleanQuestion::new(id, "yes or no?").expect("valid"))
        })
        .collect();
    let error = DecisionRequest::new(
        State::from_text("state"),
        questions,
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .expect_err("over-limit question set must be rejected");
    assert!(error.to_string().contains("questions"), "{error}");
}

#[test]
fn fact_value_accessors_return_none_for_mismatched_kinds() {
    assert_eq!(FactValue::Text("x".into()).as_f64(), None);
    assert_eq!(FactValue::Boolean(true).as_f64(), None);
    assert_eq!(FactValue::List(Vec::new()).as_f64(), None);

    assert_eq!(FactValue::Integer(1).as_str(), None);
    assert_eq!(FactValue::Float(0.5).as_str(), None);
    assert_eq!(FactValue::Boolean(true).as_str(), None);
    assert_eq!(FactValue::List(vec!["a".into()]).as_str(), None);

    assert!(!FactValue::Integer(3).list_contains("a"));
    assert!(!FactValue::Text("a".into()).list_contains("a"));
    assert!(!FactValue::Float(0.5).list_contains("a"));
}

#[test]
fn trace_iterates_entries_in_order() {
    let mut trace = DecisionTrace::new();
    trace.push(TraceEntry::new("node_a", [("step", FactValue::Integer(1))]));
    trace.push(TraceEntry::new("node_b", [("step", FactValue::Integer(2))]));

    let collected: Vec<String> = trace.into_iter().map(|entry| entry.node).collect();
    assert_eq!(collected, vec!["node_a".to_owned(), "node_b".to_owned()]);
}

#[test]
fn fact_value_displays_every_variant() {
    assert_eq!(FactValue::Text("modality".into()).to_string(), "modality");
    assert_eq!(FactValue::Integer(-3).to_string(), "-3");
    assert_eq!(FactValue::float(0.25).expect("finite").to_string(), "0.25");
    assert_eq!(FactValue::Boolean(false).to_string(), "false");
    assert_eq!(FactValue::List(vec!["a".into(), "b".into()]).to_string(), "[a, b]");
}

#[test]
fn candidate_from_id_carries_empty_description() {
    let id = CandidateId::new("glm").expect("valid");
    let candidate = Candidate::from(id);
    assert_eq!(candidate.id().as_str(), "glm");
    assert_eq!(candidate.description(), "");
}

#[test]
fn distribution_probability_of_missing_key_is_none() {
    let dist = Distribution::from_pairs([("a", 1.0)]).expect("valid");
    assert_eq!(dist.probability_of("nope"), None);
}

#[test]
fn answer_kind_accessors_report_question_id_and_confidence() {
    use opencodifier_core::DecisionAnswer;
    let answer = DecisionAnswer::Choice {
        question_id: QuestionId::new("model").expect("valid"),
        choice: CandidateId::new("qwen").expect("valid"),
        distribution: Distribution::from_pairs([("qwen", 1.0)]).expect("valid"),
        confidence: 0.9,
    };
    assert_eq!(answer.question_id().as_str(), "model");
    assert!((answer.confidence() - 0.9).abs() < f64::EPSILON);
}
