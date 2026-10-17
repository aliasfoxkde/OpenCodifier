//! `opencodifier decide`: one request in, one canonical response out.
//!
//! The payload is normalized by the selected wire-format adapter, decided
//! by the deterministic zero-ML engine, and printed in the canonical native
//! format. Abstention is a successful outcome semantically, so it is still
//! printed; only the exit code distinguishes it (2, or 0 with
//! `--abstain-is-success`).

use std::path::{Path, PathBuf};

use opencodifier_core::{
    DecisionOutcome, DecisionRequest, ImageInput, ImageMediaType, Limits, MAX_ENCODED_IMAGE_BYTES,
};
use opencodifier_schema::WireFormat;
use opencodifier_schema::native::Native;

use crate::args::DecideArgs;
use crate::error::{CODE_ESCALATED, CODE_UNREADABLE_INPUT, CODE_UNSUPPORTED_IMAGE, CliError};
use crate::{input, output, runtime};

/// The PNG magic bytes (`§89 50 4E 47 0D 0A 1A 0A`).
const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// The JPEG magic bytes (`FF D8 FF` — enough to delimit the format).
const JPEG_MAGIC: [u8; 3] = [0xFF, 0xD8, 0xFF];

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
    let request =
        (if args.image.is_empty() { Ok(request) } else { with_images(&request, &args.image) })?;

    let handle = runtime::handle(
        None,
        args.focus_budget,
        args.ladder.as_deref(),
        runtime::model_rung(&args.llama)?.as_ref(),
        runtime::vision_rung(args.vision.vision_model_dir.as_deref())?,
    )?;
    let (response, executed) = handle.decide_with_report(&request)?;

    output::print_json(&Native.encode_response(&response)?)?;
    if args.trace {
        output::print_json(&opencodifier_engine::execution_json(&executed))?;
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

/// Attaches `--image` files to the request's state.
///
/// Each file's media type is detected from the magic bytes (PNG or
/// JPEG — the IR's image media types are closed; a new format means a
/// new variant plus the decoder that justifies it) and required to fit
/// [`MAX_ENCODED_IMAGE_BYTES`] before the bytes are re-read out of the
/// buffer. The request is rebuilt through the IR's validating
/// constructor, so count and per-image limits apply exactly as an
/// in-band attachment.
///
/// # Errors
///
/// [`CliError::input`] for an unreadable file, an unrecognized image
/// format, or a rebuilt request a limit refuses.
fn with_images(request: &DecisionRequest, paths: &[PathBuf]) -> Result<DecisionRequest, CliError> {
    let mut state = request.state().clone();
    for path in paths {
        let bytes = std::fs::read(path).map_err(|error| {
            CliError::input(CODE_UNREADABLE_INPUT, format!("{}: {error}", path.display()))
        })?;
        if bytes.len() > MAX_ENCODED_IMAGE_BYTES {
            return Err(CliError::input(
                CODE_UNSUPPORTED_IMAGE,
                format!(
                    "{}: {} encoded bytes is above the {} ceiling",
                    path.display(),
                    bytes.len(),
                    MAX_ENCODED_IMAGE_BYTES
                ),
            ));
        }
        let media_type = if bytes.starts_with(&PNG_MAGIC) {
            ImageMediaType::Png
        } else if bytes.starts_with(&JPEG_MAGIC) {
            ImageMediaType::Jpeg
        } else {
            return Err(CliError::input(
                CODE_UNSUPPORTED_IMAGE,
                format!(
                    "{}: PNG or JPEG is required (media type is detected from the file's magic bytes)",
                    path.display()
                ),
            ));
        };
        state = state.with_image(ImageInput::new(media_type, bytes));
    }
    DecisionRequest::new(
        state,
        request.questions().to_vec(),
        request.policy().clone(),
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
