# OpenCodifier

An ultra-fast, deterministic-first, local AI **decision runtime**. OpenCodifier
turns unstructured state into small, machine-actionable decisions — choice,
boolean, score — using the cheapest reliable mechanism at each stage, so
expensive generative AI only runs when necessary.

It is not a chatbot, not an LLM wrapper, and not a decision-tree classifier:
it is the decision *substrate* an AI system calls before invoking a large
model. Jev/System One compatibility is one adapter mode, not the identity.

## Status

**Work in progress — pre-1.0.** The canonical decision IR
([`opencodifier-core`](crates/opencodifier-core)) is implemented and tested.
The deterministic engine, schema adapters, and interfaces are landing
phase by phase; the live plan is [`docs/PLAN.md`](docs/PLAN.md) and the
founding document is [`docs/PLANNING.md`](docs/PLANNING.md).

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
