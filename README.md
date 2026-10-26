# OpenCodifier

An ultra-fast, deterministic-first, local AI **decision runtime**. OpenCodifier
turns unstructured state into small, machine-actionable decisions — choice,
boolean, score — using the cheapest reliable mechanism at each stage, so
expensive generative AI only runs when necessary.

It is not a chatbot, not an LLM wrapper, and not a decision-tree classifier:
it is the decision *substrate* an AI system calls before invoking a large
model. Jev/System One compatibility is one adapter mode, not the identity.

**Live site & measured benchmark board:** [opencodifier.pages.dev](https://opencodifier.pages.dev)
— every number this project claims, one filterable table, no hand-typed figures.

## Status

**Work in progress — pre-1.0, but real and runnable today.** The
canonical decision IR, schema adapters (native / OpenAI / Anthropic /
Jev), deterministic engine (graphs, rules, caches, BM25 narrowing,
cross-rung escalation, position-bias-corrected rotation readouts with
certified early exit), candidate-conditioned decision-model backends,
backend traits, and the CLI, HTTP, MCP, and WASM interfaces are
implemented and tested:

- `opencodifier` CLI — `decide`, `graph validate`, `serve`,
  `mcp serve`, `models verify`, `ladder fit-boolean`,
  `recipe list`, `recipe install`
  (see `--help` for the exit-code contract).
- `POST /v1/decide`, `POST /v1/batch`, `POST /v1/chat/completions`,
  `POST /v1/graph/validate`, `POST /v1/graph/run`, `POST /v1/validate`,
  `GET /v1/models`, `GET /v1/capabilities`, `GET /v1/healthz` — axum
  server, loopback-only unless explicitly told otherwise.
- Decision-model rungs — candidate-conditioned scoring contracts
  (Kai-0.6B, julia) with committed fixtures ship unconditionally; the
  llama.cpp loopback backend (D26) sits behind the `llamacpp` cargo
  feature, with cross-rung escalation (D27) for the verify/abstain
  bands.
- MCP — `opencodifier mcp serve` exposes `codify_decide`,
  `codify_batch`, `codify_graph`, `codify_validate`, `codify_verify`,
  `codify_explain`, and `codify_gate` as tools over stdio: point any MCP host at the
  binary and the runtime's decisions become host-side tool calls. Stdio
  only, so the surface is local by construction.

The live plan is [`docs/PLAN.md`](docs/PLAN.md) and the founding
document is [`docs/PLANNING.md`](docs/PLANNING.md). Try it with the
committed native fixture (choice + score + boolean in one request):

```bash
cargo run -p opencodifier-cli -- decide --input fixtures/native/request.json
```

The response carries a full distribution per answer, a multi-dimensional
confidence report, and the deterministic execution trace. Under the
fixture's default policy the lexical engine's 0.5 top confidence lands in
the verify band, so the CLI prints the complete response and exits `2`
(policy-gate escalation) — the runtime refusing to overclaim is the
product working. Add `--abstain-is-success` to exit `0`, or `--trace`
to print the execution report. Validate any decision graph without
running it:

```bash
cargo run -p opencodifier-cli -- graph validate <graph.json>
```

Ready-to-run decision graphs — including the smallest graph that can
decide a choice question and a deliberately strict gate that answers
`verify` instead of guessing — live in [`recipes/`](recipes/) with the
requests that exercise them and the captured responses to compare
against.

## Measured results (one number per surface)

Every figure below is traceable to a row in
[`docs/BENCHMARKS.md`](docs/BENCHMARKS.md); the four eval surfaces are
kept separate on purpose — no number is quoted across surfaces.

| surface | what it is | our number |
|---|---|---|
| JevBench-231 (public split) | 231-item typed-decision board | **0.6840** (e1-e arm) |
| Internal holdout suite | gate suite (231-split disjoint) | **0.8083** (e1-e, GATE_PASS) |
| Engine rung (deterministic-only) | zero-ML ladder floor | **0.3766** |
| Board ECE (calibration, raw) | honest-uncertainty check | 0.207 (0.127 with shipped calib) |

## Design commitments

- **Deterministic first.** Exact rule → cached decision → metadata filter →
  lexical match → embedding → small classifier → verifier → escalate. Never
  run a more expensive layer than the question requires.
- **Typed probabilistic decisions.** Every answer carries a full
  distribution and *calibrated* confidence — raw softmax is never exposed
  as confidence.
- **Abstention is success.** Low confidence yields an explicit abstain or
  escalate, never a forced guess.
- **Local, offline, private.** No telemetry, no cloud calls, no accounts,
  no API keys; the server binds `127.0.0.1` unless told otherwise.
- **Useful with zero ML.** The deterministic engine and lexical layers are
  fully functional without any neural model.

## Models

Decision-model weights publish to HuggingFace, gated on the
[release policy](docs/RELEASE_POLICY.md) (suite ≥ 0.68 **and** JevBench
≥ 0.6494 before anything ships). Weights are never downloaded
automatically — not at build time, not at runtime; access is explicit
opt-in.

| release | org | what |
|---|---|---|
| [opencodifier-e1e-2b](https://huggingface.co/TaskWizerAI/opencodifier-e1e-2b) | [TaskWizerAI](https://huggingface.co/TaskWizerAI) | E1-E verdict-slot LoRA on Qwen3.5-2B — first own arm to pass **both** gate legs (suite 0.8083, JevBench 0.6840); ships the holdout-fitted temperature artifact |
| [OpenCodifier-Decision-2B-v6full](https://huggingface.co/aliasfox/OpenCodifier-Decision-2B-v6full) | [aliasfox](https://huggingface.co/aliasfox) | earlier gated 2B decision weights, per-surface numbers + withheld-corpus disclosure |

## Documentation

Start here; every page links back to this index. Dense reference docs
are broken into sections — read top-down only as deep as you need.

**Getting oriented**

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — crates, dependency
  direction, pipeline shape (5 min).
- [`docs/PROJECT_STRUCTURE.md`](docs/PROJECT_STRUCTURE.md) — where
  everything lives in the tree.
- [`docs/PLANNING.md`](docs/PLANNING.md) — the founding spec (§§1–80);
  intent, not status.

**Running it**

- [`recipes/`](recipes/) — runnable graphs + captured responses.
- [`docs/FORK_LLAMA_SERVER.md`](docs/FORK_LLAMA_SERVER.md) — the
  llama.cpp fork that serves decision rungs.
- [`docs/DECISION_RUNNER.md`](docs/DECISION_RUNNER.md) — why
  llama.cpp and not vLLM/SGLang (D44).
- [`docs/INTEGRATIONS.md`](docs/INTEGRATIONS.md) — external systems
  (incl. Amortyx router integration).

**Numbers & evidence**

- [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md) — the comparison page of
  record: board table, per-family breakdowns, footnoted provenance.
  Read §Board rows first; footnotes carry the caveats.
- [`docs/VALIDATION.md`](docs/VALIDATION.md) — the #104 validation
  campaign record.
- [`docs/COVERAGE.md`](docs/COVERAGE.md) — coverage methodology.

**Governance & decisions**

- [`docs/DECISIONS.md`](docs/DECISIONS.md) — binding decision records
  (D1–D44+); these supersede anything contradictory elsewhere.
- [`docs/PLAN.md`](docs/PLAN.md) — live execution plan + phase status.
- [`docs/RELEASE_POLICY.md`](docs/RELEASE_POLICY.md) — the gate any
  model must clear before publishing.
- [`docs/OPERATOR_ACTIONS.md`](docs/OPERATOR_ACTIONS.md) — items only
  the operator can do.

**Research**

- [`docs/RESEARCH.md`](docs/RESEARCH.md) — sectioned research log
  (§1–§32+), each section self-contained: source, findings, verdict,
  what shipped. Read the section table of contents at the top, dive
  into one section.
- [`docs/TRAINING.md`](docs/TRAINING.md) — the training record: arms,
  recipes, falsified levers.

**Planning archive** (point-in-time, kept for provenance — not live
policy): [`docs/planning/`](docs/planning/), including the
[Vulkan 4B probe](docs/planning/vulkan-4b-probe-2026-10-26.md) no-go
and the [offline campaign](docs/planning/offline-campaign-2026-10-23.md)
tracker.

## Development

```bash
git clone https://github.com/aliasfoxkde/OpenCodifier
cd OpenCodifier
just ci        # fmt + clippy (strict) + tests + doc + cargo-deny + aegis
just coverage  # line coverage (cargo llvm-cov)
```

Requires a stable Rust toolchain (MSRV 1.90, declared in `Cargo.toml`).
Contribution rules are in
[`CONTRIBUTING.md`](.github/CONTRIBUTING.md); the security policy is
in [`SECURITY.md`](.github/SECURITY.md).

## Ecosystem

Related open-source projects from the same shop:

- [Aegis](https://github.com/aliasfoxkde/aegis) — high-performance
  security scanning for DevOps pipelines; part of this repo's own
  CI gate.
- [GitForge](https://github.com/aliasfoxkde/GitForge) — self-hosted,
  fast, GitHub-compatible forge; this repo's CI/CD platform of record.
- [AgentVerify](https://github.com/aliasfoxkde/AgentVerify) — open-source
  outcome verification for agent actions.
- [SiliconRank](https://github.com/aliasfoxkde/SiliconRank) — normalized
  compute/hardware benchmarking hub: CPUs, GPUs, accelerators, systems.
- [dark-factory](https://github.com/aliasfoxkde/dark-factory) — AI-first
  template repository: CI/CD, pre-commit hooks, E2E testing framework,
  and a reusable code library, designed for 90%+ automated coverage.

## License

Apache-2.0 — see [`LICENSE`](LICENSE).
