# Coverage (2026-10-08 census — basis re-anchored to the pinned CI image)

Headline, number of record: **97.96 % on the lcov `DA` basis
(18 019 / 18 394 hit; 375 lines never executed), measured inside the
pinned CI image `opencodifier-ci-rust:2` on the default-feature tree —
the same basis the CI lane gates** (`cargo llvm-cov --workspace
--lcov`, warm target dir). The 2026-10-07 census read 98.03 %
(17 433 / 17 783); the §15.6 engine additions (Boolean gate fit,
elicited abstain, label-overlap preflight) added ~600 instrumented
lines for a −0.07-point dilution. The D36 ratchet holds the CI floor
at **97.5**, rising per tranche toward 98.5.

Basis warnings for this census:

- **`--all-features` is a different number**: 96.93 %
  (18 201 / 18 777) — the `onnx`-gated transports
  (`runtime/julia.rs` 110, `runtime/kai.rs` 92 misses) execute no
  model in CI and drag the all-features basis down. Quote the
  default-feature headline; never mix bases.
- The same tree under the local rustc 1.99.0 reads **89.68 % DA
  (17 303 / 19 295; 1 992 never-hit records)** and 78.92 % lines /
  76.59 % functions / 79.71 % regions on the text basis (measured
  2026-10-07; the toolchain artifact is structural, not per-tree).
  That spread is a **coverage-mapping artifact of the toolchain** —
  see "Two toolchains, two mappings" before quoting any percentage
  from a local run.

## Two toolchains, two mappings

