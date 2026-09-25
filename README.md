# OpenCodifier

An ultra-fast, deterministic-first, local AI **decision runtime**. OpenCodifier
turns unstructured state into small, machine-actionable decisions — choice,
boolean, score — using the cheapest reliable mechanism at each stage, so
expensive generative AI only runs when necessary.

It is not a chatbot, not an LLM wrapper, and not a decision-tree classifier:
it is the decision *substrate* an AI system calls before invoking a large
model. Jev/System One compatibility is one adapter mode, not the identity.

## Status

**Work in progress — pre-1.0, but real and runnable today.** The
canonical decision IR, schema adapters (native / OpenAI / Anthropic /
Jev), deterministic engine (graphs, rules, caches, BM25 narrowing),
backend traits, and both primary interfaces are implemented and tested:

- `opencodifier` CLI — `decide`, `graph validate`, `serve`,
  `models verify` (see `--help` for the exit-code contract).
- `POST /v1/decide`, `POST /v1/graph/validate`, `GET /v1/healthz` —
  axum server, loopback-only unless explicitly told otherwise.

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

## Development

```bash
git clone https://github.com/aliasfoxkde/OpenCodifier
cd OpenCodifier
just ci        # fmt + clippy (strict) + tests + doc + cargo-deny
just coverage  # line coverage (cargo llvm-cov)
```

Requires a stable Rust toolchain (MSRV 1.90, declared in `Cargo.toml`).
Contribution rules are in [`CONTRIBUTING.md`](CONTRIBUTING.md); the
security policy is in [`SECURITY.md`](SECURITY.md).

## License

Apache-2.0 — see [`LICENSE`](LICENSE).
