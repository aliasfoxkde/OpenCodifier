<!--
Release-notes skeleton for one OpenCodifier release.

Two kinds of slot live here, and they are not interchangeable:

- The `@@`-delimited uppercase markers are mechanical: `just release-notes`
  fills them from the attestations under `dist/` (schema
  `opencodifier.attestation/1`, produced by `scripts/generate_attestation.py`).
  Digests are never hand-copied: if a value is not in an attestation, the
  tooling owes you that field, not your clipboard.
- `<filled-by-human>` slots stay for the release engineer, because each one
  names something this repo does not measure about itself: what changed and
  why it matters, whether a gate actually ran green, and which verification
  level an artifact earned. Fill them from evidence — a DECISIONS.md entry, a
  pipeline run id, a local gate receipt — or cut the claim.

Render with `just release-notes` (writes `dist/RELEASE_NOTES.md`).
-->

# OpenCodifier @@TAG@@

Released @@DATE@@ from commit `@@COMMIT_SHORT@@` (full sha
`@@COMMIT@@`, previous tag `@@PREVIOUS_TAG@@`).

## Highlights

<!-- One bullet per user-visible change, each citing its source: the
     DECISIONS.md entry or CHANGELOG.md section it came from. A change with no
     citable source is not a highlight; it is an unreleased edit. -->
@@HIGHLIGHTS@@

## Quality-gate receipts

`just ci` green at the release commit is the gate of record; the same gates
run line-for-line in the GitForge pipeline (`.gitforge.yml`, the platform of
record — a red GitHub Actions mirror is not a code-failure signal). Every
gate below needs its own receipt: a run id, or the local command output.
`<receipt>` is where that evidence goes; an unfilled receipt means the gate
did not run for this release.

| Gate | Command | Status | Receipt |
|------|---------|--------|---------|
| Format | `cargo fmt --all -- --check` | `<PASS\|FAIL>` | `<receipt>` |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | `<PASS\|FAIL>` | `<receipt>` |
| Tests | `cargo test --workspace` | `<PASS\|FAIL>` | `<receipt>` |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` + `cargo test --doc` | `<PASS\|FAIL>` | `<receipt>` |
| Deps | `cargo deny check` | `<PASS\|FAIL>` | `<receipt>` |
| Advisories | `cargo audit` | `<PASS\|FAIL>` | `<receipt>` |
| Pattern scan | `aegis --format json scan --file . --baseline .aegis/baseline.json` (0 new) | `<PASS\|FAIL>` | `<receipt>` |
| Coverage floor | `cargo llvm-cov --workspace --lcov` + `scripts/coverage_floor.py … --floor 98.5` | `<PASS\|FAIL>` | `<receipt>` |
| End-to-end | `just e2e` (serve + MCP over the real wire) | `<PASS\|FAIL>` | `<receipt>` |

## Artifacts

Built by `just release-matrix` (D24). Each row is backed by an attestation
beside the artifact; `just release-verify` re-checks every digest against the
bytes on disk.

@@ARTIFACT_TABLE@@

### Verification level, per artifact

The matrix states what was proven, never "supported": D24 sets the named
level each target can carry, and an artifact only claims the level it earned.
Mark one per row.

<!-- Levels in use:
     - run-tested + e2e — executed on this host, `just e2e` green
     - link + file-magic — linked clean, `file(1)` magic matches the triple;
       never executed here
     - static-link check — musl artifacts additionally asserted statically
       linked
     - archive + object-magic — a static library: an `ar` archive whose
       first member is a Mach-O object of the right arch (the iOS artifact
       is never linked here — no Apple SDK; D24 records why that is the
       honest artifact)
     - bundle + load-test — the wasm dual-glue bundle loaded through both
       glue targets in Node (web initializer + nodejs entry), not a
       browser execution
-->

| Target | Level earned | Why |
|--------|--------------|-----|
| x86_64-unknown-linux-gnu | `<filled-by-human>` | `<receipt>` |
| x86_64-unknown-linux-musl | `<filled-by-human>` | `<receipt>` |
| aarch64-unknown-linux-gnu | `<filled-by-human>` | `<receipt>` |
| x86_64-pc-windows-gnu | `<filled-by-human>` | `<receipt>` |
| x86_64-apple-darwin | `<filled-by-human>` | `<receipt>` |
| aarch64-apple-darwin | `<filled-by-human>` | `<receipt>` |
| aarch64-linux-android | `<filled-by-human>` | `<receipt>` |
| aarch64-apple-ios | `<filled-by-human>` | `<receipt>` |
| wasm32-unknown-unknown | `<filled-by-human>` | `<receipt>` |

Known gaps to state on the release page rather than paper over: no foreign
arch is executed here (no qemu, no arm hardware), so aarch64 artifacts are
link + `file`-magic only; registry publication (crates.io, Homebrew, winget)
is an outward action taken only on an explicit user go.

## Reproduction

From a clean checkout at `@@COMMIT@@`, on a host with the matrix toolchains
(`just release-preflight` reports what is missing, naming each tool):

```bash
just release-preflight                 # toolchain readiness per target
just release-matrix                    # build + stage + attest every target
just release-verify                    # re-hash every artifact vs its attestation
just release-notes                     # this file, from the attestations
just ci                                # the gate of record, before tagging
just e2e                               # the run-tested claim, on the host target
```

Single-target build and its exact recorded command:

@@BUILD_COMMANDS@@

## Provenance

| Field | Value |
|-------|-------|
| Commit | `@@COMMIT@@` |
| Commit date (UTC) | `@@COMMIT_DATE@@` |
| rustc | `@@RUSTC@@` |
| cargo | `@@CARGO@@` |
| `Cargo.lock` sha256 | `@@CARGO_LOCK_SHA256@@` |
| Workspace tree sha256 | `@@CARGO_TREE_SHA256@@` |
| Feature flags | `@@FEATURES@@` |
| Attestation schema | `@@SCHEMA_VERSION@@` |

Model weights are never committed (D14): any model artifact this release
references travels with its own `models/*.json` SHA-256 manifest, and the
runtime refuses a mismatch before reading a tensor.
