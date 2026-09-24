//! Fixture locks (PLANNING.md §42, `DECISIONS.md` D10).
//!
//! Every fixture under `fixtures/` is loaded here and driven through its
//! codec: decode, re-encode, and compare against the fixture itself. A
//! change to any wire shape therefore fails a test named after the fixture
//! it broke, and the fixture file is the reviewable record of what we send
//! and accept.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

use std::sync::LazyLock;

use opencodifier_core::{
    Candidate, CandidateId, ChoiceQuestion, ConfidenceReport, DecisionAnswer, DecisionMetrics,
    DecisionOutcome, DecisionPolicy, DecisionQuestion, DecisionRequest, DecisionResponse,
    DecisionTrace, Distribution, Limits, QuestionId, RequestMetadata, ScoreLevel, ScoreQuestion,
    State,
};
use opencodifier_schema::{Native, WireFormat, anthropic::Anthropic, jev::Jev, openai::OpenAi};
use serde_json::{Value, json};

/// The limits every fixture is decoded under.
static LIMITS: LazyLock<Limits> = LazyLock::new(Limits::default);

/// Reads a fixture relative to the repository root.
fn fixture(path: &str) -> String {
    let full = format!("../../fixtures/{path}");
    std::fs::read_to_string(&full)
        .unwrap_or_else(|error| panic!("fixture {full} must be readable: {error}"))
}

/// Reads a fixture as JSON.
fn fixture_value(path: &str) -> Value {
    serde_json::from_str(&fixture(path))
        .unwrap_or_else(|error| panic!("fixture {path} must be valid JSON: {error}"))
}

/// The request the native fixture describes, built with core constructors.
fn native_request() -> DecisionRequest {
    let metadata =
        RequestMetadata { request_id: Some("fixture-0001".to_owned()), limits: Limits::default() };
    DecisionRequest::new(
        State::from_text("route this request across local models")
            .with_fact("context_tokens", opencodifier_core::FactValue::Integer(4_096)),
        vec![
            DecisionQuestion::Choice(
                ChoiceQuestion::new(
                    "model",
                    "Which model should serve this request?",
                    vec![
                        Candidate::new("local-qwen", "General coding and reasoning").unwrap(),
                        Candidate::new("local-glm", "Complex reasoning").unwrap(),
                    ],
                )
                .unwrap(),
            ),
            DecisionQuestion::Score(
                ScoreQuestion::new(
                    "difficulty",
                    "How difficult is this request?",
                    ["trivial", "easy", "moderate", "difficult", "expert"]
                        .iter()
                        .map(|label| ScoreLevel::new(*label).unwrap())
                        .collect(),
                )
                .unwrap(),
            ),
            DecisionQuestion::Boolean(
                opencodifier_core::BooleanQuestion::new(
                    "needs_tools",
                    "Does this request need tools?",
                )
                .unwrap(),
            ),
        ],
        DecisionPolicy::default(),
        metadata,
    )
    .unwrap()
}

/// The response the native fixture describes, built with core constructors.
fn native_response() -> DecisionResponse {
    let choice_distribution =
        Distribution::from_pairs([("local-qwen", 0.91), ("local-glm", 0.09)]).unwrap();
    let score_distribution = Distribution::from_pairs([
        ("trivial", 0.01),
        ("easy", 0.07),
        ("moderate", 0.42),
        ("difficult", 0.38),
        ("expert", 0.12),
    ])
    .unwrap();
    let mut trace = DecisionTrace::new();
    trace.push(opencodifier_core::TraceEntry::new(
        "schema.normalize",
        [
            ("format", opencodifier_core::FactValue::Text("native".to_owned())),
            ("questions_in", opencodifier_core::FactValue::Integer(3)),
        ],
    ));
    DecisionResponse::new(
        vec![
            DecisionAnswer::Choice {
                question_id: QuestionId::new("model").unwrap(),
                choice: CandidateId::new("local-qwen").unwrap(),
                distribution: choice_distribution,
                confidence: 0.91,
            },
            DecisionAnswer::Score {
                question_id: QuestionId::new("difficulty").unwrap(),
                expected: 2.53,
                level: "difficult".to_owned(),
                distribution: score_distribution,
                confidence: 0.71,
            },
            DecisionAnswer::Boolean {
                question_id: QuestionId::new("needs_tools").unwrap(),
                value: true,
                probability: 0.87,
                confidence: 0.87,
            },
        ],
        DecisionOutcome::Accept,
        ConfidenceReport {
            top_probability: 0.91,
            margin: 0.82,
            entropy: 0.2,
            calibrated_confidence: 0.91,
            ood_score: 0.02,
            verifier_agreement: None,
        },
        trace,
        DecisionMetrics {
            candidates_in: 2,
            candidates_out: 2,
            cache_hit: false,
            verification_triggered: false,
        },
    )
    .unwrap()
}

