# Integrating OpenCodifier — the adoption guide

One entry point for every way to wire the decision runtime into your
tooling: coding agents, MCP clients, HTTP services, CI, IDEs, Rust and
WASM hosts. First-party integrations (Amortyx, the agent harness)
appear here as **worked examples of the same public surfaces**, never
as prerequisites — see [D29](DECISIONS.md): no first-party stack is
privileged, and nothing here requires any sibling product.

**Honesty rule.** Every endpoint, flag, and tool name below is verified
against the shipped tree (audit 2026-10-02: `routes.rs`, `args.rs`,
`opencodifier-mcp/src/lib.rs`). Where a capability does *not* exist yet
it is labeled as planned and linked to its [Phase 19](PLAN.md) item —
we do not document aspirational surfaces.

---

## 1. The four surfaces at a glance

| Surface | Shape | Best for |
|---|---|---|
| HTTP (`opencodifier serve`) | JSON over loopback, native `/v1/*` | services, CI gates, routers, anything that speaks HTTP |
| MCP (`opencodifier mcp serve`) | six `codify_*` tools on stdio | coding agents and MCP clients (Claude Code, IDEs) |
| CLI (`opencodifier …`) | one-shot processes, `--format` selectable | scripts, cron, shell pipelines, format conversion |
| Library (`opencodifier-*` crates) | in-process Rust, strict layering | embedding the engine in your own binary |
| WASM (`opencodifier-wasm`) | the zero-ML decision runtime | browser and sandboxed hosts (D23) |

All four surfaces execute the **same engine** through the same
canonical IR: normalize → deterministic rules/filters → candidate
narrowing → fast semantic scoring → (optional) decision model →
confidence gate → accept / verify / abstain. A decision refused for low
confidence is a *successful* response everywhere — abstention is an
outcome, not an error.

---

## 2. HTTP — `opencodifier serve`

### 2.1 Start it

```bash
opencodifier serve                 # binds 127.0.0.1:8177 by default
opencodifier serve --addr 127.0.0.1:9200
```

The loopback bind is the default by policy (PLANNING §73); exposing a
non-loopback address requires passing one explicitly — the runtime
never opts itself into the network. `GET /v1/healthz` is the readiness
probe.

### 2.2 Routes (all shipped; verified against `opencodifier-http/src/routes.rs`)

| Route | Method | Purpose |
|---|---|---|
| `/v1/decide` | POST | one canonical request → typed decisions + trace |
| `/v1/batch` | POST | up to 16 requests; one item's failure is that item's `error` object — the transport stays `200` when the batch itself was well-formed |
| `/v1/validate` | POST | decode-and-validate only: preflight a payload (question kinds, candidate counts) before paying for execution |
| `/v1/graph/validate` | POST | validate a declarative graph document |
| `/v1/graph/run` | POST | execute a graph document (body carries `request` + graph) |
| `/v1/models` | GET | the model lanes actually active (e.g. `relational-v1\|builtin-lexical-v1` for the base binary) — reports what runs, never a catalog of aspirations |
| `/v1/capabilities` | GET | what this build can decide (kinds, rungs, features) |
| `/v1/healthz` | GET | liveness/readiness |

### 2.3 A minimal decision call

The native request names questions over a state; candidates are
runtime data, never baked into the model:

```bash
curl -s http://127.0.0.1:8177/v1/decide -H 'content-type: application/json' -d '{
  "state": "Checkout errors hit 12% eight minutes after a deploy. Rollback window closes in 20 minutes.",
  "questions": [{
    "kind": "choice",
    "id": "action",
    "text": "Which action should the on-call take?",
    "candidates": [
      {"id": "rollback", "description": "roll back to the last healthy build"},
      {"id": "hotfix",   "description": "patch forward on the broken build"},
      {"id": "wait",     "description": "watch dashboards and hold"}
    ]
  }]
}'
```

The response carries, per question: the chosen candidate, the **full
probability distribution**, calibrated confidence, and the outcome of
the confidence gate — `accept`, `verify`, or `abstain`. Treat
`abstain` as a first-class answer: your code should route on it (fall
back, ask a human, escalate) exactly as it routes on `accept`.

Error envelope, everywhere:

```json
{ "error": { "code": "schema.unknown_field", "message": "…" } }
```

Stable codes are namespaced `ir.*` (payload refused by the IR), `schema.*`
(wire decode), `engine.*` (execution), so a client can distinguish
"your payload is wrong" from "the runtime could not decide".

### 2.4 Wire formats over HTTP — `x-opencodifier-format` (D30)

The decode routes (`/v1/decide`, `/v1/batch`, `/v1/validate`) accept a
request header selecting the wire adapter:

```bash
# An existing OpenAI-shaped structured-output request, no rewriting:
curl -s http://127.0.0.1:8177/v1/decide \
     -H 'content-type: application/json' \
     -H 'x-opencodifier-format: openai' \
     -d @chat-shaped-request.json

# Anthropic-shaped messages, Jev/System-One shapes likewise:
-H 'x-opencodifier-format: anthropic'
-H 'x-opencodifier-format: jev'
```

- Values: `native` (default — also when the header is absent) |
  `openai` | `anthropic` | `jev`. Unknown values are `400`
  `schema.invalid_value`; the surface never guesses.
- **Symmetric**: the same adapter decodes the request and projects the
  response, so an OpenAI-shaped client gets an OpenAI-shaped answer.
  Batch items are each projected through the batch's adapter.
- **Errors keep one contract**: the native
  `{"error": {"code", "message"}}` envelope on every route regardless
  of format.
- **Format is transport, not identity**: cache keys are computed on the
  canonical request after decode, so the same semantic request in two
  formats shares one cache entry.
- Fidelity limits are each adapter's own documented posture — where a
  wire format cannot express an IR feature, it is refused or disclosed,
  never silently converted (PLANNING §7/§42). Graph routes
  (`/v1/graph/*`) stay native-only: a graph document has no external
  analog.

**`/v1/systemone` does not exist.** That path is the Jev/System-One
ecosystem's convention (now also used by Fastino's GLiDE demo); it is
not an OpenCodifier route and is not claimed to be. Jev-shaped callers
use the header above; whether a dedicated Jev-shaped route earns its
place is the open [Phase 19b](PLAN.md) question.

---

## 3. MCP — `opencodifier mcp serve`

Six decision tools over stdio (the only transport the local-first
posture allows):

| Tool | What it does |
|---|---|
| `codify_decide` | `POST /v1/decide` as a tool: native request in, native decisions + trace out |
| `codify_batch` | the `/v1/batch` semantics, per-item error envelopes included |
| `codify_graph` | execute a graph document |
| `codify_validate` | preflight a payload or graph without executing |
| `codify_verify` | re-verify a prior decision (the confidence-gated verifier path) |
| `codify_explain` | the deterministic execution trace — the only explanation surface; chain-of-thought is never exposed |

### 3.1 Register with an MCP client (Claude Code example)

```bash
claude mcp add opencodifier -- opencodifier mcp serve
```

Or in project scope (`.mcp.json`):

```json
{ "mcpServers": { "opencodifier": { "command": "opencodifier", "args": ["mcp", "serve"] } } }
```

Useful flags (same as the HTTP engine knobs):

```bash
opencodifier mcp serve \
  --graph my-pipeline.json \     # replace the built-in default pipeline
  --focus-budget 512 \           # decide long states on a focused view, escalate when weak
  --ladder ladders/profile.json  # per-node-kind gates + per-rung calibration (D25/D27)
```

Any MCP-capable host works the same way: a stdio command that speaks
MCP. There is no daemon to babysit — the engine answers in-process,
and the zero-ML stack answers in microseconds.

### 3.2 Using the tools from an agent

Decisions are machine-actionable: an agent asks "which of these
candidates should run?", gets back a typed decision with a
distribution and calibrated confidence, and branches on the gate
outcome. Because candidates are runtime data, the same tool call
serves routing, triage, handoff gating — any choice your harness can
enumerate. Ask `codify_explain` when you need the *why*: it returns
the deterministic execution trace (stages, scores, gate verdicts),
which is the audit story for CI logs as much as for agents.

---

## 4. CLI — scripts, cron, format conversion

```
opencodifier decide   # one request → canonical response (stdin or -i PATH)
opencodifier graph    # graph validate/run
opencodifier serve    # the HTTP surface above
opencodifier mcp      # the MCP surface above
opencodifier models   # model artifact operations
opencodifier recipe   # the built-in recipe fleet (install/list)
```

`decide` selects the wire format, which makes it a format bridge for
existing tooling:

```bash
cat openai-shaped-request.json | opencodifier decide --format openai
opencodifier decide --format anthropic -i message.json
opencodifier decide --format jev      -i systemone-request.json
```

`--format` accepts `native` (default) | `openai` | `anthropic` | `jev`.
Each adapter projects onto/normalizes from the canonical IR and
documents its fidelity limits — where a wire format cannot express an
IR feature, the adapter says so explicitly rather than pretending
(schema adapter contract, PLANNING §7/§42). Free-form generation
fields in any incoming schema are refused with
`unsupported_generation_field`: this runtime decides, it does not
write prose.

---

## 5. Rust library — embedding the engine

Consume the workspace crates directly (crates.io publication is
Phase 19d, user-gated; today the paths are a git dependency or a
vendored copy):

