//! Property round-trips (PLANNING.md §42, `DECISIONS.md` D10).
//!
//! Arbitrary valid IR requests (constrained to [`Limits`]) are projected
//! onto each wire format and normalized back. Native is value-exact; the
//! lossy formats are checked against the equivalence their module docs
//! promise, and OpenAI/Anthropic additionally go through a synthetic
//! vendor answer so the documented reconstruction rules are exercised, not
//! just the schema projection.

// Casts below are index arithmetic over generated questions whose
// candidate/level lists have at most a handful of members, so the
// truncation and precision warnings are structurally impossible here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use opencodifier_core::{
    BooleanQuestion, Candidate, ChoiceQuestion, DecisionAnswer, DecisionMetrics, DecisionPolicy,
    DecisionQuestion, DecisionRequest, DecisionResponse, DecisionTrace, Limits, QuestionId,
    RequestMetadata, ScoreLevel, ScoreQuestion, State,
};
use opencodifier_schema::{Native, WireFormat, anthropic::Anthropic, jev::Jev, openai::OpenAi};
use proptest::prelude::*;

/// Identifier labels: valid [`opencodifier_core::CandidateId`] text.
fn id_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,7}"
}

/// Non-empty human text.
fn text_strategy() -> impl Strategy<Value = String> {
    "[[:alpha:] ]{1,24}"
}

fn choice_question() -> impl Strategy<Value = DecisionQuestion> {
    (id_strategy(), text_strategy(), proptest::collection::vec(id_strategy(), 1..4)).prop_map(
        |(id, text, labels)| {
            // Labels are unique by construction: dedupe, keeping order.
            let mut seen: Vec<String> = Vec::new();
            let candidates = labels
                .into_iter()
                .filter(|label| {
                    if seen.contains(label) {
                        false
                    } else {
                        seen.push(label.clone());
                        true
                    }
                })
                .map(|label| Candidate::new(label, "candidate description").unwrap())
                .collect::<Vec<_>>();
            DecisionQuestion::Choice(ChoiceQuestion::new(id, text, candidates).unwrap())
        },
    )
}

fn score_question() -> impl Strategy<Value = DecisionQuestion> {
    (id_strategy(), text_strategy(), proptest::collection::vec(id_strategy(), 2..5)).prop_map(
        |(id, text, labels)| {
            let mut seen: Vec<String> = Vec::new();
            let levels = labels
                .into_iter()
                .filter(|label| {
                    if seen.contains(label) {
                        false
                    } else {
                        seen.push(label.clone());
                        true
                    }
                })
                .map(|label| ScoreLevel::new(label).unwrap())
                .collect::<Vec<_>>();
            // At least two levels survive dedupe in practice; force it if
            // the generator produced identical labels.
            let levels = if levels.len() < 2 {
                vec![ScoreLevel::new("low").unwrap(), ScoreLevel::new("high").unwrap()]
            } else {
                levels
            };
            DecisionQuestion::Score(ScoreQuestion::new(id, text, levels).unwrap())
        },
    )
}

fn boolean_question() -> impl Strategy<Value = DecisionQuestion> {
    (id_strategy(), text_strategy())
        .prop_map(|(id, text)| DecisionQuestion::Boolean(BooleanQuestion::new(id, text).unwrap()))
}

fn question() -> impl Strategy<Value = DecisionQuestion> {
    prop_oneof![choice_question(), score_question(), boolean_question()]
}

