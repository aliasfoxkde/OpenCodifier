//! The native `OpenCodifier` codec: the canonical IR's own JSON projection.
//!
//! This is not a foreign wire format at all — it is the IR serialized
//! through the `serde` derives on [`opencodifier_core`] types (PLANNING.md
//! §6). It exists as a [`WireFormat`] so callers can treat every ingress
//! identically and so the canonical form itself is fixture-locked.
//!
//! # Differences from the foreign adapters
//!
//! * **Strict fields.** Foreign adapters ignore unknown fields for forward
//!   compatibility; the native codec deliberately does not. It goes
//!   through the core types' own deserializers, and the IR is the internal
//!   truth — a misspelled field name here is a caller bug, not a vendor
//!   schema drift, so failing loudly is correct (`ARCHITECTURE.md` §7).
//! * **No reconstruction.** Native payloads carry full distributions,
//!   confidence reports, traces, and metrics, so round-trips are
//!   value-exact — no documented fidelity loss, nothing inferred.
//! * **Validation runs twice by design.** Decoding deserializes the IR
//!   types and then re-validates through [`opencodifier_core`]'s
//!   validating constructors, so a hand-written payload (not produced by
//!   `serde`) still cannot smuggle in an unnormalized distribution.
//!
//! # Example
//!
//! ```
//! use opencodifier_core::{
//!     BooleanQuestion, Candidate, ChoiceQuestion, DecisionQuestion, DecisionRequest, Limits,
//!     RequestMetadata, State,
//! };
//! use opencodifier_schema::WireFormat;
//! use opencodifier_schema::native::Native;
//!
//! let request = DecisionRequest::new(
//!     State::from_text("ship the release"),
//!     vec![
//!         DecisionQuestion::Choice(
//!             ChoiceQuestion::new(
//!                 "model",
//!                 "Which model?",
//!                 vec![
//!                     Candidate::new("qwen", "fast").unwrap(),
//!                     Candidate::new("glm", "deep").unwrap(),
//!                 ],
//!             )
//!             .unwrap(),
//!         ),
//!         DecisionQuestion::Boolean(BooleanQuestion::new("needs_tools", "Tools?").unwrap()),
//!     ],
//!     opencodifier_core::DecisionPolicy::default(),
//!     RequestMetadata { request_id: None, limits: Limits::default() },
//! )
//! .unwrap();
//!
//! let encoded = Native.encode_request(&request).unwrap();
//! let decoded =
//!     Native.decode_request(&encoded, &opencodifier_core::Limits::default()).unwrap();
//! assert_eq!(decoded, request);
//! ```

use serde_json::Value;

use opencodifier_core::{DecisionRequest, DecisionResponse, Limits};

use crate::codec::WireFormat;
use crate::error::{SchemaError, SchemaResult};

/// Identity codec over the canonical IR's own serde representation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Native;

impl WireFormat for Native {
    fn name(&self) -> &'static str {
        "native"
    }

    fn encode_request(&self, request: &DecisionRequest) -> SchemaResult<Value> {
        serde_json::to_value(request).map_err(|error| SchemaError::Json(error.to_string()))
    }

    fn decode_request(&self, payload: &Value, limits: &Limits) -> SchemaResult<DecisionRequest> {
        let request: DecisionRequest = serde_json::from_value(payload.clone())
            .map_err(|error| SchemaError::invalid_value("request", error.to_string()))?;
        reject_unknown_fields(
            payload,
            &serde_json::to_value(&request)
                .map_err(|error| SchemaError::Json(error.to_string()))?,
        )?;
        check_limits(&request, limits)?;
        // Re-validate through the IR constructors so a payload written by
        // hand rather than by `serde` still cannot carry an unnormalized
        // distribution or contradictory policy.
        DecisionRequest::new(
            request.state().clone(),
            request.questions().to_vec(),
            request.policy().clone(),
            request.metadata().clone(),
        )
        .map_err(SchemaError::from)
    }

    fn encode_response(&self, response: &DecisionResponse) -> SchemaResult<Value> {
        serde_json::to_value(response).map_err(|error| SchemaError::Json(error.to_string()))
    }

    fn decode_response(
        &self,
        payload: &Value,
        _request: &DecisionRequest,
        _limits: &Limits,
    ) -> SchemaResult<DecisionResponse> {
        let response: DecisionResponse = serde_json::from_value(payload.clone())
            .map_err(|error| SchemaError::invalid_value("response", error.to_string()))?;
        reject_unknown_fields(
            payload,
            &serde_json::to_value(&response)
                .map_err(|error| SchemaError::Json(error.to_string()))?,
        )?;
        // Re-validate through the IR constructors so a hand-written answer
        // cannot carry an unnormalized distribution or out-of-range
        // confidence. Answers validate their probabilities but not their
        // distribution's total mass, so the distributions are rebuilt here
        // through the validating constructor.
        for answer in response.answers() {
            revalidate_distribution(answer)?;
        }
        DecisionResponse::new(
            response.answers().to_vec(),
            response.outcome(),
            response.confidence().clone(),
            response.trace().clone(),
            *response.metrics(),
        )
        .map_err(SchemaError::from)
    }
}

