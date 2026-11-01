# Quality & coverage improvement plan — 2026-11-01

Status: active plan. Opened after the 2026-11-01 audit that closed the
pipeline capture-drift incident (`f724f94` re-record + `6764c00` aegis
regen → fully green run `2d3fa273`, all 6 lanes). This document records
the honest baseline of record and the phased plan for the gaps that
remain. Supersedes nothing binding; `docs/PLAN.md` stays the live
execution plan and gains pointers to these phases.

## 0. Baseline of record (audited 2026-11-01)

| Axis | State | Source of record |
|---|---|---|
| Pipeline | 6/6 lanes green (lint, test, doc, e2e, supply-chain, coverage) | GitForge run `2d3fa273`, commit `6764c00` |
| Line coverage | **97.56 %** (22 078 / 22 630, lcov DA, in-image); floor 97.5 fixed (D36 ratchet retired; residual proven class-infeasible per docs/COVERAGE.md 2026-10-08 census) | coverage lane result_json |
| Lint | clippy `all + pedantic` deny in workspace lints; production bans on unwrap/expect/panic/todo/unimplemented/dbg/print_*; `missing_docs`, `missing_debug_implementations`, rust_2018_idioms deny; `unsafe_code` forbidden | `Cargo.toml` [workspace.lints] |
| Docs build | rustdoc `-D warnings`, zero violations | doc lane |
| E2E | 30/30 checks + recipe-capture freshness check (read-only `--check` mode in CI) | e2e lane |
| Supply chain | cargo-deny + cargo-audit + aegis 0-new-findings gate; baseline regen discipline (in-tree, same commit as source edits) | supply-chain lane; memory `aegis-gate-mechanics` |
| Site tests | 403/403 pass via `just site-test`; WCAG AAA basics verified (contrast 7.55–12.64:1 on the oc palette, `lang`, img/alt parity, ARIA on interactive pages) | local run 2026-11-01; docs/SITE_WCAG_AUDIT.md |
| Release | v0.6.2 tag cut 2026-11-01; 9-target matrix rebuilt single-tree on Fedora; install-prebuilt contract 20/20 | `.github/CHANGELOG.md` [0.6.2]; Fedora worktree `oc-v062-release` |

What is already strong (no work queued): the lint contract is at
deny-level and enforced both locally and in CI; the CI config carries
receipts for every budget decision (timeouts sized to measured storm
behavior, `-j 4` cgroup-fit, container-local target dirs with reasons);
the GitHub mirror is least-privilege (`permissions: contents: read`);
the aegis gate runs the full catalog with a fingerprint baseline.

## Phase A — Coverage: close the line gap (task list #9)

The gap to 99 % is **552 lines** (22 630 − 22 078). The 2026-10-08
census already proved a ~298-line residual class-infeasible
(feature-gated transports that execute no model in CI), which puts the
realistic ceiling near 98.7 % on the current tree — and the tree has
grown ~4 100 lines since (vision rung, config crate, recipes). The plan:

1. **Rank, don't guess.** Generate the in-image lcov (the only basis
   the floor reads) and list files by uncovered-line count; classify
   each block: (a) testable logic, (b) provably unreachable in CI
   (feature-gated I/O), (c) defensive code that cannot fire.
2. **Write tests for (a)** in tranches of ~20 tests, same discipline as
   the 2026-10-08 tranches (each tranche committed with its measured
   delta, per docs/COVERAGE.md precedent).
3. **For (b)/(c)**: extend the census table in docs/COVERAGE.md with
   the new-tree residuals and their infeasibility argument. No
   `#[cfg(test)]`-gated production code, no `coverage(off)` attributes
   to game the number — the gate measures the shipped tree.
4. **Re-anchor the floor** to the achieved value minus 0.5, only if the
   new value exceeds the current 97.5 by a tested margin.
5. **Doc coverage**: `missing_docs` is already deny and rustdoc is
   `-D warnings`, so public-item documentation is enforced. Add a
   rustdoc-coverage measurement (`cargo doc --workspace -Zunstable-options`
   or jsondoc-based count) to docs/COVERAGE.md as a tracked metric so
   "documentation coverage" is a number, not an assertion.

Acceptance: measured ≥ 99 % on the in-image DA basis **or** a census
table that accounts for every remaining uncovered line by class; floor
re-anchored; no coverage-gaming attributes introduced.

## Phase B — Lint strictness: nursery, evaluated not adopted (task #5 remainder)

Adopting `clippy::nursery` blanket is rejected for now: known
false-positive families (`significant_drop_tightening` on await points
we don't have in sync crates; `missing_const_for_fn` churn on every
compiler bump) would generate waiver noise, and this repo's rule is
"waive once, with reasons, in Cargo.toml — never scattered."

Queued instead:

1. Evaluate `clippy::restriction` set individually: 
   `undocumented_unsafe_blocks` (moot — `unsafe_code` forbidden),
   `float_arithmetic` (no — numeric core), `unwrap_in_result` (redundant
   with `unwrap_used` deny). Verdict: nothing to adopt today; record
   the evaluation so the next pass starts from evidence.
2. Keep the `pedantic` waiver list at its current four entries; any new
   waiver requires a written reason at the waiver site.
3. Re-run `cargo clippy --workspace --all-targets` at each toolchain
   bump (the pinned-image discipline already covers this — memory
   `oc-pinned-image-toolchain-drift`).

Acceptance: an evaluation note in this file's revision history; no
new blanket lints without a waiver budget.