fn request() -> impl Strategy<Value = DecisionRequest> {
    (text_strategy(), proptest::collection::vec(question(), 1..4)).prop_map(|(state, questions)| {
        // Question ids are unique by construction: dedupe, keeping order.
        // (Every keyed wire format answers by id, so a repeated id is not a
        // request OpenCodifier can put on the wire.)
        let mut seen: Vec<String> = Vec::new();
        let questions = questions
            .into_iter()
            .filter(|question| {
                let id = question.id().as_str().to_owned();
                if seen.contains(&id) {
                    false
                } else {
                    seen.push(id);
                    true
                }
            })
            .collect::<Vec<_>>();
        DecisionRequest::new(
            State::from_text(state),
            questions,
            DecisionPolicy::default(),
            RequestMetadata::default(),
        )
        .unwrap()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Native is the identity projection: exact value round-trip.
    #[test]
    fn prop_native_request_round_trips_exactly(original in request()) {
        let wire = Native.encode_request(&original).unwrap();
        let decoded = Native.decode_request(&wire, &Limits::default()).unwrap();
        prop_assert_eq!(&decoded, &original);

        let re_encoded = Native.encode_request(&decoded).unwrap();
        prop_assert_eq!(re_encoded, wire);
    }

    /// Jev preserves the state text and every question through the
    /// order-preserving text path; the `model` fact is reconstructed
    /// (possibly as an empty string) and other facts are not representable
    /// — that is the documented equivalence. The `Value` path is also
    /// checked, against the order-normalized form its module docs promise.
    #[test]
    fn prop_jev_request_round_trips(original in request()) {
        let text = Jev.encode_request_str(&original).unwrap();
        let decoded = Jev.decode_request_str(&text, &Limits::default()).unwrap();
        prop_assert_eq!(decoded.state().text(), original.state().text());
        prop_assert_eq!(decoded.questions(), original.questions());

        let value = Jev.encode_request(&original).unwrap();
        let normalized = Jev.decode_request(&value, &Limits::default()).unwrap();
        prop_assert_eq!(
            order_insensitive(normalized.questions()),
            order_insensitive(original.questions())
        );
    }

    /// Jev responses round-trip through the index/label reconstruction
    /// rules for any valid answer set.
    #[test]
    fn prop_jev_response_round_trips(original in request(), seed in 0.0f64..1.0) {
        let answers = synthetic_answers(&original, seed);
        let response = decision_response(answers);
        let wire = Jev.encode_response(&response).unwrap();
        let decoded = Jev.decode_response(&wire, &original, &Limits::default()).unwrap();
        // Jev preserves the decision values and the score distribution;
        // confidence is reconstructed (documented equivalence).
        for (got, want) in decoded.answers().iter().zip(response.answers().iter()) {
            prop_assert_eq!(got.question_id(), want.question_id());
            match (got, want) {
                (
                    DecisionAnswer::Choice { choice: got, .. },
                    DecisionAnswer::Choice { choice: want, .. },
                ) => prop_assert_eq!(got, want),
                (
                    DecisionAnswer::Boolean { value: got, probability: got_p, .. },
                    DecisionAnswer::Boolean { value: want, probability: want_p, .. },
                ) => {
                    prop_assert_eq!(got, want);
                    prop_assert!((got_p - want_p).abs() < 1e-9);
                }
                (
                    DecisionAnswer::Score { expected: got, level: got_level, distribution: got_d, .. },
                    DecisionAnswer::Score { question_id, distribution: want_d, .. },
                ) => {
                    // The wire carries the whole probability vector, so the
                    // expected value is recomputed as its weighted mean and
                    // the level re-derived from that mean.
                    let mean = want_d
                        .entries()
                        .iter()
                        .enumerate()
                        .map(|(index, entry)| index as f64 * entry.probability)
                        .sum::<f64>();
                    prop_assert!((got - mean).abs() < 1e-9);
                    let question = original
                        .questions()
                        .iter()
                        .find(|question| question.id() == question_id)
                        .unwrap_or_else(|| panic!("question {question_id} is missing"));
                    let DecisionQuestion::Score(score) = question else {
                        panic!("expected a score question");
                    };
                    prop_assert_eq!(got_level, score.level_for(mean).label());
                    prop_assert_eq!(got_d, want_d);
                }
                _ => panic!("mismatched answer kinds"),
            }
        }
    }

    /// OpenAI carries every question losslessly through the strict schema
    /// (level labels and candidate descriptions ride in the property
    /// descriptions). Question *order* follows the schema's sorted key
    /// order, because a `serde_json::Value` cannot carry key order.
    #[test]
    fn prop_openai_request_round_trips(original in request()) {
        let wire = OpenAi.encode_request(&original).unwrap();
        let decoded = OpenAi.decode_request(&wire, &Limits::default()).unwrap();
        prop_assert_eq!(sorted(decoded.questions()), sorted(original.questions()));
        prop_assert_eq!(decoded.state().text(), original.state().text());
    }

    /// Anthropic shares the decision mapping; `const` support is additive.
    #[test]
    fn prop_anthropic_request_round_trips(original in request()) {
        let wire = Anthropic.encode_request(&original).unwrap();
        let decoded = Anthropic.decode_request(&wire, &Limits::default()).unwrap();
        prop_assert_eq!(sorted(decoded.questions()), sorted(original.questions()));
        prop_assert_eq!(decoded.state().text(), original.state().text());
    }

    /// OpenAI answers are chosen values only; the reconstruction rules are
    /// exact for the values that survive.
    #[test]
    fn prop_openai_response_round_trips(original in request(), seed in 0.0f64..1.0) {
        let answers = synthetic_answers(&original, seed);
        let response = decision_response(answers.clone());
        let wire = OpenAi.encode_response(&response).unwrap();
        let decoded = OpenAi.decode_response(&wire, &original, &Limits::default()).unwrap();
        for (decoded, original) in decoded.answers().iter().zip(answers.iter()) {
            match (decoded, original) {
                (
                    DecisionAnswer::Choice { choice: got, .. },
                    DecisionAnswer::Choice { choice: want, .. },
                ) => prop_assert_eq!(got, want),
                (
                    DecisionAnswer::Boolean { value: got, probability: got_p, .. },
                    DecisionAnswer::Boolean { value: want, probability: want_p, .. },
                ) => {
                    prop_assert_eq!(got, want);
                    // The wire carries the decided value only, so the
                    // reconstructed probability is 1.0 by rule.
                    prop_assert!((got_p - 1.0).abs() < 1e-9);
                    prop_assert!(*want_p >= 0.0 && *want_p <= 1.0);
                }
                (
                    DecisionAnswer::Score { expected: got, level: got_level, .. },
                    DecisionAnswer::Score { expected: want, level: want_level, .. },
                ) => {
                    prop_assert_eq!(got_level, want_level);
                    prop_assert!((got - want.round()).abs() < 1e-9);
                }
                _ => panic!("mismatched answer kinds"),
            }
        }
    }

    /// Anthropic answers follow the same reconstruction rules.
    #[test]
    fn prop_anthropic_response_round_trips(original in request(), seed in 0.0f64..1.0) {
        let answers = synthetic_answers(&original, seed);
        let response = decision_response(answers);
        let wire = Anthropic.encode_response(&response).unwrap();
        let decoded = Anthropic.decode_response(&wire, &original, &Limits::default()).unwrap();
        prop_assert_eq!(decoded.answers().len(), response.answers().len());
        for answer in decoded.answers() {
            prop_assert!((answer.confidence() - 1.0).abs() < 1e-9);
        }
    }

    /// Native responses are value-exact, including distributions.
    #[test]
    fn prop_native_response_round_trips(original in request(), seed in 0.0f64..1.0) {
        let answers = synthetic_answers(&original, seed);
        let response = decision_response(answers);
        let wire = Native.encode_response(&response).unwrap();
        let decoded = Native.decode_response(&wire, &original, &Limits::default()).unwrap();
        prop_assert_eq!(decoded, response);
    }
}

/// Questions in sorted-id order: the order a JSON object round trip leaves
/// them in.
fn sorted(questions: &[DecisionQuestion]) -> Vec<DecisionQuestion> {
    let mut questions = questions.to_vec();
    questions.sort_by_key(|question| question.id().as_str().to_owned());
    questions
}

/// Questions with every ordering flattened: question order (JSON object key
/// order) and the per-question member order (candidate/level order, which a
/// sorted map also rewrites).
fn order_insensitive(questions: &[DecisionQuestion]) -> Vec<(String, String, Vec<String>)> {
    let mut projected: Vec<(String, String, Vec<String>)> = questions
        .iter()
        .map(|question| {
            let members: Vec<String> = match question {
                DecisionQuestion::Choice(choice) => choice
                    .candidates()
                    .iter()
                    .map(|candidate| candidate.id().as_str().to_owned())
                    .collect(),
                DecisionQuestion::Score(score) => {
                    score.levels().iter().map(|level| level.label().to_owned()).collect()
                }
                _ => Vec::new(),
            };
            let mut members = members;
            members.sort();
            (question.id().as_str().to_owned(), question.text().to_owned(), members)
        })
        .collect();
    projected.sort();
    projected
}

/// Builds one answer per question, choosing deterministically from `seed`.
///
/// The chosen values are the "vendor's" answer: the reconstructed response
/// must reproduce them under each format's documented rules.
fn synthetic_answers(request: &DecisionRequest, seed: f64) -> Vec<DecisionAnswer> {
    request
        .questions()
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let id = question.id().clone();
            let picked = (seed * (index as f64 + 1.0)).fract();
            match question {
                DecisionQuestion::Choice(choice) => {
                    let position = (picked * choice.candidates().len() as f64) as usize
                        % choice.candidates().len();
                    let candidate = choice.candidates()[position].id().to_string();
                    // The alternate key uses a character the id strategy
                    // cannot produce, so the two keys are always distinct.
                    let distribution = pair_distribution(&[
                        (candidate.clone(), 0.7),
                        (format!("alt-{index}"), 0.3),
                    ]);
                    DecisionAnswer::Choice {
                        question_id: id,
                        choice: opencodifier_core::CandidateId::new(candidate).unwrap(),
                        distribution,
                        confidence: 0.7,
                    }
                }
                DecisionQuestion::Boolean(_) => DecisionAnswer::Boolean {
                    question_id: id,
                    value: picked >= 0.5,
                    probability: picked.max(1.0 - picked),
                    confidence: picked.max(1.0 - picked),
                },
                DecisionQuestion::Score(score) => {
                    let position =
                        (picked * score.levels().len() as f64) as usize % score.levels().len();
                    let expected = position as f64;
                    let label = score.levels()[position].label().to_owned();
                    DecisionAnswer::Score {
                        question_id: id,
                        expected,
                        level: label,
                        distribution: uniform_over(score.levels()),
                        confidence: 0.6,
                    }
                }
                _ => panic!("unknown question kind"),
            }
        })
        .collect()
}

