//! Hostile-input serialization contract: deserialization enforces exactly
//! the same invariants as construction (PLANNING.md §73 "treat all input
//! as hostile").
//!
//! Every payload here is syntactically valid JSON that would previously
//! have bypassed constructor validation via derived `Deserialize`. Each
//! must be rejected with the constructor's own error semantics.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

use opencodifier_core::{
    CandidateId, ChoiceQuestion, DecisionAnswer, DecisionPolicy, DecisionQuestion, DecisionRequest,
    DecisionResponse, Distribution, DistributionEntry, FactValue, Limits, QuestionId,
    RequestMetadata, RiskLevel,
};

#[test]
fn unnormalized_distribution_is_rejected() {
    let bad = r#"{"entries":[{"key":"a","probability":0.9},{"key":"b","probability":0.9}]}"#;
    assert!(serde_json::from_str::<Distribution>(bad).is_err());
    // A single entry carrying 0.9 is unnormalized (sum must be 1).
    let short = r#"{"entries":[{"key":"a","probability":0.9}]}"#;
    assert!(serde_json::from_str::<Distribution>(short).is_err());
}

#[test]
fn out_of_range_entry_probability_is_rejected() {
    let entry = r#"{"key":"a","probability":1.5}"#;
    assert!(serde_json::from_str::<DistributionEntry>(entry).is_err());
    let negative = r#"{"key":"a","probability":-0.1}"#;
    assert!(serde_json::from_str::<DistributionEntry>(negative).is_err());
    let nan = r#"{"key":"a","probability":null}"#;
    assert!(serde_json::from_str::<DistributionEntry>(nan).is_err());
}

#[test]
fn non_finite_fact_floats_are_rejected() {
    // serde_json parses overflowing literals as infinity.
    let infinite = r#"{"kind":"float","value":1e999}"#;
    assert!(serde_json::from_str::<FactValue>(infinite).is_err());
    let negative_infinite = r#"{"kind":"float","value":-1e999}"#;
    assert!(serde_json::from_str::<FactValue>(negative_infinite).is_err());
    // Finite floats remain fine.
    let fine = r#"{"kind":"float","value":0.25}"#;
    assert!(serde_json::from_str::<FactValue>(fine).is_ok());
}

#[test]
fn identifiers_are_validated_on_deserialize() {
    for bad in ["", "has space", "slash/no", "é"] {
        let payload = serde_json::to_string(bad).expect("json string");
        assert!(
            serde_json::from_str::<CandidateId>(&payload).is_err(),
            "candidate id {bad:?} must be rejected"
        );
        assert!(
            serde_json::from_str::<QuestionId>(&payload).is_err(),
            "question id {bad:?} must be rejected"
        );
    }
}

#[test]
fn choice_question_invariants_survive_deserialization() {
    let empty = r#"{"id":"model","text":"pick","candidates":[]}"#;
    assert!(serde_json::from_str::<ChoiceQuestion>(empty).is_err());
    let duplicate = concat!(
        r#"{"id":"model","text":"pick","candidates":"#,
        r#"[{"id":"qwen","description":"one"},{"id":"qwen","description":"two"}]}"#
    );
    assert!(serde_json::from_str::<ChoiceQuestion>(duplicate).is_err());
    let empty_text = r#"{"id":"model","text":"","candidates":[{"id":"qwen","description":"d"}]}"#;
    assert!(serde_json::from_str::<ChoiceQuestion>(empty_text).is_err());
}

#[test]
fn score_question_rejects_single_level_via_json() {
    let one_level = r#"{"id":"difficulty","text":"how hard?","levels":[{"label":"easy"}]}"#;
    assert!(serde_json::from_str::<DecisionQuestion>(one_level).is_err());
    let duplicate =
        r#"{"id":"difficulty","text":"how hard?","levels":[{"label":"easy"},{"label":"easy"}]}"#;
    assert!(serde_json::from_str::<DecisionQuestion>(duplicate).is_err());
}

#[test]
fn contradictory_policy_is_rejected_via_json() {
    let inverted = r#"{"min_confidence":0.5,"verify_below":0.9,"abstain_below":0.5,"risk":"low"}"#;
    assert!(serde_json::from_str::<DecisionPolicy>(inverted).is_err());
    let out_of_range =
        r#"{"min_confidence":1.5,"verify_below":0.9,"abstain_below":0.5,"risk":"low"}"#;
    assert!(serde_json::from_str::<DecisionPolicy>(out_of_range).is_err());
}

