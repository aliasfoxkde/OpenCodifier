# CI facts files — the schema the CI recipes consume

The CI recipes (`ci-test-selection`, `ci-queue-priority`,
`ci-stale-process`, `ci-risk-gate`) decide over **facts files**: JSON
documents a repo-analysis tool produces *before* the engine runs. This is
the boundary that must not move (ESCALATION_LANES_PLAN §7): OpenCodifier is
the runtime, not a CI system. Import graphs, test→file maps, heartbeats,
and ownership data are collected by a separate tool; the engine consumes
its output as `State.facts` and stays pure.

## The shape

A facts file is a canonical decision request whose `state.facts` carries
the collected evidence. Each fact is a typed `{kind, value}` pair — the
IR's own `FactValue` (PLANNING.md §9):

| kind | value | example |
|---|---|---|
| `text` | string | `{"kind": "text", "value": "flaky-test"}` |
| `integer` | i64 | `{"kind": "integer", "value": 412}` |
| `float` | finite f64 | `{"kind": "float", "value": 0.71}` |
| `boolean` | bool | `{"kind": "boolean", "value": true}` |
| `list` | string tokens | `{"kind": "list", "value": ["a.rs", "b.rs"]}` |

The rest of the request is the usual canonical document: one `questions`
entry (the CI question), a `policy`, and `metadata`. `recipe install
ci-test-selection` writes a complete worked example.

## The four CI questions

| Recipe | Question | Kind | Facts the analysis tool supplies |
|---|---|---|---|
| `ci-test-selection` | "Which test scope should run for this diff?" | choice | `changed_files` (list), `suite_size` (integer), `import_graph_depth` (integer) |
| `ci-queue-priority` | "How urgently should this queued job start?" | score | `wait_time_minutes` (integer), `priority_class` (text), `estimated_cost_usd` (float), `queue_depth` (integer) |
| `ci-stale-process` | "Is this runner process stale?" | boolean | `heartbeat_age_seconds` (integer), `pid_alive` (boolean), `last_log_age_seconds` (integer), `owned_lock` (boolean) |
| `ci-risk-gate` | "Is this failure worth a bigger model?" | boolean | `failure_class` (text), `prior_occurrences` (integer), `diagnosis_confidence` (float), `blast_radius_files` (integer) |

Fact names are conventions, not enforced keys — the zero-ML stack reads
the state text and the lexical evidence, so a facts file with different
names still decides (more weakly). The names above are what the shipped
example requests use; an analysis tool should emit them so the recipes'
captures stay meaningful.

## What the engine contributes that a CI script does not have

- a **calibrated** answer whose threshold can be *fitted from evidence*
  (`ladder fit-boolean` over the feedback JSONL from #193) rather than
  hand-tuned;
- a **deterministic trace** — "why this test scope" is a replayable fact
  list, not a log line;
- an **abstention that is a success** — "I could not narrow this to fewer
  than 40 tests" beats a confidently wrong 3-test subset. The shipped
  captures include one abstention (`ci-queue-priority`) on purpose: it is
  the honest zero-ML answer and it demonstrates the posture.

## End to end

```bash
opencodifier recipe install ci-stale-process --dest ./ci
opencodifier serve --graph ci/ci-stale-process/graph.json --evidence ./evidence.jsonl &
# ...the repo-analysis tool writes the facts file (the installed
# request.json is the worked example)...
opencodifier decide -i ci/ci-stale-process/request.json
# ...labels recorded by POST /v1/feedback close the loop:
opencodifier ladder fit-boolean --evidence evidence.jsonl --id ci-stale-v1 \
  --out ladder.json
opencodifier serve --graph ci/ci-stale-process/graph.json --ladder ladder.json
```

The last line re-serves with the *fitted* gate: the loop from facts file
to calibrated threshold with no hand-tuned number anywhere.
