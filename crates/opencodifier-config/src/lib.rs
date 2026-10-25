//! Configuration for `OpenCodifier` (#190 — the gate program's
//! prerequisite): discovery, precedence, and strict validation over one
//! JSON document.
//!
//! There was no config file anywhere in the workspace; everything was a
//! CLI flag. This crate gives the lane/escalation/HTTP surfaces a place
//! to live without growing a flag each (`ESCALATION_LANES_PLAN` §4).
//!
//! # Shape
//!
//! ```text
//! discovery:  $OPENCODIFIER_CONFIG → project ./opencodifier.json
//!             → $XDG_CONFIG_HOME/opencodifier/config.json
//!             → ~/.config/opencodifier/config.json   (first hit wins)
//! precedence: CLI flag > env var > project file > user file > default
//! ```
//!
//! # The byte-identity rule
//!
//! With no file present and no overrides passed, [`ResolvedConfig`]
//! carries only `None`s — a caller that turns it into engine settings
//! byte-identically produces the pre-config behaviour (D25's standing
//! "empty = byte-identical" discipline, now testable). Strictness cuts
//! the other way: unknown keys and out-of-range values fail with stable
//! `config.*` error codes, never silently degrade.

pub mod error;
pub mod resolved;

pub use error::{ConfigError, ConfigResult};
pub use resolved::{
    EngineSection, EscalationSection, HttpSection, LadderSection, LaneSection, ModelSection,
    RawConfig, ResolvedConfig, RungsSection,
};

use std::path::{Path, PathBuf};

/// The project-local config file name (highest file precedence).
pub const PROJECT_FILE: &str = "opencodifier.json";

/// Environment variable that names a config file explicitly; it wins
/// over every discovered location, and a set-but-unreadable value is a
/// hard error (silently ignoring an explicit pointer hides the operator's
/// intent).
pub const ENV_VAR: &str = "OPENCODIFIER_CONFIG";

/// Returns the ordered discovery candidates: explicit env pointer
/// (marked as required), then the project file, then the user file under
/// the XDG config root. All environment inputs are injected by the
/// caller (the edge reads the process environment; this core stays
/// pure, per the repo's environment rule).
#[must_use]
pub fn candidates(
    env_value: Option<&str>,
    home: &Path,
    xdg_root: Option<&Path>,
) -> Vec<(PathBuf, bool)> {
    let mut list = Vec::new();
    if let Some(explicit) = env_value {
        list.push((PathBuf::from(explicit), true));
    }
    list.push((PathBuf::from(PROJECT_FILE), false));
    let base = xdg_root
        .filter(|p| p.is_absolute())
        .map_or_else(|| home.join(".config"), Path::to_path_buf);
    list.push((base.join("opencodifier").join("config.json"), false));
    list
}

/// Reads and validates the first existing candidate. Absent everywhere ⇒
/// [`ResolvedConfig::default`] — the byte-identity posture. Environment
/// values are injected (the edge reads the process environment; the
/// decision core stays pure, per the repo's environment rule).
///
/// # Errors
///
/// [`ConfigError::EnvPointeeMissing`] when the injected env pointer names
/// a file that does not exist; [`ConfigError::Parse`] or
/// [`ConfigError::UnknownKey`] for a malformed or over-specified
/// document.
pub fn load(
    env_value: Option<&str>,
    home: &Path,
    xdg_root: Option<&Path>,
) -> ConfigResult<ResolvedConfig> {
    for (path, required) in candidates(env_value, home, xdg_root) {
        if path.exists() {
            let text = std::fs::read_to_string(&path).map_err(|error| ConfigError::Unreadable {
                path: path.display().to_string(),
                reason: error.to_string(),
            })?;
            let raw = RawConfig::parse(&text)?;
            return raw.resolve().map(|mut resolved| {
                resolved.source = Some(path);
                resolved
            });
        }
        if required {
            return Err(ConfigError::EnvPointeeMissing { path: path.display().to_string() });
        }
    }
    Ok(ResolvedConfig::default())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn absent_everywhere_is_default() {
        let resolved = load(None, Path::new("/nonexistent-home"), None).unwrap();
        assert_eq!(resolved, ResolvedConfig::default());
        assert!(resolved.source.is_none());
    }

    #[test]
    fn explicit_env_pointee_missing_is_a_hard_error() {
        let error = load(Some("/nonexistent/config.json"), Path::new("/nonexistent-home"), None)
            .unwrap_err();
        assert!(matches!(error, ConfigError::EnvPointeeMissing { .. }));
    }

    #[test]
    fn candidates_order_is_env_project_xdg_home() {
        let list = candidates(None, Path::new("/home/u"), None);
        assert_eq!(list.len(), 2, "no env pointer ⇒ project + user file");
        assert_eq!(list[0].0, PathBuf::from(PROJECT_FILE));
        assert!(!list[0].1, "project file is optional");
        assert_eq!(list[1].0, PathBuf::from("/home/u/.config/opencodifier/config.json"));
        let with_env = candidates(Some("/cfg/o.json"), Path::new("/home/u"), None);
        assert_eq!(with_env.len(), 3);
        assert_eq!(with_env[0].0, PathBuf::from("/cfg/o.json"));
        assert!(with_env[0].1, "explicit pointer is required");
    }

    #[test]
    fn xdg_override_is_honored_when_absolute() {
        let list = candidates(None, Path::new("/home/u"), Some(Path::new("/etc/opencodifier-xdg")));
        assert_eq!(list[1].0, PathBuf::from("/etc/opencodifier-xdg/opencodifier/config.json"));
    }

    #[test]
    fn relative_xdg_falls_back_to_home() {
        let list = candidates(None, Path::new("/home/u"), Some(Path::new("rel/path")));
        assert_eq!(list[1].0, PathBuf::from("/home/u/.config/opencodifier/config.json"));
    }
}
