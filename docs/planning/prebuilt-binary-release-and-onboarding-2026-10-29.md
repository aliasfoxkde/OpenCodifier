# Prebuilt binary releases and clean OpenCodifier onboarding

> **Status:** active plan and implementation record — 2026-10-29. The v0.6.2 candidate is built and locally validated for Linux x86_64 GNU and musl; the release is not published yet.

## Objective

Make OpenCodifier's deterministic engine easy to install and update from a prebuilt CLI binary. A normal setup must not clone the Rust workspace, install Rust/Cargo, or compile OpenCodifier. The no-AI engine remains the default; model weights and local model backends remain separate and opt-in.

The upstream OpenCodifier project owns and improves the engine. Amortyx consumes the stable CLI/service boundary and remains responsible for policy, provider eligibility, routing, failover, and orchestration. Users can later compose deterministic classification, a local OpenCodifier model rung, an external JEV-compatible endpoint, and Amortyx enforcement in user/team/enterprise workflows without turning Amortyx into a monolith.

## Audit findings

Observed 2026-10-29 from source, release metadata, and a real Fedora-host run:

- OpenCodifier `v0.6.1` is already published with CLI binaries for Linux x86_64 (GNU and musl), Linux aarch64, macOS x86_64/arm64, Windows x86_64 GNU, Android, iOS static library, and WASM. Each asset has a JSON SHA-256 attestation. The release is not missing binaries.
- `v0.6.1`'s x86_64 GNU asset hash matched its attestation, reported version `opencodifier 0.6.1`, and passed the current `scripts/e2e_validate.py` suite on Fedora 44: **30/30 checks**.
- That same attestation says `working_tree_clean: false`, `dirty_path_count: 29`. Its recorded source commit differs from the GitHub tag commit, although both resolve to the same Git tree hash. Therefore the asset is a working compatibility baseline, but its build record does not meet the clean-source provenance standard for the next release.
- The GitForge `.gitforge.yml` pipeline is the quality-gate authority. GitHub Actions is explicitly a mirror and must not become a competing CI/release gate.
- The Amortyx-Stack bootstrap currently clones OpenCodifier and invokes Cargo. That makes the target host install Rust and compile the full workspace even though prebuilt CLI assets already exist.
- The existing per-binary attestation verifies artifact bytes and records build metadata, but it is not itself a cryptographic signature. GitHub documents release immutability and signed build attestations as stronger provenance/integrity mechanisms; configure/enable these where the hosting policy supports them.

## Validation evidence (2026-10-29)

- PR `#11` is based on `origin/main`. The Linux artifact test below ran on clean code commit `9fc6e64`; later commits only changed planning/changelog docs. GitHub mirror checks for lint, tests, docs, E2E, supply chain, and code analysis passed on `9fc6e64`; the current tip still needs its mirror checks and final release artifacts rebuilt from the release tag commit.
- Fedora 44 built both `x86_64-unknown-linux-gnu` and the installer-selected `x86_64-unknown-linux-musl` assets from the clean PR commit. Their generated per-asset attestations verify. Both binaries passed **30/30** E2E checks.
- A fresh `python:3.12-slim` consumer container received curl and CA certificates, with no Rust compiler or Cargo present. The actual installer downloaded the staged musl asset from a local release-shaped directory, verified its attestation, installed `opencodifier 0.6.2`, and passed **30/30** E2E checks. No daemon or service was started.
- The installer, attestation, and release-set contract suite passes **20/20** tests. It covers the complete release set plus missing tag/target, dirty source, commit mismatch, toolchain drift, checksum omissions, release-note placeholders, installer integrity failures, and previous-install preservation. Shell syntax, Python byte-compilation, locked Cargo metadata, and `git diff --check` pass.
- Fedora `just ci` passed format, clippy, workspace tests, docs, cargo-deny, and cargo-audit, then stopped at Aegis. The host Aegis is 0.6.1 while the cached CI image is 0.6.3. That image also reports zero baseline suppressions and 20,832 unmatched findings on `origin/main`; this is pre-existing scanner/baseline drift, not a clean PR receipt. The baseline was left unchanged rather than regenerated to silence the mismatch.
- GitForge reports degraded/build-drift service state. Its OpenCodifier pipeline has no observed run for the PR commit; the listed run is pinned to another commit. The required GitForge receipt is still outstanding.
- Fedora's release preflight has all listed targets ready except Android NDK and `wasm-pack`. The full release matrix therefore remains incomplete.