The 18j note recorded local-vs-image divergence of ~0.2 points and
concluded local runs "understate slightly". That held for the
12k-line tree of 2026-10-02. It does **not** hold for the post-B5/B6
expansion: on the identical commit, rustc 1.90.0 (image) emits 17 783
`DA` records with 350 misses while rustc 1.99.0 (local stable) emits
19 295 records with 1 992 misses — 1 512 extra never-hit records,
concentrated in exactly the files the expansion added. The mechanism
is mapping granularity: the newer rustc splits one-line regions
(chained expressions, packed match arms) into separately-instrumented
records whose unexecuted halves count as full misses; the older rustc
fuses them, so a line touched by any region counts as hit. The stale
profraw-pool explanation was ruled out first (pool audit showed only
the measuring run's own records).

Ground-truthing the artifact: the escalation core that the
pre-rewrite banner called "the debt" is **unchanged** — `executor.rs`
shows 980 text-missed lines locally but only **36 `DA`-missed lines
in-image (36/973 = 96.30 %)**; `classifier.rs` 24/439 in-image vs 424
text-missed locally. Nothing regressed; the basis moved.

Standing rule (in force from this census on):

- **The pinned image is the number of record.** It is what the
  coverage lane measures, so it is what the floor ratchets against.
- **Local runs are for tranche targeting only** — finding *which*
  behavior lacks a test — never for quoting percentages or for
  comparing against any number in this document or in history.
- Only same-toolchain deltas are meaningful. A cross-toolchain
  percentage comparison is noise by construction.

## Where the 375-line residual lives (in-image `DA`, this census)

Per crate (missed / instrumented → %):

| crate | missed/total | % |
|---|---:|---:|
| runtime | 1 / 489 | 99.80 |
| core | 3 / 1 266 | 99.76 |
| mcp | 2 / 337 | 99.41 |
| engine | 90 / 6 397 | 98.59 |
| schema | 47 / 3 674 | 98.72 |
| http | 16 / 739 | 97.83 |
| model | 55 / 2 364 | 97.67 |
| itemgen | 46 / 1 950 | 97.64 |
| ffi | 4 / 168 | 97.62 |
| wasm | 33 / 352 | 90.62 |
| cli | 78 / 658 | 88.15 |

Heaviest files: `cli/ladder.rs` **67/67 — the §15.6 item-2 `ladder`
subcommand, entirely unexecuted by the workspace suite; the single
largest testable residual and the next tranche's first target** ·
`engine/executor.rs` 36/1 112 · `wasm/lib.rs` 33/352 ·
`engine/classifier.rs` 24/439 · `model/kai.rs` 20/926 ·
`schema/macjev.rs` 20/496 · `http/routes.rs` 16/515 ·
`model/embedding.rs` 14/301 · `itemgen/lib.rs` 12/339 ·
`model/llamacpp.rs` 12/375 · `schema/jev.rs` 11/833 ·
`itemgen/main.rs` 10/87.

## Residual classification (group level)

The pre-expansion ledger classified every one of 125 lines
individually. This census's residual is 375 lines; it is classified
at **group level** — per-line restoration is Phase 20g's exit
criterion, once the tranche work shrinks the residual below ~150
where line-by-line reasoning pays again. Groups, largest first
(78 + 90 + 55 + 47 + 46 + 33 + 16 + 10 = 375):

1. **CLI shell residual (78)** — dominated by `cli/ladder.rs`
   **67/67**: the §15.6 item-2 ladder-fitting subcommand shipped with
   its engine core fully tested (`ladder_fit.rs` 1 miss) but no
   integration test driving the CLI wrapper end-to-end. Unit-reachable
   with one `assert_cmd`-style test; the first tranche target. The
   remaining ~11 are arg-conflict / IO-failure arms.
2. **Engine residual (90)** — `executor.rs` 36 (escalation-walk
   guards implied by earlier checks: cycle/no-root/starvation arms
   precluded by construction, unchanged reasons from the 125-line
   ledger), `classifier.rs` 24 (fuzzer-targeted adversarial arms),
   `graph.rs` 9, scattered rules/relational/cache/serde-infallibility
   degrade arms, `ladder_fit.rs` 1.
3. **Model-backend failure arms (55)** — `kai` 20, `embedding` 14,
   `llamacpp` 12, `julia` 7: transport-error arms (connection
   refused, mid-stream EOF, malformed JSON from a backend) exercised
   against real servers in the model-eval harness, not under unit
   instrumentation.
4. **Schema adapter refusal arms (47)** — `macjev` 20, `jev` 11,
   `openai` 5: `#[non_exhaustive]` future-variant wildcards (unchanged
   class from the 125-line ledger) plus
   `unsupported_generation_field` / decode-refusal arms reached only
   by wire shapes no adapter test synthesizes.
5. **Item-generation residual (46)** — `lib` 12, `main` 10 (binary
   entry points reached only by running the real binaries, which the
   e2e corpus runs do outside instrumentation), `sample` 10,
   `verify` 7, plus adversarial-vocab failure arms the in-crate
   fuzzer targets rather than unit tests.
6. **Wasm/JS boundary (33)** — the standing 18j class: any
   `JsValue` construction SIGABRTs a host test, so error-boundary
   JSON is proven by the Node smoke test over the real `pkg/`
   artifact, which llvm-cov cannot see.
7. **Interface error arms (16)** — `http/routes.rs` server-fault
   encode arms (engine-produced responses cannot fail to serialize).
8. **Boundary guards (10)** — `ffi/lib.rs` 4 null/UTF-8 guard arms
   reachable only from a C caller violating the contract, `core` 3
   (`#[non_exhaustive]` `Distribution::from_pairs` error arms fed by
   in-crate data), `mcp` 2 node-limit refuse arms, `runtime` 1.

No group is silently skipped; every group names the fact that makes
unit instrumentation the wrong tool for it. Groups 1–5 are also the
tranche-20b–20f worklist: each has testable cores (e.g. itemgen
verify/sample arms *are* unit-reachable; only the binary mains are
not), and closing the reachable fraction is what lifts the floor
toward 98.5.

## Measurement integrity

- **Ghost files**: a renamed source leaves stale instrumentation in
  `target/llvm-cov-target` and llvm-cov merges 0 %-rows for files
  that no longer exist. Warning sign at census start: `cargo clean ...
  cannot clean target/llvm-cov-target: missing or invalid CACHEDIR.TAG
  (exit status 101)` — recurring but harmless when no path was
  renamed: verify with (a) every reported path exists on disk,
  (b) consecutive runs report identical totals, (c) per-file numbers
  match a fresh lcov export. Purge recipe if a rename did happen:
  `find target/llvm-cov-target -mindepth 1 -depth -delete`, rerun.
  The 2026-10-07 in-image runs used cold and warm target dirs and
  agreed to the line, so no ghost pollution is possible in this
  census.
- **Concurrent coverage runs pollute each other** through the shared
  `llvm-cov-target` profraw pool (bogus zero-count records; an `LF`
  count jumping for an untouched file is the symptom). Any coverage
  measurement must be the only one running; use an isolated
  `CARGO_TARGET_DIR` if a second build must proceed alongside.
- **Per-file `DA` rollups across binaries are not a basis.** A source
  file linked into N test binaries contributes its lines N times to
  summed rollups; a `lib.rs` shim re-exported everywhere lands at
  exactly 50.00 % (every line counted hit in one binary, missed in
  another) and inflates denominators. Concentration analysis must use
  a single lcov export (as this census does) or the text basis —
  never summed per-file `DA` summaries.
- **Column trap (text tables, historical)**: the wide first column is
  regions, the narrow third is lines — the 18a census transcribed
  them swapped; the lcov cross-check proved it. Any text-basis quote
  must use the corrected reading.
- **Cross-toolchain divergence**: recorded above; the reason the
  census basis moved to the pinned image. Re-verify this section's
  numbers only under the same image.

## History

- **18a → 18j, post-18j push** (2026-09-29): baseline census, 18b
  fault-injection tests (mutex poisoning, SIGINT, IR last-line
  validation), and the twelve-test push that closed the 18b gap
  table (203 → 125 `DA`-missed, every line classified). Details in
  git history; the standing waiver classes live on in groups 2, 4, 6
  above.
- **B5** (2026-10-02): llama.cpp model rung landed with 15 transport
  tests; 13 residual dark lines classified; first in-image-vs-local
  divergence note (~0.2 points, true of that tree).
- **2026-10-06 ratchet** (D36): the B5/B6/Kai/Julia expansion (8 404
  lines) read 89.28 % on the lane basis locally; floor ratcheted
  98.5 → 89.0 to keep the lane honest while Phase 20 tranches ran.
  Its banner's "debt concentrated in the escalation core" reading was
  an artifact of the local mapping — corrected by this census.
- **2026-10-07 census (this document)**: basis re-anchored to the
  pinned image. Two independent in-image runs: 17 433/17 783 =
  98.03 %, 350-line residual classified in six groups. Floor
  ratcheted 89.0 → 97.5. Found and fixed en route: the tree had
  silently stopped compiling under the pinned rustc 1.90.0
  (`rng.pick` inference break in `itemgen/sample.rs`, fixed with an
  explicit turbofish) — every in-image lane would have failed on
  grounds unrelated to coverage; the local toolchain had drifted past
  the pinned one without any local gate noticing.
- **2026-10-08 census (this document)**: post-§15.6 re-baseline.
  18 019/18 394 = 97.96 %; residual regrouped to eight groups, now
  dominated by the untested `cli/ladder.rs` shell (67/67) — the
  engine-side ladder-fit core it wraps is itself tested to 1 miss.
  Basis warnings recorded: `--all-features` reads 96.93 % (onnx-gated
  transports), and the local rustc 1.99.0 mapping gap is structural.
