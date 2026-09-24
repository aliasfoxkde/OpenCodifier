# Decision Records

Binding architecture and dependency decisions, with the reasoning that
produced them. Each entry supersedes anything contradictory in
PLANNING.md. Ordered by decision number; new decisions append.

## D1 — MCP SDK: `rmcp = "=2.2"`, stable MCP `2025-11-25`

PLANNING.md §47 named rmcp loosely. `rmcp` 3.0.0-beta is in flight but
beta; the stable line is 2.2, which speaks the MCP `2025-11-25` stable
revision. Exact-pin (`=`) because rmcp 2.x → 3.x is a protocol-surface
change, not a semver-compatible bump. Revisit when 3.0 reaches stable.

## D2 — ONNX runtime: `ort = "=2.0.0-rc.13"`

`ort` rc.12 → rc.13 changed the API and rc.14 may again. fastembed pins
rc.12, which is why fastembed is **not** used (D4). Pin exactly, keep
behind the `onnx` feature, and bump only as a deliberate PR that re-runs
the model round-trip tests. `default-features = false`,
`features = ["std", "ndarray", "api-27"]`, plus the `load-dynamic`
feature packaged as the `runtime-onnx-dynamic` feature so distros can
point at a system ONNX runtime.

## D3 — MSRV: 1.90, stable channel

Edition 2024. Every dependency must support 1.90 (checked: criterion
0.8 needs 1.86, burn 0.20 needs 1.89 — both fine). `rust-toolchain.toml`
pins `stable` locally; CI images use `rust:1.90`. MSRV bumps are
release-note items.

## D4 — No fastembed, no tantivy: hand-written BM25 + adapter-supplied
embeddings

- fastembed → hard-pinned to ort rc.12, incompatible with D2, and drags
  hf-hub/model download into a privacy-first local crate. Rejected.
- tantivy → full search engine (index files, concurrency, git dep for a
  fix) for what is an in-memory top-k over ≤256 candidates. Rejected.
- Instead: ~200 lines of hand-written BM25 (`opencodifier-engine`),
  property-tested against hand-computed IDF/tf values, and an
  `EmbeddingBackend` trait so embedding models are supplied by the
  runtime crate — nothing downloads at runtime, ever.

## D5 — Synchronous core, async at the edges

No Tokio (or any async runtime) in core/engine/schema/runtime. The
engine is a sync library; `Clock`, `Deadline`, and
`CancellationToken` are traits implemented by the caller. Interface
crates (axum, rmcp, wasm-bindgen) adapt. Rationale: deterministic
tests, no runtime contagion, trivial embedding in non-Tokio hosts.

## D6 — Single normative cache-key site

`CacheKeyBuilder` in `opencodifier-engine` is the only code that
computes a decision-cache key: SHA-256 (sha2 0.10) over length-prefixed
fields — canonical request with sorted candidates, graph version, model
id, calibration version, engine semver. Other crates reference
`CacheKey`, never re-derive it.

## D7 — Float policy: decision math in Rust, f64

All decision math is Rust-side f64 (softmax, calibration, margins,
entropy, score means). ONNX graphs produce logits only. Guarantees the
same decision on any backend and keeps quantization out of the
decision path. `float_cmp` lint is denied outside test modules.

## D8 — Trace contract frozen now

`trace_version = 1` on every trace, `#[non_exhaustive]` on public
enums, stable `code()` strings (`ir.*` prefix in core). Applied in
Phase 1 — before any consumer existed — so no break can ever be
"grandfathered".

## D9 — Benchmarks: criterion from Phase 3, per-stage budgets replace §70

PLANNING.md §70's single whole-pipeline budget was too coarse to act
on. Replaced by per-stage budgets (below), measured with criterion
(harness = false) committed under `benchmarks/` per release. Regression
= p99 over budget on the reference machine class (4-core laptop).

| Stage | Budget (p99) |
|-------|--------------|
| Normalize (schema → IR) | 1 ms |
| Rule match + cache hit path | 100 µs |
| BM25 narrowing, 256 candidates | 1 ms |
| Embedding scoring (supplied backend) | backend-declared |
| Deterministic decision path end-to-end | 10 ms |
| HTTP overhead added by `opencodifier-http` | 2 ms |

## D10 — Property testing: proptest + round-trip fixtures

Invariants that must never break, encoded as proptest cases: adapter
round-trips (native ⇄ wire), distribution normalization under
arbitrary probability vectors, cache-key stability under map
re-ordering, candidate narrowing subset-safety. Fixtures under
`fixtures/` lock every wire format byte-for-byte.

## D11 — Feature-flag matrix (heavyweight deps confined to one crate)

| Feature | Crate | Pulls in |
|---------|-------|----------|
| *(default)* | core, engine, schema | serde, thiserror, sha2 — nothing heavy |
| `onnx` | opencodifier-runtime | ort rc.13 (D2) |
| `runtime-onnx-dynamic` | opencodifier-runtime | ort `load-dynamic` |
| `http` | opencodifier-http | axum 0.9, tokio |
| `mcp` | opencodifier-mcp | rmcp 2.2 (D1) |
| `cli` | opencodifier-cli | clap v4 |
| `wasm` | opencodifier-wasm | wasm-bindgen, getrandom js |

No crate may enable another crate's heavyweight feature transitively;
the default build of the whole workspace stays ML-free and runtime-free.

## D12 — HTTP surface: axum 0.9, `/v1` prefix

Endpoints: `POST /v1/decide`, `POST /v1/graph/validate`,
`GET /v1/healthz`. Errors map `code()` → stable strings in the body
(`{"error": {"code", "message"}}`), HTTP status by error class (4xx
input, 5xx internal). Binding default `127.0.0.1:8177`; non-loopback
binds require `--allow-remote` (documented as a real risk in
SECURITY.md).

## D13 — CLI: clap v4 with an exit-code table

`decide`, `graph validate`, `serve`, `models verify` subcommands. Exit
codes: 0 success/abstain-with-flag, 1 input error, 2 policy gate
escalation, 3 internal error — documented in `--help` and the book so
scripts can branch on them.

## D14 — Model integrity: SHA-256 manifests, never commit weights

`models/*.json` manifests record {file, sha256, bytes, source, license};
the runtime refuses a mismatch before any tensor is read. `models/`
weights are gitignored; manifests are committed.

## D15 — Calibration storage

Per-question-class calibration parameters (isotonic or Platt) are
versioned artifacts alongside model manifests, referenced by
`calibration_version` in the cache key (D6). Un-calibrated runs report
`calibrated_confidence = raw` with `calibration_version = "none"`, so
the cache never conflates calibrated and uncalibrated results.
