# Coverage census (Phase 18a)

Source of truth: `cargo llvm-cov --workspace --summary-only` (the
`just coverage-summary` recipe), run 2026-09-29 on the 0.3.0 tree at
commit `3f6537c`. Baseline: **97.47 % lines / 95.56 % functions /
98.20 % regions** over 19 230 lines. The `just test-cov`-style ≥ 99 %
target (18b) therefore needs ~390 of the 487 missed lines and ~44 of
the 57 missed functions covered or explicitly waived below.

> Measurement-integrity note: the first census attempt this day was
> **contaminated** — a renamed source file (`cli/src/report.rs` →
> `output.rs`) left stale instrumentation in `target/llvm-cov-target`
> after a failed `cargo clean` (missing `CACHEDIR.TAG`), and llvm-cov
> merged ghost-file rows into the report (a file at 0.00 % that no
> longer exists, and ~80 % totals for crates that were really at
> 97 %). The numbers here are from the post-purge rerun with a
> ghost-file check (`every path in the table must exist`) applied.
> Purge recipe: `find target/llvm-cov-target -mindepth 1 -depth
> -delete`, then rerun.

## Per-crate table

| crate | lines | miss | line % | fns | miss | fn % |
|---|---:|---:|---:|---:|---:|---:|
| opencodifier-runtime | 548 | 0 | 100.00 % | 43 | 0 | 100.00 % |
| opencodifier-core | 1781 | 27 | 98.48 % | 162 | 2 | 98.77 % |
| opencodifier-model | 1053 | 21 | 98.01 % | 75 | 3 | 96.00 % |
| opencodifier-schema | 6033 | 123 | 97.96 % | 278 | 16 | 94.24 % |
| opencodifier-engine | 7921 | 190 | 97.60 % | 549 | 15 | 97.27 % |
| opencodifier-http | 550 | 17 | 96.91 % | 53 | 3 | 94.34 % |
| opencodifier-mcp | 651 | 36 | 94.47 % | 54 | 7 | 87.04 % |
| opencodifier-cli | 693 | 73 | 89.47 % | 69 | 11 | 84.06 % |
| **total** | **19230** | **487** | **97.47 %** | **1283** | **57** | **95.56 %** |

## Gap classification (18b input)

Every gap with ≥ 5 missed lines, tagged `testable-now` (a normal
integration/unit test in this workspace can reach it — no new harness)
or `needs-harness` (requires fault injection, a mock `Clock`/backend,
an ONNX model file, or real transport I/O; gets a named plan or a
waiver, never a silent skip).

| file | missed lines | missed fns | class | note |
|---|---:|---:|---|---|
| schema/openai.rs | 50 | 4 | testable-now | adapter edge branches: malformed wire variants, `unsupported_generation_field` paths |
| schema/jev.rs | 38 | 5 | testable-now | Jev codec edge branches (partial readouts, invalid verdict slots) |
| mcp/lib.rs | 36 | 7 | testable-now | tool-call error paths over in-process handler calls; stdio-transport internals are the only needs-harness slice |
| engine/executor.rs | 32 | 1 | mixed | deadline/async branches `needs-harness` (mock `Clock`); policy-gate branches `testable-now` |
| cli/serve.rs | 29 | 3 | testable-now | bind failures, flag combos via `assert_cmd` (pattern already in `cli_test.rs`) |
| engine/graph.rs | 28 | 4 | testable-now | validate error arms beyond cycle (non-terminal output, missing dep, node limits) |
| engine/relational.rs | 24 | 2 | testable-now | unproven-delegate and disagreement arms |
| engine/cache.rs | 23 | 3 | mixed | key-version-mismatch arms `testable-now`; concurrent-slot behavior `needs-harness` |
| engine/classifier.rs | 23 | 3 | mixed | ONNX-absent error arms `testable-now`; session-lifecycle `needs-harness` (feature-gated) |
| model/embedding.rs | 21 | 3 | needs-harness | requires a real ONNX model file; plan: fixture MiniLM export + `onnx` feature gate |
| engine/calibration.rs | 18 | 2 | testable-now | degenerate-fit and clip arms (the D15 fitter's gate logic mirrored in-repo) |
| schema/anthropic.rs | 15 | 1 | testable-now | `cache_control` placement and malformed block arms |
| schema/native.rs | 15 | 5 | testable-now | limit-violation and candidate-set arms |
| cli/output.rs | 11 | 3 | testable-now | output-format branches |
| engine/focus.rs | 11 | 0 | testable-now | extraction-decline arms |
| http/routes.rs + lib.rs | 16 | 3 | testable-now | error arms; the black-box suite (`scripts/e2e_validate.py`) covers the happy paths |
| cli/{decide,graph,mcp,input}.rs | 28 | 5 | testable-now | subcommand dispatch + stdin/error paths |
| core/{ids,question,request}.rs | 17 | 0 | testable-now | validation branches |
| engine/rules.rs | 15 | 0 | testable-now | remaining rule-match arms |
| engine/engine.rs | 6 | 0 | testable-now | handle-identity arms |

Roll-up: ~425 of the 487 missed lines are `testable-now` or the
`testable-now` slice of a `mixed` row; ~40–45 lines are `needs-harness`
(embedding model file, cache concurrency, executor deadline) plus the
thin transport slices in mcp — those carry the 18b waivers with the
plans above. Acceptance for 18b: ≥ 99/99/99 on `cargo llvm-cov
--workspace --summary-only`, with any `needs-harness` remainder
waived line-by-line in this file.
