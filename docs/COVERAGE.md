# Coverage (Phases 18a + 18b)

Source of truth: `cargo llvm-cov --workspace --summary-only` (the
`just coverage-summary` recipe), 2026-09-29 on the post-18b tree.
Headline: **98.69 % lines (136 / 10 388 missed) · 96.02 % functions
(52 / 1 305) · 97.91 % regions (412 / 19 718)**.

Acceptance for 18b is by the explicit per-gap waiver clause of
PLAN.md: every line that llvm-cov can still report missing is
classified below as **unreachable-by-construction**, with the reason
stated per group and the enforcing source fact named. No gap was
silently skipped; no test asserts fake behavior.

> Column correction (2026-09-29): the 18a census transcribed the
> llvm-cov text table with the **Lines and Regions columns swapped**.
> The 18a headline ("97.47 % lines over 19 230") was actually the
> regions column; the true 18a line baseline was 98.20 % (183 / 10 166
> missed). The per-file lines column of the text table matches the
> lcov export's `DA` records one-for-one, which is what made the swap
> provable. Numbers below are from the corrected reading.

## What 18b added

Eighteen tests across five crates, written against the 18a gap table:

- `opencodifier-engine` (13): calibration artifact round-trip and
  identity fallback; relational transitive root-cause and single-entity
  health; focus sentence/budget/no-op extraction arms (file now 100 %);
  **mutex-poisoning fault injection** in `cache.rs` and `clock.rs`
  (real `panic!` under the held lock via scoped `catch_unwind`, then
  assert degraded-but-correct service: probes miss, inserts drop, the
  recovered clock keeps deadline checks alive); the IR's last-line
  validation pinned via a public-seam `Calibration` that returns 1.5
  (run fails `ir.invalid`, nothing cached).
- `opencodifier-core`: `policy()`/`metadata()` accessors.
- `opencodifier-mcp`: validate shape-error and node-limit error paths
  (including the strictly-`>` boundary at 128 nodes).
- `opencodifier-cli`: a real SIGINT delivered to the spawned `serve`
  binary asserts a clean exit (serve.rs now 100 %).
- `opencodifier-http`: `shutdown_signal()` resolved by a real SIGINT to
  the test process.

Line truth after 18b, from a fresh workspace lcov export: **78 missed
lines** (down from 121 at 18a). All 78 are in the waiver table below;
nothing reachable by a normal test remains.

## Three measurement views (reconciliation)

The same run exports three numbers that disagree by construction. Keep
them straight before quoting any percentage:

| view | unit | this tree | use for |
|---|---|---:|---|
| text table `Lines` / lcov `DA` | instrumented source lines | 10 388 / 136 miss vs 78 miss | per-line gap targeting |
| text table `Functions` | function *instantiations* (per monomorphization) | 1 305 / 52 miss | API-shape review |
| text table `Regions` / JSON `segments` | llvm-cov regions (a line can host several) | 19 718 / 412 miss | branch-level review |

A line hosting both executed and unexecuted regions counts as missed
in the text `Lines` column but has a non-zero `DA` count in lcov —
that is the whole gap between 136 and 78 (example: `cli/serve.rs` is
8/83 in the text table, 5 in lcov, 0 after 18b's SIGINT test).
"Function" misses include uninstantiated generics whose physical entry
points all executed; there is no test left to write for them.

## Waiver table (unreachable-by-construction, 78 lines)

Groups, with the source fact that makes each group unreachable. File
paths under `crates/`.

| lines | group | why no test can reach it |
|---|---:|---|
| engine/classifier.rs 72, 123–126, 267–270; engine/calibration.rs 85; engine/executor.rs 731, 811–814, 1031, 1042–1043, 1045; schema/jev.rs 310, 512, 546, 613, 703; schema/openai.rs 210, 521, 690, 713; schema/native.rs 179; model/embedding.rs 179–182 (33) | `#[non_exhaustive]` future-variant arms | the matched enums (`DecisionQuestion`, `DecisionAnswer`, `DecisionOutcome`) are `#[non_exhaustive]` in `opencodifier-core` with all current variants covered explicitly; no code outside core can construct a further variant, and adding one is a production change, not a test (executor 1042–1043 additionally map outcomes the current cascade never emits — revisit if `ConfidenceReport::outcome_for` learns to escalate) |
| engine/classifier.rs 201–203, 227–229, 248–250; model/embedding.rs 115–117, 136–138, 164–166 (18) | `Distribution::from_pairs` error arms fed by in-crate data | the constructors validate uniqueness and ≥ 2 levels in core, softmax degrades to uniform on non-finite input (`engine/lexical.rs`), and `cosine` returns 0.0 (never NaN) on zero-norm vectors — the inputs that would trip these arms are precluded by construction |
| engine/executor.rs 271, 692, 754–758, 1015; engine/graph.rs 282, 289–293, 443; engine/cache.rs 199; engine/relational.rs 115–117; engine/rules.rs 387; engine/engine.rs 413 (21) | guards implied by an earlier check | cyclic graphs are rejected by Kahn's algorithm in `DecisionGraph::new` and re-validated on deserialization; pruning always retains ≥ 1 candidate; renormalized unique probabilities cannot fail validation; `dependents` is built from the same node list as `remaining`; validated candidates cannot fail re-validation; the proven answer is filtered against the candidate list so proven probabilities sum to exactly 1; `Rule::validate` has a single error constructor; a cached response is rebuilt from an already-validated one |
| http/routes.rs 82–85; mcp/lib.rs 296–297 (6) | serde infallibility | `serde_json::to_value` over already-validated IR types (non-finite floats serialize as `null`, never an error) |

Cascading effect on the other two columns: the wildcard arms above sit
inside entered functions (they cost regions, not function entries), and
the `Functions` column additionally counts uninstantiated generics —
the remaining 52 / 412 are the same construction facts viewed at
coarser granularity, not separate gaps.

## Measurement integrity

- **Ghost files**: a renamed source leaves stale instrumentation in
  `target/llvm-cov-target` and llvm-cov merges 0 %-rows for files that
  no longer exist. Warning sign at census start: `cargo clean ...
  cannot clean target/llvm-cov-target: missing or invalid CACHEDIR.TAG
  (exit status 101)` — this clean failure is currently **recurring**
  (both 2026-09-29 post-18b censuses hit it) but harmless when no path
  was renamed: verify with (a) every reported path exists on disk,
  (b) consecutive runs report identical totals, (c) per-file numbers
  match a fresh lcov export. Purge recipe if a rename did happen:
  `find target/llvm-cov-target -mindepth 1 -depth -delete`, rerun.
- **Concurrent coverage runs pollute each other** through the shared
  `llvm-cov-target` profraw pool (bogus zero-count records; an `LF`
  count jumping 129 → 155 for an untouched file is the symptom). Any
  coverage measurement must be the only one running; use an isolated
  `CARGO_TARGET_DIR` if a second build must proceed alongside.

## 18a gap table (superseded)

The 18a classification table (per-file `testable-now` /
`needs-harness` / `mixed`) was the input to 18b and is retained in git
history; its needs-harness predictions were mostly retired by 18b —
the "ONNX-model-file" and "mock Clock" slices turned out to be
unreachable-by-construction arms (groups 1–2 above), and the cache /
clock concurrency slice was covered by direct fault injection instead
of a harness.
