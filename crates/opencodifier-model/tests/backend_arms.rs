//! Refusal arms of the model layer's contracts (PLANNING.md §73).
//!
//! Every arm a caller can drive from the public API, and what it reports:
//! payload rendering, tokenizer-asset validation, empty-segment refusal,
//! pointer and marker invariant checks, tensor-shape agreement, and the
//! backend boundary of the Kai serving path. Each assertion pins a stable
//! [`ModelError`] code or message, because those are what the interface
//! layers map onto — an arm that reports the wrong invariant is as broken
//! as one that does not report at all.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

use std::collections::BTreeMap;

use opencodifier_core::{
    BooleanQuestion, Candidate, ChoiceQuestion, DecisionQuestion, FactValue, State,
};
use opencodifier_engine::Classifier;
use opencodifier_model::ModelError;
use opencodifier_model::embedding::EmbeddingClassifier;
use opencodifier_model::julia::{JuliaEncoding, JuliaKind, JuliaOption, JuliaQuestion};
use opencodifier_model::julia::{JuliaTokenizer, MAX_LENGTH};
use opencodifier_model::kai::{
    BpeTokenizer, FIXTURE_TOKENIZER, KaiBatch, KaiEncoding, KaiKind, KaiOption, KaiPayload,
    KaiQuestion, KaiTokenizer, MAX_INPUT_TOKENS, canonical_json, probabilities, score_batch,
};
use opencodifier_runtime::{DenseTensor, EmbeddingBackend, InferenceBackend, RuntimeError};

// ---------------------------------------------------------------- kai --

/// A two-candidate question, the smallest shape the contract accepts.
fn two_option_choice() -> KaiQuestion {
    KaiQuestion::new(
        "q",
        KaiPayload::Text("state".to_owned()),
        KaiPayload::Text("pick one".to_owned()),
        KaiKind::Choice,
        vec![KaiOption::bare("left"), KaiOption::bare("right")],
    )
    .unwrap()
}

/// A two-candidate encoding built by hand: pointers in range, increasing,
/// and consistent, so only the boundary under test can fail.
fn two_way_encoding() -> KaiEncoding {
    KaiEncoding {
        ids: vec![1, 2, 3, 4],
        option_pos: vec![1, 2],
        answer_pos: 3,
        keys: vec!["left".to_owned(), "right".to_owned()],
        kind: KaiKind::Choice,
    }
}

