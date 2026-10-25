//! The config document schema and its resolution into engine-facing
//! sections. Every section is optional: an absent document resolves to
//! all-`None`, which is the byte-identity posture.

use crate::error::{ConfigError, ConfigResult};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The raw document as parsed from JSON. Kept distinct from
/// [`ResolvedConfig`] so validation failures carry key paths.
#[derive(Debug, Default, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawConfig {
    /// Named lane selection and lane table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<LaneSection>,
    /// Engine settings: graph path and focus budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<EngineSection>,
    /// Ordered rung tail overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rungs: Option<RungsSection>,
    /// Model backend settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelSection>,
    /// Ladder profile path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ladder: Option<LadderSection>,
    /// Escalation target, threshold, and mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation: Option<EscalationSection>,
    /// HTTP bind settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<HttpSection>,
}

impl RawConfig {
    /// Parses a document, failing on unknown keys with the offending
    /// key named.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Parse`] for malformed JSON;
    /// [`ConfigError::UnknownKey`] for a key outside the schema.
    pub fn parse(text: &str) -> ConfigResult<Self> {
        serde_path_to_result(text)
    }

    /// Validates ranges and produces the resolved configuration.
    ///
    /// # Errors
    ///
    /// [`ConfigError::OutOfRange`] when a value violates its documented
    /// range.
    pub fn resolve(self) -> ConfigResult<ResolvedConfig> {
        if let Some(port) = self.http.as_ref().and_then(|http| http.port)
            && port == 0
        {
            return Err(ConfigError::OutOfRange {
                path: String::new(),
                key: "http.port".to_owned(),
                reason: "must be 1-65535".to_owned(),
            });
        }
        if let Some(threshold) = self.escalation.as_ref().and_then(|esc| esc.threshold)
            && !(0.0..=1.0).contains(&threshold)
        {
            return Err(ConfigError::OutOfRange {
                path: String::new(),
                key: "escalation.threshold".to_owned(),
                reason: "must be within [0, 1]".to_owned(),
            });
        }
        Ok(ResolvedConfig {
            lane: self.lane,
            engine: self.engine,
            rungs: self.rungs,
            model: self.model,
            ladder: self.ladder,
            escalation: self.escalation,
            http: self.http,
            source: None,
        })
    }
}

/// serde with `deny_unknown_fields` does not report which key offended,
/// so parse to a generic value first and diff the top-level and
/// section-level keys ourselves.
fn serde_path_to_result(text: &str) -> ConfigResult<RawConfig> {
    const KNOWN: &[&str] = &["lane", "engine", "rungs", "model", "ladder", "escalation", "http"];
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| ConfigError::Parse { path: String::new(), reason: error.to_string() })?;
    let map: BTreeMap<String, serde_json::Value> = serde_json::from_value(value)
        .map_err(|error| ConfigError::Parse { path: String::new(), reason: error.to_string() })?;
    for key in map.keys() {
        if !KNOWN.contains(&key.as_str()) {
            return Err(ConfigError::UnknownKey { path: String::new(), key: key.clone() });
        }
    }
    serde_json::from_value(serde_json::to_value(&map).unwrap_or_default())
        .map_err(|error| ConfigError::Parse { path: String::new(), reason: error.to_string() })
}

/// The resolved configuration: every section optional, plus the file it
/// came from (absent when defaulted).
#[derive(Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ResolvedConfig {
    /// Lane selection.
    pub lane: Option<LaneSection>,
    /// Engine settings.
    pub engine: Option<EngineSection>,
    /// Rung tail overrides.
    pub rungs: Option<RungsSection>,
    /// Model backend settings.
    pub model: Option<ModelSection>,
    /// Ladder profile path.
    pub ladder: Option<LadderSection>,
    /// Escalation settings.
    pub escalation: Option<EscalationSection>,
    /// HTTP settings.
    pub http: Option<HttpSection>,
    /// The file the configuration was read from, when one was.
    pub source: Option<PathBuf>,
}

/// Named lane selection and lane table.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaneSection {
    /// The active lane name (e.g. `zero`, `guard`, `local-2b`).
    pub name: String,
}

/// Engine settings.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineSection {
    /// Path to the decision graph document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<PathBuf>,
    /// Focus budget in bytes for trace detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_budget: Option<u64>,
}

/// Ordered rung tail overrides.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RungsSection {
    /// Rung identifiers in execution order.
    pub order: Vec<String>,
}

/// Model backend settings.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSection {
    /// Backend identifier (`onnx`, `llamacpp`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// Model identifier within the backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Timeout in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Artifact paths (backend-specific).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<PathBuf>>,
}

/// Ladder profile settings.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LadderSection {
    /// Path to the ladder profile document.
    pub profile: PathBuf,
}

/// Escalation settings.
#[derive(Debug, Clone, PartialEq, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EscalationSection {
    /// URL the escalation payload is posted to; the engine never calls
    /// a model vendor itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Confidence floor below which decisions escalate, within [0, 1].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    /// Escalation mode (`post`, `block`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// HTTP settings.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpSection {
    /// Bind address; the default stays `127.0.0.1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind: Option<String>,
    /// Port, 1-65535.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}
