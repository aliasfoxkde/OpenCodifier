# Recipes

Runnable decision graphs for OpenCodifier. Each recipe is a JSON graph the
engine executes as-is — no scripting, no hidden nodes — paired with the
request that exercises it and the response the runtime actually produced.
Everything here was captured by running the recipes against the committed
engine, not written by hand.

All recipes use the zero-ML engine (`opencodifier serve` never loads a
model — the default stack is the relational solver over the lexical
classifier), so every number is reproducible on any machine. The traces
name the deciding stack by its composed model id
(`relational-v1|builtin-lexical-v1`).

## The recipes

| Graph | Demonstrates | Outcome with the paired request |
|-------|--------------|--------------------------------|
| [`minimal-choice.json`](minimal-choice.json) | The smallest graph that can decide a choice question: normalize → filter → choice → threshold → output. The `filter` node is not optional — choice questions are decided only over candidates that survived narrowing, and a graph without narrowing abstains by design. | `verify` — top probability 0.687 sits between `verify_below` (0.65) and `min_confidence` (0.8), so the runtime asks for a second opinion instead of overclaiming. |
| [`strict-verify.json`](strict-verify.json) | The full escalation ladder (rule, cache, filter, lexical) with a deliberately strict 0.95 gate. | `verify` — even the full pipeline's lexical evidence does not clear 0.95, and the refusal is the correct answer. |
| [`boolean-score.json`](boolean-score.json) | A graph without a `choice` node: boolean and score questions only. | `abstain` — lexical evidence for both questions is below `abstain_below` (0.5). Abstention is a successful outcome (HTTP 200); the answers array still carries the full distributions. |
| [`registry/`](registry/) | The §63 Decision Registry end to end (D20): [`registry/model-selection.definition.json`](registry/model-selection.definition.json) is a decision *definition*; its request is the exact output of `DecisionDefinition::instantiate` (pinned by a test, not hand-written), run through `minimal-choice.json`'s graph. | `verify` — the registry path feeds the same IR and engine as a hand-written request: top probability 0.687 for `local-glm`, between `verify_below` (0.65) and `min_confidence` (0.8). |

### The fleet (§34)

The twelve decision areas of PLANNING.md §34, one runnable recipe each.
These ship *inside the binary*: `opencodifier recipe list` prints this
table and `opencodifier recipe install <name>` writes the same three
files (byte-identical to the committed copies) into a directory of your
project. Several abstain with the paired request — on the zero-ML
default stack that is the honest, correct outcome, and it demonstrates
the posture (an abstention is a successful outcome) better than a
forced answer would.

| Recipe | Area | Outcome with the paired request |
|--------|------|---------------------------------|
| [`model-routing.json`](model-routing.json) | Route a request to a model | `verify` (0.681) |
| [`task-classification.json`](task-classification.json) | Classify a task's workflow class | `abstain` (0.25) |
| [`tool-selection.json`](tool-selection.json) | Pick the tool for a step | `abstain` (0.333) |
| [`tool-gating.json`](tool-gating.json) | Gate whether a tool may run at all | `verify` (0.76) |
| [`context-pruning.json`](context-pruning.json) | How much context survives pruning | `verify` (0.571) |
| [`cache-eligibility.json`](cache-eligibility.json) | May this be answered from cache | `verify` (0.64) |
| [`skill-selection.json`](skill-selection.json) | Which skill owns the task | `abstain` (0.25) |
| [`memory-selection.json`](memory-selection.json) | Which memory scope holds a fact | `abstain` (0.333) |
| [`escalation.json`](escalation.json) | Escalate to a human (0.95 gate) | `abstain` (0.571) |
| [`verification.json`](verification.json) | Does an answer need verification | `verify` (0.703) |
| [`document-relevance.json`](document-relevance.json) | Score a document's relevance | `abstain` (0.333) |
| [`code-review-risk.json`](code-review-risk.json) | Score a change's review risk | `abstain` (0.25) |

Installing and running one:

```bash
opencodifier recipe install model-routing
opencodifier serve --bind 127.0.0.1:8971 --graph recipes/model-routing/graph.json
curl -s -X POST http://127.0.0.1:8971/v1/decide -H 'Content-Type: application/json' \
  --data-binary @recipes/model-routing/request.json
```

An install refuses to overwrite an existing directory unless `--force`
is passed (`cli.recipe_exists`), and unknown names are input errors
(`cli.unknown_recipe`).

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

The registry recipe's *request* has the same rule with one step
upstream: it is captured by
`cargo test -p opencodifier-schema the_registry_recipe`, which
instantiates the committed definition and refuses to pass until the
file under [`registry/requests/`](registry/requests/) matches byte for
byte.