| Crate | Gives you | Depends on |
|---|---|---|
| `opencodifier-core` | the canonical IR: validating constructors, stable `ir.*` error codes, typed decisions | nothing |
| `opencodifier-engine` | `EngineHandle` — the full zero-ML stack (rules, narrowing, BM25, relational solver), exact-decision cache, ladder policy | core |
| `opencodifier-schema` | the wire adapters (native/openai/anthropic/jev) + the content-hashed Decision Registry (D20) | core |
| `opencodifier-runtime` | `InferenceBackend`/`EmbeddingBackend` traits; ONNX behind the `onnx` feature | core |
| `opencodifier-model` | the candidate-conditioned decision model (ONNX logits, all decision math in Rust f64) | core, runtime |

The layering is enforced, not advisory: `core` never depends on HTTP,
async runtimes, or ML; heavyweight dependencies are confined to one
crate each. Build the engine the way the binary does —
`EngineHandle::lexical()` is the default zero-ML stack — and you get
the same determinism guarantees the surfaces above are tested against
(double-replay bit-identical traces).

---

## 6. WASM — the decision runtime in the browser

`opencodifier-wasm` compiles the **zero-ML decision runtime** (D23):
rules, narrowing, lexical scoring, cache — no model rungs. Time and
threads are seam-gated (`clock.rs`, `THREADS_AVAILABLE`), so the same
API works single-threaded in a browser. Build and smoke it with
`just check-wasm` (wasm32 check + Node smoke over `pkg/`). Typical
use: pre-screening decisions client-side, escalating to a full engine
over HTTP only when confidence is thin.

---

## 7. CI and harnesses — decisions as gates

The deterministic-first pattern in a pipeline: run the decision
runtime next to your build and let a typed decision — not a regex —
gate the step. Because `abstain` is a first-class outcome, a gate can
be configured to *fail open* (abstain → human review) or *fail
closed* (abstain → block) purely through `DecisionPolicy`
(`min_confidence` / `verify_below` / `abstain_below`), per node and
per decision kind via `LadderPolicy` (D25/D27) — no workflow DSL
required.

- **Decide via HTTP** in any CI: start `opencodifier serve` as a
  service, `curl /v1/decide`, branch on the outcome. Everything stays
  local; no vendor call, no keys.
- **Batch triage** with `/v1/batch` (≤16 items, per-item errors, one
  wake-up).
- **Audit** with the trace: every accepted decision carries its
  execution trace and the artifact versions (graph / model /
  calibration / policy) folded into its cache key, so an artifact
  update invalidates stale cached decisions by construction.
- OpenCodifier's own pipeline of record is GitForge (`.gitforge.yml`
  mirrors `just ci` line-for-line); a decision-gated job is just
  another step in that shape on your side.

---

## 8. First-party integrations — worked examples, not requirements

These sibling products integrate over the exact surfaces documented
above. They are listed because the stack is ours, not because they are
special: **D29 forbids any capability that only a first-party consumer
can reach.**

### 8.1 Amortyx (LLM router) — designed, promotion-gated

Doc of record: [`INTEGRATION_AMORTYX.md`](INTEGRATION_AMORTYX.md).
Amortyx asks OpenCodifier semantic questions (`task_complexity`,
`model_selection` among eligible candidates, …) over loopback
`/v1/decide`, and treats `abstain` — or a dead runtime — as "run the
existing heuristic, byte-for-byte". Decision ids promote from shadow
to advisory to active only on measured would-have-positive evidence.
The boundary is doctrinal (§60): OpenCodifier never sees provider keys
or prices; Amortyx never guesses semantics.

### 8.2 The agent harness (hooks + MCP)

The same pattern as §3: a hooks binary or an agent session registers
`opencodifier mcp serve` and asks typed questions at decision points
(model choice, handoff gating, severity) instead of pattern-matching
prose. Deterministic-first: the cheap rungs answer inline; the
confidence gate decides whether anything more expensive may run at
all.

---

## 9. Planned surfaces (tracked, not yet shipped)

| Item | Phase | State |
|---|---|---|
| HTTP wire-format selection (`x-opencodifier-format`) | 19a | **shipped** (D30) |
| Jev-shaped HTTP route (the `systemone` convention) | 19b | open question, adapter rules apply |
| Copy-paste quickstart set exercised by tests | 19c | planned |
| crates.io / Homebrew / winget publication | 19d | user-gated (§58) |
| Amortyx implementation (shadow → advisory → active) | §60 | designed; promotion-gated |

---

## 10. Versioning and stability expectations

- The IR and its `ir.*` error codes are the stable core; wire adapters
  project onto it and document fidelity limits rather than faking
  support.
- Decision definitions are content-hashed artifacts (D20): a
  definition's identity is the SHA-256 of its canonical serialization
  — `id`/`version` are labels.
- Cache keys fold graph/model/calibration/policy versions, so an
  artifact update cannot serve stale decisions.
- Public enums are `#[non_exhaustive]`; match arms should carry a
  fallback.
