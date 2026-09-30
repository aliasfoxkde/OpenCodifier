# Coverage (Phases 18a → 18j push)

Source of truth: `cargo llvm-cov --workspace --summary-only` (the
`just coverage-summary` recipe), cross-checked one-for-one against a
fresh `--lcov` export, 2026-09-29 on the post-18j tree.

Headline: **98.34 % lines text basis (201 / 12 134) — 98.97 % on the
lcov `DA` basis (125 / 12 132) · 95.61 % functions (66 / 1 504) ·
97.68 % regions (531 / 22 902)**.

Acceptance is by the explicit per-gap waiver clause of PLAN.md: every
line llvm-cov can still report missing is classified below with the
reason and the enforcing source fact named. No gap was silently
skipped; no test asserts fake behavior.

> Column correction (2026-09-29, still in force): the text table's
> wide first column is **regions**, the narrow third is **lines** —
> the 18a census transcribed them swapped. The proof is the lcov
> cross-check: the lines column (12 134 missed-201) matches the
> export's `LF` total (12 132 instrumented lines) to within two
> partial-line records, while the regions column (22 902) has no lcov
> counterpart at all. Every number below uses the corrected reading.

## What the post-18j push added

Twelve tests and one test rewrite across six crates, written against
the 18b gap table (203 `DA`-missed lines → 125):

- `opencodifier-wasm`: the host tests now drive the **public**
  `decide` / `validate_graph` / `run_graph` / `identity` /
  `opencodifier_version` methods (the wrappers 18j left to the
  `*_impl` fns), plus a semver shape test. Constraint discovered by
  doing it: a host test cannot touch any `JsValue` — construction
  calls a wasm import (`__wbindgen_string_new`) that has no host
  implementation and SIGABRTs — so error paths stay on `*_impl` here,
  and the boundary's error JSON `{"code","message"}` remains proven by
  the Node smoke test over the real `pkg/` artifact.
- `opencodifier-schema/registry`: the six decode-refusal arms (empty
  id, version 0, choice with neither candidate source, empty `static`
  list, score with candidates, score with one level) and the first
  score-definition decode → instantiate → levels round trip.
- `opencodifier-engine/graph`: the kind-name table completed (all 14
  kinds) and the D21 knob-validation refusals (knob on the wrong kind,
  `retrieve` without/with-zero `top_n`, floor outside [0, 1] or
  non-finite, unknown reranker signal).
- `opencodifier-engine/tests/semantic_nodes`: a **miscounting
  embedding backend** (one-vector-per-text violation →
  `engine.backend_failed` naming both counts) through both the
  semantic node and `EmbeddingReranker`, a failing backend through the
  reranker seam, and a `filter` → `embedding` → `retrieve` composition
  proving semantic scoring sees rule-filtered survivors.
- `opencodifier-http`: malformed batch and graph-run bodies refused at
  the boundary with `schema.invalid_value` (handler-level, real
  async).
- `opencodifier-cli/recipes`: the two install-failure arms — a file
  where the recipe directory belongs (destination untouched), and an
  unwritable file slot (a directory planted at `graph.json`; chosen
  over `chmod` because root in CI containers bypasses permission bits).

Aegis note: the fmt pass shifted columns of pre-existing findings and
this ledger's rewrite re-fingerprinted its own prose rows, so the
baseline was regenerated wholesale after the gates
(`.aegis/baseline.json`, 3 027 findings, gate = 0 new; the delta was
exclusively documentation prose — marker/line-length/magic-number
classes already saturating the baseline).

## Three measurement views (reconciliation)

The same run exports numbers that disagree by construction. Keep them
straight before quoting any percentage:

| view | unit | this tree | use for |
|---|---|---:|---|
| lcov `DA` (count == 0) | instrumented lines never executed | 12 132 / 125 miss | **the per-line truth**; waiver targeting |
| text table lines / regions columns | lines with any missed region; llvm-cov regions | 12 134 / 201 miss; 22 902 / 531 miss | headline percentages |
| text table functions | function instantiations (per monomorphization) | 1 504 / 66 miss | API-shape review |

The 201-vs-125 gap is exactly the ledger's standing subtlety: a line
hosting both executed and unexecuted regions counts as missed in the
text column but has a non-zero `DA` count (76 such lines this census).
"Function" misses add uninstantiated generics and the binary
entrypoints (`serve run`, `mcp serve`, `decide run`, `main`) whose
coverage is proven by the e2e suite over the real binary — llvm-cov
cannot see those executions; there is no unit test left to write for
them.

## Waiver table (all 125 remaining `DA`-missed lines)

Groups, with the source fact that makes each group unreachable from a
normal test. File paths under `crates/`.

