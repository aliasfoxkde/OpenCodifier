//! Model manifests: the identity documents that make model artifacts
//! auditable and cache-correct (DECISIONS.md D14).
//!
//! A manifest pins the exact bytes of a model artifact with a SHA-256
//! digest, the model id that must match the serving backend, and the
//! tensor contract. Every cached decision folds the model id into its
//! key, so a manifest update — a new digest — is what legitimately
//! invalidates old decisions. An artifact whose digest does not match is
//! refused, never loaded "anyway".

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::ModelError;

/// The identity document of one shipped model artifact.
///
/// Serialized as JSON next to the artifact it describes (e.g.
/// `model.onnx` + `model.manifest.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelManifest {
    /// The model id the serving backend must report. Feeds decision cache
    /// keys; changing the artifact without changing this id is the bug
    /// the digest check exists to catch.
    pub model_id: String,
    /// Lowercase hex SHA-256 of the artifact's bytes.
    pub sha256: String,
    /// Artifact format, e.g. `"onnx"`. Declared, not sniffed: a format
    /// the runtime does not support fails at load time with its own error.
    pub format: String,
    /// Semantic version of the training run that produced the artifact.
    pub revision: String,
}

impl ModelManifest {
    /// Validates manifest invariants: 64 lowercase hex characters in
    /// `sha256`, non-empty `model_id`/`format`/`revision`.
    pub fn validate(&self) -> Result<(), ModelError> {
        let reason = if self.model_id.is_empty() {
            Some("model_id is empty".to_owned())
        } else if self.format.is_empty() {
            Some("format is empty".to_owned())
        } else if self.revision.is_empty() {
            Some("revision is empty".to_owned())
        } else if self.sha256.len() != 64 || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            Some("sha256 must be exactly 64 hex characters".to_owned())
        } else {
            None
        };
        match reason {
            Some(reason) => {
                Err(ModelError::InvalidManifest { manifest: self.model_id.clone(), reason })
            }
            None => Ok(()),
        }
    }

    /// Loads, parses, and validates a manifest from a JSON file.
    pub fn load(path: &Path) -> Result<Self, ModelError> {
        let bytes = fs::read(path).map_err(|error| ModelError::UnreadableArtifact {
            artifact: path.to_path_buf(),
            reason: error.to_string(),
        })?;
        let manifest: Self =
            serde_json::from_slice(&bytes).map_err(|error| ModelError::InvalidManifest {
                manifest: path.display().to_string(),
                reason: error.to_string(),
            })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Verifies that the artifact at `artifact_path` exists and hashes to
    /// this manifest's digest.
    ///
    /// This is the gate every model load must pass before a session is
    /// created: the digest is computed from the bytes on disk, not taken
    /// on trust from anywhere.
    pub fn verify_artifact(&self, artifact_path: &Path) -> Result<(), ModelError> {
        let bytes = fs::read(artifact_path).map_err(|error| ModelError::UnreadableArtifact {
            artifact: artifact_path.to_path_buf(),
            reason: error.to_string(),
        })?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let actual = format!("{:x}", hasher.finalize());
        if actual != self.sha256.to_ascii_lowercase() {
            return Err(ModelError::Sha256Mismatch {
                artifact: artifact_path.to_path_buf(),
                expected: self.sha256.clone(),
                actual,
            });
        }
        Ok(())
    }

    /// The conventional manifest path for an artifact:
    /// `model.onnx` → `model.manifest.json`.
    pub fn manifest_path_for(artifact_path: &Path) -> PathBuf {
        let mut path = artifact_path.to_path_buf();
        path.set_extension("manifest.json");
        path
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn valid() -> ModelManifest {
        ModelManifest {
            model_id: "decision-v0".into(),
            sha256: "a".repeat(64),
            format: "onnx".into(),
            revision: "r2026.09.1".into(),
        }
    }

    #[test]
    fn validates_shape_and_reports_each_failure() {
        assert!(valid().validate().is_ok());

        let mut empty_id = valid();
        empty_id.model_id = String::new();
        assert!(empty_id.validate().unwrap_err().to_string().contains("model_id is empty"));

        let mut empty_format = valid();
        empty_format.format = String::new();
        assert!(empty_format.validate().unwrap_err().to_string().contains("format is empty"));

        let mut empty_revision = valid();
        empty_revision.revision = String::new();
        assert!(
            empty_revision.validate().unwrap_err().to_string().contains("revision is empty"),
            "revision arm"
        );

        let mut bad_hash = valid();
        bad_hash.sha256 = "ABCDEF".into();
        assert!(bad_hash.validate().unwrap_err().to_string().contains("64 hex"));
        bad_hash.sha256 = "g".repeat(64);
        assert!(bad_hash.validate().unwrap_err().to_string().contains("64 hex"));
    }

    #[test]
    fn verifies_artifact_bytes_and_rejects_mismatch() {
        let dir = std::env::temp_dir().join("oc-model-manifest-test");
        fs::create_dir_all(&dir).unwrap();
        let artifact = dir.join("artifact.bin");
        fs::write(&artifact, b"deterministic bytes").unwrap();

        let mut manifest = valid();
        manifest.sha256 = {
            let mut hasher = Sha256::new();
            hasher.update(b"deterministic bytes");
            format!("{:x}", hasher.finalize())
        };
        manifest.validate().unwrap();
        assert!(manifest.verify_artifact(&artifact).is_ok());

        manifest.sha256 = "b".repeat(64);
        let error = manifest.verify_artifact(&artifact).unwrap_err();
        assert_eq!(error.code(), "model.sha256_mismatch");
        assert!(error.to_string().contains("sha256 mismatch"), "{error}");

        let missing = manifest.verify_artifact(&dir.join("absent.bin")).unwrap_err();
        assert_eq!(missing.code(), "model.unreadable_artifact");
    }

    #[test]
    fn load_rejects_missing_and_malformed_files() {
        let missing = ModelManifest::load(Path::new("/nonexistent/manifest.json")).unwrap_err();
        assert_eq!(missing.code(), "model.unreadable_artifact");

        let dir = std::env::temp_dir().join("oc-model-manifest-test");
        fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("bad.manifest.json");
        fs::write(&bad, b"{not json}").unwrap();
        let malformed = ModelManifest::load(&bad).unwrap_err();
        assert_eq!(malformed.code(), "model.invalid_manifest");
    }

    #[test]
    fn load_accepts_valid_json_and_revalidates_its_invariants() {
        let dir = std::env::temp_dir().join("oc-model-manifest-test");
        fs::create_dir_all(&dir).unwrap();

        let good_path = dir.join("good.manifest.json");
        fs::write(&good_path, serde_json::to_vec(&valid()).unwrap()).unwrap();
        let loaded = ModelManifest::load(&good_path).unwrap();
        assert_eq!(loaded, valid());

        // JSON that parses but violates manifest invariants must fail the
        // in-`load` validation, not deserialize into a trusted manifest.
        let dishonest = r#"{"model_id":"lying","sha256":"zz","format":"","revision":""}"#;
        let bad_path = dir.join("dishonest.manifest.json");
        fs::write(&bad_path, dishonest).unwrap();
        let rejected = ModelManifest::load(&bad_path).unwrap_err();
        assert_eq!(rejected.code(), "model.invalid_manifest");
        // The first violated invariant is reported.
        assert!(rejected.to_string().contains("format is empty"), "{rejected}");
    }

    #[test]
    fn round_trips_through_json_and_names_its_sibling_manifest() {
        let manifest = valid();
        let json = serde_json::to_string(&manifest).unwrap();
        assert!(json.contains(r#""sha256":"aaaaaaaa"#), "{json}");
        let back: ModelManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, manifest);

        assert_eq!(
            ModelManifest::manifest_path_for(Path::new("models/decision.onnx")),
            PathBuf::from("models/decision.manifest.json")
        );
    }
}
