# The gate, not the oracle — escalation, lanes, first-run detection, and CI/CD

Status: **proposal** (2026-10-21). Author: Michael Kinney + Claude.
Supersedes nothing. Complements `docs/PLAN.md` (phase status), `docs/DECISIONS.md`
(binding records), `docs/ARCHITECTURE.md` (shape). Where this document and
`DECISIONS.md` disagree, `DECISIONS.md` wins — the records there are binding and
this file is not yet a record.

---

## 0. The concept in one page

OpenCodifier should not try to *replace* the user's LLM. It should sit **in
front of it** as a fast, deterministic gate that answers in milliseconds when
it can, refuses in milliseconds when it cannot, and hands the hard cases to the
model the user already has — with the engine's partial work attached, so the
model starts from evidence instead of from zero.

That reframes the product:

| framing | unit of value | who wins |
|---|---|---|
| **decision oracle** (today's framing) | "the engine answered the question" | OpenCodifier vs JEV on accuracy |
| **gate / grounder** (this proposal) | "the LLM did not have to figure this out" | OpenCodifier vs *nothing*, and vs a bare LLM |

The board story and the benchmark story stay exactly where they are — the
engine's own accuracy is still measured and still matters. What changes is
that the engine gains a second, larger surface: it becomes a *component of
someone else's AI system*. That surface is where the daily usage lives, and
it is the surface that justifies MCP, lanes, first-run detection, and CI/CD.

**The companion framing (Amortyx).** Amortyx finds patterns in code — ASTs,
call graphs, diffs. OpenCodifier makes calibrated decisions. Together they
are the two halves of "cheap, deterministic signal first, expensive model
second": Amortyx *narrows the code*, OpenCodifier *decides what to do about
it*. The MCP surface is where the two meet.

---

## 1. What already exists (so we build the delta, not the thing)

This is the most important section, because most of the concept is already
half-implemented and the risk is rebuilding it under new names.

| the concept | what is already in the tree | what is missing |
|---|---|---|
| escalate to a bigger model | `DecisionOutcome::Escalate` **exists in the IR and is never constructed**; the engine already has an ordered rung tail (`fallbacks: Vec<Rung>`) that escalates *internally* | the outcome is never produced; there is no *external* handoff; there is no payload |
| cheap rung first, expensive rung last | the ladder is exactly this: rules → relational → lexical → embedding → model → verifier, gated by `DecisionPolicy` | no way to *configure* a whole stack in one place; no named stacks |
| "should I do the expensive thing?" | `BooleanQuestion`, `boolean_threshold`, `fit_boolean_threshold` | no first-class "gate" tool for agents; the calibration evidence to fit the threshold has no capture path |
| presets | 12 compiled-in **recipes** (`recipe list` / `recipe install`) and 3 **ladder profiles** (`/ladders/*.json`) | recipes are *graphs*; there is no preset that spans engine + model + ladder + escalation |
| configuration | **none at all** — every input is an explicit CLI flag pointing at a JSON document; no config file, no env vars, no discovery | everything |
| first run | **none** — no `init`, no `doctor`, no detection | everything |
| the site's engine | `site/wasm/` (wasm-pack build) already loads the real Rust engine in the browser via the Decisions SDK `engineBridge` | no QA surface, no yes/no tool |
| CI/CD | the engine's decision kinds map onto CI's decisions (which test, which job first, is this stale) | no recipes, no facts extractor, no gate wiring |
| feedback loop | `ladder fit-boolean` already consumes `{p_true, label}` JSONL evidence | nothing *collects* that evidence; the loop is open |

**The sharpest observation:** `DecisionOutcome::Escalate` was designed for
precisely this and never wired. The repo already has the vocabulary; it lacks
the mechanism and the payload.

**The second sharpest observation:** the engine's own benchmark says the
0.8B model rung (accuracy 0.650) is **worse than the zero-ML floor** (0.683).
So "more model = better" is false inside this repo's own measurements. Lanes
therefore must be labelled by **contract and cost**, not by model size — and
"engine only, hand the hard cases to the host LLM" is not a *degraded* lane,
it is a *different* lane with a different value proposition.

---

## 2. The escalation contract

### 2.1 Where it fires

`DecisionOutcome` is `#[non_exhaustive]` and already carries `Escalate`. The
production rule stays the same shape as today's gate, with one extra band
resolved inside the existing "not decisive" region:

```
c = calibrated confidence

c < abstain_below                         -> Abstain          (terminal; no evidence to hand off)
abstain_below <= c < escalate_below       -> Escalate         (NEW: hand to the host)
c >= escalate_below, c < min_confidence   -> Verify           (unchanged)
c >= min_confidence, risk <= Medium       -> Accept           (unchanged)
```

`escalate_below` is `Option<f64>` on `DecisionPolicy`, default `None` — so
**absent means the current behaviour, byte for byte**. A deployment opts in by
setting one number.

Escalation also fires on `NoValidCandidate` (no candidate survived
deterministic filtering — the host model may still know), and on
`risk: High` when a verifier disagreed.

### 2.2 What the host model receives

The handoff is worthless if it is just "I'm not sure". The payload must carry
the engine's partial work so the host model starts from evidence. Proposed
shape (native format; adapters project it):

```json
{
  "outcome": "escalate",
  "confidence": { "top_probability": 0.44, "margin": 0.07, "entropy": 1.91,
                  "calibrated_confidence": 0.31, "ood_score": 0.22,
                  "verifier_agreement": null },
  "answers": [ { "kind": "boolean", "question_id": "q1", "value": false,
                 "probability": 0.31, "confidence": 0.31 } ],
  "escalation": {
    "reason": "below_confidence",
    "gate": { "kind": "confidence", "threshold": 0.62, "observed": 0.31 },
    "requested_action": "ask_the_host_model",
    "engine": { "model_id": "relational-v2|builtin-lexical-v2|focused-v1@512",
                "graph_version": "...", "calibration_version": "...", "engine_semver": "..." },
    "payload": {
      "question": { "kind": "boolean", "text": "Is the sky blue?" },
      "state_summary": { "text_bytes": 41, "fact_count": 3, "image_count": 0 },
      "candidates_ruled_in":  [ { "id": "a", "why": "matched rule: R3" } ],
      "candidates_ruled_out": [ { "id": "b", "why": "filtered by metadata: tier" } ],
      "partial_answer": { "value": false, "probability": 0.31 },
      "trace": [ { "node": "narrow", "detail": { "out": 2 } } ],
      "guidance": "No exact rule matched. Answer from the state above; say 'unknown' if the state does not settle it."
    }
  }
}
```

Hard rules for the payload:

- **No chain of thought.** The `trace` is the deterministic execution trace
  (repo rule), never model reasoning.
- **Bounded.** The payload is going into another model's context. It gets a
  size cap (`Limits.max_input_bytes` family) and is truncated deterministically
  with an explicit `truncated: true` marker — never silently.
- **Self-contained.** The host model must not need a second round trip to
  understand what was asked. If it does, the engine has not finished its job.
- **Deterministic.** Same request + same engine version → byte-identical
  payload. The payload is part of the cache-key material already.

### 2.3 Two surfaces, not one

The user asked for "escalation to the user's LLM (tooling, agents, workflow,
pipeline)". These are two different mechanisms and conflating them is a design
error:

| surface | mechanism | why |
|---|---|---|
| **delegate** (MCP, agent clients) | the engine returns `outcome: "escalate"` + payload; **the client** (which already owns the LLM) acts on it | the MCP client *is* the LLM host; calling back into it from the server is circular, and it would require the server to hold provider credentials |
| **delegate-to-endpoint** (HTTP/CLI pipelines, CI) | the engine POSTs the escalation payload to a configured endpoint | the pipeline has no client in the loop; the endpoint is the pipeline's own step |

`delegate-to-endpoint` is **opt-in, loopback by default, and never a provider
SDK**: the engine does not know what an LLM is. It POSTs a JSON document to a
URL the operator configured. A local `llama-server` is just one such endpoint.
This keeps the default posture (no cloud, no keys, no telemetry) intact and
keeps `core` free of HTTP.

### 2.4 The MCP surface

The existing six tools are projections of engine decisions and stay as they
are. Add:

- `codify_gate` — a compact, agent-facing "should I proceed?" call. Boolean
  question + state + optional options → `{ proceed: bool, confidence, reason,
  evidence }`. Purpose-built for an agent loop: cheap, small, one call, no
  JSON editor. This is the tool an agent calls *before* spending tokens.
- `codify_feedback` — record `(request digest, p_true, label, source)` to a
  calibration-evidence file (opt-in path). Closes the loop with
  `ladder fit-boolean`, which already speaks `{p_true, label}` JSONL.
