# Coverage (2026-10-08 census — basis re-anchored to the pinned CI image)

Headline, number of record: **98.39 % on the lcov `DA` basis
(18 187 / 18 485 hit; 298 lines never executed), measured inside the
pinned CI image `opencodifier-ci-rust:2` on the default-feature tree —
the same basis the CI lane gates** (`cargo llvm-cov --workspace
--lcov`, warm target dir). The day's earlier census read 97.96 %
(18 019 / 18 394): the §15.6 engine additions (Boolean gate fit,
elicited abstain, label-overlap preflight) diluted −0.07 points, and
the first 20g test tranche (end-to-end `ladder fit-boolean` CLI
tests) recovered 66 of the 67 `cli/ladder.rs` misses for +0.36. The
second 20g tranche, same day: four itemgen tests (the twin-guard
collision arm, the detached-side scan's iterate edge,
`fact_mentions` over every fact kind, the `count_statuses` skip),
the non-UTF-8 evidence boundary on `ladder fit-boolean`, and the one
genuinely reachable schema line (macjev's render fall-through for a
record without `request.questions`) — 11 lines net, with the
remaining residual proven class-infeasible (see the groups below and
the D36 amendment). The 2026-10-07 census read 98.03 % (17 433 /
17 783). The CI floor rests at **97.5** as the fixed constant; the
D36 ratchet is retired at the reachable ceiling.

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

## Where the 298-line residual lives (in-image `DA`, this census)

Per crate (missed / instrumented → %):

| crate | missed/total | % |
|---|---:|---:|
| runtime | 1 / 489 | 99.80 |
| core | 3 / 1 266 | 99.76 |
| mcp | 2 / 337 | 99.41 |
| schema | 46 / 3 683 | 98.75 |
| engine | 90 / 6 397 | 98.59 |
| cli | 10 / 658 | 98.48 |
| itemgen | 38 / 2 032 | 98.13 |
| http | 16 / 739 | 97.83 |
| model | 55 / 2 364 | 97.67 |
| ffi | 4 / 168 | 97.62 |
| wasm | 33 / 352 | 90.62 |

Heaviest files: `engine/executor.rs` 36/1 112 · `wasm/lib.rs` 33/352 ·
`engine/classifier.rs` 24/439 · `model/kai.rs` 20/926 ·
`schema/macjev.rs` 19/496 · `http/routes.rs` 16/515 ·
`model/embedding.rs` 14/301 · `model/llamacpp.rs` 12/375 ·
`schema/jev.rs` 11/833 · `itemgen/lib.rs` 10/364 ·
`itemgen/main.rs` 10/87 · `itemgen/sample.rs` 9/493 ·
`schema/registry.rs` 8/362 · `model/julia.rs` 7/504 ·
`cli/runtime.rs` 7/104.

## Residual classification (group level)

