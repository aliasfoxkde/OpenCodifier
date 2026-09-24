//! Wire-format adapters for the `OpenCodifier` decision IR (PLANNING.md §7,
//! §8, §42).
//!
//! The canonical IR in [`opencodifier_core`] is the internal truth. Every
//! external format — native JSON, `OpenAI` structured outputs, `Anthropic`
//! tool schemas, `Jev`/System One — is a *projection* of that truth, and this
//! crate is the only place those formats are named. Nothing here executes a
//! decision; adapters only translate.
//!
//! ```text
//!            OpenAI      Anthropic      Jev        native
//!                \           |           /            |
//!                 └──────────┼──────────┘             |
//!                            ▼                        ▼
//!                     canonical decision IR  ◄── (identity)
//! ```
//!
//! # Adapter contract
//!
//! * **Totality.** Any input within the request resource limits produces a
//!   typed [`SchemaError`] — never a panic, never a silently guessed
//!   answer, never a discarded candidate. Malformed payloads are inputs,
//!   not bugs (`ARCHITECTURE.md` §7).
//! * **Unknown wire fields are ignored.** Vendor payloads gain fields over
//!   time; an adapter must not become a version lock, so wire structs
//!   deliberately avoid `deny_unknown_fields`. The native format is the
//!   exception (see the `native` module docs).
//! * **Limits are enforced here and in the IR.** Decoders check
//!   [`opencodifier_core::Limits`] ceilings up front so oversized payloads
//!   report `schema.limit_exceeded`, then re-validate through the IR's own
//!   constructors, which remain the final word.
//! * **Fidelity is documented, never faked.** Formats that cannot carry a
//!   full probability distribution (`OpenAI`, `Anthropic`, `Jev` choice
//!   answers)
//!   reconstruct one deterministically and say so in their module docs.
//!   That reconstruction is a statement about the wire payload, not
//!   invented evidence about model certainty.
//!
//! # Example
//!
//! Decode a `Jev`/System One request into the canonical IR, decide with core
//! types, and project the answer back onto the wire:
//!
//! ```
//! use opencodifier_core::{CandidateId, DecisionAnswer, Distribution, Limits};
//! use opencodifier_schema::{SchemaError, WireFormat, jev::Jev};
//!
//! fn main() -> Result<(), SchemaError> {
//!     let wire = r#"{
//!         "state": "refactor the parser module",
//!         "model": "local-qwen",
//!         "questions": {
//!             "model": {
//!                 "type": "choice",
//!                 "instructions": "Which model should run this?",
//!                 "criteria": {
//!                     "local-qwen": "general coding",
//!                     "local-glm": "deep reasoning"
//!                 }
//!             }
//!         }
//!     }"#;
//!
//!     // 1. Normalize the wire request into the canonical IR.
//!     let request = Jev.decode_request_str(wire, &Limits::default())?;
//!
//!     // 2. Decide — here with the certain answer an engine would produce
//!     //    after calibration.
//!     let answer = DecisionAnswer::Choice {
//!         question_id: request.questions()[0].id().clone(),
//!         choice: CandidateId::new("local-qwen")?,
//!         distribution: Distribution::from_pairs([("local-qwen", 0.91), ("local-glm", 0.09)])?,
//!         confidence: 0.91,
//!     };
//!
//!     // 3. Project the answer back onto the `Jev` response shape.
//!     let response = opencodifier_core::DecisionResponse::new(
//!         vec![answer],
//!         opencodifier_core::DecisionOutcome::Accept,
//!         opencodifier_core::ConfidenceReport {
//!             top_probability: 0.91, margin: 0.82, entropy: 0.2,
//!             calibrated_confidence: 0.91, ood_score: 0.02,
//!             verifier_agreement: None,
//!         },
//!         opencodifier_core::DecisionTrace::new(),
//!         opencodifier_core::DecisionMetrics::default(),
//!     )?;
//!     let encoded = Jev.encode_response(&response)?;
//!     assert_eq!(
//!         encoded["answers"]["model"],
//!         serde_json::json!("local-qwen"),
//!         "Jev choice answers carry the chosen label"
//!     );
//!     Ok(())
//! }
//! ```

