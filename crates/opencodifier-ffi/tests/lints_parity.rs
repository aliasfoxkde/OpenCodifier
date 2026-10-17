//! The FFI crate restates the workspace lint table by hand (Cargo cannot
//! mix `lints.workspace = true` with a per-crate override), because the
//! C ABI needs exactly one exception: `unsafe_code` is allowed here, and
//! only here, where pointer handling *is* the boundary. This test is the
//! drift guard — if `[workspace.lints]` in the root manifest changes and
//! this crate's copy does not, the build fails with the key that moved.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;

/// The one documented exception, as `table -> key`: every other lint in
/// the workspace table must appear here with an identical value.
const EXCEPTION: (&str, &str) = ("rust", "unsafe_code");

/// The `key = value` lines of one TOML table, comments and blanks
/// stripped. Values compare verbatim — the crate's copy is a literal
/// restatement, not a reformat, and any reformat of the workspace table
/// shows up here to be mirrored deliberately.
fn table(manifest: &str, header: &str) -> BTreeMap<String, String> {
    let mut inside = false;
    let mut entries = BTreeMap::new();
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            inside = trimmed == header;
            continue;
        }
        if !inside || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (key, value) =
            trimmed.split_once('=').unwrap_or_else(|| panic!("lint line without '=': {trimmed:?}"));
        entries.insert(key.trim().to_owned(), value.trim().to_owned());
    }
    entries
}

#[test]
fn the_lint_copy_matches_the_workspace_table() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap();
    let workspace = std::fs::read_to_string(repo_root.join("Cargo.toml")).unwrap();
    let own =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();

    for (namespace, workspace_header) in
        [("rust", "[workspace.lints.rust]"), ("clippy", "[workspace.lints.clippy]")]
    {
        let expected = table(&workspace, workspace_header);
        let actual = table(&own, &format!("[lints.{namespace}]"));
        assert!(
            !expected.is_empty(),
            "{workspace_header} parsed to nothing — the parser or the layout moved"
        );
        for (key, value) in &expected {
            if namespace == EXCEPTION.0 && key == EXCEPTION.1 {
                continue;
            }
            assert_eq!(
                actual.get(key),
                Some(value),
                "{namespace}.{key} drifted from the workspace table \
                 (workspace: {value:?}); restate it in the FFI crate's [lints.{namespace}]"
            );
        }
        for key in actual.keys() {
            assert!(
                expected.contains_key(key),
                "{namespace}.{key} exists only in the FFI crate's table; \
                 the workspace table is the gate of record"
            );
        }
    }
}

#[test]
fn the_exception_is_still_the_only_one() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap();
    let workspace = std::fs::read_to_string(repo_root.join("Cargo.toml")).unwrap();
    let own =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();

    let workspace_rust = table(&workspace, "[workspace.lints.rust]");
    let own_rust = table(&own, "[lints.rust]");
    let forbidden = own_rust
        .iter()
        .filter(|(key, value)| {
            workspace_rust.get(*key).is_some_and(|workspace_value| workspace_value != *value)
        })
        .map(|(key, value)| format!("{key} = {value}"))
        .collect::<Vec<_>>();
    assert_eq!(
        forbidden,
        // Values are verbatim TOML fragments (quotes included), so the
        // expected override renders as `unsafe_code = "allow"` — no
        // `{:?}`, which would double-quote an already-quoted value.
        vec![format!("{} = {}", EXCEPTION.1, own_rust[EXCEPTION.1])],
        "the FFI crate overrides a workspace lint that is not the documented \
         `unsafe_code` exception"
    );
}