#[test]
fn fixture_native_request_round_trips() {
    let expected = native_request();
    let decoded: DecisionRequest =
        serde_json::from_str(&fixture("native/request.json")).expect("fixture decodes");
    assert_eq!(decoded, expected, "the native fixture must match the IR it claims to describe");

    let encoded = Native.encode_request(&decoded).unwrap();
    assert_eq!(encoded, fixture_value("native/request.json"), "canonical bytes must be locked");

    let round_tripped = Native.decode_request(&encoded, &LIMITS).unwrap();
    assert_eq!(round_tripped, expected);
}

#[test]
fn fixture_native_response_round_trips() {
    let expected = native_response();
    let decoded: DecisionResponse =
        serde_json::from_str(&fixture("native/response.json")).expect("fixture decodes");
    assert_eq!(decoded, expected);

    let encoded = Native.encode_response(&decoded).unwrap();
    assert_eq!(encoded, fixture_value("native/response.json"), "canonical bytes must be locked");

    let round_tripped = Native.decode_response(&encoded, &native_request(), &LIMITS).unwrap();
    assert_eq!(round_tripped, expected);
}

#[test]
fn fixture_openai_choice_enum_format_round_trips() {
    let fixture = fixture_value("openai/choice-enum.format.json");
    let questions = OpenAi
        .decode_schema(&fixture["schema"], &LIMITS)
        .expect("the fixture schema must normalize");
    let request = DecisionRequest::new(
        State::from_text("Select the model to serve this coding request"),
        questions,
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap();
    // Re-encoding the decoded questions must reproduce the fixture exactly.
    let encoded = OpenAi.encode_request(&request).unwrap();
    assert_eq!(&encoded["format"], &fixture, "choice-enum.format.json is byte-locked");

    // The synthetic answer reconstructs per the documented rules.
    let answer = json!({"model": "local-glm"});
    let response = OpenAi.decode_response(&answer, &request, &LIMITS).unwrap();
    let DecisionAnswer::Choice { choice, distribution, .. } = &response.answers()[0] else {
        panic!("expected a choice answer");
    };
    assert_eq!(choice.as_str(), "local-glm");
    assert_eq!(distribution.top().key, "local-glm");
    assert_eq!(response.outcome(), DecisionOutcome::Verify);
}

#[test]
fn fixture_openai_score_bounded_integer_format_round_trips() {
    let fixture = fixture_value("openai/score-bounded-integer.format.json");
    let questions = OpenAi
        .decode_schema(&fixture["schema"], &LIMITS)
        .expect("the fixture schema must normalize");
    let DecisionQuestion::Score(score) = &questions[0] else {
        panic!("expected a score question");
    };
    let labels: Vec<&str> = score.levels().iter().map(ScoreLevel::label).collect();
    assert_eq!(labels, ["trivial", "easy", "moderate", "difficult", "expert"]);

    let request = DecisionRequest::new(
        State::from_text("Refactor the parser module and add regression tests"),
        questions,
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap();
    let encoded = OpenAi.encode_request(&request).unwrap();
    assert_eq!(&encoded["format"], &fixture, "score-bounded-integer.format.json is byte-locked");

    let answer = json!({"difficulty": 3});
    let response = OpenAi.decode_response(&answer, &request, &LIMITS).unwrap();
    let DecisionAnswer::Score { expected, level, .. } = &response.answers()[0] else {
        panic!("expected a score answer");
    };
    assert!((expected - 3.0).abs() < 1e-9);
    assert_eq!(level, "difficult");
}

#[test]
fn fixture_openai_boolean_format_round_trips() {
    let fixture = fixture_value("openai/boolean.format.json");
    let questions = OpenAi
        .decode_schema(&fixture["schema"], &LIMITS)
        .expect("the fixture schema must normalize");
    let request = DecisionRequest::new(
        State::from_text("The user asked to rename every occurrence of the symbol"),
        questions,
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap();
    let encoded = OpenAi.encode_request(&request).unwrap();
    assert_eq!(&encoded["format"], &fixture, "boolean.format.json is byte-locked");

    let response =
        OpenAi.decode_response(&json!({"needs_tools": true}), &request, &LIMITS).unwrap();
    let DecisionAnswer::Boolean { value, probability, .. } = &response.answers()[0] else {
        panic!("expected a boolean answer");
    };
    assert!(*value);
    assert!((probability - 1.0).abs() < 1e-9);
}

#[test]
fn fixture_anthropic_tool_choice_round_trips() {
    let fixture = fixture_value("anthropic/tool-choice.tool.json");
    let request = Anthropic.decode_request(&fixture, &LIMITS).unwrap();
    // Both properties survive, including through the `const`-tolerant path.
    assert_eq!(request.questions().len(), 2);
    let encoded = Anthropic.encode_request(&request).unwrap();
    assert_eq!(&encoded["tools"][0], &fixture, "tool-choice.tool.json is byte-locked");

    let input = json!({"model": "local-qwen", "engine": "sqlite"});
    let response = Anthropic.decode_response(&input, &request, &LIMITS).unwrap();
    assert_eq!(response.answers().len(), 2);
    assert_eq!(Anthropic.encode_response(&response).unwrap(), input);
}

#[test]
fn fixture_anthropic_tool_score_round_trips() {
    let fixture = fixture_value("anthropic/tool-score.tool.json");
    let request = Anthropic.decode_request(&fixture, &LIMITS).unwrap();
    let encoded = Anthropic.encode_request(&request).unwrap();
    assert_eq!(&encoded["tools"][0], &fixture, "tool-score.tool.json is byte-locked");

    let response = Anthropic.decode_response(&json!({"difficulty": 2}), &request, &LIMITS).unwrap();
    let DecisionAnswer::Score { level, .. } = &response.answers()[0] else {
        panic!("expected a score answer");
    };
    assert_eq!(level, "moderate");
}

#[test]
fn fixture_anthropic_tool_boolean_round_trips() {
    let fixture = fixture_value("anthropic/tool-boolean.tool.json");
    let request = Anthropic.decode_request(&fixture, &LIMITS).unwrap();
    let encoded = Anthropic.encode_request(&request).unwrap();
    assert_eq!(&encoded["tools"][0], &fixture, "tool-boolean.tool.json is byte-locked");

    let response =
        Anthropic.decode_response(&json!({"needs_tools": false}), &request, &LIMITS).unwrap();
    let DecisionAnswer::Boolean { value, .. } = &response.answers()[0] else {
        panic!("expected a boolean answer");
    };
    assert!(!*value);
}

#[test]
fn fixture_jev_systemone_choice_request_round_trips() {
    let text = fixture("jev/systemone-choice-request.json");
    let request = Jev.decode_request_str(&text, &LIMITS).unwrap();
    let DecisionQuestion::Choice(choice) = &request.questions()[0] else {
        panic!("expected a choice question");
    };
    assert_eq!(choice.candidates()[0].id().as_str(), "local-qwen");
    assert_eq!(choice.candidates()[0].description(), "General coding and reasoning");
    assert_eq!(
        request.state().fact("model").map(opencodifier_core::FactValue::as_str),
        Some(Some("local-qwen"))
    );

    // Encoding is deterministic, and the encoded form re-decodes to the
    // same questions — compared with candidate order normalized, because a
    // `serde_json::Value` round trip sorts object keys (see the `jev`
    // module docs; the order-preserving path is `decode_request_str`).
    let encoded = Jev.encode_request(&request).unwrap();
    assert_eq!(encoded, Jev.encode_request(&request).unwrap(), "encoding must be deterministic");
    let again = Jev.decode_request(&encoded, &LIMITS).unwrap();
    assert_eq!(
        normalized(again.questions()),
        normalized(request.questions()),
        "every question survives, with normalized candidate order"
    );
}

#[test]
fn fixture_jev_systemone_choice_response_round_trips() {
    let request =
        Jev.decode_request_str(&fixture("jev/systemone-choice-request.json"), &LIMITS).unwrap();
    let fixture = fixture_value("jev/systemone-choice-response.json");
    let response = Jev.decode_response(&fixture, &request, &LIMITS).unwrap();

    let DecisionAnswer::Choice { choice, distribution, .. } = &response.answers()[0] else {
        panic!("expected a choice answer");
    };
    assert_eq!(choice.as_str(), "local-glm", "the fixture reports local-glm");
    assert_eq!(distribution.top().key, "local-glm");

    // Canonical (sorted-key) comparison of the answers object: Jev
    // responses are compared as values, not as raw text.
    let encoded = Jev.encode_response(&response).unwrap();
    assert_eq!(encoded["answers"], fixture["answers"], "choice answers are byte-locked");
}

#[test]
fn fixture_jev_systemone_score_request_round_trips() {
    let text = fixture("jev/systemone-score-request.json");
    let request = Jev.decode_request_str(&text, &LIMITS).unwrap();
    let DecisionQuestion::Score(score) = &request.questions()[0] else {
        panic!("expected a score question");
    };
    // The fixture's criteria are deliberately not in sorted key order: the
    // document order is the level order.
    let labels: Vec<&str> = score.levels().iter().map(ScoreLevel::label).collect();
    assert_eq!(labels, ["trivial", "moderate", "difficult", "expert"]);

    // Level descriptions are not representable in the IR and a
    // `serde_json::Value` does not carry key order, so this fixture is
    // locked as an input (the decode above) rather than byte-for-byte.
    // What must hold: encoding is deterministic, the level *set* survives,
    // and re-decoding the encoded text lands on the sorted-key order that
    // [`Jev::decode_request`] documents.
    let encoded = Jev.encode_request(&request).unwrap();
    assert_eq!(encoded, Jev.encode_request(&request).unwrap(), "encoding must be deterministic");
    let re_decoded = Jev.decode_request(&encoded, &LIMITS).unwrap();
    let DecisionQuestion::Score(re_score) = &re_decoded.questions()[0] else {
        panic!("expected a score question");
    };
    let mut re_labels: Vec<&str> = re_score.levels().iter().map(ScoreLevel::label).collect();
    re_labels.sort_unstable();
    let mut want = labels;
    want.sort_unstable();
    assert_eq!(re_labels, want, "every level survives, in normalized order");
    assert_eq!(
        normalized(re_decoded.questions()),
        normalized(request.questions()),
        "the normalized projection matches the decoded fixture"
    );
}

#[test]
fn fixture_jev_systemone_score_response_round_trips() {
    let request =
        Jev.decode_request_str(&fixture("jev/systemone-score-request.json"), &LIMITS).unwrap();
    let fixture = fixture_value("jev/systemone-score-response.json");
    let response = Jev.decode_response(&fixture, &request, &LIMITS).unwrap();

    let DecisionAnswer::Score { expected, level, distribution, .. } = &response.answers()[0] else {
        panic!("expected a score answer");
    };
    // 0.05*0 + 0.24*1 + 0.46*2 + 0.25*3 = 1.91 -> "moderate" (index 1).
    assert!((expected - 1.91).abs() < 1e-9, "{expected}");
    assert_eq!(level, "moderate");
    assert_eq!(distribution.entries().len(), 4);

    let encoded = Jev.encode_response(&response).unwrap();
    assert_eq!(encoded["answers"], fixture["answers"], "score answers are byte-locked");
}

#[test]
fn fixture_jev_systemone_noul_request_round_trips() {
    let text = fixture("jev/systemone-noul-request.json");
    let request = Jev.decode_request_str(&text, &LIMITS).unwrap();
    assert_eq!(
        request.questions()[0],
        DecisionQuestion::Boolean(
            opencodifier_core::BooleanQuestion::new(
                "needs_tools",
                "Does this request require tool access?"
            )
            .unwrap()
        )
    );
    let encoded = Jev.encode_request(&request).unwrap();
    assert_eq!(
        encoded["questions"]["needs_tools"]["type"],
        serde_json::json!("noul"),
        "noul projects to the Jev boolean discriminator"
    );
}

#[test]
fn fixture_jev_systemone_noul_response_round_trips() {
    let request =
        Jev.decode_request_str(&fixture("jev/systemone-noul-request.json"), &LIMITS).unwrap();
    let fixture = fixture_value("jev/systemone-noul-response.json");
    let response = Jev.decode_response(&fixture, &request, &LIMITS).unwrap();

    let DecisionAnswer::Boolean { value, probability, .. } = &response.answers()[0] else {
        panic!("expected a boolean answer");
    };
    assert!(*value, "0.87 >= 0.5 is true");
    assert!((probability - 0.87).abs() < 1e-9);

    let encoded = Jev.encode_response(&response).unwrap();
    assert_eq!(encoded["answers"], fixture["answers"], "noul answers are byte-locked");
}

/// A question projected to `(id, text, sorted member pairs)` so ordering
/// stops mattering.
type ProjectedQuestion = (String, String, Vec<(String, String)>);

/// Projects questions onto an order-insensitive form.
///
/// `serde_json::Value` sorts object keys, so a Jev request that passes
/// through [`WireFormat::decode_request`] loses `criteria` document order
/// (the level order for `score`, the candidate order for `choice`). The IR
/// question order is a list and is preserved; only the per-question lists
/// are normalized here.
fn normalized(questions: &[DecisionQuestion]) -> Vec<ProjectedQuestion> {
    questions
        .iter()
        .map(|question| {
            let members = match question {
                DecisionQuestion::Choice(choice) => choice
                    .candidates()
                    .iter()
                    .map(|candidate| {
                        (candidate.id().as_str().to_owned(), candidate.description().to_owned())
                    })
                    .collect::<Vec<_>>(),
                DecisionQuestion::Score(score) => score
                    .levels()
                    .iter()
                    .map(|level| (level.label().to_owned(), String::new()))
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            };
            let mut members = members;
            members.sort();
            (question.id().as_str().to_owned(), question.text().to_owned(), members)
        })
        .collect()
}

/// Every fixture named in the README must exist, and every fixture on disk
/// must be named in the README.
#[test]
fn readme_names_every_fixture() {
    let readme = fixture("README.md");
    let mut on_disk: Vec<String> = Vec::new();
    for (format, files) in [
        ("native", &["native/request.json", "native/response.json"][..]),
        (
            "openai",
            &[
                "openai/choice-enum.format.json",
                "openai/score-bounded-integer.format.json",
                "openai/boolean.format.json",
            ][..],
        ),
        (
            "anthropic",
            &[
                "anthropic/tool-choice.tool.json",
                "anthropic/tool-score.tool.json",
                "anthropic/tool-boolean.tool.json",
            ][..],
        ),
        (
            "jev",
            &[
                "jev/systemone-choice-request.json",
                "jev/systemone-choice-response.json",
                "jev/systemone-score-request.json",
                "jev/systemone-score-response.json",
                "jev/systemone-noul-request.json",
                "jev/systemone-noul-response.json",
            ][..],
        ),
    ] {
        for file in files {
            on_disk.push((*file).to_owned());
            assert!(
                readme.contains(file),
                "fixtures/README.md must name {file} and the test that locks it ({format})"
            );
        }
    }
    assert_eq!(on_disk.len(), 14, "every fixture file is catalogued");
}