| lines | group | why no test can reach it |
|---|---:|---|
| engine/classifier.rs 72, 123–126, 201–203, 227–229, 248–250, 267–270; engine/calibration.rs 85; engine/executor.rs 1052, 1132–1135, 1387–1388, 1390; schema/jev.rs 310, 512, 546, 613, 703; schema/openai.rs 210, 521, 690, 713; schema/native.rs 179; model/embedding.rs 115–117, 136–138, 164–166, 179–182 (43) | `#[non_exhaustive]` future-variant arms, and `Distribution::from_pairs` error arms fed by in-crate data | the matched enums are `#[non_exhaustive]` in `opencodifier-core` with all current variants covered explicitly — no code outside core can construct a further variant; the `from_pairs` arms are precluded by construction (constructors validate uniqueness and ≥ 2 levels, softmax degrades to uniform on non-finite input, `cosine` never returns NaN) |
| engine/executor.rs 328, 604–607, 751–754, 760–770; engine/graph.rs 404, 411–415, 565 (18) | guards implied by an earlier check | `run()`'s cycle arm cannot fire (`DecisionGraph::new` runs Kahn's algorithm, re-validated on deserialization); the no-root arm sits after that same acyclicity check; one root + acyclic mathematically implies reachability (checked defensively at 409–415, per its own comment); the Kahn `remaining` guard assumes duplicate edges, which construction rejects; assembly refuses backend-less semantic and embedding-rerank graphs and validation requires a known `reranker` before any node runs |
| engine/executor.rs 779, 1008 (2) | starvation guards the pipeline cannot trigger | `retrieve` keeps top-1 and the lexical prune knob retains ≥ 1, so `candidates_in_play` cannot be empty when a rerank or decision stage runs |
| engine/executor.rs 1075–1079; engine/handle.rs 108–110; mcp/lib.rs 294–296; engine/engine.rs 469; engine/cache.rs 210 (11) | serde infallibility / documented degrade arms | `serde_json` over already-validated IR types cannot fail (non-finite floats serialize as `null`); the cached-response rebuild and cache re-sort degrade arms are documented at the site as unreachable for validated inputs |
| http/routes.rs 87–90, 142–145, 289–292 (12) | server-fault encode arms | encoding an **engine-produced** response cannot fail; the arm exists so a future bug surfaces as `engine.serialization`, not a bad-request |
| wasm/lib.rs 206–210, 212, 216–220 (11) | platform-excluded: the error boundary | `wasm_error` builds a `JsValue` via an import that exists only under a JS runtime — host tests SIGABRT the moment one is constructed. The arm is exercised for real by the Node smoke test (`tests/node/smoke.cjs`) over the wasm-pack artifact, asserting `JSON.parse(thrown).code` for `schema.invalid_json`, `graph.cycle`, and `engine.missing_backend`; llvm-cov cannot see JS execution |
| schema/registry.rs 606–608 (3) | first-capture write path | the recipe request it writes is committed and pinned byte-for-byte by the drift test itself; deleting the capture to re-run the branch would be self-inflicted |
| schema/registry.rs 426, 481, 576, 688; engine/optimize.rs 360 (5) | test-support panics | `panic!` in a test's `let-else`/failure message formatting — firing one means the harness itself broke, which is the desired state |
| engine/rules.rs 387 (1) | error-mapping fall-through | rule application surfaces exactly one error constructor (`InvalidRule`); the `other => other` arm keeps the mapping total if that ever changes |

Cascading effect on the other columns: the wildcard arms above sit
inside entered functions, and the functions column adds
monomorphization duplicates and the e2e-covered binary entrypoints —
the remaining 66 / 531 are the same construction facts viewed at
coarser granularity, not separate gaps.

## History

- **18a** (2026-09-29): baseline census + gap classification; its
  per-file table is retained in git history. Its headline carried the
  regions/lines column swap corrected above.
- **18b** (2026-09-29): 18 tests across five crates — calibration
  artifact round-trip, relational transitive proofs, focus arms,
  **mutex-poisoning fault injection** in `cache.rs`/`clock.rs` (real
  panic under the held lock, then assert degraded-but-correct
  service), IR last-line validation via a 1.5-returning `Calibration`
  seam, MCP shape/node-limit refusals, and real-SIGINT tests for the
  CLI `serve` and the HTTP shutdown signal. 121 → 78 `DA`-missed.
- **Post-18j push** (2026-09-29): the twelve tests above; 203 → 125
  `DA`-missed, every survivor classified in the table.

## Measurement integrity

- **Ghost files**: a renamed source leaves stale instrumentation in
  `target/llvm-cov-target` and llvm-cov merges 0 %-rows for files that
  no longer exist. Warning sign at census start: `cargo clean ...
  cannot clean target/llvm-cov-target: missing or invalid CACHEDIR.TAG
  (exit status 101)` — recurring but harmless when no path was
  renamed: verify with (a) every reported path exists on disk,
  (b) consecutive runs report identical totals, (c) per-file numbers
  match a fresh lcov export. Purge recipe if a rename did happen:
  `find target/llvm-cov-target -mindepth 1 -depth -delete`, rerun.
  Both post-18j censuses ran on a purged pool (the 18j renames made
  the purge mandatory) and reconcile with the lcov export.
- **Concurrent coverage runs pollute each other** through the shared
  `llvm-cov-target` profraw pool (bogus zero-count records; an `LF`
  count jumping for an untouched file is the symptom). Any coverage
  measurement must be the only one running; use an isolated
  `CARGO_TARGET_DIR` if a second build must proceed alongside.