## Target behavior

1. A user runs one documented installer command on a supported desktop OS.
2. The installer detects OS/architecture, downloads one pinned release binary plus its attestation, verifies schema/target/size/SHA-256 and clean source provenance, checks the binary's `--version`, then atomically installs to a user-writable directory.
3. It never installs Rust, downloads model weights, starts a daemon, changes a system service, or enables a provider.
4. OpenCodifier's local deterministic HTTP/MCP services remain explicit operator choices. Amortyx falls back safely when OpenCodifier is disabled or unavailable.
5. A failed download, unsupported platform, corrupt/incorrect attestation, incompatible binary, or failed self-check leaves the current installation untouched and gives a precise error.

## Platform and artifact contract

The existing release matrix includes more targets than the first-party installer needs. First-party CLI installation initially covers:

| Host | Target asset | Notes |
|---|---|---|
| Linux x86_64 | `x86_64-unknown-linux-musl` | Prefer static binary for portable Linux use; validate on Fedora and a second Linux environment before claiming broad compatibility. |
| Linux aarch64 | `aarch64-unknown-linux-gnu` | Validate on native ARM64 hardware/runner before claiming runtime-tested support. |
| macOS x86_64 | `x86_64-apple-darwin` | Validate on a native Intel macOS runner. |
| macOS arm64 | `aarch64-apple-darwin` | Validate on native Apple Silicon. |
| Windows x86_64 | `x86_64-pc-windows-gnu.exe` | Provide a PowerShell installer or document manual download; do not route a PE executable through the POSIX shell installer. |

Android, iOS static library, and WASM remain downloadable library/runtime artifacts, not host CLI installs. The asset contract is `<binary>-<version>-<target>[.exe]` plus an adjacent `.attestation.json` using schema `opencodifier.attestation/1`.

## Release integrity and quality gates

A release candidate must satisfy all of these before the public GitHub mirror is published:

- GitForge `just ci` receipt matches the exact release commit. It remains the CI gate of record.
- Release source is a clean, immutable commit and tag; every target attestation names that same commit, Cargo.lock digest, dependency-tree digest, and version.
- `just release-preflight`, `just release-matrix`, `just release-verify`, `just release-checksums`, and `just release-notes` complete. The release set includes every target currently listed in `release_targets`; no partial matrix can be mistaken for a full release.
- Native-host CLI runs `--version`, `--help`, the E2E suite, and the release binary's real serve/MCP contract. Cross-compiled binaries earn only their measured verification level; link/file-magic is not called runtime testing.
- A clean disposable Fedora environment installs the published candidate without Cargo/Rust, verifies the per-binary attestation, starts no service by default, and passes E2E.
- Release notes contain the gate receipt, source commit, explicit verification level per artifact, checksums, and unsupported/unrun target caveats.
- Publish as a draft first; install and validate the exact downloaded bytes; then publish the release. Prefer immutable releases and GitHub release attestations when enabled. GitHub's general artifact/release attestations are distinct from this repository's self-described JSON SHA manifest.

## Phased implementation and task ledger

### Phase 0 — Audit and baseline (complete)

- [x] Inspect release policy, current GitForge gate, release matrix, and published assets.
- [x] Verify the existing Linux x86_64 release binary on Fedora, including E2E.
- [x] Record the dirty provenance and installer/build-time Rust dependency gaps.

### Phase 1 — Installer and consumer contract (in progress)

