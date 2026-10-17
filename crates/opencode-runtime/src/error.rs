//! Error type for tensor validation and backend execution failures.
//!
//! Like every `OpenCodifier` error type, [`RuntimeError`] carries a stable
//! machine-readable [`RuntimeError::code`]. Codes are part of the public
//! contract: new codes may appear, existing strings never change meaning.
//! Interface layers map them onto HTTP statuses and MCP payloads, so a
//! backend failure is diagnosable without pattern-matching on prose.

/// Everything that can go wrong inside the runtime layer.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum RuntimeError {
    /// A tensor was malformed: empty shape, a zero dimension, a
    /// shape/length mismatch, or an element count above
    /// [`MAX_TENSOR_ELEMENTS`](crate::tensor::MAX_TENSOR_ELEMENTS).
    #[error("invalid tensor: {reason}")]
    InvalidTensor {
        /// Human-readable explanation of which invariant failed.
        reason: String,
    },

    /// A tensor or embedding contained a non-finite value (`NaN` or
    /// infinity). Non-finite probabilities poison calibration downstream,
    /// so they are rejected at the boundary rather than propagated.
    #[error("non-finite value in {where_}")]
    NonFiniteValue {
        /// Which argument carried the value, e.g. `"tensor data"` or
        /// `"embedding for text 3"`.
        where_: String,
    },

    /// A named input the backend requires was absent from the call.
    #[error("required input `{name}` was not supplied")]
    MissingInput {
        /// The name of the absent input tensor.
        name: String,
    },

    /// The backend itself failed. The message is whatever the underlying
    /// implementation reported; the runtime layer never fabricates one.
    #[error("backend `{model_id}` failed: {message}")]
    BackendFailed {
        /// The [`InferenceBackend::model_id`](crate::backend::InferenceBackend::model_id)
        /// of the failing backend.
        model_id: String,
        /// The underlying failure message, passed through unchanged.
        message: String,
    },
}

impl RuntimeError {
    /// The stable machine-readable code for this error.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidTensor { .. } => "runtime.invalid_tensor",
            Self::NonFiniteValue { .. } => "runtime.non_finite",
            Self::MissingInput { .. } => "runtime.missing_input",
            Self::BackendFailed { .. } => "runtime.backend_failed",
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn codes_are_stable_and_distinct() {
        let errors = [
            RuntimeError::InvalidTensor { reason: "empty shape".into() },
            RuntimeError::NonFiniteValue { where_: "tensor data".into() },
            RuntimeError::MissingInput { name: "logits".into() },
            RuntimeError::BackendFailed { model_id: "m".into(), message: "boom".into() },
        ];
        let codes: Vec<&str> = errors.iter().map(RuntimeError::code).collect();
        for code in &codes {
            assert!(code.starts_with("runtime."), "{code} must be namespaced");
        }
        let unique: std::collections::HashSet<&&str> = codes.iter().collect();
        assert_eq!(unique.len(), codes.len(), "codes must be distinct: {codes:?}");

        assert_eq!(
            errors[3].to_string(),
            "backend `m` failed: boom",
            "Display must carry the model id and the underlying message"
        );
    }
}
