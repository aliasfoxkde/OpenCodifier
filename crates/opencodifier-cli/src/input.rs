//! Reading the bytes and JSON documents the CLI is handed.
//!
//! Everything read here is hostile input: it is parsed, validated by the
//! owning crate, and never allowed to influence anything but the request it
//! describes.

use std::fs;
use std::io::Read as _;
use std::path::Path;

use opencodifier_core::DecisionPolicy;
use serde_json::Value;

use crate::error::{CODE_INVALID_JSON, CODE_INVALID_POLICY, CODE_UNREADABLE_INPUT, CliError};

/// Reads the payload named by `--input`: a file path, or `-` for stdin.
///
/// # Errors
///
/// [`CliError::input`] with [`CODE_UNREADABLE_INPUT`] when the source
/// cannot be read.
pub(crate) fn read(source: &str) -> Result<Vec<u8>, CliError> {
    if source == "-" {
        let mut buffer = Vec::new();
        std::io::stdin()
            .lock()
            .read_to_end(&mut buffer)
            .map_err(|error| CliError::input(CODE_UNREADABLE_INPUT, format!("stdin: {error}")))?;
        Ok(buffer)
    } else {
        read_path(Path::new(source))
    }
}

/// Reads a whole file, reporting a missing or unreadable file as an input
/// error.
///
/// # Errors
///
/// [`CliError::input`] with [`CODE_UNREADABLE_INPUT`].
pub(crate) fn read_path(path: &Path) -> Result<Vec<u8>, CliError> {
    fs::read(path).map_err(|error| {
        CliError::input(CODE_UNREADABLE_INPUT, format!("{}: {error}", path.display()))
    })
}

/// Parses JSON bytes.
///
/// # Errors
///
/// [`CliError::input`] with [`CODE_INVALID_JSON`], the schema layer's own
/// code, so a payload the adapters never saw is reported like one they
/// refused.
pub(crate) fn parse_json(payload: &[u8]) -> Result<Value, CliError> {
    serde_json::from_slice(payload)
        .map_err(|error| CliError::input(CODE_INVALID_JSON, error.to_string()))
}

/// Reads a JSON document from `path`.
///
/// # Errors
///
/// [`CliError::input`] for an unreadable file or malformed JSON.
pub(crate) fn read_json(path: &Path) -> Result<Value, CliError> {
    parse_json(&read_path(path)?)
}

/// Reads and validates a [`DecisionPolicy`] document.
///
/// # Errors
///
/// [`CliError::input`] with [`CODE_INVALID_POLICY`] when the file is not a
/// policy the IR accepts: the gates must be ordered and in `[0, 1]`.
pub(crate) fn read_policy(path: &Path) -> Result<DecisionPolicy, CliError> {
    serde_json::from_value(read_json(path)?).map_err(|error| {
        CliError::input(CODE_INVALID_POLICY, format!("{}: {error}", path.display()))
    })
}