/// A `tokenizer.json` skeleton with the vocab and merges rendered in.
fn tokenizer_asset(vocab: &str, merges: &str) -> String {
    format!(r#"{{"model": {{"vocab": {vocab}, "merges": {merges}}}}}"#)
}

#[test]
fn structured_payloads_render_as_canonical_json() {
    let question = KaiQuestion::new(
        "q",
        KaiPayload::Structured(serde_json::json!({"b": 2, "a": [1, {"z": true, "y": null}]})),
        KaiPayload::Text("pick one".to_owned()),
        KaiKind::Choice,
        vec![KaiOption::bare("left"), KaiOption::bare("right")],
    )
    .unwrap();
    // Sorted keys, compact separators, non-ASCII and nulls literal — the
    // upstream `json.dumps(..., sort_keys=True, separators=(",", ":"))`.
    assert_eq!(
        question.segments().unwrap().prefix,
        "Context:\n{\"a\":[1,{\"y\":null,\"z\":true}],\"b\":2}\n\nTask type: choice\n\
         Question:\npick one\nOptions:"
    );
}

#[test]
fn payloads_that_render_empty_are_refused() {
    let options = vec![KaiOption::bare("left"), KaiOption::bare("right")];

    let error = KaiQuestion::new(
        "q",
        KaiPayload::Text("   \n\t".to_owned()),
        KaiPayload::Text("pick one".to_owned()),
        KaiKind::Choice,
        options.clone(),
    )
    .unwrap_err();
    assert_eq!(error.code(), "model.contract_violation");
    assert!(
        error.to_string().contains("`q`: state must be nonempty text or structured data"),
        "{error}"
    );

    // The instructions slot is gated identically, with its own name.
    let error = KaiQuestion::new(
        "q",
        KaiPayload::Text("state".to_owned()),
        KaiPayload::Text(" \n\t".to_owned()),
        KaiKind::Choice,
        options,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("`q`: instructions must be nonempty text or structured data"),
        "{error}"
    );
}

#[test]
fn canonical_json_descends_into_arrays() {
    let value = serde_json::json!([{"z": 1}, ["nested", 2, null]]);
    assert_eq!(canonical_json(&value).unwrap(), r#"[{"z":1},["nested",2,null]]"#);
}

#[test]
fn malformed_tokenizer_assets_are_refused_with_typed_reasons() {
    // `model.vocab` must be a token→id map.
    let error = BpeTokenizer::from_tokenizer_json(&tokenizer_asset("[1, 2]", "[]")).unwrap_err();
    assert_eq!(error.code(), "model.tokenizer");
    assert!(error.to_string().contains("`model.vocab` is not a token→id map"), "{error}");

    // `model.merges` must be a list.
    let error = BpeTokenizer::from_tokenizer_json(&tokenizer_asset(r#"{"a":0}"#, r#"{"a":0}"#))
        .unwrap_err();
    assert!(error.to_string().contains("`model.merges` is not a list"), "{error}");

    // A merge string without a space names no pair.
    let error =
        BpeTokenizer::from_tokenizer_json(&tokenizer_asset(r#"{"a":0,"b":1,"ab":2}"#, r#"["ab"]"#))
            .unwrap_err();
    assert!(error.to_string().contains("merge `ab` is not a space-separated pair"), "{error}");

    // A merge pair that is not two strings.
    let error = BpeTokenizer::from_tokenizer_json(&tokenizer_asset(
        r#"{"a":0,"b":1,"ab":2}"#,
        r#"[["a", 3]]"#,
    ))
    .unwrap_err();
    assert!(error.to_string().contains("merge pair is not two strings"), "{error}");

    // A merge that is not a pair at all.
    let error =
        BpeTokenizer::from_tokenizer_json(&tokenizer_asset(r#"{"a":0}"#, "[7]")).unwrap_err();
    assert!(error.to_string().contains("merge is not a pair"), "{error}");
}

#[test]
fn a_symbol_outside_the_vocabulary_is_a_typed_failure() {
    let tokenizer = BpeTokenizer::from_tokenizer_json(FIXTURE_TOKENIZER).unwrap();
    // `~` is absent from the fixture vocabulary's byte alphabet, so the
    // encoder must report the missing id instead of guessing one.
    // `BpeTokenizer` implements both tokenizer traits, so the Kai-side
    // method is named explicitly.
    let error = KaiTokenizer::encode(&tokenizer, "~").unwrap_err();
    assert_eq!(error.code(), "model.tokenizer");
    assert!(error.to_string().contains("vocabulary has no id for"), "{error}");
}

/// Which rendered segment a [`SlotTokenizer`] answers with nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// Everything before the first option line.
    Prefix,
    /// An `<option>` line.
    Option,
    /// The query tail after the last option line.
    Suffix,
}

/// A tokenizer that encodes one named slot to nothing: an empty segment
/// must be refused, because a pointer into no tokens anchors nothing.
#[derive(Debug)]
struct SlotTokenizer {
    /// The slot that answers empty.
    blank: Slot,
}

impl KaiTokenizer for SlotTokenizer {
    fn encode(&self, text: &str) -> Result<Vec<i64>, ModelError> {
        let slot = if text.contains("Context:") {
            Slot::Prefix
        } else if text.contains("<option>") {
            Slot::Option
        } else {
            Slot::Suffix
        };
        if slot == self.blank { Ok(Vec::new()) } else { Ok(vec![1, 2, 3]) }
    }
}

#[test]
fn an_empty_segment_is_refused_rather_than_pointered_into() {
    let question = two_option_choice();
    for (blank, expected) in [
        (Slot::Prefix, "the prefix encoded to no tokens"),
        (Slot::Option, "option 0 encoded to no tokens"),
        (Slot::Suffix, "the query suffix encoded to no tokens"),
    ] {
        let error =
            KaiEncoding::encode(&question, &SlotTokenizer { blank }, MAX_INPUT_TOKENS).unwrap_err();
        assert_eq!(error.code(), "model.tokenizer", "{expected}");
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn validate_refuses_a_solitary_candidate_and_duplicate_pointers() {
    // One pointer for one candidate is consistent — and still refused,
    // because a one-candidate question is not a decision.
    let solitary = KaiEncoding {
        ids: vec![1, 2],
        option_pos: vec![0],
        answer_pos: 1,
        keys: vec!["left".to_owned()],
        kind: KaiKind::Choice,
    };
    let error = solitary.validate().unwrap_err();
    assert_eq!(error.code(), "model.contract_violation");
    assert!(error.to_string().contains("candidate count 1 outside 2..=255"), "{error}");

    // Two pointers on the same token would read one option twice.
    let duplicated = KaiEncoding {
        ids: vec![1, 2, 3, 4],
        option_pos: vec![2, 2],
        answer_pos: 3,
        keys: vec!["left".to_owned(), "right".to_owned()],
        kind: KaiKind::Choice,
    };
    let error = duplicated.validate().unwrap_err();
    assert!(error.to_string().contains("option pointers must be distinct"), "{error}");
}

#[test]
fn collate_refuses_a_row_whose_keys_and_pointers_disagree() {
    let row = KaiEncoding {
        ids: vec![1, 2, 3],
        option_pos: vec![1],
        answer_pos: 2,
        keys: vec!["left".to_owned(), "right".to_owned()],
        kind: KaiKind::Choice,
    };
    let error = KaiBatch::collate(&[&row]).unwrap_err();
    assert!(error.to_string().contains("row 0: 2 keys but 1 option pointers"), "{error}");
}

#[test]
fn a_batch_whose_tensors_do_not_agree_with_its_shape_is_refused() {
    let batch = KaiBatch {
        input_ids: vec![1],
        attention_mask: vec![1, 0],
        answer_pos: vec![1],
        option_pos: vec![0],
        batch: 1,
        length: 2,
        width: 1,
    };
    let error = batch.to_tensors().unwrap_err();
    assert_eq!(error.code(), "model.contract_violation");
    assert!(error.to_string().contains("`input_ids`: invalid tensor"), "{error}");
}

#[test]
fn probabilities_refuse_a_logit_key_width_mismatch() {
    let keys = vec!["left".to_owned(), "right".to_owned()];

    let error = probabilities(KaiKind::Choice, &keys, &[0.0]).unwrap_err();
    assert_eq!(error.code(), "model.contract_violation");
    assert!(error.to_string().contains("1 logits must match 2 keys"), "{error}");

    // The score rung is width-gated the same way, before any bias is applied.
    let error = probabilities(KaiKind::Score, &keys, &[0.0, 0.0, 0.0]).unwrap_err();
    assert!(error.to_string().contains("3 logits must match 2 keys"), "{error}");
}

/// A backend that always fails: the rung must flatten the failure into
/// its reason rather than wrap it opaquely.
#[derive(Debug)]
struct FailingBackend {
    /// Stable identity the error is expected to name.
    model: &'static str,
}

impl InferenceBackend for FailingBackend {
    fn model_id(&self) -> &str {
        self.model
    }

    fn infer(
        &self,
        _inputs: &BTreeMap<String, DenseTensor>,
    ) -> Result<BTreeMap<String, DenseTensor>, RuntimeError> {
        Err(RuntimeError::BackendFailed {
            model_id: self.model.to_owned(),
            message: "session closed".to_owned(),
        })
    }
}

/// A backend that answers without the `logits` output.
#[derive(Debug)]
struct LogitsLessBackend {
    /// Stable identity of the silent backend.
    model: &'static str,
}

impl InferenceBackend for LogitsLessBackend {
    fn model_id(&self) -> &str {
        self.model
    }

    fn infer(
        &self,
        _inputs: &BTreeMap<String, DenseTensor>,
    ) -> Result<BTreeMap<String, DenseTensor>, RuntimeError> {
        Ok(BTreeMap::new())
    }
}

#[test]
fn score_batch_flattens_a_backend_failure_into_its_reason() {
    let error =
        score_batch(&FailingBackend { model: "failing-kai" }, &[&two_way_encoding()]).unwrap_err();
    assert_eq!(error.code(), "model.contract_violation");
    assert!(
        error.to_string().contains("backend failed: backend `failing-kai` failed: session closed"),
        "{error}"
    );
}

#[test]
fn score_batch_refuses_a_backend_that_omits_the_logits_output() {
    let error = score_batch(&LogitsLessBackend { model: "logits-less" }, &[&two_way_encoding()])
        .unwrap_err();
    assert!(error.to_string().contains("did not return `logits`"), "{error}");
}

// -------------------------------------------------------------- julia --

/// Two keyed options with distinct non-empty text.
fn two_julia_options() -> Vec<JuliaOption> {
    vec![
        JuliaOption { key: "a".to_owned(), text: "alpha".to_owned() },
        JuliaOption { key: "b".to_owned(), text: "beta".to_owned() },
    ]
}

#[test]
fn julia_task_types_carry_their_upstream_names() {
    assert_eq!(JuliaKind::Choice.as_str(), "choice");
    assert_eq!(JuliaKind::Score.as_str(), "score");
    assert_eq!(JuliaKind::Noul.as_str(), "noul");

    let noul = JuliaQuestion::new(
        "q",
        "state",
        "is it urgent?",
        JuliaKind::Noul,
        vec![
            JuliaOption { key: "false".to_owned(), text: "No".to_owned() },
            JuliaOption { key: "true".to_owned(), text: "Yes".to_owned() },
        ],
    )
    .unwrap();
    assert_eq!(noul.head_text(), "noul question: is it urgent?");
}

#[test]
fn julia_cardinality_and_text_rules_are_enforced() {
    let error = JuliaQuestion::new(
        "q",
        "state",
        "pick one",
        JuliaKind::Choice,
        vec![JuliaOption { key: "a".to_owned(), text: "alpha".to_owned() }],
    )
    .unwrap_err();
    assert_eq!(error.code(), "model.contract_violation");
    assert!(
        error.to_string().contains("`q`: choice carries 1 options, outside 2..=255"),
        "{error}"
    );

    let wide: Vec<JuliaOption> = (0..256)
        .map(|index| JuliaOption { key: index.to_string(), text: index.to_string() })
        .collect();
    let error = JuliaQuestion::new("q", "state", "pick one", JuliaKind::Choice, wide).unwrap_err();
    assert!(error.to_string().contains("carries 256 options, outside 2..=255"), "{error}");

    let error = JuliaQuestion::new("q", "state", "   ", JuliaKind::Choice, two_julia_options())
        .unwrap_err();
    assert!(error.to_string().contains("the question text must be nonempty"), "{error}");

    let error = JuliaQuestion::new(
        "q",
        "state",
        "pick one",
        JuliaKind::Choice,
        vec![
            JuliaOption { key: "  ".to_owned(), text: "alpha".to_owned() },
            JuliaOption { key: "b".to_owned(), text: "beta".to_owned() },
        ],
    )
    .unwrap_err();
    assert!(error.to_string().contains("option 0 has an empty key"), "{error}");

    let error = JuliaQuestion::new(
        "q",
        "state",
        "pick one",
        JuliaKind::Choice,
        vec![
            JuliaOption { key: "a".to_owned(), text: "alpha".to_owned() },
            JuliaOption { key: "b".to_owned(), text: " ".to_owned() },
        ],
    )
    .unwrap_err();
    assert!(error.to_string().contains("option 1 has empty text"), "{error}");
}

#[test]
fn julia_refuses_typed_facts_instead_of_inventing_a_rendering() {
    let state =
        State::from_text("hub is down").with_fact("modality", FactValue::Text("text".into()));
    let error = JuliaQuestion::from_decision(
        &state,
        &DecisionQuestion::Boolean(BooleanQuestion::new("b", "is it urgent?").unwrap()),
    )
    .unwrap_err();
    assert_eq!(error.code(), "model.contract_violation");
    assert!(error.to_string().contains("text states only"), "{error}");
}

/// A tokenizer that encodes one exact text to nothing: an empty head or
/// option segment would shift every marker, so it is refused.
#[derive(Debug)]
struct EmptyFor {
    /// The text that encodes to nothing.
    blank: &'static str,
}

impl JuliaTokenizer for EmptyFor {
    fn encode(&self, text: &str) -> Result<Vec<i64>, ModelError> {
        if text == self.blank { Ok(Vec::new()) } else { Ok(vec![7, 8]) }
    }
}

#[test]
fn julia_refuses_an_empty_head_or_option_segment() {
    let question =
        JuliaQuestion::new("q", "state", "pick one", JuliaKind::Choice, two_julia_options())
            .unwrap();

    let error = JuliaEncoding::encode(
        &question,
        &EmptyFor { blank: "choice question: pick one" },
        MAX_LENGTH,
    )
    .unwrap_err();
    assert_eq!(error.code(), "model.tokenizer");
    assert!(error.to_string().contains("`q`: the question head encoded to no tokens"), "{error}");

    let error =
        JuliaEncoding::encode(&question, &EmptyFor { blank: " alpha" }, MAX_LENGTH).unwrap_err();
    assert!(error.to_string().contains("`q`: option 0 encoded to no tokens"), "{error}");
}

#[test]
fn julia_marker_violations_are_typed_errors_not_readouts() {
    let count_mismatch = JuliaEncoding {
        ids: vec![2, 4, 1],
        marker_pos: vec![1],
        keys: vec!["a".to_owned(), "b".to_owned()],
        kind: JuliaKind::Choice,
    };
    let error = count_mismatch.validate().unwrap_err();
    assert_eq!(error.code(), "model.contract_violation");
    assert!(error.to_string().contains("marker count 1 must equal the option count 2"), "{error}");

    let not_increasing = JuliaEncoding {
        ids: vec![2, 4, 4, 1],
        marker_pos: vec![2, 1],
        keys: vec!["a".to_owned(), "b".to_owned()],
        kind: JuliaKind::Choice,
    };
    let error = not_increasing.validate().unwrap_err();
    assert!(error.to_string().contains("markers must be strictly increasing"), "{error}");

    let outside = JuliaEncoding {
        ids: vec![2, 4, 1],
        marker_pos: vec![1, 7],
        keys: vec!["a".to_owned(), "b".to_owned()],
        kind: JuliaKind::Choice,
    };
    let error = outside.validate().unwrap_err();
    assert!(error.to_string().contains("marker 7 is outside the 3-token prompt"), "{error}");

    // A marker on a token that is not `<mask>` reads another option's slot.
    let not_a_mask = JuliaEncoding {
        ids: vec![2, 4, 9, 1],
        marker_pos: vec![1, 2],
        keys: vec!["a".to_owned(), "b".to_owned()],
        kind: JuliaKind::Choice,
    };
    let error = not_a_mask.validate().unwrap_err();
    assert!(error.to_string().contains("marker 2 does not land on a `<mask>` token"), "{error}");
}

#[test]
fn julia_tensors_refuse_a_marker_lane_that_disagrees_with_the_keys() {
    let encoding = JuliaEncoding {
        ids: vec![2, 4, 1],
        marker_pos: vec![1],
        keys: vec!["a".to_owned(), "b".to_owned()],
        kind: JuliaKind::Choice,
    };
    let error = encoding.to_tensors().unwrap_err();
    assert!(error.to_string().contains("`marker_pos`: invalid tensor"), "{error}");
}

// ---------------------------------------------------------- embedding --

/// An embedding backend with scripted vectors: the query and each
/// candidate answer exactly what the test pins, whatever their texts.
#[derive(Debug)]
struct Scripted {
    /// Stable identity of the fake encoder.
    model: &'static str,
    /// The width of every scripted vector.
    width: usize,
    /// One vector per text, in call order.
    vectors: Vec<Vec<f32>>,
}

impl EmbeddingBackend for Scripted {
    fn model_id(&self) -> &str {
        self.model
    }

    fn dims(&self) -> usize {
        self.width
    }

    fn embed(&self, _texts: &[&str]) -> Result<Vec<Vec<f32>>, RuntimeError> {
        Ok(self.vectors.clone())
    }
}

/// A two-candidate question, so the scripted vectors decide the scores.
fn two_candidates() -> ChoiceQuestion {
    ChoiceQuestion::new(
        "model",
        "Which model should answer?",
        vec![
            Candidate::new("local-a", "first candidate").unwrap(),
            Candidate::new("local-b", "second candidate").unwrap(),
        ],
    )
    .unwrap()
}

#[test]
fn a_zero_query_vector_is_no_evidence_not_a_nan() {
    let backend = Scripted {
        model: "scripted-embed",
        width: 4,
        vectors: vec![vec![0.0; 4], vec![1.0, 0.0, 0.0, 0.0], vec![0.0, 1.0, 0.0, 0.0]],
    };
    let classifier = EmbeddingClassifier::new(Box::new(backend));
    let distribution = classifier
        .decide(&State::from_text("s"), &DecisionQuestion::Choice(two_candidates()))
        .unwrap();
    // A query with no norm carries no evidence, so every cosine is the
    // zero-vector convention's 0.0 and the softmax is exactly uniform.
    for candidate in ["local-a", "local-b"] {
        let probability = distribution.probability_of(candidate).unwrap();
        assert!(probability.is_finite(), "{candidate} = {probability}");
        assert!((probability - 0.5).abs() < 1e-12, "{candidate} = {probability}");
    }
}

#[test]
fn a_zero_candidate_vector_carries_no_evidence_either() {
    let backend = Scripted {
        model: "scripted-embed",
        width: 4,
        vectors: vec![vec![1.0, 0.0, 0.0, 0.0], vec![0.0; 4], vec![1.0, 0.0, 0.0, 0.0]],
    };
    let classifier = EmbeddingClassifier::new(Box::new(backend));
    let distribution = classifier
        .decide(&State::from_text("s"), &DecisionQuestion::Choice(two_candidates()))
        .unwrap();
    // The scripted order is query, candidate `local-a` (zero vector),
    // candidate `local-b` (the query's direction): the degenerate
    // candidate scores 0.0 instead of poisoning the pass with a NaN, so
    // the logits are exactly [0, 1] and the softmax is e0/(e0+e1).
    assert!(
        (distribution.probability_of("local-b").unwrap() - 0.731_058_578_630_004_9).abs() < 1e-12,
        "{distribution:?}"
    );
    assert!(
        (distribution.probability_of("local-a").unwrap() - 0.268_941_421_369_995_1).abs() < 1e-12,
        "{distribution:?}"
    );
}

// -------------------------------------------------------------- error --

#[test]
fn model_error_display_strings_are_pinned() {
    assert_eq!(
        ModelError::InvalidManifest { manifest: "m.json".to_owned(), reason: "no sha".to_owned() }
            .to_string(),
        "invalid model manifest `m.json`: no sha"
    );
    assert_eq!(
        ModelError::UnreadableArtifact { artifact: "m.onnx".into(), reason: "missing".to_owned() }
            .to_string(),
        "model artifact `m.onnx` could not be read: missing"
    );
    assert_eq!(
        ModelError::ContractViolation { reason: "rank 3".to_owned() }.to_string(),
        "candidate-conditioned contract violated: rank 3"
    );
    assert_eq!(
        ModelError::PromptTooLong { id: "q".to_owned(), tokens: 9_000, max_tokens: 8_192 }
            .to_string(),
        "prompt is 9000 tokens, above the 8192 token budget for `q`"
    );
    assert_eq!(
        ModelError::Tokenizer { reason: "no vocab".to_owned() }.to_string(),
        "tokenizer failed: no vocab"
    );
}