- `codify_decide` gains the `escalation` block automatically (it is part of the
  native response).

---

## 3. Lanes — the presets

A **lane** is a named, complete answer to "what stack should serve this
request?": the engine configuration, the rung tail, the model backend, the
ladder profile, the escalation policy, and the cost envelope it implies.

Proposed lane table (numbers are the repo's own measured rows, `docs/BENCHMARKS.md`):

| lane | stack | accuracy | p50 | footprint | contract |
|---|---|---:|---:|---:|---|
| `zero` | rules + relational + lexical | **0.683** | 1.3 ms | 0 MiB | offline, deterministic, no artifacts |
| `guard` | `zero` + local embedding model (gte-modernbert int8) | *unmeasured as a combination* | *unmeasured* | ~130 MiB | better semantic narrowing, still no generative model |
| `delegate` | `zero` + escalate-to-host-LLM | 0.683 engine-side | 1.3 ms + host | 0 MiB | **the user's ask: no decision model at all, just the fast gate** |
| `local-2b` | `guard` + local llama.cpp 2B (CPU) | **0.725** | 1.7 s | 1.2 GiB | self-contained accuracy |
| `local-4b` | `guard` + local llama.cpp 4B Q4_K_M | **0.800** | 4.9 s | 2.8 GiB | self-contained accuracy, GPU/Vulkan optional |
| `local-9b` | `guard` + 9B Q3_K_S | **0.817** | 14.3 s | 4.1 GiB | the accuracy ceiling, the latency floor |
| `verify` | `local-2b` as decider + a second model as verifier | *unmeasured* | *unmeasured* | ~2.4 GiB | agreement-gated |
| `ci` | `zero` + CI facts graph + escalation gate | n/a (task-specific) | <5 ms | 0 MiB | CI decision support |

**What the bold numbers are.** They are the repo's own measured rows
(`docs/BENCHMARKS.md`, the "Measured tiers" table) for the *closest* measured
stack — the model rows are the model-rung arms, and the `zero` row is the
engine floor. They are **not** measurements of the composed lane: no run has
measured `zero` + embedding + model stacked the way a lane composes them.
A lane that ships without a measured row of its own says so, and says which
row stands in for it. `guard` and `verify` are exactly that case today, and
both need their own arm before they are advertised.

Rules for lanes:

- **A lane is a contract, not a size.** `delegate` is not "the degraded lane";
  it is the cheapest lane and the one that composes best with an existing
  agent.
- **Absence is explicit.** A lane that names a model artifact names its
  *expected* artifact and fails loudly with `model.missing` when it is absent.
- **Selection is by constraint, not preference.** Each lane declares tags:
  `offline`, `cpu-only`, `gpu`, `mobile`, `server`, `batch`, `interactive`.
  First-run detection proposes lanes by *what the host has*, and the user
  confirms.
- **The 0.8B rung is not a lane.** At 0.650 it is below the zero-ML floor
  (0.683); it is only interesting as a *verifier*, never as a decider.

CLI surface:

```
opencodifier lane list          # table with cost/accuracy/latency envelope
opencodifier lane show <name>    # the exact resolved config it expands to
opencodifier lane use <name>     # writes the chosen lane into the config
```

---

## 4. The configuration layer (the real gap)

There is **no config file** anywhere in the workspace: no TOML, no YAML, no
JSON config discovery, no environment variables, no profiles. Everything is a
CLI flag. Every feature in §2–§3 needs a place to live, so this is the
prerequisite, not an optional extra.

Proposed:

- New crate **`opencodifier-config`** (depends on `core` + `engine` only —
  no HTTP, no MCP, no ML runtime; it may be depended on by every interface
  crate).
- File format: **JSON**, because every other artifact in this repo is JSON
  (graph, policy, ladder profile, model manifest). Consistency beats
  ergonomics here.
- Location: `$OPENCODIFIER_CONFIG` → `$XDG_CONFIG_HOME/opencodifier/config.json`
  → `~/.config/opencodifier/config.json`. Project-local override:
  `./opencodifier.json` (highest precedence below explicit flags) — because
  CI wants a repo-local config.
- Precedence: **CLI flag > env var > project file > user file > built-in
  default.** Every existing flag becomes an override of the resolved config.
- Schema sections: `lane`, `engine` (graph, focus budget), `rungs` (ordered
  tail), `model` (backend, id, timeout, artifact paths), `ladder` (profile
  path), `escalation` (target, threshold, mode), `http` (bind).
- Validation is strict and fails with stable `config.*` error codes.
- **Byte-identity rule**: with no config file present and no flags passed, the
  resolved configuration must be byte-identical to today's behaviour. This is
  the repo's standing discipline (D25's "empty = byte-identical") and it is
  testable: the existing fixture suite must produce identical responses.

