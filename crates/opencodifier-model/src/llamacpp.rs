//! The llama.cpp decision rung (D26): a prompt-shaped [`Classifier`]
//! over a running llama-server.
//!
//! The `parallel-decision` fork of llama.cpp is the only serving path
//! that ever scored the Jev-class decision weights honestly (REPORT
//! F26: 0.8083 through its native verdict-slot readout, vs 0.217 for
//! the tree-mode mismatch that treated the same weights as a scorer).
//! This module binds that measured interface — `POST /v1/decision`
//! with a choice schema, the full per-choice softmax coming back in
//! `results[0].fields.choice.distribution` (fork builds after
//! 2026-09-29) — as an engine [`Classifier`], so the ladder (D25) can
//! escalate to a real model rung through the same seam every other
//! rung uses.
//!
//! The shape decision is the point (D26):
//! [`InferenceBackend`](opencodifier_runtime::InferenceBackend) is the
//! tensor-in/tensor-out contract of the ONNX path; this rung is
//! prompt-shaped. Forcing one onto the other would fabricate tensors
//! and re-create the F10 interface mismatch.
//!
//! Honesty rules carried over from the benchmark harness:
//!
//! * **No synthesized probability.** A response without the full
//!   `distribution` map is refused, not filled in — a winner-only
//!   response cannot be turned into a distribution without inventing
//!   mass, and this engine does not invent evidence.
//! * **Rust owns the decision math.** The server's numbers are parsed
//!   as `f64` and renormalized here (sum-guarded) before they become a
//!   [`Distribution`]; no probability crosses a gate un-normalized.
//! * **Determinism.** Entries are emitted in the question's declared
//!   answer order regardless of the server's key order, and sampling
//!   is pinned greedy server-side — the same `(state, question)` pair
//!   yields the same distribution, which is what cache keys assume.
//!
//! The prompt renderer is byte-faithful to the measured tree-mode
//! contract (choices = answer keys, question text in the schema
//! description, state text as the single context, instructions from
//! [`LlamaConfig`]). Richer renderings — folding candidate
//! descriptions into the prompt, per-kind instructions — are measured
//! levers, not defaults; they change the model's effective interface
//! and must be A/B'd before they ship.

use std::time::Duration;

use crate::render::{answers_of, question_text};
use opencodifier_core::{DecisionQuestion, Distribution, State};
use opencodifier_engine::{Classifier, EngineError, EngineResult};
use serde_json::json;

/// Instructions sent with every decision — the tree-mode contract's
/// system prompt. The benchmark arms measured with
/// `"Select the correct option."`; changing it is a measured lever
/// (prompt content is first-order, RESEARCH §7.3), not a default.
pub const DEFAULT_INSTRUCTIONS: &str = "Select the correct option.";

/// Default server timeout for one decision request. The board's CPU
/// p50 for the 2B arm is ~1.7 s and the 9B tails ran past 600 s, so
/// callers with slow models raise this explicitly.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Configuration for the llama.cpp decision rung.
#[derive(Debug, Clone)]
pub struct LlamaConfig {
    /// Model identity folded into cache keys — it must change when the
    /// served model (or the readout contract) changes, or cached
    /// decisions go stale (D6). Compose build + GGUF + readout, e.g.
    /// `pd-fork-ad129b0|qwen3.5-2b-q4_k_m|tree-v2`.
    pub model_id: String,
    /// The tree-mode system prompt.
    pub instructions: String,
    /// Per-request server timeout.
    pub timeout: Duration,
}

