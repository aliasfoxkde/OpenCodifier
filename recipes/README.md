# Recipes

Runnable decision graphs for OpenCodifier. Each recipe is a JSON graph the
engine executes as-is — no scripting, no hidden nodes — paired with the
request that exercises it and the response the runtime actually produced.
Everything here was captured by running the recipes against the committed
engine, not written by hand.

All recipes use the zero-ML lexical engine (`opencodifier serve` never
loads a model), so every number is reproducible on any machine.

## The recipes

| Graph | Demonstrates | Outcome with the paired request |
|-------|--------------|--------------------------------|
| [`minimal-choice.json`](minimal-choice.json) | The smallest graph that can decide a choice question: normalize → filter → choice → threshold → output. The `filter` node is not optional — choice questions are decided only over candidates that survived narrowing, and a graph without narrowing abstains by design. | `verify` — top probability 0.687 sits between `verify_below` (0.65) and `min_confidence` (0.8), so the runtime asks for a second opinion instead of overclaiming. |
| [`strict-verify.json`](strict-verify.json) | The full escalation ladder (rule, cache, filter, lexical) with a deliberately strict 0.95 gate. | `verify` — even the full pipeline's lexical evidence does not clear 0.95, and the refusal is the correct answer. |
| [`boolean-score.json`](boolean-score.json) | A graph without a `choice` node: boolean and score questions only. | `abstain` — lexical evidence for both questions is below `abstain_below` (0.5). Abstention is a successful outcome (HTTP 200); the answers array still carries the full distributions. |

Each recipe's request lives in [`requests/`](requests/) and its captured
response in [`expected/`](expected/).

## Running a recipe

Validate the graph without running it:

```bash
cargo run -p opencodifier-cli -- graph validate recipes/minimal-choice.json
# ok: graph `recipes/minimal-choice.json` valid (version 1, 5 nodes, 5 waves)
```

Serve it and post the paired request:

```bash
cargo run -p opencodifier-cli -- serve --bind 127.0.0.1:8971 --graph recipes/minimal-choice.json
# opencodifier-http listening on http://127.0.0.1:8971

curl -s -X POST http://127.0.0.1:8971/v1/decide \
  -H 'Content-Type: application/json' \
  --data-binary @recipes/requests/choice.json
```

Compare against the captured response. Two contract notes:

- **Use a fresh server for the comparison.** The strict recipe's cache node
  means a *second* identical request on the same server reports
  `"cache_hit": true`; that is the cache working, not a different decision.
  Restarting `serve` (or comparing the first response only) reproduces the
  committed files byte-for-byte.
- **Abstain and verify are successes.** `verify`/`abstain` arrive as HTTP
  200 with the outcome field naming the gate decision. Over the CLI the
  same non-decisive outcomes exit `2` (see `decide --help`), which is the
  documented escalation contract, not an error.

## Deriving a new expected output

Expected outputs are captured, never hand-written:

1. Start `serve` with the graph on a fresh process.
2. `POST` the request once.
3. Save the response body.

The engine is deterministic (D7: all decision math in Rust f64; no ML in
the default build), so the same graph + request + fresh server always
produces the same bytes.