#[test]
fn answer_confidence_invariants_survive_deserialization() {
    let bad_confidence = concat!(
        r#"{"type":"choice","question_id":"model","choice":"qwen","#,
        r#""distribution":{"entries":[{"key":"qwen","probability":1.0}]},"#,
        r#""confidence":1.5}"#
    );
    assert!(serde_json::from_str::<DecisionAnswer>(bad_confidence).is_err());
    let bad_probability = concat!(
        r#"{"type":"boolean","question_id":"needs_tools","value":true,"#,
        r#""probability":1.2,"confidence":0.9}"#
    );
    assert!(serde_json::from_str::<DecisionAnswer>(bad_probability).is_err());
}

#[test]
fn request_rejects_over_limit_and_empty_question_sets() {
    let no_questions = format!(
        r#"{{"state":{{"text":"x","facts":{{}}}},"questions":[],
            "policy":{},"metadata":{}}}"#,
        serde_json::to_string(&DecisionPolicy::default()).expect("policy"),
        serde_json::to_string(&RequestMetadata::default()).expect("metadata"),
    );
    assert!(serde_json::from_str::<DecisionRequest>(&no_questions).is_err());

    // Hand-written wire payload with one candidate more than
    // `Limits::max_candidates` allows — construction would reject this
    // too, but the wire path must reject it just the same.
    let limit = Limits::default().max_candidates;
    let candidates: Vec<String> =
        (0..=limit).map(|index| format!(r#"{{"id":"c{index}","description":"d"}}"#)).collect();
    let over_limit = format!(
        r#"{{"state":{{"text":"x","facts":{{}}}},"questions":[
            {{"type":"choice","id":"model","text":"pick","candidates":[{}]}}],
            "policy":{},"metadata":{}}}"#,
        candidates.join(","),
        serde_json::to_string(&DecisionPolicy::default()).expect("policy"),
        serde_json::to_string(&RequestMetadata::default()).expect("metadata"),
    );
    assert!(
        serde_json::from_str::<DecisionRequest>(&over_limit).is_err(),
        "a request over the candidate limit must not deserialize"
    );
}

#[test]
fn decisive_response_requires_answers_even_via_json() {
    // Hand-written wire payload: decisive outcome with an empty answer
    // list — the exact shape construction rejects.
    let decisive_without_answers = concat!(
        r#"{"answers":[],"outcome":"accept","#,
        r#""confidence":{"top_probability":0.9,"margin":0.9,"entropy":0.0,"#,
        r#""calibrated_confidence":0.9,"ood_score":0.0,"verifier_agreement":null},"#,
        r#""trace":{"trace_version":1,"entries":[]},"#,
        r#""metrics":{"candidates_in":0,"candidates_out":0,"cache_hit":false,"#,
        r#""verification_triggered":false}}"#
    );
    assert!(serde_json::from_str::<DecisionResponse>(decisive_without_answers).is_err());

    // The abstaining shape with no answers is legitimate.
    let abstain =
        decisive_without_answers.replace(r#""outcome":"accept""#, r#""outcome":"abstain""#);
    assert!(serde_json::from_str::<DecisionResponse>(&abstain).is_ok());
}

#[test]
fn valid_payloads_still_round_trip_with_unchanged_wire_shape() {
    let dist = Distribution::from_pairs([("qwen", 0.91), ("glm", 0.09)]).expect("valid");
    let json = serde_json::to_string(&dist).expect("serialize");
    assert_eq!(
        json,
        r#"{"entries":[{"key":"qwen","probability":0.91},{"key":"glm","probability":0.09}]}"#
    );
    let back: Distribution = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, dist);

    let policy = DecisionPolicy::new(0.8, 0.65, 0.5, RiskLevel::High).expect("valid");
    let policy_json = serde_json::to_string(&policy).expect("serialize");
    let policy_back: DecisionPolicy = serde_json::from_str(&policy_json).expect("deserialize");
    assert_eq!(policy_back, policy);
}