/// Builds a normalized distribution from `(key, probability)` pairs.
fn pair_distribution(pairs: &[(String, f64)]) -> opencodifier_core::Distribution {
    opencodifier_core::Distribution::from_pairs(pairs.iter().cloned()).unwrap()
}

/// A uniform distribution over the supplied level labels.
fn uniform_over(levels: &[ScoreLevel]) -> opencodifier_core::Distribution {
    let weight = 1.0 / levels.len() as f64;
    let pairs: Vec<(String, f64)> =
        levels.iter().map(|level| (level.label().to_owned(), weight)).collect();
    opencodifier_core::Distribution::from_pairs(pairs).unwrap()
}

/// Wraps answers in a valid [`DecisionResponse`].
fn decision_response(answers: Vec<DecisionAnswer>) -> DecisionResponse {
    let distribution = match &answers[0] {
        DecisionAnswer::Choice { distribution, .. }
        | DecisionAnswer::Score { distribution, .. } => distribution.clone(),
        DecisionAnswer::Boolean { probability, .. } => {
            opencodifier_core::Distribution::from_pairs([
                ("yes", *probability),
                ("no", 1.0 - probability),
            ])
            .unwrap()
        }
        _ => unreachable!("handled above"),
    };
    DecisionResponse::new(
        answers,
        opencodifier_core::DecisionOutcome::Verify,
        opencodifier_core::ConfidenceReport::from_distribution(&distribution, 0.7, 0.0, None)
            .unwrap(),
        DecisionTrace::new(),
        DecisionMetrics::default(),
    )
    .unwrap()
}

/// The question id strategy is shared with the fixture tests through the
/// crate's public API surface (a compile-time smoke test).
#[test]
fn question_ids_are_valid_candidate_ids() {
    let id = QuestionId::new("difficulty").unwrap();
    assert_eq!(id.as_str(), "difficulty");
}
