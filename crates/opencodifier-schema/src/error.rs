//! Typed errors for wire-format normalization (PLANNING.md §42, §8).
//!
//! Every failure mode of every adapter lives here so callers can branch on
//! a single enum while still reporting stable machine-readable codes. The
//! code strings share the `schema.` prefix and are part of the public
//! contract: new codes may appear, existing ones never change meaning.
//!
//! Errors are *total-input* errors, not internal failures: an adapter fed
//! any input within the request resource limits returns one of these
//! variants rather than panicking (PLANNING.md §7 "Failure posture",
//! `ARCHITECTURE.md` §7).

use opencodifier_core::CoreError;

/// Everything that can go wrong while normalizing a wire format into the
/// canonical IR, or projecting the IR back onto a wire format.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SchemaError {
    /// The wire payload uses a construct this format cannot express as a
    /// decision (e.g. an `OpenAI` strict schema using `oneOf`, which `OpenAI`
    /// does not document for structured outputs).
    #[error("unsupported construct `{construct}`{context}")]
    UnsupportedConstruct {
        /// The JSON Schema keyword or wire construct that was rejected.
        construct: &'static str,
        /// Extra explanation appended to the message, possibly empty.
        context: String,
    },

    /// A free-form generation field (`"type": "string"` with no enum,
    /// bounds, or other decision semantics) appeared in a schema
    /// (PLANNING.md §8: "do not pretend to support arbitrary generation").
    #[error("free-form generation field `{field}` cannot become a decision task")]
    UnsupportedGenerationField {
        /// Name of the offending schema property.
        field: String,
    },

    /// A required wire field was absent.
    #[error("missing required field `{field}`{context}")]
    MissingField {
        /// Name of the absent field.
        field: String,
        /// Extra explanation appended to the message, possibly empty.
        context: String,
    },

    /// A field was present but had the wrong JSON type (string where an
    /// object was required, a `noul` answer where a number was expected).
    #[error("field `{field}` has an invalid type: expected {expected}, got {got}")]
    InvalidType {
        /// Name of the offending field.
        field: String,
        /// The JSON type the adapter needed.
        expected: &'static str,
        /// The JSON type that was actually found.
        got: String,
    },

    /// A value was well-typed but outside its domain: an unknown `Jev`
    /// `type` discriminator, a probability outside `[0, 1]`, a level index
    /// past the end of the level list, an empty candidate map.
    #[error("invalid value for `{field}`: {reason}")]
    InvalidValue {
        /// Name of the offending field.
        field: String,
        /// Why the value was rejected.
        reason: String,
    },

    /// A wire payload exceeded a [`opencodifier_core::Limits`] ceiling
    /// (question count, candidate count, input size).
    #[error("limit `{limit}` exceeded: {actual} > {max}")]
    LimitExceeded {
        /// Which limit was hit.
        limit: &'static str,
        /// The observed magnitude.
        actual: usize,
        /// The configured ceiling.
        max: usize,
    },

    /// A request carried no questions; a request that asks for no
    /// decision is not decidable.
    #[error("request must contain at least one question")]
    EmptyQuestions,

    /// The underlying canonical IR refused a value the wire layer had
    /// already accepted (e.g. a candidate id with illegal characters).
    /// The IR is the final validator; this variant preserves its code.
    #[error(transparent)]
    Core(#[from] CoreError),

    /// The payload was not valid JSON at all.
    #[error("payload is not valid JSON: {0}")]
    Json(String),
}

impl SchemaError {
    /// Convenience constructor for [`SchemaError::UnsupportedConstruct`]
    /// with no extra context.
    #[must_use]
    pub fn unsupported(construct: &'static str) -> Self {
        Self::UnsupportedConstruct { construct, context: String::new() }
    }

    /// Convenience constructor for [`SchemaError::MissingField`] with no
    /// extra context.
    #[must_use]
    pub fn missing(field: impl Into<String>) -> Self {
        Self::MissingField { field: field.into(), context: String::new() }
    }

    /// Convenience constructor for [`SchemaError::InvalidType`].
    #[must_use]
    pub fn invalid_type(
        field: impl Into<String>,
        expected: &'static str,
        got: &serde_json::Value,
    ) -> Self {
        Self::InvalidType { field: field.into(), expected, got: json_type_name(got).to_owned() }
    }

    /// Convenience constructor for [`SchemaError::InvalidValue`].
    #[must_use]
    pub fn invalid_value(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidValue { field: field.into(), reason: reason.into() }
    }