---

## 5. First-run detection

`opencodifier init` — probe, propose, confirm, write. **Never downloads
anything.** A model is fetched only by an explicit, separate, opt-in command
(consistent with the release policy: "explicit opt-in pull only, never
auto-download at build").

Probes (all local, all read-only):

| probe | how | what it decides |
|---|---|---|
| CPU | `nproc`, `/proc/cpuinfo` flags (AVX2/AVX512), `uname -m` | which local model tiers are viable |
| RAM | `/proc/meminfo` | whether a 2B/4B model fits alongside the caller |
| GPU | Vulkan ICDs (`/usr/share/vulkan/icd.d`), `vulkaninfo --summary`, `rocm-smi`, `nvidia-smi` | `local-4b` CPU vs Vulkan, and the measured latency difference |
| local server | `llama-server` on `PATH`, in `forks/llama.cpp/build`, in common model dirs | whether a `local-*` lane is live |
| model artifacts | `*.gguf` under known dirs; ONNX model manifests; `ORT_DYLIB_PATH` | which lane the artifacts support |
| existing config | the config search path | whether this is a first run at all |

Output: **two or three candidate lanes**, each with a one-line rationale, the
artifacts found, the expected latency/footprint, and what it would cost to
upgrade to the next lane. Then a confirm:

```
opencodifier init                 # interactive: pick 1..N
opencodifier init --yes           # accept the top recommendation (non-interactive)
opencodifier init --json          # machine-readable proposal, no writes
opencodifier init --lane local-2b # skip the prompt
```

`opencodifier doctor` — the same probes, **no writes**, actionable output, exit
0/1. This mirrors the house pattern (the harness already has a `doctor`), and
it is the natural home for "your llama-server is not running" / "ORT dylib not
found" / "model artifact digest mismatch".

---

## 6. The feedback loop (systems that play well with each other)

The user described the loop: the engine answers in milliseconds, the model
gets immediate feedback, the model uses it to become more accurate, and the
result "gets truthed out quickly". The repo already has one half of this — the
calibration-fitting tool — and is missing the other half.

```
engine decides  ->  host model answers (or the human does)
                 ->  POST /v1/feedback { digest, p_true, label, source }
                 ->  evidence.jsonl  ->  ladder fit-boolean  ->  new threshold
                 ->  the engine's gate is now better calibrated
```

- `POST /v1/feedback` and `codify_feedback` append `{p_true, label}` rows to a
  caller-named JSONL file. The file is the exact format `ladder fit-boolean`
  already reads — so the loop closes with **no new mathematics**.
- Labels come from three sources, distinguished by a `source` field: `human`
  (a person corrected the engine), `host_model` (a bigger model answered and
  was trusted), `outcome` (the engine's own decision was confirmed correct).
  Only `human` labels should drive thresholds; the others are self-confirming
  and would drag calibration toward the model's own bias. That distinction is
  the whole reason the field exists.
- The engine already stores its own answers; the digest (`sha256` of the
  canonical request bytes) is the join key. No new storage.

This is the "companion to Amortyx" mechanism in its most concrete form: the
engine and the model improve each other, and the improvement is *measured*
(ECE) rather than asserted.

---

## 7. CI/CD — GitForge, Actions, pipelines

The CI use cases are all **choice/score/boolean decisions over a candidate
list**, which is the engine's native shape:

| CI question | engine decision kind | candidate list | determinism source |
|---|---|---|---|
| "Which tests should run for this diff?" | Choice | the test suite | the repo's facts (import graph, test→file map) |
| "Which queued job goes next?" | Score | the queue | facts: wait time, priority, estimated cost |
| "Is this process stale?" | Boolean | one process | facts: heartbeat age, pid liveness |
| "Is this failure worth a bigger model?" | Boolean | the failure | the engine's own confidence on the diagnosis |
| "Is this PR risky enough to require review?" | Boolean | one PR | facts: files touched, ownership, blast radius |

**The boundary that must not move:** OpenCodifier is the *runtime*. It does
not become a CI system. The repo-analysis (import graph, test→file map,
heartbeats) happens in a **separate tool** that emits a JSON facts file; the
engine consumes that file as `State.facts`. The runtime stays pure, the
dependency direction stays intact, and the CI integration is a *consumer*.

What the engine contributes that a CI script does not have:

- a **calibrated** answer with a threshold that can be *fitted from evidence*
  (`ladder fit-boolean`) rather than hand-tuned;
- a **deterministic trace** — "why did it choose this test" is a replayable
  fact list, not a log line;
- an **abstention** that is a success — "I could not narrow this to fewer than
  40 tests" is a better answer than a confident wrong 3-test subset.

Ship as recipes (the repo's existing mechanism for graphs):
`ci-test-selection`, `ci-queue-priority`, `ci-stale-process`, `ci-risk-gate`,
plus the facts-file schema the graphs consume.

---

## 8. The Playground QA tool

"Ask a yes/no question, get a confident score back." The site already loads
the **real Rust engine in the browser** (`site/wasm/`, wasm-pack, the Decisions
SDK `engineBridge`) with a live "Network: 0 cross-origin" counter. So the QA
tool is a thin UI over the real engine — no server, no model, no network.

**Honesty constraint.** The engine has no world knowledge. "Is the sky blue?"
cannot be answered by a lexical classifier over an empty state, and a QA tool
that returned 0.97 for it would be lying — which is exactly what this repo's
"Claims we refuse to make" card exists to prevent. So the tool ships with a
**visible, editable knowledge panel**: a small set of rules that bind a
question to a fact (e.g. `sky_color = blue` ⇒ the question "is the sky blue"
answers *yes* at p = 1.0). The score on screen is the engine's real output
from a real rule — with the firing rule visible in the trace.

Three states, all honest and all interesting:

1. **Decided** — a rule fired; show the answer, the calibrated confidence, the
   6-component confidence report, and the trace with the firing rule named.
2. **Abstained** — no rule matched; show *that*, and show the escalation
   payload the engine would hand to a host model. This is the concept demo.
3. **Escalated** — the lane is configured with an escalation threshold; show
   the payload and the gate that tripped.

Design constraints from the site's own rules (memory): disclosure panels are
**mutually exclusive** (opening one collapses the rest), the panel must be
responsive, and the page must keep its WCAG audit clean. The knowledge panel is
a disclosure; the trace is a disclosure; the escalation payload is a
disclosure — all three collapse each other.

---

## 9. The measurement that validates the whole concept

**This is the most important item in the document.** Without it, everything
above is an API with no evidence. If the gated arm does not beat the bare-LLM
arm, the lanes and the escalation contract are decoration.

Design (a new arm in `benchmarks/decision-model/`):

- **Task set**: a real agentic workload with a known answer key — not a
  one-shot QA set. The point of a gate is the *loop*, so the workload should be
  multi-turn or multi-step (e.g. "which of these 12 files does this bug live
  in", "should I run the full suite or the subset").
- **Arm A (control)**: the LLM alone, with the same prompt budget.
- **Arm B (treatment)**: the LLM, preceded by the engine's gate. When the
  engine is confident, its answer is used and the LLM is not called. When it is
  not, the LLM is called *with the escalation payload*.
- **Metrics**: accuracy (vs the answer key), tokens consumed, wall-clock
  latency, and the three diagnostic rates:
  - `engine_sufficient` — fraction of calls the engine alone got right;
  - `engine_wrong` — fraction where the engine was confident and wrong
    (the cost of the gate — this is the number that can kill the concept);
  - `saved_calls` — fraction of LLM calls avoided.

The decision rule, fixed **before** the run: the gate ships if `engine_wrong`
stays below the host LLM's own error rate on the same items *and*
`saved_calls` is material (say ≥ 20% of calls skipped). If `engine_wrong`
exceeds the host's error rate, the honest conclusion is "the gate hurts" and
the escalation contract becomes a *confidence report for humans*, not a
routing mechanism. That is a publishable result either way, which is the point
of running it early.

The provider: run it against a **local** model first (the 4B GGUF and the
2B arm are on this host) so the experiment costs nothing and is reproducible;
then, if the result is positive, against the frontier model the user actually
uses.

---

## 10. Phased plan

Numbered tasks continue the existing list (#180–#187). Each phase has an
acceptance gate; nothing advances on "it compiles".

| # | phase | deliverable | gate |
|---|---|---|---|
| 188 | **M — premise** | gated-LLM A/B harness + first result on a local model | result recorded in `docs/BENCHMARKS.md` with `engine_wrong` vs the host's error rate |
| 189 | **E — contract** | `DecisionPolicy.escalate_below`; `EscalationReport` in `core`; `Escalate` reachable; native/OpenAI/Anthropic/Jev projections; HTTP `escalation` block; MCP carries it | engine tests: absent ⇒ byte-identical responses; present ⇒ payload shape + bounded size + determinism |
| 190 | **C — config** | `opencodifier-config` crate; JSON schema; discovery + precedence; strict `config.*` errors | no-config ⇒ byte-identical to today (fixture suite); every existing flag is an override |
| 191 | **L — lanes** | lane table + `opencodifier lane list/show/use`; lane → resolved config expansion | each lane resolves to a runnable engine; missing artifact ⇒ loud `model.missing` |
| 192 | **D — detection** | `opencodifier init` (probe → propose → confirm → write) and `opencodifier doctor` (probe → report, no writes) | never downloads; never writes without confirmation; `--json` proposal is stable |
| 193 | **F — feedback** | `POST /v1/feedback` + `codify_feedback`; evidence JSONL; `ladder fit-boolean` round-trip | a recorded label changes a fitted threshold (closed loop, measured) |
| 194 | **MCP gate** | `codify_gate` (compact agent-facing "should I proceed?") | one call, small response, no JSON editor needed |
| 195 | **CI recipes** | `ci-test-selection`, `ci-queue-priority`, `ci-stale-process`, `ci-risk-gate` + the facts-file schema | each recipe runs end-to-end on a real repo and the trace explains its choice |
| 196 | **W — playground** | yes/no QA tool on the in-browser WASM engine; knowledge panel; trace; escalation demo | three states (decided / abstained / escalated) all reachable; WCAG clean; network counter still 0 |

**Ordering.** 188 first (it can invalidate the rest). 189 before 193/195/196
(they consume the escalation payload). 190 before 191/192 (lanes and detection
write the config). 191 and 192 can run in parallel once 190 lands.

**Not in scope, deliberately:** a provider SDK for any LLM vendor (the engine
posts a JSON document to a URL; it does not know what an LLM is); a plugin
system (the recipe/lane mechanism covers the same ground with less machinery);
auto-downloading weights; any cloud dependency in the default lane.

---

## 11. Risks

| risk | mitigation |
|---|---|
| the gate is confidently wrong | the §9 experiment measures exactly this (`engine_wrong`) *before* the build lands; the escalation threshold is per-deployment, so a bad threshold is a config fix, not a code fix |
| the payload bloats the host model's context | hard size cap + explicit truncation marker; the payload is measured, not assumed |
| the config layer becomes a dumping ground | one crate, one schema, strict validation, `config.*` error codes, and the byte-identity rule as its acceptance test |
| lane sprawl | lanes are a fixed, small table with declared tags; a lane that cannot be measured does not ship |
| the site's QA tool overclaims | the knowledge panel is visible and editable; the abstained state is a first-class, good outcome; no score is shown without a rule that produced it |
| scope creep into "a CI system" | the runtime stays pure; CI supplies facts, the engine decides |