## Phase C — Site: WCAG AAA deep pass (task #10)

The basics are verified (contrast, lang, alt parity, ARIA presence).
The deep pass targets what spot-checks cannot see:

1. **Canvas games** (12+ games): every canvas needs an accessible
   name/role and a text alternative path (the games already ship
   truthful play buttons and pause-first boot per memory
   `oc-site-games-paused-boot`; verify each game's non-visual
   equivalent, or document the exception per game in
   docs/SITE_WCAG_AUDIT.md).
2. **Keyboard walkthrough**: tab order, focus-visible on the eco flip
   cards and nav menus, accordion exclusivity (standing rule), no
   keyboard traps in the maze/pong-style games.
3. **prefers-reduced-motion**: the eco-cards honor it; sweep the game
   cores for animation loops that should idle or slow.
4. **Forms/controls** on decide.html and try.html: label association,
   error announcement (aria-live), focus management on state changes.
5. **Tooling**: Playwright + axe-core sweep as a repeatable script
   (`site/tests/a11y_scan.mjs`), wired into `just site-test` as an
   advisory lane first, gating after the found issues are fixed.

Acceptance: per-game a11y disposition recorded (fixed or documented
exception), zero axe critical/serious findings on the four key pages,
a11y scan wired into CI's site-test path.

## Phase D — CI/CD best-practice review (from other projects)

Comparators: GitForge's own pipeline layout, typical Rust org layouts
(tokio/rust-analyzer style). Verified against the current config:

Already compliant (no action): baked-in-tools image (no runtime
installs), receipts-documented budgets, least-privilege GH mirror,
fan-out graph documented with its safety condition, coverage lane in
the default graph with reasoning.

Queued improvements, sized small:

1. **GH mirror supply-chain drift**: the mirror installs cargo-deny/
   cargo-audit at runtime (`cargo install` on every run) — the exact
   anti-pattern the GitForge lane exists to avoid. Pre-build a mirror
   image or pin tool versions with a cache; alternatively drop those
   two steps from the mirror (they duplicate the lane of record) and
   note it in the mirror header.
2. **E2E parity in the mirror**: the mirror e2e lacks the
   recipe-capture check the GitForge lane runs; either add it or
   record the mirror as partial by design in its header comment.
3. **Concurrency**: the mirror should add `concurrency: group per ref,
   cancel-in-progress` so redundant pushes don't queue GH minutes.
   GitForge side needs no change (its runner serializes by design).
4. **Retention/artifacts**: no artifacts are uploaded anywhere; the
   lcov.info and doc HTML die with the container. If the coverage
   number ever needs forensics, add an artifact upload (retention ≤ 7d)
   on the coverage lane — low priority, one-line change.
5. **Pipeline-trigger hygiene**: `trigger_on: [push, pull_request]`
   means PRs double-run everything; acceptable at current volume,
   revisit if the queue ever backs up (the runner is serial).

Acceptance: items 1–3 landed (they are one-commit changes each); 4–5
decided and recorded.

## Phase E — E2E expansion

The 30-check suite covers serve + MCP contract and the recipe-capture
freshness check covers distribution drift. Gaps worth closing, in
order:

1. **Failure-path checks**: the suite exercises the happy path; add
   checks for the contract errors a consumer actually hits
   (`unsupported_generation_field`, malformed candidate lists,
   abstain-on-weak-evidence surfacing).
2. **First-run flow**: `init`/`doctor` shipped in this release but have
   no e2e presence; one check each (idempotent re-run, non-zero on a
   broken install).
3. **Feedback loop**: `/v1/feedback` acceptance and the CLI append path
   (one round-trip check).

Acceptance: +6–8 checks added with the same self-contained style as
the current 30; suite stays under its 30m budget.

## Phase F — Release engineering follow-through (task #3/#7 tail)

The v0.6.2 cut is in flight (tag pushed; matrix building on Fedora in
the single tree `oc-v062-release`). Remaining, in order:

1. Matrix completes → `just release-verify`, `release-checksums`,
   `release-notes`, `release-set-verify` in the worktree.
2. Native verification: Linux gnu + musl binaries run `--version`,
   `--help`, and the 30-check E2E on Fedora; other targets keep their
   honest link-only labels per D24.
3. Draft release on GitForge (platform of record) with
   `RELEASE_NOTES.md` + `sha256sums.txt` + per-target attestations.
4. GitHub mirror release (same assets), after the GitForge receipt is
   green at the tag commit.
5. Record the release in docs/PLAN.md (new Phase 27 entry) and close
   the prebuilt-release plan's Phase 3 checkboxes in
   docs/planning/prebuilt-binary-release-and-onboarding-2026-10-29.md.
6. Phase 4 of that plan (Amortyx-Stack installer integration) stays
   open — it is a consumer-side change with its own validation
   environment.

Acceptance: release live on both forges with the clean-source
attestation (0 dirty paths, one commit/lockfile across targets), native
E2E receipt for the Linux legs, plan checkboxes updated.

## Sequencing

- **Now**: Phase F (release tail) — everything else waits for the
  matrix.
- **Next**: Phase A tranche 1 (coverage) — highest-value quality gap.
- **Then**: Phase C/D (site a11y deep pass and the three one-commit CI
  fixes land together in a site+CI commit).
- **Last**: Phase B note and Phase E checks ride the next natural
  feature PR to avoid a lint/e2e-only commit storm.

Progress notes land in this file as phases close; docs/PLAN.md gains
the pointer entries.
