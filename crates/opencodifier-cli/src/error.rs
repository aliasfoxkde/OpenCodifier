//! The CLI's typed error: every diagnostic carries a stable code and an
//! exit code.
//!
//! Exit codes are part of the binary's contract with scripts and CI:
//!
//! * `0` — success, including an abstention when `--abstain-is-success`
//!   is passed.
//! * `1` — input error: unreadable input, malformed JSON, a schema decode
//!   failure, a rejected graph or manifest, bad arguments, or a
//!   non-loopback `serve` bind without `--allow-remote`.
//! * `2` — policy-gate escalation: the engine refused to decide and
//!   `--abstain-is-success` was not passed.
//! * `3` — internal error: the decision runtime itself failed.
//!
//! Codes are relayed verbatim from the crate that owns the failure
//! (`schema.*`, `graph.*`, `engine.*`, `ir.*`, `model.*`, `http.*`); only
//! the `cli.*` codes below are this crate's own, because only argument and
//! stream handling is CLI-specific.

use std::io::Write as _;
use std::process::ExitCode;

use opencodifier_core::CoreError;
use opencodifier_engine::EngineError;
use opencodifier_model::ModelError;
use opencodifier_schema::SchemaError;

/// Exit code for a successful run.
pub const EXIT_SUCCESS: u8 = 0;

/// Exit code for an input error: the caller gave the CLI something it
/// cannot use.
pub const EXIT_INPUT: u8 = 1;

/// Exit code for a policy-gate escalation: the engine answered, but not
/// decisively enough to act on.
pub const EXIT_ESCALATION: u8 = 2;

/// Exit code for an internal error: the runtime itself failed.
pub const EXIT_INTERNAL: u8 = 3;

/// Stable code reported when the engine's outcome is not decisive.
pub const CODE_ESCALATED: &str = "cli.escalated";

/// Stable code reported when an input file (or stdin) cannot be read.
pub const CODE_UNREADABLE_INPUT: &str = "cli.unreadable_input";

/// Stable code for malformed JSON, matching [`SchemaError::Json`]'s own
/// code so a payload the schema layer never saw is reported identically to
/// one it rejected.
pub const CODE_INVALID_JSON: &str = "schema.invalid_json";

/// Stable code for a `--policy` document that is not a `DecisionPolicy` the
/// IR accepts. Matches [`SchemaError::InvalidValue`]'s code: a policy file
/// is caller input of the same standing as a request payload.
pub const CODE_INVALID_POLICY: &str = "schema.invalid_value";

/// Stable code reported when a graph document does not have the graph wire
/// shape (`version` plus a `nodes` array of node specs).
pub const CODE_INVALID_GRAPH_JSON: &str = "cli.invalid_graph_json";

/// Stable code reported when `--bind` is not a valid socket address.
pub const CODE_INVALID_BIND: &str = "cli.invalid_bind";

/// Stable code reported when a non-loopback bind is requested without
/// `--allow-remote`.
pub const CODE_BIND_REQUIRES_ALLOW_REMOTE: &str = "cli.bind_requires_allow_remote";

/// Stable code reported when a `--policy` file cannot be used by the
/// subcommand that received it.
pub const CODE_POLICY_INAPPLICABLE: &str = "cli.policy_inapplicable";

/// Stable code reported when the CLI cannot write its own output.
pub const CODE_OUTPUT_FAILED: &str = "cli.output_failed";

/// Stable code reported when the async runtime cannot be built for `serve`.
pub const CODE_RUNTIME_FAILED: &str = "cli.runtime_failed";

/// Everything the CLI can report, with the exit code it maps onto.
#[derive(Debug)]
pub enum CliError {
    /// The caller's input was rejected: bad arguments, unreadable files,
    /// malformed JSON, or a payload the schema, graph, or manifest
    /// validators refused. Always exit [`EXIT_INPUT`].
    Input {
        /// Stable machine-readable code.
        code: &'static str,
        /// Human-readable explanation.
        message: String,
    },

    /// The decision runtime failed while assembling or executing.
    /// Always exit [`EXIT_INTERNAL`].
    Engine {
        /// Stable machine-readable code.
        code: &'static str,
        /// Human-readable explanation.
        message: String,
    },