- [x] Add a user-local prebuilt installer with platform detection, version pinning, bounded downloads, strict attestation checks, atomic install, and non-destructive failure behavior.
- [x] Add contract tests for supported/unsupported platforms, wrong target, malformed schema, SHA/size mismatch, dirty provenance, version mismatch, and preserving the previous install.
- [ ] Document offline/manual asset installation and exact requirements (`curl`, Python 3 standard library, a supported shell); Rust is not an install-time dependency.
- [x] Re-key the version-dependent WASM conformance cache hashes for 0.6.2; the three regenerated diffs change only cache hashes, not answers/confidence/outcomes.

### Phase 2 — Release provenance and runbook

- [x] Strengthen release-set verification to require clean source, tag/version agreement, one commit/lockfile/dependency tree across all targets, and a complete matrix.
- [x] Verify the checksum manifest itself against every staged artifact and reject omitted, duplicate, or unexpected entries.
- [ ] Add a reproducible release runbook using GitForge as gate authority, Fedora for heavy release builds, checksum/attestation verification, draft upload, clean consumer install, then publication.
- [x] Add an update/rollback contract: explicit version pin, verify-before-replace, retain prior binary or provide a one-command reinstall of the previous pinned version.
- [x] Document signed provenance/immutable release options and the limitation of unsigned JSON SHA records.

### Phase 3 — Clean release candidate

- [ ] Merge the reviewed implementation through the normal PR path and obtain a GitForge gate receipt for the exact commit.
- [ ] Build a new patch release from a clean tagged commit; build every existing release target and record native versus link-only evidence honestly.
- [x] Validate the staged Linux x86_64 musl candidate in a fresh consumer container without Rust/Cargo; installation and E2E passed.
- [ ] Re-download the exact draft-release asset and repeat attestation/install/E2E before publication.
- [ ] Publish a draft release, verify the exact downloaded bytes and metadata, and publish only after the consumer test passes.

### Phase 4 — Amortyx-Stack integration

- [ ] Replace source clone/Cargo build in the recommended clean-host setup with the verified prebuilt installer and a pinned OpenCodifier version.
- [ ] Preserve the Amortyx-only path and an explicit offline/manual binary path.
- [ ] Validate the complete setup in a clean Fedora container/VM with no Rust toolchain, no provider secrets, and no service/cloud startup.

## Rollback and operations

- Keep prior release assets immutable and available.
- The installer validates a candidate in a temporary location before replacing `opencodifier`; failed validation never overwrites a working installation.
- Record installed version, target, source commit, and attestation digest in a local non-secret install receipt.
- Roll back by reinstalling the prior pinned version. Do not delete user config, local model artifacts, or state during binary updates.
- Release publication is separate from merge: a green code gate does not prove all cross-platform binaries were runtime tested.

## Research references

- OpenCodifier's release assets and per-target attestations: [GitHub releases](https://github.com/aliasfoxkde/OpenCodifier/releases).
- Repository-defined release matrix and attestation contract: `justfile`, `scripts/generate_attestation.py`, `scripts/generate_release_notes.py`, `.gitforge.yml`, and `docs/RELEASE_NOTES_TEMPLATE.md`.
- GitHub recommends immutable releases to prevent released tags/assets from changing and describes release attestations as a verifiable association between tag, source commit, and assets: [Immutable releases](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases).
- GitHub Actions build attestations record repository, workflow, commit, event, and artifact provenance; these provide stronger provenance than a self-generated checksum record when generated and verified through the configured platform: [Artifact attestations](https://docs.github.com/en/actions/concepts/security/artifact-attestations).
- Current GitHub-hosted runners include native Linux ARM64 and distinct macOS Intel/Apple Silicon labels; actual runner availability and policy must be checked when adding CI target lanes: [Choosing a runner](https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job).

## Current handoff

The worktree/branch for implementation is `codex/prebuilt-release-installer-20261029` in `/var/tmp/opencodifier-prebuilt-release-20261029`, based on current GitHub `origin/main` at `dacddc6`. A local `main` checkout is seven commits ahead of that mirror tip and has an unrelated untracked benchmark file; neither is included in this worktree.
