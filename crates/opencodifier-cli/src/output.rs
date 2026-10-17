//! Writing results: canonical JSON and verdicts to `stdout`, warnings and
//! diagnostics to `stderr`.
//!
//! The workspace denies the `print!`/`println!` macros, so every write goes
//! through `write!` on a locked stream. Failing to write `stdout` is a real
//! failure (a closed pipe means nobody read the answer) and is reported as
//! an internal error; failing to write `stderr` is never worth failing a
//! run over.

use std::io::Write as _;

use serde_json::Value;

use crate::error::{CODE_OUTPUT_FAILED, CliError};

/// Pretty-prints `value` as one JSON document followed by a newline.
///
/// # Errors
///
/// [`CliError::engine`] with [`CODE_OUTPUT_FAILED`] when the document
/// cannot be serialized or written.
pub(crate) fn print_json(value: &Value) -> Result<(), CliError> {
    let mut document = serde_json::to_string_pretty(value)
        .map_err(|error| CliError::engine(CODE_OUTPUT_FAILED, error.to_string()))?;
    document.push('\n');
    print_line(&document)
}

/// Writes one line to `stdout`.
///
/// # Errors
///
/// [`CliError::engine`] with [`CODE_OUTPUT_FAILED`] when the line cannot be
/// written or flushed.
pub(crate) fn print_line(text: &str) -> Result<(), CliError> {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{text}")
        .map_err(|error| CliError::engine(CODE_OUTPUT_FAILED, error.to_string()))?;
    stdout.flush().map_err(|error| CliError::engine(CODE_OUTPUT_FAILED, error.to_string()))
}

/// Writes one warning line to `stderr`. Best effort: a broken `stderr`
/// never fails a run.
pub(crate) fn print_warning(text: &str) {
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "opencodifier: warning: {text}");
    let _ = stderr.flush();
}