    /// Convenience constructor for [`SchemaError::LimitExceeded`].
    #[must_use]
    pub fn limit(limit: &'static str, actual: usize, max: usize) -> Self {
        Self::LimitExceeded { limit, actual, max }
    }

    /// Stable machine-readable code for this error, suitable for mapping
    /// onto HTTP statuses and MCP error payloads.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedConstruct { .. } => "schema.unsupported_construct",
            Self::UnsupportedGenerationField { .. } => "schema.unsupported_generation_field",
            Self::MissingField { .. } => "schema.missing_field",
            Self::InvalidType { .. } => "schema.invalid_type",
            Self::InvalidValue { .. } => "schema.invalid_value",
            Self::LimitExceeded { .. } => "schema.limit_exceeded",
            Self::EmptyQuestions => "schema.empty_questions",
            Self::Core(error) => match error.code() {
                // Empty questions is the one IR failure wire decoding can
                // produce directly, and callers expect the schema-prefixed
                // code for it.
                "ir.empty_questions" => "schema.empty_questions",
                // The IR's own validating constructors catch an empty
                // required field on the way through; everything else is a
                // value the wire payload got wrong.
                "ir.empty_field" => "schema.missing_field",
                _ => "schema.invalid_value",
            },
            Self::Json(_) => "schema.invalid_json",
        }
    }
}

/// The JSON type of `value`, as the error messages name it.
fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Alias used throughout the crate for fallible adapter calls.
pub type SchemaResult<T> = Result<T, SchemaError>;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use opencodifier_core::QuestionId;

    #[test]
    fn codes_are_stable_and_schema_prefixed() {
        let cases: [(SchemaError, &str); 9] = [
            (SchemaError::unsupported("oneOf"), "schema.unsupported_construct"),
            (
                SchemaError::UnsupportedGenerationField { field: "explanation".into() },
                "schema.unsupported_generation_field",
            ),
            (SchemaError::missing("state"), "schema.missing_field"),
            (
                SchemaError::invalid_type("state", "string", &serde_json::json!(1)),
                "schema.invalid_type",
            ),
            (SchemaError::invalid_value("type", "unknown discriminator"), "schema.invalid_value"),
            (SchemaError::limit("max_questions", 9, 8), "schema.limit_exceeded"),
            (SchemaError::EmptyQuestions, "schema.empty_questions"),
            (SchemaError::Json("eof".into()), "schema.invalid_json"),
            (
                SchemaError::from(opencodifier_core::CoreError::EmptyQuestions),
                "schema.empty_questions",
            ),
        ];
        for (error, code) in cases {
            assert!(code.starts_with("schema."), "{code}");
            assert_eq!(error.code(), code);
        }
    }

    #[test]
    fn core_errors_map_onto_schema_codes() {
        let invalid_id = QuestionId::new("has space").unwrap_err();
        assert_eq!(SchemaError::from(invalid_id).code(), "schema.invalid_value");

        let not_normalized =
            opencodifier_core::CoreError::DistributionNotNormalized { sum: 0.9, tolerance: 1e-6 };
        assert_eq!(SchemaError::from(not_normalized).code(), "schema.invalid_value");
    }

    #[test]
    fn convenience_constructors_render_readable_messages() {
        let error = SchemaError::unsupported("oneOf");
        assert_eq!(error.to_string(), "unsupported construct `oneOf`");

        let error = SchemaError::missing("questions");
        assert_eq!(error.to_string(), "missing required field `questions`");

        let error = SchemaError::invalid_type("state", "string", &serde_json::json!([1]));
        assert_eq!(
            error.to_string(),
            "field `state` has an invalid type: expected string, got array"
        );

        let error = SchemaError::invalid_type("state", "string", &serde_json::Value::Null);
        assert_eq!(
            error.to_string(),
            "field `state` has an invalid type: expected string, got null"
        );

        let error = SchemaError::invalid_type("state", "string", &serde_json::json!(true));
        assert_eq!(
            error.to_string(),
            "field `state` has an invalid type: expected string, got boolean"
        );

        let error = SchemaError::invalid_value("noul", "probability below 0.5");
        assert_eq!(error.to_string(), "invalid value for `noul`: probability below 0.5");

        let error = SchemaError::limit("max_candidates", 300, 256);
        assert_eq!(error.to_string(), "limit `max_candidates` exceeded: 300 > 256");

        let error = SchemaError::UnsupportedGenerationField { field: "explanation".into() };
        assert_eq!(
            error.to_string(),
            "free-form generation field `explanation` cannot become a decision task"
        );
    }

    #[test]
    fn errors_are_cloneable_and_comparable() {
        let error = SchemaError::missing("state");
        assert_eq!(error.clone(), error);
    }
}