/// Rejects keys the canonical IR does not have (`ARCHITECTURE.md` §7).
///
/// Native is `OpenCodifier`'s own format: unlike the foreign adapters it does
/// not have to tolerate vendor extension, so an unrecognized field is far
/// more likely to be a typo or a forged payload than a forward-compatible
/// addition. The check compares key sets recursively against a re-encoding
/// of the value just decoded, so it can never drift from the IR's own
/// serialization.
fn reject_unknown_fields(payload: &Value, canonical: &Value) -> SchemaResult<()> {
    match (payload, canonical) {
        (Value::Object(payload), Value::Object(canonical)) => {
            for (key, value) in payload {
                let Some(expected) = canonical.get(key) else {
                    return Err(SchemaError::invalid_value(
                        key,
                        format!("unknown field `{key}` in a native payload"),
                    ));
                };
                reject_unknown_fields(value, expected)?;
            }
        }
        (Value::Array(payload), Value::Array(canonical)) => {
            for (value, expected) in payload.iter().zip(canonical.iter()) {
                reject_unknown_fields(value, expected)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Rebuilds an answer's distribution through the validating constructor.
///
/// [`opencodifier_core::DecisionAnswer::validate`] checks probabilities but
/// not the distribution's total mass, and the native format promises the IR
/// is normalized — so a payload that sums to 0.5 is refused here rather
/// than accepted and mis-reported downstream.
fn revalidate_distribution(answer: &opencodifier_core::DecisionAnswer) -> SchemaResult<()> {
    let distribution = match answer {
        opencodifier_core::DecisionAnswer::Choice { distribution, .. }
        | opencodifier_core::DecisionAnswer::Score { distribution, .. } => distribution,
        opencodifier_core::DecisionAnswer::Boolean { .. } => return Ok(()),
        _ => return Err(SchemaError::unsupported("answer type")),
    };
    let entries = distribution.entries().to_vec();
    opencodifier_core::Distribution::new(entries)
        .map(|_| ())
        .map_err(|error| SchemaError::invalid_value("distribution", error.to_string()))
}

/// The native payload declares its own limits in `metadata.limits`; the
/// caller-supplied `limits` ceiling is the outer bound, and the decoded
/// request must satisfy both.
fn check_limits(request: &DecisionRequest, limits: &Limits) -> SchemaResult<()> {
    let payload_limits = &request.metadata().limits;
    if request.state().text().len() > limits.max_input_bytes {
        return Err(SchemaError::limit(
            "max_input_bytes",
            request.state().text().len(),
            limits.max_input_bytes,
        ));
    }
    if request.questions().len() > limits.max_questions {
        return Err(SchemaError::limit(
            "max_questions",
            request.questions().len(),
            limits.max_questions,
        ));
    }
    for question in request.questions() {
        if let opencodifier_core::DecisionQuestion::Choice(choice) = question
            && choice.candidates().len() > limits.max_candidates
        {
            return Err(SchemaError::limit(
                "max_candidates",
                choice.candidates().len(),
                limits.max_candidates,
            ));
        }
    }
    if payload_limits.max_input_bytes > limits.max_input_bytes {
        return Err(SchemaError::limit(
            "max_input_bytes",
            payload_limits.max_input_bytes,
            limits.max_input_bytes,
        ));
    }
    if payload_limits.max_questions > limits.max_questions {
        return Err(SchemaError::limit(
            "max_questions",
            payload_limits.max_questions,
            limits.max_questions,
        ));
    }
    if payload_limits.max_candidates > limits.max_candidates {
        return Err(SchemaError::limit(
            "max_candidates",
            payload_limits.max_candidates,
            limits.max_candidates,
        ));
    }
    if payload_limits.max_graph_nodes > limits.max_graph_nodes {
        return Err(SchemaError::limit(
            "max_graph_nodes",
            payload_limits.max_graph_nodes,
            limits.max_graph_nodes,
        ));
    }
    if payload_limits.max_retrieval_results > limits.max_retrieval_results {
        return Err(SchemaError::limit(
            "max_retrieval_results",
            payload_limits.max_retrieval_results,
            limits.max_retrieval_results,
        ));
    }
    if payload_limits.max_execution_time > limits.max_execution_time {
        let actual = payload_limits.max_execution_time.as_secs();
        let max = limits.max_execution_time.as_secs();
        return Err(SchemaError::limit(
            "max_execution_time",
            usize::try_from(actual).unwrap_or(usize::MAX),
            usize::try_from(max).unwrap_or(usize::MAX),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use opencodifier_core::{
        BooleanQuestion, Candidate, ChoiceQuestion, ConfidenceReport, DecisionAnswer,
        DecisionMetrics, DecisionOutcome, DecisionPolicy, DecisionQuestion, DecisionTrace,
        Distribution, QuestionId, RequestMetadata, ScoreLevel, ScoreQuestion, State,
    };

    use serde_json::json;

    fn sample_request() -> DecisionRequest {
        DecisionRequest::new(
            State::from_text("refactor the parser module")
                .with_fact("context_tokens", opencodifier_core::FactValue::Integer(42_000)),
            vec![
                DecisionQuestion::Choice(
                    ChoiceQuestion::new(
                        "model",
                        "Which model should run this?",
                        vec![
                            Candidate::new("local-qwen", "General coding").unwrap(),
                            Candidate::new("local-glm", "Deep reasoning").unwrap(),
                        ],
                    )
                    .unwrap(),
                ),
                DecisionQuestion::Score(
                    ScoreQuestion::new(
                        "difficulty",
                        "How difficult is this?",
                        ["trivial", "difficult"]
                            .iter()
                            .map(|label| ScoreLevel::new(*label).unwrap())
                            .collect(),
                    )
                    .unwrap(),
                ),
                DecisionQuestion::Boolean(BooleanQuestion::new("needs_tools", "Tools?").unwrap()),
            ],
            DecisionPolicy::default(),
            RequestMetadata::default(),
        )
        .unwrap()
    }

    fn sample_response(_request: &DecisionRequest) -> DecisionResponse {
        let distribution =
            Distribution::from_pairs([("local-qwen", 0.91), ("local-glm", 0.09)]).unwrap();
        let answers = vec![
            DecisionAnswer::Choice {
                question_id: QuestionId::new("model").unwrap(),
                choice: opencodifier_core::CandidateId::new("local-qwen").unwrap(),
                distribution: distribution.clone(),
                confidence: 0.91,
            },
            DecisionAnswer::Score {
                question_id: QuestionId::new("difficulty").unwrap(),
                expected: 1.6,
                level: "difficult".into(),
                distribution: Distribution::from_pairs([("trivial", 0.4), ("difficult", 0.6)])
                    .unwrap(),
                confidence: 0.83,
            },
            DecisionAnswer::Boolean {
                question_id: QuestionId::new("needs_tools").unwrap(),
                value: false,
                probability: 0.72,
                confidence: 0.72,
            },
        ];
        let report = ConfidenceReport::from_distribution(&distribution, 0.91, 0.02, None).unwrap();
        DecisionResponse::new(
            answers,
            DecisionOutcome::Accept,
            report,
            DecisionTrace::new(),
            DecisionMetrics::default(),
        )
        .unwrap()
    }

    #[test]
    fn name_is_stable() {
        // The trait is dyn-compatible: a boxed `Native` still reports its
        // stable name, which is what a registry (PLANNING.md §36) relies on.
        let boxed: Box<dyn WireFormat> = Box::new(Native);
        assert_eq!(boxed.name(), "native");
    }

    #[test]
    fn request_round_trips_value_exactly() {
        let request = sample_request();
        let encoded = Native.encode_request(&request).unwrap();
        let decoded = Native.decode_request(&encoded, &Limits::default()).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn response_round_trips_value_exactly() {
        let request = sample_request();
        let response = sample_response(&request);
        let encoded = Native.encode_response(&response).unwrap();
        let decoded = Native.decode_response(&encoded, &request, &Limits::default()).unwrap();
        assert_eq!(decoded, response);
    }

    #[test]
    fn malformed_json_shapes_report_invalid_value() {
        let error = Native.decode_request(&json!("not an object"), &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");

        let error = Native.decode_request(&json!({"state": 5}), &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");

        let error = Native
            .decode_response(&json!(["answers"]), &sample_request(), &Limits::default())
            .unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
    }

    #[test]
    fn unknown_fields_are_rejected_in_native_payloads() {
        let mut encoded = Native.encode_request(&sample_request()).unwrap();
        encoded
            .as_object_mut()
            .unwrap()
            .insert("surprise".to_owned(), json!("ignored-by-foreign-formats"));
        let error = Native.decode_request(&encoded, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert!(error.to_string().contains("unknown field"), "{error}");
    }

    #[test]
    fn oversized_requests_report_limit_exceeded() {
        let request = sample_request();
        let encoded = Native.encode_request(&request).unwrap();
        // The payload declares 1 MiB limits; the caller allows far fewer
        // questions.
        let tight = Limits { max_questions: 1, ..Limits::default() };
        let error = Native.decode_request(&encoded, &tight).unwrap_err();
        assert_eq!(error.code(), "schema.limit_exceeded");
        assert!(matches!(error, SchemaError::LimitExceeded { limit: "max_questions", .. }));

        let tight = Limits { max_input_bytes: 4, ..Limits::default() };
        let error = Native.decode_request(&encoded, &tight).unwrap_err();
        assert!(matches!(error, SchemaError::LimitExceeded { limit: "max_input_bytes", .. }));

        let tight = Limits { max_candidates: 1, ..Limits::default() };
        let error = Native.decode_request(&encoded, &tight).unwrap_err();
        assert!(matches!(error, SchemaError::LimitExceeded { limit: "max_candidates", .. }));
    }

    #[test]
    fn oversized_declared_limits_are_rejected() {
        let request = sample_request();
        let mut encoded = Native.encode_request(&request).unwrap();
        encoded.as_object_mut().unwrap()["metadata"]["limits"]["max_questions"] =
            json!(Limits::default().max_questions + 1);
        let error = Native.decode_request(&encoded, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.limit_exceeded");
    }

    /// Every declared ceiling is checked against the caller's, not just
    /// `max_questions`: a payload may not widen a limit it disagrees with.
    #[test]
    fn every_declared_limit_is_checked_against_the_caller() {
        let defaults = Limits::default();
        let declared: [(&str, Value); 5] = [
            ("max_input_bytes", json!(defaults.max_input_bytes + 1)),
            ("max_candidates", json!(defaults.max_candidates + 1)),
            ("max_graph_nodes", json!(defaults.max_graph_nodes + 1)),
            ("max_retrieval_results", json!(defaults.max_retrieval_results + 1)),
            // `Duration` serializes as `{"secs", "nanos"}`.
            ("max_execution_time", json!({"secs": 100_000u64, "nanos": 0u32})),
        ];
        for (field, value) in declared {
            let request = sample_request();
            let mut encoded = Native.encode_request(&request).unwrap();
            encoded.as_object_mut().unwrap()["metadata"]["limits"][field] = value;
            let error = Native.decode_request(&encoded, &Limits::default()).unwrap_err();
            assert_eq!(error.code(), "schema.limit_exceeded", "{field}");
            assert!(
                matches!(error, SchemaError::LimitExceeded { limit, .. } if limit == field),
                "{field}: {error}"
            );
        }
    }

    #[test]
    fn unknown_fields_in_responses_are_rejected_too() {
        let request = sample_request();
        let encoded = Native.encode_response(&sample_response(&request)).unwrap();
        let mut forged = encoded;
        forged.as_object_mut().unwrap().insert("extra".to_owned(), json!(1));
        let error = Native.decode_response(&forged, &request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert!(error.to_string().contains("unknown field"), "{error}");
    }

    #[test]
    fn nested_unknown_fields_are_rejected_too() {
        let request = sample_request();
        let mut encoded = Native.encode_response(&sample_response(&request)).unwrap();
        // The forged field sits inside one answer, so it is only visible to
        // the recursive comparison against the re-encoded payload.
        encoded["answers"][1]["level_extra"] = json!("unrepresentable");
        let error = Native.decode_response(&encoded, &request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert!(error.to_string().contains("level_extra"), "{error}");
    }

    #[test]
    fn hand_written_unnormalized_distribution_is_rejected() {
        let request = sample_request();
        // A hand-forged response whose answers do not even deserialize is
        // rejected with a typed error rather than a panic.
        let error = Native
            .decode_response(
                &json!({"answers": [{"type": "boolean"}]}),
                &request,
                &Limits::default(),
            )
            .unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");

        // A distribution summing to 0.5 must not survive decode either:
        // the IR is re-validated on the way in.
        let forged = json!({
            "answers": [{
                "type": "score",
                "question_id": "difficulty",
                "expected": 0.5,
                "level": "trivial",
                "distribution": {"entries": [{"key": "trivial", "probability": 0.5}]},
                "confidence": 0.5,
            }],
            "outcome": "abstain",
            "confidence": {
                "top_probability": 0.5, "margin": 0.5, "entropy": 1.0,
                "calibrated_confidence": 0.5, "ood_score": 0.0,
                "verifier_agreement": null,
            },
            "trace": {"trace_version": 1, "entries": []},
            "metrics": {
                "candidates_in": 0, "candidates_out": 0,
                "cache_hit": false, "verification_triggered": false,
            },
        });
        let error = Native.decode_response(&forged, &request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert!(error.to_string().contains("sums to"), "{error}");

        // The same payload with a bare array instead of the `entries`
        // object is a different failure: the wire shape itself is wrong.
        let mut wrong_shape = forged;
        wrong_shape["answers"][0]["distribution"] = json!([{"key": "trivial", "probability": 1.0}]);
        let error = Native.decode_response(&wrong_shape, &request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert!(error.to_string().contains("type"), "{error}");
    }
}
