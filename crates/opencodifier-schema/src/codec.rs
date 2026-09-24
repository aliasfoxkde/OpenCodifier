//! The adapter contract every wire format implements (PLANNING.md §7, §42).
//!
//! [`WireFormat`] is deliberately shaped around [`serde_json::Value`] in
//! and out rather than typed wire structs: `OpenAI`, `Anthropic`, and
//! `Jev` payloads have completely different field shapes, and forcing them
//! through one typed envelope would either invent fields the vendors do
//! not send or drop fields they do.
//!
//! # Contract
//!
//! * **Totality.** `decode_*` accepts any [`serde_json::Value`] within the
//!   supplied [`Limits`] and returns a typed [`SchemaError`] — never a
//!   panic, never an abandoned candidate, never a silently guessed answer.
//! * **Limits.** Every decoder enforces the same resource ceilings as
//!   [`opencodifier_core::DecisionRequest::new`] (question count, candidate
//!   count, input size) and reports violations as
//!   `schema.limit_exceeded`.
//! * **Forward compatibility.** Unknown wire fields are ignored. Wire
//!   structs deliberately do **not** use `deny_unknown_fields`; vendor
//!   payloads gain fields over time and an adapter must not become a
//!   version lock. (The native format is the exception — see the
//!   [`crate::native`] module docs.)
//! * **Fidelity is documented, not faked.** Where a wire format cannot
//!   carry the full IR (`OpenAI` and `Anthropic` answers carry the chosen
//!   value but no distribution), the adapter reconstructs a deterministic
//!   distribution and says so in its module docs.
//! * **The IR is the truth.** Encoders project the IR onto a wire format;
//!   decoders normalize wire payloads back into the IR. Nothing here
//!   executes a decision.

use opencodifier_core::{DecisionRequest, DecisionResponse, Limits};

use crate::error::{SchemaError, SchemaResult};

/// A bidirectional adapter between one external wire format and the
/// canonical decision IR.
///
/// Implementations are stateless value objects; the trait is object-safe
/// so a registry of formats can be built later (PLANNING.md §36 exposes
/// `/v1/decide` and `/v1/systemone` behind one server).
pub trait WireFormat: std::fmt::Debug {
    /// Stable, lowercase name of this wire format (`"native"`, `"openai"`,
    /// `"anthropic"`, `"jev"`).
    ///
    /// A method rather than an associated const so the trait stays
    /// dyn-compatible: [`BoxedWireFormat`] exists so a server can hold a
    /// registry of formats (PLANNING.md §36).
    fn name(&self) -> &'static str;

    /// Projects a canonical request onto this wire format.
    ///
    /// Returns [`SchemaError::UnsupportedConstruct`] when the request uses
    /// IR features this format cannot express.
    fn encode_request(&self, request: &DecisionRequest) -> SchemaResult<serde_json::Value>;

    /// Normalizes a wire request payload into the canonical IR.
    ///
    /// `limits` are the resource ceilings the decoded request must respect.
    fn decode_request(
        &self,
        payload: &serde_json::Value,
        limits: &Limits,
    ) -> SchemaResult<DecisionRequest>;

    /// Projects a canonical response onto this wire format.
    fn encode_response(&self, response: &DecisionResponse) -> SchemaResult<serde_json::Value>;

    /// Normalizes a wire response payload into the canonical IR.
    ///
    /// `request` is the request these answers respond to: adapters need the
    /// question shapes (candidate ids, ordered score levels) to map wire
    /// values back onto IR keys and to reconstruct distributions.
    fn decode_response(
        &self,
        payload: &serde_json::Value,
        request: &DecisionRequest,
        limits: &Limits,
    ) -> SchemaResult<DecisionResponse>;
}

/// Object-safe alias for a boxed format, ready for a registry.
pub type BoxedWireFormat = Box<dyn WireFormat>;

/// Enforces the resource ceilings shared by every decoder.
///
/// Called before any IR constructor runs so oversized payloads fail with a
/// precise `schema.limit_exceeded` code naming the violated ceiling instead
/// of the IR's generic policy error.
pub(crate) fn enforce_limits(
    state_bytes: usize,
    question_count: usize,
    limits: &Limits,
) -> SchemaResult<()> {
    if state_bytes > limits.max_input_bytes {
        return Err(SchemaError::limit("max_input_bytes", state_bytes, limits.max_input_bytes));
    }
    if question_count == 0 {
        return Err(SchemaError::EmptyQuestions);
    }
    if question_count > limits.max_questions {
        return Err(SchemaError::limit("max_questions", question_count, limits.max_questions));
    }
    Ok(())
}

/// Enforces the per-choice-question candidate ceiling.
pub(crate) fn enforce_candidate_limit(candidate_count: usize, limits: &Limits) -> SchemaResult<()> {
    if candidate_count > limits.max_candidates {
        return Err(SchemaError::limit("max_candidates", candidate_count, limits.max_candidates));
    }
    Ok(())
}

/// Rejects a request whose question ids repeat.
///
/// Every keyed wire format (`OpenAI` properties, `Anthropic` properties,
/// `Jev` answer maps) answers by question id, so a repeated id has no
/// unrepresentable-but-harmless form: it would silently collapse two
/// questions into one wire field. Encoders refuse it instead; the native
/// format keeps the IR's list shape and does not need this check.
pub(crate) fn enforce_unique_questions(request: &DecisionRequest) -> SchemaResult<()> {
    let mut seen: Vec<&str> = Vec::with_capacity(request.questions().len());
    for question in request.questions() {
        let id = question.id().as_str();
        if seen.contains(&id) {
            return Err(SchemaError::invalid_value(
                "questions",
                format!(
                    "question id `{id}` appears twice; every format but native keys answers by id"
                ),
            ));
        }
        seen.push(id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    #[test]
    fn enforce_limits_rejects_oversized_state() {
        let limits = Limits { max_input_bytes: 4, ..Limits::default() };
        let error = enforce_limits(5, 1, &limits).unwrap_err();
        assert_eq!(error.code(), "schema.limit_exceeded");
        assert!(matches!(error, SchemaError::LimitExceeded { limit: "max_input_bytes", .. }));
    }

    #[test]
    fn enforce_limits_rejects_empty_and_oversized_question_counts() {
        let limits = Limits { max_questions: 2, ..Limits::default() };
        assert_eq!(enforce_limits(0, 0, &limits).unwrap_err().code(), "schema.empty_questions");
        let error = enforce_limits(0, 3, &limits).unwrap_err();
        assert!(matches!(error, SchemaError::LimitExceeded { limit: "max_questions", .. }));
        assert!(enforce_limits(0, 2, &limits).is_ok());
    }

    #[test]
    fn enforce_unique_questions_accepts_distinct_ids() {
        let request = crate::tests::sample_request();
        assert!(enforce_unique_questions(&request).is_ok());
    }

    #[test]
    fn enforce_unique_questions_rejects_repeated_ids() {
        let first = crate::tests::sample_request();
        let request = opencodifier_core::DecisionRequest::new(
            first.state().clone(),
            vec![first.questions()[0].clone(), first.questions()[0].clone()],
            opencodifier_core::DecisionPolicy::default(),
            opencodifier_core::RequestMetadata::default(),
        )
        .unwrap();
        let error = enforce_unique_questions(&request).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert!(error.to_string().contains("appears twice"), "{error}");
    }
}