pub mod anthropic;
pub mod codec;
pub mod error;
pub mod jev;
pub mod native;
pub mod openai;
pub(crate) mod support;

pub use codec::{BoxedWireFormat, WireFormat};
pub use error::{SchemaError, SchemaResult};
pub use native::Native;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use opencodifier_core::{
        BooleanQuestion, Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion,
        DecisionRequest, RequestMetadata, State,
    };
    use serde_json::json;

    /// A request exercising all three question shapes, shared by the
    /// format-level tests below.
    pub(crate) fn sample_request() -> DecisionRequest {
        DecisionRequest::new(
            State::from_text("route this request across local models")
                .with_fact("context_tokens", opencodifier_core::FactValue::Integer(4_096)),
            vec![
                DecisionQuestion::Choice(
                    ChoiceQuestion::new(
                        "model",
                        "Which model?",
                        vec![
                            Candidate::new("local-qwen", "general coding").unwrap(),
                            Candidate::new("local-glm", "deep reasoning").unwrap(),
                        ],
                    )
                    .unwrap(),
                ),
                DecisionQuestion::Score(
                    opencodifier_core::ScoreQuestion::new(
                        "difficulty",
                        "How difficult?",
                        ["trivial", "hard", "expert"]
                            .iter()
                            .map(|label| opencodifier_core::ScoreLevel::new(*label).unwrap())
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

    #[test]
    fn every_format_has_a_stable_name() {
        assert_eq!(Native.name(), "native");
        assert_eq!(openai::OpenAi.name(), "openai");
        assert_eq!(anthropic::Anthropic.name(), "anthropic");
        assert_eq!(jev::Jev.name(), "jev");
    }

    #[test]
    fn formats_are_object_safe() {
        let formats: Vec<BoxedWireFormat> =
            vec![Box::new(Native), Box::new(openai::OpenAi), Box::new(jev::Jev)];
        assert_eq!(formats.len(), 3);
        assert!(!format!("{:?}", formats[0]).is_empty());
    }

    #[test]
    fn unknown_fields_are_tolerated_only_by_foreign_formats() {
        // A payload with extra vendor fields must decode through the
        // foreign adapters: no adapter is a version lock. Native is the
        // exception (`ARCHITECTURE.md` §7) — an unrecognized field there is
        // a caller bug, and it is reported, not ignored.
        let request = sample_request();
        let mut foreign = openai::OpenAi.encode_request(&request).unwrap();
        foreign.as_object_mut().unwrap().insert("x_future".to_owned(), json!(1));
        let limits = opencodifier_core::Limits::default();
        assert!(openai::OpenAi.decode_request(&foreign, &limits).is_ok());

        let mut native = Native.encode_request(&request).unwrap();
        native.as_object_mut().unwrap().insert("x_future".to_owned(), json!(1));
        let error = Native.decode_request(&native, &limits).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert!(error.to_string().contains("unknown field"), "{error}");
    }

    /// Every format that keys answers by question id refuses to encode a
    /// request whose ids repeat: two questions would collapse into one wire
    /// field. Native keeps the IR's list shape, so it is the one format
    /// that can carry them.
    #[test]
    fn keyed_formats_refuse_repeated_question_ids() {
        let first = sample_request();
        let repeated = opencodifier_core::DecisionRequest::new(
            first.state().clone(),
            vec![first.questions()[0].clone(), first.questions()[0].clone()],
            opencodifier_core::DecisionPolicy::default(),
            opencodifier_core::RequestMetadata::default(),
        )
        .unwrap();
        assert!(Native.encode_request(&repeated).is_ok(), "native is a list, not a keyed map");

        for (name, error) in [
            ("openai", openai::OpenAi.encode_request(&repeated).unwrap_err()),
            ("anthropic", anthropic::Anthropic.encode_request(&repeated).unwrap_err()),
            ("jev", jev::Jev.encode_request(&repeated).unwrap_err()),
        ] {
            assert_eq!(error.code(), "schema.invalid_value", "{name}");
            assert!(error.to_string().contains("appears twice"), "{name}: {error}");
        }
    }
}
