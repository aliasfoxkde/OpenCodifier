//! `opencodifier decide`: one request in, one canonical response out.
//!
//! The payload is normalized by the selected wire-format adapter, decided
//! by the deterministic zero-ML engine, and printed in the canonical native
//! format. Abstention is a successful outcome semantically, so it is still
//! printed; only the exit code distinguishes it (2, or 0 with
//! `--abstain-is-success`).

use std::path::Path;

use opencodifier_core::{DecisionOutcome, DecisionRequest, Limits};
use opencodifier_engine::{EngineConfig, EngineHandle};
use opencodifier_schema::WireFormat;
use opencodifier_schema::native::Native;

use crate::args::DecideArgs;
use crate::error::{CODE_ESCALATED, CliError};
use crate::{input, output, report};

/// Runs `decide`.
///
/// # Errors
///
/// [`CliError::input`] for anything the caller supplied (unreadable or
/// malformed payload, a refused wire format, a rejected policy), and
/// [`CliError::engine`] when the runtime itself fails.
pub(crate) fn run(args: &DecideArgs) -> Result<(), CliError> {
    let payload = input::read(&args.input)?;
    let document = input::parse_json(&payload)?;

    let request = args.format.wire_format().decode_request(&document, &Limits::default())?;
    let request = match &args.policy {
        Some(path) => with_policy_override(&request, path)?,
        None => request,
    };

    let handle = EngineHandle::lexical(EngineConfig::with_default_pipeline()?)?;
    let (response, executed) = handle.decide_with_report(&request)?;

    output::print_json(&Native.encode_response(&response)?)?;
    if args.trace {
        output::print_json(&report::execution_json(&executed))?;
    }

    finish(response.outcome(), args.abstain_is_success)
}

/// Replaces the request's policy with the one in `path`.
///
/// The request is rebuilt through the IR's validating constructor, so a
/// contradictory `--policy` is rejected exactly as an in-band one would be
/// — a payload cannot smuggle an incoherent cascade past the CLI.
///
/// # Errors
///
/// [`CliError::input`] when the file is not a valid
/// [`DecisionPolicy`](opencodifier_core::DecisionPolicy) or
/// the rebuilt request violates the IR's own limits.
fn with_policy_override(
    request: &DecisionRequest,
    path: &Path,
) -> Result<DecisionRequest, CliError> {
    let policy = input::read_policy(path)?;
    DecisionRequest::new(
        request.state().clone(),
        request.questions().to_vec(),
        policy,
        request.metadata().clone(),
    )
    .map_err(CliError::from)
}

/// Maps the outcome onto the exit code: decisive outcomes and
/// `--abstain-is-success` succeed, anything else is an escalation.
///
/// # Errors
///
/// [`CliError::escalated`] when the outcome is not decisive and the caller
/// did not ask for abstention-as-success.
fn finish(outcome: DecisionOutcome, abstain_is_success: bool) -> Result<(), CliError> {
    if outcome.is_decisive() || abstain_is_success {
        return Ok(());
    }
    Err(CliError::escalated(
        CODE_ESCALATED,
        format!(
            "engine outcome `{}` is not decisive; the response above is still the answer — \
             pass --abstain-is-success to exit 0",
            outcome_label(outcome)
        ),
    ))
}

/// The `snake_case` wire name of an outcome (`accept`, `abstain`, ...), as
/// it appears in the printed response.
fn outcome_label(outcome: DecisionOutcome) -> String {
    serde_json::to_value(outcome)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{outcome:?}"))
}