impl LlamaConfig {
    /// Configuration for `model_id` with the measured defaults.
    #[must_use]
    pub fn new(model_id: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
            instructions: DEFAULT_INSTRUCTIONS.to_owned(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Sets the tree-mode system prompt.
    #[must_use]
    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = instructions.into();
        self
    }

    /// Sets the per-request timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// Why one HTTP decision exchange failed.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct LlamaError {
    /// HTTP status when the server answered, `None` on transport
    /// failure (connection refused, timeout, body decode).
    pub status: Option<u16>,
    /// What went wrong, suitable for an `engine.classifier_failed`
    /// reason.
    pub message: String,
}

impl LlamaError {
    /// A transport-level failure — no HTTP status (connection refused,
    /// timeout, body decode). Public because custom [`Transport`]
    /// implementors raise these too.
    #[must_use]
    pub fn transport(message: impl Into<String>) -> Self {
        Self { status: None, message: message.into() }
    }

    /// A non-2xx response. The status must live in the message: the
    /// engine keeps only the reason string, so a status that is not in
    /// the text is lost.
    #[must_use]
    pub fn status(status: u16, message: impl Into<String>) -> Self {
        Self { status: Some(status), message: format!("server said {status}: {}", message.into()) }
    }
}

/// The one HTTP verb this rung needs, as a seam: JSON request in, JSON
/// response out. Everything testable about the classifier — payload
/// shape, response parsing, normalization, refusal of winner-only
/// builds — is testable through a scripted transport; no live server
/// ever runs in the test suite (D26).
pub trait Transport: std::fmt::Debug + Send + Sync {
    /// POSTs `body` to `path` under the configured base URL.
    ///
    /// # Errors
    ///
    /// [`LlamaError`] with the HTTP status when the server answered,
    /// without one on transport failure.
    fn post_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, LlamaError>;
}

/// The default [`Transport`]: a blocking HTTP client confined to the
/// `llamacpp` feature (D26 — one crate, one feature, one new
/// dependency). Plain HTTP by design: the endpoint is loopback by
/// contract, and a TLS stack would add a C build for nothing here.
#[cfg(feature = "llamacpp")]
#[derive(Debug)]
pub struct UreqTransport {
    base_url: String,
    timeout: Duration,
}

#[cfg(feature = "llamacpp")]
impl UreqTransport {
    /// A transport for `base_url` (e.g. `http://127.0.0.1:8080`) with
    /// `timeout` per request.
    ///
    /// # Errors
    ///
    /// [`LlamaError`] when `base_url` is not an absolute `http://`
    /// URL with a host — a typo'd base URL must fail at construction,
    /// not on the first decision.
    pub fn new(base_url: impl Into<String>, timeout: Duration) -> Result<Self, LlamaError> {
        let base_url = base_url.into();
        let host = base_url
            .strip_prefix("http://")
            .and_then(|rest| rest.trim_end_matches('/').split('/').next())
            .filter(|host| !host.is_empty());
        match host {
            Some(host) => Ok(Self { base_url: format!("http://{host}"), timeout }),
            None => Err(LlamaError::transport(format!(
                "base URL must be an absolute http:// URL with a host, got {base_url:?}"
            ))),
        }
    }
}

#[cfg(feature = "llamacpp")]
impl Transport for UreqTransport {
    fn post_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, LlamaError> {
        let url = format!("{}{}", self.base_url, path);
        let agent = ureq::AgentBuilder::new().timeout(self.timeout).build();
        match agent.post(&url).send_json(body) {
            Ok(response) => response.into_json::<serde_json::Value>().map_err(|error| {
                LlamaError::transport(format!("response body is not JSON: {error}"))
            }),
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
                let detail = body.get(..200).unwrap_or(&body);
                Err(LlamaError::status(status, detail))
            }
            Err(error) => Err(LlamaError::transport(error.to_string())),
        }
    }
}

/// The decision rung: renders the tree-mode payload, exchanges it
/// through a [`Transport`], and turns the server's per-choice softmax
/// into a validated [`Distribution`].
#[derive(Debug)]
pub struct LlamaDecisionClassifier {
    transport: Box<dyn Transport>,
    config: LlamaConfig,
}

impl LlamaDecisionClassifier {
    /// Builds the rung over any transport.
    #[must_use]
    pub fn new(transport: Box<dyn Transport>, config: LlamaConfig) -> Self {
        Self { transport, config }
    }

