//! `opencodifier decide`: one request in, one canonical response out.
//!
//! The payload is normalized by the selected wire-format adapter, decided
//! by the deterministic zero-ML engine, and printed in the canonical native
//! format. Abstention is a successful outcome semantically, so it is still
//! printed; only the exit code distinguishes it (2, or 0 with
//! `--abstain-is-success`).

use std::io::Read;
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

/// Reads at most [`MAX_ENCODED_IMAGE_BYTES`] + 1 bytes from `path`.
///
/// The read is capped rather than size-checked first: a device node or a
/// FIFO reports a length of zero but yields unbounded data, so a
/// metadata-based ceiling check would let an attacker-controlled path
/// stream gigabytes into memory before the rejection fires.
fn read_bounded(path: &Path) -> Result<Vec<u8>, CliError> {
    let file = std::fs::File::open(path).map_err(|error| {
        CliError::input(CODE_UNREADABLE_INPUT, format!("{}: {error}", path.display()))
    })?;
    let mut bytes = Vec::new();
    let limit = u64::try_from(MAX_ENCODED_IMAGE_BYTES).unwrap_or(u64::MAX);
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes).map_err(
        |error| CliError::input(CODE_UNREADABLE_INPUT, format!("{}: {error}", path.display())),
    )?;
    Ok(bytes)
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
        let bytes = read_bounded(path)?;
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use opencodifier_core::{ChoiceQuestion, DecisionQuestion, RequestMetadata, State};

    fn base_request() -> DecisionRequest {
        let policy: opencodifier_core::DecisionPolicy = serde_json::from_value(serde_json::json!({
            "min_confidence": 0.0, "verify_below": 0.0,
            "abstain_below": 0.0, "risk": "low",
        }))
        .unwrap();
        let question = DecisionQuestion::Choice(
            ChoiceQuestion::new(
                "q",
                "which is red",
                vec![
                    opencodifier_core::Candidate::new("a", "red cube").unwrap(),
                    opencodifier_core::Candidate::new("b", "blue sphere").unwrap(),
                ],
            )
            .unwrap(),
        );
        DecisionRequest::new(
            State::from_text("a red cube left of a blue sphere"),
            vec![question],
            policy,
            RequestMetadata::default(),
        )
        .unwrap()
    }

    /// Writes `bytes` into a private tempdir; the guard deletes it on
    /// drop, so no fixed names in the shared temp dir and no manual
    /// cleanup to swallow failures.
    fn temp_image(dir: &tempfile::TempDir, bytes: &[u8], name: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn with_images_accepts_png_magic() {
        let dir = tempfile::tempdir().unwrap();
        let png = temp_image(&dir, &PNG_MAGIC, "png.bin");
        let rebuilt = with_images(&base_request(), &[png]).unwrap();
        assert_eq!(rebuilt.state().images().len(), 1);
    }

    #[test]
    fn with_images_accepts_jpeg_magic() {
        let dir = tempfile::tempdir().unwrap();
        let jpeg = temp_image(&dir, &JPEG_MAGIC, "jpeg.bin");
        let rebuilt = with_images(&base_request(), &[jpeg]).unwrap();
        assert_eq!(rebuilt.state().images().len(), 1);
    }

    #[test]
    fn with_images_attaches_multiple_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let png = temp_image(&dir, &PNG_MAGIC, "a.png");
        let jpeg = temp_image(&dir, &JPEG_MAGIC, "b.jpg");
        let rebuilt = with_images(&base_request(), &[png, jpeg]).unwrap();
        assert_eq!(rebuilt.state().images().len(), 2);
    }

    #[test]
    fn with_images_refuses_unknown_magic() {
        let dir = tempfile::tempdir().unwrap();
        let junk = temp_image(&dir, b"not-an-image", "junk.bin");
        let error = with_images(&base_request(), &[junk]).unwrap_err();
        assert_eq!(error.code(), CODE_UNSUPPORTED_IMAGE);
    }

    #[test]
    fn with_images_refuses_unreadable_path() {
        let error = with_images(&base_request(), &[PathBuf::from("/nonexistent/oc/missing.png")])
            .unwrap_err();
        assert_eq!(error.code(), CODE_UNREADABLE_INPUT);
    }

    #[test]
    fn with_images_refuses_an_image_above_the_encoded_ceiling() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.png");
        // A PNG-magic header whose declared length is above the ceiling;
        // the file itself stays small — the check is on the byte count.
        let big = {
            let mut bytes = PNG_MAGIC.to_vec();
            bytes.resize(MAX_ENCODED_IMAGE_BYTES + 1, 0u8);
            bytes
        };
        std::fs::write(&path, &big).unwrap();
        let error = with_images(&base_request(), &[path]).unwrap_err();
        assert_eq!(error.code(), CODE_UNSUPPORTED_IMAGE);
        assert!(error.to_string().contains("ceiling"));
    }

    #[test]
    fn with_images_refuses_a_directory_path_as_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let error = with_images(&base_request(), &[dir.path().to_path_buf()]).unwrap_err();
        assert_eq!(error.code(), CODE_UNREADABLE_INPUT);
    }

    #[test]
    fn read_bounded_stops_at_the_ceiling_plus_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.bin");
        std::fs::write(&path, vec![0u8; MAX_ENCODED_IMAGE_BYTES + 4096]).unwrap();
        let bytes = read_bounded(&path).unwrap();
        assert_eq!(bytes.len(), MAX_ENCODED_IMAGE_BYTES + 1);
    }

    #[test]
    fn finish_maps_decisive_and_abstain_paths() {
        assert!(finish(DecisionOutcome::Accept, false).is_ok());
        assert!(finish(DecisionOutcome::Abstain, true).is_ok());
        let error = finish(DecisionOutcome::Abstain, false).unwrap_err();
        assert_eq!(error.code(), CODE_ESCALATED);
    }

    #[test]
    fn outcome_label_is_the_wire_snake_case() {
        assert_eq!(outcome_label(DecisionOutcome::Accept), "accept");
        assert_eq!(outcome_label(DecisionOutcome::Abstain), "abstain");
    }
}
