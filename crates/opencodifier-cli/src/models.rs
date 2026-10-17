//! `opencodifier models verify`: the digest gate of DECISIONS.md D14.
//!
//! An artifact whose bytes do not hash to its manifest's digest never runs:
//! cache keys and calibration are pinned to those exact bytes, so the check
//! is the identity check, not a formality.

use std::path::{Path, PathBuf};

use opencodifier_model::ModelManifest;

use crate::args::ModelsSubcommand;
use crate::error::CliError;
use crate::output;

/// Suffix the model crate's convention appends to an artifact's name.
const MANIFEST_SUFFIX: &str = ".manifest.json";

/// Runs `models`.
///
/// # Errors
///
/// [`CliError::input`] when the manifest cannot be read or validated, or
/// the artifact is missing, unreadable, or does not hash to the manifest's
/// digest (`model.*` codes).
pub(crate) fn run(command: &ModelsSubcommand) -> Result<(), CliError> {
    match command {
        ModelsSubcommand::Verify { manifest, artifact } => verify(manifest, artifact.as_deref()),
    }
}

/// Verifies one artifact against its manifest and prints the verdict.
fn verify(manifest_path: &Path, artifact: Option<&Path>) -> Result<(), CliError> {
    let manifest = ModelManifest::load(manifest_path)?;
    let artifact_path =
        artifact.map_or_else(|| default_artifact_path(manifest_path), Path::to_path_buf);
    manifest.verify_artifact(&artifact_path)?;
    output::print_line(&format!(
        "ok: artifact `{}` matches manifest `{}` (model `{}`, revision `{}`, format `{}`, sha256 {})",
        artifact_path.display(),
        manifest_path.display(),
        manifest.model_id,
        manifest.revision,
        manifest.format,
        manifest.sha256
    ))
}

/// The artifact a manifest sits beside: the inverse of
/// [`ModelManifest::manifest_path_for`], so `decision.onnx.manifest.json`
/// names `decision.onnx` without a flag.
///
/// A manifest that does not follow the convention names its own stem
/// (`decision.json` → `decision`); `--artifact` always wins.
fn default_artifact_path(manifest_path: &Path) -> PathBuf {
    if let Some(name) = manifest_path.file_name().and_then(std::ffi::OsStr::to_str)
        && let Some(stem) = name.strip_suffix(MANIFEST_SUFFIX)
        && !stem.is_empty()
    {
        return manifest_path.with_file_name(stem);
    }
    manifest_path.with_extension("")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn default_artifact_names_follow_the_convention_and_fall_back_to_the_stem() {
        // The exact inverse of `ModelManifest::manifest_path_for`.
        assert_eq!(
            default_artifact_path(Path::new("models/decision.onnx.manifest.json")),
            PathBuf::from("models/decision.onnx")
        );
        // A manifest that does not follow the convention names its own
        // stem, never a silently wrong sibling.
        assert_eq!(
            default_artifact_path(Path::new("models/plain.json")),
            PathBuf::from("models/plain")
        );
        // An empty stem (a manifest literally named `.manifest.json`)
        // must not produce an empty file name.
        assert_eq!(
            default_artifact_path(Path::new("models/.manifest.json")),
            PathBuf::from("models/.manifest")
        );
    }
}