The pre-expansion ledger classified every one of 125 lines
individually. This census's residual is 298 lines; it is classified
at **group level**, and every group is now a proven-infeasible
class — the unit-reachable pool is exhausted (per-line restoration
is no longer Phase 20g's exit criterion; see the D36 amendment).
Groups, largest first (90 + 55 + 46 + 38 + 33 + 16 + 10 + 10 = 298):

1. **Engine residual (90)** — `executor.rs` 36 (escalation-walk
   guards implied by earlier checks: cycle/no-root/starvation arms
   precluded by construction, unchanged reasons from the 125-line
   ledger), `classifier.rs` 24 (fuzzer-targeted adversarial arms),
   `graph.rs` 9, scattered rules/relational/cache/calibration/handle
   /serde-infallibility degrade arms, `ladder.rs` 1, `ladder_fit.rs` 1.
2. **Model-backend residual (55)** — the transports themselves are
   mock-tested (`llamacpp`'s `Transport` fault seam drives connection
   and decode failures); what remains is four named classes:
   **`#[non_exhaustive]` future-variant wildcards (26)** — `kai` 9,
   `llamacpp` 8, `julia` 5, `embedding` 4: the typed refusals for
   question/answer kinds that do not exist yet and cannot be
   constructed by in-crate tests; **defensive-unreachable typed
   arms (17)** — `embedding`'s three `Distribution::from_pairs`
   map_errs over unique normalized keys (documented in-line as
   "kept typed so a future change fails loudly"), `kai`'s
   non-finite-JSON refusal (serde_json `Number` cannot carry one —
   the crate's own test proves the constructor returns `None`) and
   its infallible-serialization map_err, `llamacpp`'s
   `from_pairs` after normalize; **algorithm guards (2)** — the
   softmax sum guard and the BPE merge-loop's dead-right edge (any
   merge that kills the right side leaves an alive join at a scanned
   position or bumps the version — the guard is provably
   unreached); **test-code message regions (10)**.
3. **Schema adapter residual (46)** — **IR wildcards (13)** — `jev` 5,
   `openai` 4, `native` 1, plus `macjev`'s `Visitor::expecting` (3),
   which serde reads only while composing an error no `serde_json`
   input can trigger; **test-code message regions (30)**; the
   **registry first-run capture bootstrap (3)** that writes the
   fixture it then requires; and one line this census closed —
   `macjev`'s render fall-through for a record whose
   `request.questions` is absent, now driven by its own test.
4. **Item-generation residual (38)** — in four named classes:
   **rejection safety nets proven never to fire (7)** — `lib`
   181-184 and 224-226, the verification-rejection arms; a probe
   over 400+ draws in both family sweeps recorded zero rejections,
   so they guard the contract without a reachable trigger;
   **dead-by-construction (7)** — `sample` 566-572, the ballot-fill
   loop: chain symptoms take(2) + standalone + unrelated = 4
   distractors before the fill ever runs; **test-code
   assert/panic-message lines (6)** — `lib` 461 and 590-591,
   `sample` 758 and 887, `verify` 356: multi-line assert messages'
   `format_args`, evaluated only when a test fails;
   **binary-entry class (14)** — `main` 10, `corpus_merge` 4,
   reached only by running the real binaries, which the e2e corpus
   runs do outside instrumentation; plus `verify` 132 (the
   classifier-error `else` on a probe the lexical classifier cannot
   fail over this input class), `render` 197 and `vocab` 132/147
   (adversarial fallbacks the in-crate fuzzer targets rather than
   unit tests).
5. **Wasm/JS boundary (33)** — the standing 18j class: any
   `JsValue` construction SIGABRTs a host test, so error-boundary
   JSON is proven by the Node smoke test over the real `pkg/`
   artifact, which llvm-cov cannot see.
6. **Interface error arms (16)** — `http/routes.rs` server-fault
   encode arms (engine-produced responses cannot fail to serialize).
7. **CLI residual, no default-build test (10)** — `runtime.rs` 7 is
   the `--llama` `with_rungs` assembly, reachable only in a
   `--features llamacpp` build; the default build refuses at
   `cli.model_rung_unavailable` before assembly (the 20c
   feature-floor class). `recipes.rs` 3 are multi-line
   assert-message lines inside the `#[cfg(test)]` recipe validation.
   The rest of the cli shell is now test-covered: the `ladder
   fit-boolean` surface by nine integration tests including the
   non-UTF-8 evidence boundary (this census closed `ladder.rs`
   92-93).
8. **Boundary guards (10)** — `ffi/lib.rs` 4 null/UTF-8 guard arms
   reachable only from a C caller violating the contract, `core` 3
   (`#[non_exhaustive]` `Distribution::from_pairs` error arms fed by
   in-crate data), `mcp` 2 node-limit refuse arms, `runtime` 1.

No group is silently skipped; every group names the fact that makes
unit instrumentation the wrong tool for it. That is now true in the
strong sense: a line-by-line pass over groups 2–4 (this census)
found the reachable pool empty outside the macjev fall-through this
tranche closed — every other line belongs to a class above. The
residual is the permanent waiver ledger D36's revisit clause
anticipated; the ratchet retires at the reachable ceiling, and a
future tranche that finds a genuinely reachable line reopens it.

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
  18 019/18 394 = 97.96 %; residual regrouped to eight groups, then
  dominated by the untested `cli/ladder.rs` shell (67/67) — the
  engine-side ladder-fit core it wraps is itself tested to 1 miss.
  Basis warnings recorded: `--all-features` reads 96.93 % (onnx-gated
  transports), and the local rustc 1.99.0 mapping gap is structural.
  First 20g tranche the same day: eight integration tests drive
  `ladder fit-boolean` end-to-end (fit → profile write → `--ladder`
  reload, reserved ids, malformed-row naming, non-probability and
  empty-evidence refusals, unwritable output, base-policy carry);
  65 of 67 ladder lines recovered, cli residual 78 → 12, census
  98.32 % (18 085/18 394).
- **2026-10-08 second tranche (this census)**: four itemgen tests —
  the twin-guard collision arm (two-phase: run 1 captures a run's
  twin renders into a guard, run 2 re-runs the same seed against it
  and asserts every twin collides exactly once — the twin attempt
  seed does not depend on which base attempt accepted), the
  detached-side scan's iterate edge (seed 3 redraws where seed 23
  accepted first draw), `fact_mentions` over every fact kind, the
  `count_statuses` skip — plus `ladder fit-boolean`'s non-UTF-8
  evidence boundary as a ninth cli integration test. Empirical
  probes proved the rejection arms (lib 181-184/224-226) never fire
  and the ballot-fill loop (sample 566-572) cannot fire, so both
  joined the documented defensive classes instead of gaining
  unreachable tests. Census 98.38 % (18 177/18 476; the +82
  denominator is the new tests' own instrumented lines); residual
  309 → 299. A process note: multi-line `assert!(cond, "msg")`
  messages put lazy `format_args` on their own never-executed lines —
  new tests keep messages single-line.
- **2026-10-08 pool exhausted (this census)**: the third same-day
  census closed the one genuinely reachable line the line-by-line
  pass found — `macjev`'s render fall-through for a record without
  `request.questions`, driven by its own test (single-line let-else
  so the refusal arm adds no dead line). Census 98.39 % (18 187 /
  18 485; 298 residual; the +9 denominator is the new test's own
  lines). The line-by-line pass over groups 2–4 found everything
  else class-infeasible: the unit-reachable pool is empty, the D36
  ratchet retires at the reachable ceiling (amendment in
  DECISIONS.md), and the floor rests at 97.5 as the fixed constant.
