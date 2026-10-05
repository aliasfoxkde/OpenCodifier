//! Error type for the model layer: manifests, embedding scoring, and the
//! candidate-conditioned inference contract.
//!
//! [`ModelError`] never wraps a backend error opaquely: the underlying
//! [`RuntimeError`](opencodifier_runtime::RuntimeError) is flattened into
//! its message when surfaced through the engine's
//! [`Classifier`](opencodifier_engine::Classifier) seam, and kept typed
//! only where the model layer itself produced it.

use std::path::PathBuf;

/// Everything that can go wrong inside the model layer.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum ModelError {
    /// A manifest file was malformed: not JSON, missing required fields,
    /// or carrying values outside their documented shape (a SHA-256 field
    /// that is not 64 hex characters, a non-positive dimension, ...).
    #[error("invalid model manifest `{manifest}`: {reason}")]
    InvalidManifest {
        /// Path or name the manifest was loaded from.
        manifest: String,
        /// Which invariant failed.
        reason: String,
    },

    /// A model artifact's SHA-256 digest did not match its manifest.
    ///
    /// A mismatched artifact must never run: cache keys and calibration
    /// are tied to the exact bytes the manifest pins (DECISIONS.md D14).
    #[error("model artifact `{artifact}` sha256 mismatch: expected {expected}, got {actual}")]
    Sha256Mismatch {
        /// Path of the offending artifact.
        artifact: PathBuf,
        /// Digest recorded in the manifest.
        expected: String,
        /// Digest actually computed from the file's bytes.
        actual: String,
    },

    /// A model artifact could not be read (missing file, permission
    /// denied, I/O error).
    #[error("model artifact `{artifact}` could not be read: {reason}")]
    UnreadableArtifact {
        /// Path of the artifact.
        artifact: PathBuf,
        /// The underlying I/O error message.
        reason: String,
    },

    /// A backend returned a tensor that violates the candidate-conditioned
    /// contract (wrong name, wrong rank, logits row count disagreeing with
    /// the candidate count).
    #[error("candidate-conditioned contract violated: {reason}")]
    ContractViolation {
        /// Which part of the contract was violated.
        reason: String,
    },

    /// A request rendered to more tokens than the Kai-0.6B-ONNX rung's
    /// input budget. The upstream contract refuses over-budget input
    /// (`max_length_exceeded`) and never truncates, so neither does this
    /// one: a truncated prompt would move every option pointer.
    #[error("prompt is {tokens} tokens, above the {max_tokens} token budget for `{id}`")]
    PromptTooLong {
        /// The request identifier carried into the message.
        id: String,
        /// How many tokens the rendered prompt actually needs.
        tokens: usize,
        /// The rung's token budget.
        max_tokens: usize,
    },

    /// A tokenizer asset could not be loaded, or produced an encoding the
    /// contract cannot use (an empty option segment, an unknown symbol).
    #[error("tokenizer failed: {reason}")]
    Tokenizer {
        /// What the tokenizer refused, and why.
        reason: String,
    },
}

impl ModelError {
    /// The stable machine-readable code for this error.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidManifest { .. } => "model.invalid_manifest",
            Self::Sha256Mismatch { .. } => "model.sha256_mismatch",
            Self::UnreadableArtifact { .. } => "model.unreadable_artifact",
            Self::ContractViolation { .. } => "model.contract_violation",
            Self::PromptTooLong { .. } => "model.prompt_too_long",
            Self::Tokenizer { .. } => "model.tokenizer",
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
            ModelError::InvalidManifest { manifest: "m.json".into(), reason: "no sha".into() },
            ModelError::Sha256Mismatch {
                artifact: PathBuf::from("model.onnx"),
                expected: "a".repeat(64),
                actual: "b".repeat(64),
            },
            ModelError::UnreadableArtifact {
                artifact: PathBuf::from("model.onnx"),
                reason: "not found".into(),
            },
            ModelError::ContractViolation { reason: "rank 3".into() },
            ModelError::PromptTooLong { id: "q".into(), tokens: 9_000, max_tokens: 8_192 },
            ModelError::Tokenizer { reason: "no vocab".into() },
        ];
        let codes: Vec<&str> = errors.iter().map(ModelError::code).collect();
        assert!(codes.iter().all(|code| code.starts_with("model.")), "{codes:?}");
        let unique: std::collections::HashSet<&&str> = codes.iter().collect();
        assert_eq!(unique.len(), codes.len(), "codes must be distinct: {codes:?}");

        assert_eq!(
            errors[1].to_string(),
            format!(
                "model artifact `model.onnx` sha256 mismatch: expected {}, got {}",
                "a".repeat(64),
                "b".repeat(64)
            )
        );
    }
}
