# Contributing to OpenCodifier

Thanks for contributing. This document describes the working agreement;
project structure and architecture live in [`docs/`](docs/) and the
engineering rules in [`docs/PLANNING.md`](docs/PLANNING.md) (§73 is binding).

## Development quickstart

```bash
git clone https://github.com/aliasfoxkde/OpenCodifier
cd OpenCodifier
just ci          # fmt + clippy + test + doc + deny — must pass before every PR
just coverage    # line coverage report (cargo llvm-cov)
```

Commits follow [Conventional Commits](https://www.conventionalcommits.org/)
(`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`); subject ≤ 50
chars, body wrapped at 72.

## Ground rules

1. **Deterministic-first.** Never add an ML inference path where a
   deterministic rule, filter, or cached result can decide with sufficient
   confidence (PLANNING.md §3).
2. **No placeholders.** No `TODO`/`FIXME`/unimplemented stubs land on
   `main`; the workspace lints deny `clippy::todo` and `clippy::unimplemented`.
3. **No `unwrap()`/`expect()`/`panic!`** in production paths; return
   `Result`. Test modules may re-allow unwrap locally.
4. **`core` stays pure.** `opencodifier-core` must not gain dependencies on
   HTTP, MCP, CLI, Tokio, or any ML runtime (PLANNING.md §39).
5. **Abstention is a feature.** Never force a decision when confidence is
   insufficient; return an explicit abstain/escalate outcome.
6. **Raw probabilities are not confidence.** Calibrated confidence requires
   the calibration layer; do not surface softmax output as final confidence.
7. **Document public APIs.** `missing_docs` is warn + CI-enforced
   `-D warnings`: undocumented public items fail the build.
8. **Tests travel with behavior.** Every fix or feature lands with tests
   that fail without it. Schema changes land with fixtures under
   `fixtures/`.

## Pull requests

- Keep PRs scoped to one phase/concern; reference the PLANNING.md phase.
- CI must be green: `cargo fmt --check`, `cargo clippy -D warnings`,
  `cargo test --workspace`, `cargo test --doc`, `cargo deny check`.
- New public APIs need rustdoc with at least one example or invariant note.

## License

By contributing you agree your work is released under the repository's
[Apache-2.0 license](LICENSE).
