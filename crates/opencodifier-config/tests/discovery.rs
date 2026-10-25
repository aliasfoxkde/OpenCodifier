//! Integration tests for discovery and precedence. Each test takes the
//! env lock and injects values; no test mutates the shared environment.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opencodifier_config::{ENV_VAR, PROJECT_FILE, RawConfig, ResolvedConfig, load};
use std::path::Path;

/// Serializes tests that touch the process environment.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn parses_minimal_document() {
    let raw = RawConfig::parse(r#"{"http": {"port": 8080}}"#).unwrap();
    let resolved = raw.resolve().unwrap();
    assert_eq!(resolved.http.as_ref().unwrap().port, Some(8080));
}

#[test]
fn unknown_top_level_key_is_rejected_with_name() {
    let error = RawConfig::parse(r#"{"nope": {}}"#).unwrap_err();
    assert_eq!(error.code(), "config.unknown_key");
    assert!(error.to_string().contains("nope"));
}

#[test]
fn out_of_range_threshold_is_rejected() {
    let raw = RawConfig::parse(r#"{"escalation": {"threshold": 1.5}}"#).unwrap();
    let error = raw.resolve().unwrap_err();
    assert_eq!(error.code(), "config.out_of_range");
}

#[test]
fn empty_document_is_default() {
    let raw = RawConfig::parse("{}").unwrap();
    assert_eq!(raw.resolve().unwrap(), ResolvedConfig::default());
}

#[test]
fn project_file_is_discovered_from_cwd() {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(PROJECT_FILE), r#"{"lane": {"name": "guard"}}"#).unwrap();
    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    let resolved = load(None, Path::new("/nonexistent-home"), None);
    std::env::set_current_dir(prev).unwrap();
    let resolved = resolved.unwrap();
    assert_eq!(resolved.lane.as_ref().unwrap().name, "guard");
    assert!(resolved.source.as_ref().unwrap().ends_with(PROJECT_FILE));
    let _ = ENV_VAR;
}
