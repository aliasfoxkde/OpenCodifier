//! Stable `config.*` error codes for configuration failures (#190).

use std::fmt;

/// Errors raised while discovering, reading, or validating configuration.
#[derive(Debug)]
#[non_exhaustive]
pub enum ConfigError {
    /// `$OPENCODIFIER_CONFIG` names a file that does not exist. Hard
    /// error: an explicit pointer whose target is missing hides operator
    /// intent if silently ignored.
    EnvPointeeMissing {
        /// The missing path.
        path: String,
    },
    /// The file exists but could not be read.
    Unreadable {
        /// The path.
        path: String,
        /// The underlying I/O message.
        reason: String,
    },
    /// The file is not valid JSON.
    Parse {
        /// The path.
        path: String,
        /// The serde message.
        reason: String,
    },
    /// A key outside the documented schema. Strict: unknown keys fail,
    /// they never silently degrade.
    UnknownKey {
        /// The path the document came from.
        path: String,
        /// The offending key, dotted.
        key: String,
    },
    /// A value is outside its documented range.
    OutOfRange {
        /// The path the document came from.
        path: String,
        /// The offending key, dotted.
        key: String,
        /// Why the value is out of range.
        reason: String,
    },
}

impl ConfigError {
    /// The stable machine-readable code (`config.*`), suitable for wire
    /// error envelopes.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::EnvPointeeMissing { .. } => "config.env_pointee_missing",
            Self::Unreadable { .. } => "config.unreadable",
            Self::Parse { .. } => "config.parse",
            Self::UnknownKey { .. } => "config.unknown_key",
            Self::OutOfRange { .. } => "config.out_of_range",
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EnvPointeeMissing { path } => {
                write!(f, "{}: $OPENCODIFIER_CONFIG names a missing file: {path}", self.code())
            }
            Self::Unreadable { path, reason } => {
                write!(f, "{}: cannot read {path}: {reason}", self.code())
            }
            Self::Parse { path, reason } => {
                write!(f, "{}: {path}: {reason}", self.code())
            }
            Self::UnknownKey { path, key } => {
                write!(f, "{}: {path}: unknown key `{key}`", self.code())
            }
            Self::OutOfRange { path, key, reason } => {
                write!(f, "{}: {path}: `{key}` {reason}", self.code())
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// Convenience alias for configuration results.
pub type ConfigResult<T> = Result<T, ConfigError>;