    /// Builds the rung against a loopback llama-server — the default
    /// posture of the binary (`127.0.0.1`, never a remote host).
    ///
    /// # Errors
    ///
    /// [`EngineError::ClassifierFailed`] when `base_url` is not an
    /// absolute `http://` URL.
    #[cfg(feature = "llamacpp")]
    pub fn loopback(base_url: impl Into<String>, config: LlamaConfig) -> EngineResult<Self> {
        let transport = UreqTransport::new(base_url, config.timeout).map_err(|error| {
            EngineError::ClassifierFailed {
                model_id: config.model_id.clone(),
                reason: error.to_string(),
            }
        })?;
        Ok(Self::new(Box::new(transport), config))
    }

    /// The tree-mode exchange for one question: declared answers in,
    /// the server's full per-choice distribution out.
    fn decide_distribution(
        &self,
        state: &State,
        question: &DecisionQuestion,
    ) -> EngineResult<Vec<(String, f64)>> {
        let Some(answers) = answers_of(question) else {
            return Err(EngineError::ClassifierFailed {
                model_id: self.config.model_id.clone(),
                reason: "unsupported question kind: the tree-mode contract defines \
                         choice, boolean, and score readouts only"
                    .to_owned(),
            });
        };
        let payload = json!({
            "instructions": self.config.instructions,
            "schema": {
                "choice": {
                    "type": "enum",
                    "choices": answers,
                    "description": question_text(question),
                }
            },
            "contexts": [state.text()],
            "mode": "tree",
        });
        let response = self.transport.post_json("/v1/decision", &payload).map_err(|error| {
            EngineError::ClassifierFailed {
                model_id: self.config.model_id.clone(),
                reason: error.to_string(),
            }
        })?;

        let choice = response.pointer("/results/0/fields/choice").ok_or_else(|| {
            EngineError::ClassifierFailed {
                model_id: self.config.model_id.clone(),
                reason: "malformed response: missing results[0].fields.choice".to_owned(),
            }
        })?;
        let raw_distribution =
            choice.get("distribution").ok_or_else(|| EngineError::ClassifierFailed {
                model_id: self.config.model_id.clone(),
                reason: "response carries no `distribution`: a winner-only fork build \
                         cannot be served as a decision rung (no probability may be \
                         synthesized); run a fork build with the full per-choice readout \
                         (post-2026-09-29)"
                    .to_owned(),
            })?;
        let map = raw_distribution.as_object().ok_or_else(|| EngineError::ClassifierFailed {
            model_id: self.config.model_id.clone(),
            reason: "malformed response: `distribution` is not an object".to_owned(),
        })?;

        // Emit entries in the question's declared order — the server's
        // JSON key order must never reach a cache key or a trace. Every
        // declared answer must be present and no undeclared label may
        // appear: an out-of-set label is the F10 mismatch wearing a
        // distribution.
        let mut pairs = Vec::with_capacity(map.len());
        for answer in &answers {
            let value = map.get(answer).ok_or_else(|| EngineError::ClassifierFailed {
                model_id: self.config.model_id.clone(),
                reason: format!("response `distribution` is missing declared answer {answer:?}"),
            })?;
            let probability = value.as_f64().ok_or_else(|| EngineError::ClassifierFailed {
                model_id: self.config.model_id.clone(),
                reason: format!("response `distribution` entry {answer:?} is not a number"),
            })?;
            if !probability.is_finite() || probability < 0.0 {
                return Err(EngineError::ClassifierFailed {
                    model_id: self.config.model_id.clone(),
                    reason: format!(
                        "response `distribution` entry {answer:?} is not a probability: \
                         {probability}"
                    ),
                });
            }
            pairs.push((answer.clone(), probability));
        }
        if map.len() != answers.len() {
            return Err(EngineError::ClassifierFailed {
                model_id: self.config.model_id.clone(),
                reason: "response `distribution` carries labels outside the declared \
                         answer set"
                    .to_owned(),
            });
        }
        Ok(pairs)
    }
}

