//! Validated identifier newtypes shared across the IR.
//!
//! All identifiers serialize transparently as plain JSON strings so the
//! canonical form is stable and human-readable. The same character set is
//! accepted everywhere (`[A-Za-z0-9_.-]`, max 128 bytes) which keeps
//! identifiers safe to embed in cache keys, trace output, and log lines
//! without escaping.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// Maximum byte length for any identifier.
const MAX_ID_LEN: usize = 128;

/// Validates the shared identifier charset and length bound.
fn validate(value: &str) -> CoreResult<()> {
    if value.is_empty() {
        return Err(CoreError::InvalidId {
            value: value.to_owned(),
            reason: "must not be empty".into(),
        });
    }
    if value.len() > MAX_ID_LEN {
        return Err(CoreError::InvalidId {
            value: value.to_owned(),
            reason: format!("exceeds {MAX_ID_LEN} bytes"),
        });
    }
    if !value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) {
        return Err(CoreError::InvalidId {
            value: value.to_owned(),
            reason: "only [A-Za-z0-9_.-] are allowed".into(),
        });
    }
    Ok(())
}

macro_rules! define_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Validates and wraps `value`.
            pub fn new(value: impl Into<String>) -> CoreResult<Self> {
                let value = value.into();
                validate(&value)?;
                Ok(Self(value))
            }

            /// Borrows the underlying string.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

define_id! {
    /// Identifies one candidate answer inside a choice question.
    CandidateId
}

define_id! {
    /// Identifies one question within a request.
    QuestionId
}

define_id! {
    /// Identifies one node in a decision graph.
    NodeId
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    #[test]
    fn accepts_valid_identifiers() {
        for ok in ["qwen", "amortyx.model_selection", "tool-gate_1", "A9-._"] {
            CandidateId::new(ok).unwrap_or_else(|e| panic!("rejected {ok}: {e}"));
        }
    }

    #[test]
    fn rejects_empty_and_illegal_identifiers() {
        assert!(matches!(CandidateId::new(""), Err(CoreError::InvalidId { .. })));
        assert!(matches!(CandidateId::new("has space"), Err(CoreError::InvalidId { .. })));
        assert!(matches!(CandidateId::new("tab\tchar"), Err(CoreError::InvalidId { .. })));
        assert!(matches!(CandidateId::new("é"), Err(CoreError::InvalidId { .. })));
    }

    #[test]
    fn rejects_overlong_identifiers() {
        let long = "a".repeat(MAX_ID_LEN + 1);
        assert!(matches!(
            CandidateId::new(long),
            Err(CoreError::InvalidId { reason, .. }) if reason.contains("128")
        ));
    }

    #[test]
    fn serializes_transparently_as_string() {
        let id = QuestionId::new("task_type").expect("valid");
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, r#""task_type""#);
        let back: QuestionId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, id);
    }
}
