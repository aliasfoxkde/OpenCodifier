//! Image-carrier limit arms: every ceiling the IR applies to attached
//! images, exercised through the same public surface a caller uses.
//!
//! Posture: images are hostile input like everything else on the wire.
//! The IR decodes base64 at its boundary (typed refusals for non-canonical
//! encodings), `DecisionRequest::new` enforces the caller's `Limits`, and
//! an imageless request serializes byte-identically to the text-only
//! shape every existing consumer depends on.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

use opencodifier_core::{
    Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion, DecisionRequest, ImageInput,
    ImageMediaType, Limits, RequestMetadata, State,
};

/// A one-candidate choice question, the smallest legal request body.
fn sample_question() -> DecisionQuestion {
    DecisionQuestion::Choice(
        ChoiceQuestion::new(
            "model",
            "Which model?",
            vec![Candidate::new("qwen", "general").expect("valid")],
        )
        .expect("valid"),
    )
}

/// An image of `bytes` decoded bytes.
fn image(media_type: ImageMediaType, bytes: usize) -> ImageInput {
    ImageInput::new(media_type, vec![0xCD; bytes])
}

#[test]
fn imageless_request_matches_the_historical_canonical_bytes() {
    // The cache-key and fixture contract: an imageless state must emit
    // exactly the wire shape that predates the images field.
    let state = State::from_text("hello").with_fact("k", opencodifier_core::FactValue::Integer(1));
    let request = DecisionRequest::new(
        state,
        vec![sample_question()],
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .expect("valid");
    let json = serde_json::to_string(&request).expect("serialize");
    assert!(json.starts_with(r#"{"state":{"text":"hello","facts":{"k""#));
    assert!(!json.contains("images"), "empty images must not appear: {json}");
}

#[test]
fn canonical_wire_carries_canonical_base64_regardless_of_input_flavor() {
    // Decode → canonical re-encode must be stable across round trips, so
    // two callers sending the same bytes cache-key identically even if
    // one sends them with different (still canonical) padding alignment.
    let bytes: Vec<u8> = (0u8..=63).collect();
    let first = DecisionRequest::new(
        State::from_text("what is pictured")
            .with_image(ImageInput::new(ImageMediaType::Png, bytes.clone())),
        vec![sample_question()],
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .expect("valid");
    let second: DecisionRequest =
        serde_json::from_str(&serde_json::to_string(&first).expect("serialize"))
            .expect("deserialize");
    assert_eq!(first, second, "wire round trip is stable");
    assert_eq!(second.state().images()[0].data(), bytes.as_slice());
}

#[test]
fn per_image_and_count_limits_are_enforced_together() {
    // One image over the per-image ceiling refuses even when the count
    // fits; one image over the count ceiling refuses even when each fits.
    let oversized = RequestMetadata {
        request_id: None,
        limits: Limits { max_image_bytes: 4, ..Limits::default() },
        family: None,
    };
    let request = DecisionRequest::new(
        State::from_text("what is pictured").with_image(image(ImageMediaType::Png, 5)),
        vec![sample_question()],
        DecisionPolicy::default(),
        oversized,
    );
    assert!(matches!(
        request,
        Err(opencodifier_core::CoreError::ImageTooLarge { bytes: 5, limit: 4 })
    ));

    let too_many = RequestMetadata {
        request_id: None,
        limits: Limits { max_images: 1, ..Limits::default() },
        family: None,
    };
    let request = DecisionRequest::new(
        State::from_text("two pictures")
            .with_image(image(ImageMediaType::Png, 1))
            .with_image(image(ImageMediaType::Jpeg, 1)),
        vec![sample_question()],
        DecisionPolicy::default(),
        too_many,
    );
    assert!(matches!(
        request,
        Err(opencodifier_core::CoreError::TooManyImages { count: 2, limit: 1 })
    ));
}

#[test]
fn wire_refuses_unknown_media_types_with_the_typed_code_message() {
    // The adapter-facing contract: an unsupported media type surfaces the
    // stable `ir.unsupported_media_type` semantics, not a serde mystery.
    let payload = r#"{
        "state": {
            "text": "what is pictured",
            "images": [{"media_type": "image/webp", "data": "QQ=="}]
        },
        "questions": [
            {"type": "choice", "id": "model", "text": "Which model?",
             "candidates": [{"id": "qwen", "description": "general"}]}
        ]
    }"#;
    let error = serde_json::from_str::<DecisionRequest>(payload).expect_err("refused");
    assert!(
        error.to_string().contains("unsupported image media type `image/webp`"),
        "typed refusal expected, got: {error}"
    );
}

#[test]
fn wire_refuses_uncanonical_base64_with_the_invalid_image_message() {
    // URL-safe `-_` alphabet characters are not the canonical encoding.
    let payload = r#"{
        "state": {
            "text": "what is pictured",
            "images": [{"media_type": "png", "data": "a-bc"}]
        },
        "questions": [
            {"type": "choice", "id": "model", "text": "Which model?",
             "candidates": [{"id": "qwen", "description": "general"}]}
        ]
    }"#;
    let error = serde_json::from_str::<DecisionRequest>(payload).expect_err("refused");
    assert!(error.to_string().contains("invalid image payload"), "got: {error}");
}

#[test]
fn media_type_aliases_normalize_to_canonical_mime_on_the_wire() {
    // `png` in, `image/png` out: adapters may accept aliases, but the
    // canonical form is what serializes.
    let payload = r#"{
        "state": {
            "text": "what is pictured",
            "images": [{"media_type": "jpg", "data": "/9j/4AAQ"}]
        },
        "questions": [
            {"type": "choice", "id": "model", "text": "Which model?",
             "candidates": [{"id": "qwen", "description": "general"}]}
        ]
    }"#;
    let request: DecisionRequest = serde_json::from_str(payload).expect("decoded");
    let json = serde_json::to_string(&request).expect("serialize");
    assert!(json.contains(r#""media_type":"image/jpeg""#), "canonical MIME expected: {json}");
    assert!(!json.contains("\"jpg\""), "alias must not survive the wire: {json}");
}