impl Classifier for LlamaDecisionClassifier {
    fn decide(
        &self,
        state: &State,
        question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError> {
        let pairs = self.decide_distribution(state, question)?;
        // Rust owns the decision math (D26): renormalize the server's
        // numbers here. A valid softmax arrives near-normalized, but
        // the sum guard makes the invariant local instead of trusted.
        let total: f64 = pairs.iter().map(|(_, probability)| *probability).sum();
        if !total.is_finite() || total <= 0.0 {
            return Err(EngineError::ClassifierFailed {
                model_id: self.config.model_id.clone(),
                reason: format!(
                    "response `distribution` sums to {total}, not a \
                                 normalizable mass"
                ),
            });
        }
        Distribution::from_pairs(
            pairs.into_iter().map(|(key, probability)| (key, probability / total)),
        )
        .map_err(|error| EngineError::ClassifierFailed {
            model_id: self.config.model_id.clone(),
            reason: error.to_string(),
        })
    }

    fn model_id(&self) -> &str {
        &self.config.model_id
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use opencodifier_core::{
        BooleanQuestion, Candidate, ChoiceQuestion, ScoreLevel, ScoreQuestion,
    };
    use std::sync::{Arc, Mutex};

    /// A transport scripted with one response per call, recording the
    /// requests it received behind a handle the test keeps after the
    /// classifier takes ownership of the transport.
    #[derive(Debug)]
    struct Scripted {
        responses: Mutex<Vec<Result<serde_json::Value, LlamaError>>>,
        requests: Arc<Mutex<Vec<serde_json::Value>>>,
    }

    impl Scripted {
        fn ok(distribution: serde_json::Value) -> Self {
            let mut choice = json!({ "value": "local-qwen", "probability": 0.7 });
            choice["distribution"] = distribution;
            Self::responses(json!({ "results": [ { "fields": { "choice": choice } } ] }))
        }

        fn responses(response: serde_json::Value) -> Self {
            Self::with_response(Ok(response))
        }

        /// A transport-level failure instead of a response.
        fn failure(error: LlamaError) -> Self {
            Self::with_response(Err(error))
        }

        fn with_response(response: Result<serde_json::Value, LlamaError>) -> Self {
            Self {
                responses: Mutex::new(vec![response]),
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl Transport for Scripted {
        fn post_json(
            &self,
            path: &str,
            body: &serde_json::Value,
        ) -> Result<serde_json::Value, LlamaError> {
            assert_eq!(path, "/v1/decision", "the tree-mode endpoint is the only route");
            self.requests.lock().unwrap().push(body.clone());
            self.responses
                .lock()
                .unwrap()
                .pop()
                .expect("test consumed more responses than scripted")
        }
    }

    /// Classifies with `scripted`, returning the classifier and the
    /// request log it will fill.
    fn classifier(
        scripted: Scripted,
    ) -> (LlamaDecisionClassifier, Arc<Mutex<Vec<serde_json::Value>>>) {
        let requests = Arc::clone(&scripted.requests);
        let rung =
            LlamaDecisionClassifier::new(Box::new(scripted), LlamaConfig::new("pd-fork|test|v1"));
        (rung, requests)
    }

    fn two_way() -> serde_json::Value {
        json!({ "local-qwen": 0.7, "cloud-large": 0.3 })
    }

    fn choice_question() -> DecisionQuestion {
        let candidates = vec![
            Candidate::new("local-qwen", "fast general coding").unwrap(),
            Candidate::new("cloud-large", "deep reasoning specialist").unwrap(),
        ];
        DecisionQuestion::Choice(
            ChoiceQuestion::new("model", "Which model should answer?", candidates).unwrap(),
        )
    }

    #[test]
    fn renders_the_measured_tree_mode_payload() {
        let state = State::from_text("Summarize research across many sources");
        let scripted = Scripted::ok(two_way());
        let requests = Arc::clone(&scripted.requests);
        let rung =
            LlamaDecisionClassifier::new(Box::new(scripted), LlamaConfig::new("pd-fork|test|v1"));
        rung.decide(&state, &choice_question()).unwrap();

        let request = &requests.lock().unwrap()[0];
        assert_eq!(request["instructions"], DEFAULT_INSTRUCTIONS);
        assert_eq!(request["mode"], "tree");
        assert_eq!(request["contexts"][0], "Summarize research across many sources");
        let schema = &request["schema"]["choice"];
        assert_eq!(schema["type"], "enum");
        assert_eq!(schema["choices"], json!(["local-qwen", "cloud-large"]));
        assert_eq!(schema["description"], "Which model should answer?");
    }

    #[test]
    fn distribution_arrives_normalized_in_declared_order() {
        let state = State::from_text("Summarize research");
        let (rung, _) = classifier(Scripted::ok(json!({ "cloud-large": 0.3, "local-qwen": 0.7 })));
        let distribution = rung.decide(&state, &choice_question()).unwrap();
        // Declared order, not the server's key order: local-qwen first.
        let keys: Vec<&str> =
            distribution.entries().iter().map(|entry| entry.key.as_str()).collect();
        assert_eq!(keys, vec!["local-qwen", "cloud-large"]);
        assert_eq!(distribution.top().key, "local-qwen");
        assert_eq!(distribution.top().probability, 0.7);
    }

    #[test]
    fn near_normalized_mass_is_renormalized_in_rust() {
        // A softmax printed at reduced precision sums to 0.9999…; the
        // engine must not care.
        let state = State::from_text("s");
        let (rung, _) = classifier(Scripted::ok(json!({
            "local-qwen": 0.666_666_6, "cloud-large": 0.333_333_3
        })));
        let distribution = rung.decide(&state, &choice_question()).unwrap();
        let sum: f64 = distribution.entries().iter().map(|entry| entry.probability).sum();
        assert!((sum - 1.0).abs() < 1e-12, "renormalized to unit mass, got {sum}");
    }

    #[test]
    fn boolean_and_score_kinds_use_their_declared_answers() {
        let state = State::from_text("s");
        let boolean = DecisionQuestion::Boolean(
            BooleanQuestion::new("tools", "Does this need tools?").unwrap(),
        );
        let (rung, requests) = classifier(Scripted::ok(json!({ "true": 0.9, "false": 0.1 })));
        let distribution = rung.decide(&state, &boolean).unwrap();
        assert_eq!(distribution.top().key, "true");
        assert_eq!(
            requests.lock().unwrap()[0]["schema"]["choice"]["choices"],
            json!(["true", "false"])
        );

        let score = DecisionQuestion::Score(
            ScoreQuestion::new(
                "load",
                "How demanding is this?",
                vec![ScoreLevel::new("light").unwrap(), ScoreLevel::new("heavy").unwrap()],
            )
            .unwrap(),
        );
        let (rung, requests) = classifier(Scripted::ok(json!({ "light": 0.4, "heavy": 0.6 })));
        let distribution = rung.decide(&state, &score).unwrap();
        assert_eq!(distribution.top().key, "heavy");
        assert_eq!(
            requests.lock().unwrap()[0]["schema"]["choice"]["choices"],
            json!(["light", "heavy"])
        );
    }

    #[test]
    fn custom_instructions_ride_the_payload() {
        let state = State::from_text("s");
        let scripted = Scripted::ok(two_way());
        let requests = Arc::clone(&scripted.requests);
        let rung = LlamaDecisionClassifier::new(
            Box::new(scripted),
            LlamaConfig::new("m").with_instructions("Pick the local model."),
        );
        rung.decide(&state, &choice_question()).unwrap();
        assert_eq!(requests.lock().unwrap()[0]["instructions"], "Pick the local model.");
    }

    #[test]
    fn a_winner_only_build_is_refused_not_filled_in() {
        let state = State::from_text("s");
        let choice = json!({ "value": "local-qwen", "probability": 0.9 });
        let (rung, _) = classifier(Scripted::responses(json!({
            "results": [ { "fields": { "choice": choice } } ]
        })));
        let error = rung.decide(&state, &choice_question()).unwrap_err();
        assert_eq!(error.code(), "engine.classifier_failed");
        assert!(error.to_string().contains("distribution"), "{error}");
    }

    #[test]
    fn malformed_responses_are_typed_not_panics() {
        let state = State::from_text("s");
        for response in [
            json!({}),                                  // no results
            json!({ "results": [] }),                   // empty results
            json!({ "results": [ { "fields": {} } ] }), // no choice
            json!({ "results": [ { "fields": { "choice": {
                "distribution": [0.5, 0.5] } } } ] }), // not an object
        ] {
            let (rung, _) = classifier(Scripted::responses(response));
            let error = rung.decide(&state, &choice_question()).unwrap_err();
            assert_eq!(error.code(), "engine.classifier_failed", "{error}");
        }
    }

    #[test]
    fn out_of_set_and_missing_labels_are_refused() {
        let state = State::from_text("s");
        // An undeclared label: the answer set is the question's.
        let (rung, _) = classifier(Scripted::ok(json!({
            "local-qwen": 0.6, "cloud-large": 0.3, "sneaky": 0.1
        })));
        let error = rung.decide(&state, &choice_question()).unwrap_err();
        assert!(error.to_string().contains("outside the declared answer set"), "{error}");

        // A missing declared answer: refusing beats inventing mass.
        let (rung, _) = classifier(Scripted::ok(json!({ "local-qwen": 0.7 })));
        let error = rung.decide(&state, &choice_question()).unwrap_err();
        assert!(error.to_string().contains("missing declared answer"), "{error}");
    }

    #[test]
    fn non_probability_and_degenerate_masses_are_refused() {
        let state = State::from_text("s");
        let (rung, _) = classifier(Scripted::ok(json!({ "local-qwen": -0.2, "cloud-large": 1.2 })));
        let error = rung.decide(&state, &choice_question()).unwrap_err();
        assert!(error.to_string().contains("not a probability"), "{error}");

        let (rung, _) = classifier(Scripted::ok(json!({ "local-qwen": 0.0, "cloud-large": 0.0 })));
        let error = rung.decide(&state, &choice_question()).unwrap_err();
        assert!(error.to_string().contains("not a normalizable mass"), "{error}");
    }

    #[test]
    fn transport_failures_name_the_model_and_the_cause() {
        let state = State::from_text("s");
        let (rung, _) = classifier(Scripted::failure(LlamaError::status(503, "slot busy")));
        let error = rung.decide(&state, &choice_question()).unwrap_err();
        assert_eq!(error.code(), "engine.classifier_failed");
        assert!(error.to_string().contains("pd-fork|test|v1"), "{error}");
        assert!(error.to_string().contains("503"), "{error}");

        let (rung, _) = classifier(Scripted::failure(LlamaError::transport("connection refused")));
        let error = rung.decide(&state, &choice_question()).unwrap_err();
        assert!(error.to_string().contains("connection refused"), "{error}");
    }

    #[test]
    fn model_id_is_the_configured_identity() {
        let (rung, _) = classifier(Scripted::ok(two_way()));
        assert_eq!(rung.model_id(), "pd-fork|test|v1");
    }

    #[test]
    fn the_config_builders_are_honored() {
        let config = LlamaConfig::new("m")
            .with_instructions("Pick well.")
            .with_timeout(Duration::from_secs(7));
        assert_eq!(config.instructions, "Pick well.");
        assert_eq!(config.timeout, Duration::from_secs(7));
        assert_eq!(config.model_id, "m");
    }

    #[test]
    fn a_non_numeric_distribution_entry_is_refused() {
        let state = State::from_text("s");
        let (rung, _) =
            classifier(Scripted::ok(json!({ "local-qwen": "high", "cloud-large": 0.3 })));
        let error = rung.decide(&state, &choice_question()).unwrap_err();
        assert!(error.to_string().contains("is not a number"), "{error}");
    }

    #[test]
    fn escalates_through_the_engine_as_the_model_rung() {
        // The B5 acceptance: the llama.cpp rung assembles through the
        // same seam as every other classifier and the ladder's rung
        // overrides govern it like any other rung (D26).
        use opencodifier_core::{
            DecisionAnswer, DecisionOutcome, DecisionPolicy, DecisionRequest, RequestMetadata,
            RiskLevel,
        };
        use opencodifier_engine::{EngineConfig, EngineHandle, LadderPolicy, NodeKind};
        use std::collections::BTreeMap;

        let request = DecisionRequest::new(
            State::from_text("Summarize research across many sources"),
            vec![choice_question()],
            DecisionPolicy::default(), // accepts at 0.80
            RequestMetadata::default(),
        )
        .unwrap();

        let (rung, requests) =
            classifier(Scripted::ok(json!({ "local-qwen": 0.9, "cloud-large": 0.1 })));
        let engine =
            EngineHandle::new(EngineConfig::with_default_pipeline().unwrap(), Arc::new(rung), None)
                .unwrap();
        let response = engine.decide(&request).unwrap();
        assert_eq!(response.outcome(), DecisionOutcome::Accept);
        match &response.answers()[0] {
            DecisionAnswer::Choice { choice, confidence, .. } => {
                assert_eq!(choice.as_str(), "local-qwen");
                assert!((*confidence - 0.9).abs() < 1e-9, "{confidence}");
            }
            other => panic!("expected a choice answer, got {other:?}"),
        }
        // Exactly one exchange: the model rung was consulted once, not
        // once as decider and again as verifier.
        assert_eq!(requests.lock().unwrap().len(), 1);

        // The same verdict under a `kind:choice` rung demanding 0.95
        // verifies — escalation policy applies to the model rung.
        let strict = DecisionPolicy::new(0.95, 0.90, 0.475, RiskLevel::Low).unwrap();
        let (rung, _) = classifier(Scripted::ok(json!({ "local-qwen": 0.9, "cloud-large": 0.1 })));
        let laddered = EngineHandle::with_ladder(
            EngineConfig::with_default_pipeline().unwrap(),
            Arc::new(rung),
            None,
            LadderPolicy {
                id: "model-strict-v1".to_owned(),
                per_kind: BTreeMap::from([(NodeKind::Choice, strict)]),
                ..LadderPolicy::default()
            },
        )
        .unwrap();
        let response = laddered.decide(&request).unwrap();
        assert_eq!(response.outcome(), DecisionOutcome::Verify);
        let source = response.trace().entries().iter().find_map(|entry| {
            match entry.detail.get("policy_source") {
                Some(opencodifier_core::FactValue::Text(source)) => Some(source.clone()),
                _ => None,
            }
        });
        assert_eq!(source.as_deref(), Some("kind:choice"));
    }

    #[cfg(feature = "llamacpp")]
    #[test]
    fn loopback_transport_rejects_non_http_base_urls() {
        for bad in ["https://example.com", "127.0.0.1:8080", "", "ftp://x", "http://"] {
            assert!(UreqTransport::new(bad, DEFAULT_TIMEOUT).is_err(), "{bad:?}");
        }
        assert!(UreqTransport::new("http://127.0.0.1:8080/", DEFAULT_TIMEOUT).is_ok());
    }

    /// A one-shot loopback HTTP stub: accepts one connection, drains the
    /// request head and announced body, answers once, and closes. This
    /// is a test double for the HTTP contract, not a live model server —
    /// D26's "no live server" bars the llama-server, not a socket.
    #[cfg(feature = "llamacpp")]
    fn stub(status_line: &str, body: &str) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let status_line = status_line.to_owned();
        let body = body.to_owned();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut received = Vec::new();
            let mut buffer = [0u8; 4096];
            let head_end = loop {
                let read = stream.read(&mut buffer).unwrap();
                assert!(read > 0, "peer closed before sending headers");
                received.extend_from_slice(&buffer[..read]);
                if let Some(position) = received.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    break position + 4;
                }
            };
            let head = String::from_utf8_lossy(&received[..head_end]).to_lowercase();
            let content_length = head
                .lines()
                .find_map(|line| {
                    line.strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            while received.len() < head_end + content_length {
                let read = stream.read(&mut buffer).unwrap();
                assert!(read > 0, "peer closed before sending the body");
                received.extend_from_slice(&buffer[..read]);
            }
            let response = format!(
                "{status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
                 connection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        (format!("http://127.0.0.1:{port}"), handle)
    }

    /// The real transport exchanges JSON over loopback HTTP and
    /// normalizes nothing — the value comes back exactly as sent.
    #[cfg(feature = "llamacpp")]
    #[test]
    fn the_real_transport_exchanges_json_over_loopback_http() {
        let (base_url, server) = stub("HTTP/1.1 200 OK", r#"{"results":[{"ok":true}]}"#);
        let transport = UreqTransport::new(base_url, DEFAULT_TIMEOUT).unwrap();
        let response = transport.post_json("/v1/decision", &json!({ "mode": "tree" })).unwrap();
        assert_eq!(response["results"][0]["ok"], true);
        server.join().unwrap();
    }

    /// A non-2xx answer keeps the status and truncates the body detail —
    /// a hostile server cannot flood the error channel.
    #[cfg(feature = "llamacpp")]
    #[test]
    fn a_non_2xx_answer_keeps_the_status_and_truncates_the_detail() {
        let body = "x".repeat(300);
        let (base_url, server) = stub("HTTP/1.1 503 busy", &body);
        let transport = UreqTransport::new(base_url, DEFAULT_TIMEOUT).unwrap();
        let error = transport.post_json("/v1/decision", &json!({})).unwrap_err();
        assert_eq!(error.status, Some(503));
        assert!(error.message.contains("server said 503"), "{error}");
        assert!(error.message.contains(&"x".repeat(200)), "{error}");
        assert!(!error.message.contains(&"x".repeat(201)), "{error}");
        server.join().unwrap();
    }

    /// A 200 answer whose body is not JSON is a transport failure, not a
    /// decision — status stays `None` because HTTP itself succeeded.
    #[cfg(feature = "llamacpp")]
    #[test]
    fn a_non_json_body_is_a_transport_failure() {
        let (base_url, server) = stub("HTTP/1.1 200 OK", "not json");
        let transport = UreqTransport::new(base_url, DEFAULT_TIMEOUT).unwrap();
        let error = transport.post_json("/v1/decision", &json!({})).unwrap_err();
        assert_eq!(error.status, None);
        assert!(error.message.contains("response body is not JSON"), "{error}");
        server.join().unwrap();
    }

    /// A closed port is a transport failure without a status: nothing
    /// answered.
    #[cfg(feature = "llamacpp")]
    #[test]
    fn a_closed_port_is_a_transport_failure_without_a_status() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        drop(listener);
        let transport = UreqTransport::new(base_url, Duration::from_secs(5)).unwrap();
        let error = transport.post_json("/v1/decision", &json!({})).unwrap_err();
        assert_eq!(error.status, None);
    }

    /// `loopback` maps a bad base URL to `engine.classifier_failed`
    /// naming the model, and assembles without contacting anything on a
    /// good one.
    #[cfg(feature = "llamacpp")]
    #[test]
    fn loopback_maps_a_bad_base_url_to_classifier_failed() {
        let error = LlamaDecisionClassifier::loopback("not a url", LlamaConfig::new("pd|m|v1"))
            .unwrap_err();
        assert_eq!(error.code(), "engine.classifier_failed");
        assert!(error.to_string().contains("pd|m|v1"), "{error}");
        assert!(
            LlamaDecisionClassifier::loopback("http://127.0.0.1:9/", LlamaConfig::new("m")).is_ok()
        );
    }
}