    /// The engine produced a response whose outcome is not decisive
    /// (`abstain`, `escalate`, `verify`, `no_valid_candidate`). This is the
    /// runtime working as designed, not a fault: exit [`EXIT_ESCALATION`]
    /// so callers can distinguish "no answer" from "broken", unless
    /// `--abstain-is-success` was passed.
    Escalated {
        /// Stable machine-readable code.
        code: &'static str,
        /// Human-readable explanation.
        message: String,
    },
}

impl CliError {
    /// Builds an input error (exit 1).
    pub fn input(code: &'static str, message: impl Into<String>) -> Self {
        Self::Input { code, message: message.into() }
    }

    /// Builds an internal error (exit 3).
    pub fn engine(code: &'static str, message: impl Into<String>) -> Self {
        Self::Engine { code, message: message.into() }
    }

    /// Builds a policy-gate escalation (exit 2).
    pub fn escalated(code: &'static str, message: impl Into<String>) -> Self {
        Self::Escalated { code, message: message.into() }
    }

    /// The stable machine-readable code for this failure.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Input { code, .. } | Self::Engine { code, .. } | Self::Escalated { code, .. } => {
                code
            }
        }
    }

    /// The exit code this failure maps onto.
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::Input { .. } => ExitCode::from(EXIT_INPUT),
            Self::Escalated { .. } => ExitCode::from(EXIT_ESCALATION),
            Self::Engine { .. } => ExitCode::from(EXIT_INTERNAL),
        }
    }

    /// Writes the diagnostic to `stderr` as `opencodifier: <code> <message>`
    /// and returns the exit code to terminate with.
    pub fn report(&self) -> ExitCode {
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "opencodifier: {} {}", self.code(), self);
        let _ = stderr.flush();
        self.exit_code()
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Input { message, .. }
            | Self::Engine { message, .. }
            | Self::Escalated { message, .. } => formatter.write_str(message),
        }
    }
}

impl std::error::Error for CliError {}

impl From<EngineError> for CliError {
    fn from(error: EngineError) -> Self {
        Self::engine(error.code(), error.to_string())
    }
}

impl From<SchemaError> for CliError {
    fn from(error: SchemaError) -> Self {
        Self::input(error.code(), error.to_string())
    }
}

impl From<ModelError> for CliError {
    fn from(error: ModelError) -> Self {
        Self::input(error.code(), error.to_string())
    }
}

impl From<CoreError> for CliError {
    fn from(error: CoreError) -> Self {
        Self::input(error.code(), error.to_string())
    }
}

/// Reports a clap parse failure and returns the exit code to terminate
/// with.
///
/// Clap's own default is exit `2`, which this CLI reserves for a
/// policy-gate escalation, so bad arguments are normalized to
/// [`EXIT_INPUT`]. `--help` and `--version` remain a success: clap writes
/// them to the right stream itself.
pub fn report_clap_error(error: &clap::Error) -> ExitCode {
    let _ = error.print();
    match error.kind() {
        clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
            ExitCode::from(EXIT_SUCCESS)
        }
        _ => ExitCode::from(EXIT_INPUT),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use opencodifier_core::CoreError;
    use opencodifier_engine::EngineError;
    use opencodifier_schema::SchemaError;

    #[test]
    fn conversions_carry_the_owning_crate_code_and_exit() {
        let engine = CliError::from(EngineError::Cancelled);
        assert_eq!(engine.code(), "engine.cancelled");
        assert_eq!(engine.exit_code(), ExitCode::from(EXIT_INTERNAL));

        let schema = CliError::from(SchemaError::Json("eof while parsing".into()));
        assert_eq!(schema.code(), "schema.invalid_json");
        assert_eq!(schema.exit_code(), ExitCode::from(EXIT_INPUT));

        let core = CliError::from(CoreError::EmptyQuestions);
        assert!(core.code().starts_with("ir."));
        assert_eq!(core.exit_code(), ExitCode::from(EXIT_INPUT));
    }

    #[test]
    fn the_escalation_exit_code_is_two_and_the_display_is_the_message() {
        let escalated =
            CliError::escalated(CODE_ESCALATED, "engine outcome `abstain` is not decisive");
        assert_eq!(escalated.exit_code(), ExitCode::from(EXIT_ESCALATION));
        assert_eq!(escalated.to_string(), "engine outcome `abstain` is not decisive");
        let internal = CliError::engine(CODE_RUNTIME_FAILED, "no async runtime");
        assert_eq!(internal.to_string(), "no async runtime");
        assert_eq!(internal.exit_code(), ExitCode::from(EXIT_INTERNAL));
    }
}
