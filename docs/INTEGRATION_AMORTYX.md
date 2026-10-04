# Amortyx Integration Design — Router Decisions as IR

Status: design + first V1 integration shipped (2026-10-03, §12). The
design sections were written at task #40 against PLANNING.md §60–§62;
every code anchor cites the tree as it existed on 2026-09-28 (the
Amortyx-side routing-plane anchors were re-verified at Amortyx HEAD
`520bd60a` — see the correction in §2). Amortyx lives at
`/nas/Temp/repos/Amortyx`.

## 1. The boundary (non-negotiable)

PLANNING.md §60 states it and both trees independently restate it:

> OpenCodifier makes semantic decisions. Amortyx makes
> economic/operational decisions. Neither should absorb the other's
> responsibilities.

Concretely:

- **OpenCodifier answers**: what kind of request is this, how complex is
  it, what does it require (context, tools, freshness), and which
  eligible candidate fits — each as a typed decision with calibrated
  confidence, or an abstention.
- **Amortyx answers**: price, latency targets, availability, quota,
  user policy, privacy lane, and which provider adapter actually
  dispatches.
- OpenCodifier never prices, never sees provider keys, never owns the
  fallback chain. Amortyx never guesses semantics — when it needs a
  semantic judgment, it asks, and handles `abstain` by falling back to
  its current heuristic (see §4).

## 2. What Amortyx's router decides today (the replacement target)

Amortyx's routing decisions are currently inline heuristics in
`amortyx-router`:

| Decision | Today | Anchor |
|---|---|---|
| Task complexity | character counts — `> 500` Complex, `> 200` Medium — plus a "complex keyword" match | `assess_complexity`, `amortyx-router/src/handlers/main.rs` (`COMPLEXITY_COMPLEX_CHARS = 500`, `COMPLEXITY_MEDIUM_CHARS = 200`), `ComplexityLevel` in `handlers/types.rs` |
| Provider selection | model-prefix routing rules in config order, default provider, "try openai first, fall back to anthropic" when nothing decides | `select_provider`, `handlers/main.rs` |
| Escalation | Complex requests bypass rules and take the fallback chain head | `select_provider` (auto-escalation branch) |
| Attribution/audit | the routing trail records `Selection { provider, model, score, policy_score }` | `handlers/routing_trail/mod.rs` |
| Measurement arm | session-sticky SHA-256 holdout: 10% control (unoptimized) vs treatment | `holdout.rs`, `DEFAULT_CONTROL_FRACTION` |

These heuristics work and are cheap — but they are untyped, uncalibrated,
and silent: a wrong complexity guess looks identical to a right one.
Exactly the gap OpenCodifier's IR exists to close: same decisions, typed,
with calibrated confidence, an execution trace, and an explicit
abstention outcome.

> **Correction (HEAD `520bd60a`, verified 2026-10-03).** The anchors in
> the table above are stale: `assess_complexity` and `select_provider`
> are **test-only** at HEAD. The live routing chain is
> `server.rs` → `chat_completions_with_headers` → `chat_completions_inner`
> → `chat_completions_route` (`amortyx-router/src/handlers/main.rs`),
> with the capability plane at `TaskProfile::classify` →
> `registry.select_for_task` → `policy_engine.decide` and the adapter
> bound inside `chat_completions_route`. `doc/INTEGRATION_AMORTYX.md`
> lines citing the two functions as the live path describe tests, not
> production. The replacement target is the *live chain* above; the
> trail attribution (`Selection` in `handlers/routing_trail/mod.rs`)
> and holdout rows are unchanged.

## 3. The decisions as IR (§63 registry shape)

Each heuristic above becomes a named, versioned decision definition
(PLANNING.md §63). The candidate list for model selection is dynamic —
Amortyx computes eligibility (price, quota, privacy lane) and sends the
surviving candidate ids; OpenCodifier ranks only among what Amortyx
already considers legal.

| Decision id | Type | Question | Candidates / scale |
|---|---|---|---|
| `amortyx.task_complexity` | Score | "How complex is this request?" | simple / medium / complex (full distribution + EV retained) |
| `amortyx.context_requirement` | Boolean | "Does this request need long-context handling?" | — |
| `amortyx.tool_requirement` | Boolean | "Does this request require tool use?" | — |
| `amortyx.freshness_requirement` | Boolean | "Does this request require fresh data?" | — |
| `amortyx.model_selection` | Choice | "Which model should process the request?" | dynamic: the eligible model ids Amortyx passes in |

Policy block per decision (§63): `min_confidence 0.80`,
`verify_below 0.65`, `abstain_below 0.50` as the starting defaults — the
same gate values the engine's `DecisionPolicy` carries today.

The state each decision reads is derived from the `NormalizedRequest`
Amortyx already builds: message texts, lengths, tool definitions,
attached context — never the raw provider payload with keys. Input is
hostile by OpenCodifier's standing rule: request text can never modify
policy, thresholds, graphs, or paths, and the §45 focus extraction keeps
long request bodies from dominating the decision input.

## 4. What each side does with each outcome

The contract that makes abstention safe:

- **accept (confidence ≥ gate)** — Amortyx acts on the decision and
  records it in the routing trail. A `Score` outcome carries its full
  distribution, so "complex 0.62 / medium 0.30" can drive soft behavior
  (e.g. a larger completion budget) without a second decision.
- **abstain** — Amortyx falls back to the heuristic that decision
  replaces (`assess_complexity`'s char counts; prefix rules). The ladder
  never blocks a request: an OC outage or an abstention degrades to
  today's behavior, byte-for-byte.
- **escalate to verifier** — the D16 frontier tier (MiMo-9B, 14.3 s) is
  a *bulk/offline* instrument only. It never sits in the request path;
  it re-decides held routing decisions overnight and feeds §62 training.

## 5. Ladder and latency mapping (who may answer inline)

OpenCodifier's cost ladder is also the latency budget, and Amortyx's
lanes differ in what they can afford:

| Lane | Budget | Allowed rungs |
|---|---|---|
| Hot path (per request, interactive) | single-digit ms | exact rule → cache → metadata filter → lexical. The engine arm measured 1.3 ms p50 on the 120-item suite — inline-safe. |
| Warm path (session start, retry storms, batch boundaries) | ≤ 1 s | embedding rung; D16 fast tier (Qwen3.5-0.8B, 613 ms) |
| Pre-decision / shadow lane | seconds | D16 reference tier (Qwen3.5-4B, 4.9 s) — runs beside traffic, never in it |
| Verifier / bulk re-decision | minutes | frontier tier, offline |

Two properties make the hot path real rather than aspirational:

1. **Amortyx's own traffic shape is cache-shaped.** Retry storms and
   concurrent duplicates are Amortyx's measured dominant savings class
   (its VALIDATION-REPORT: blended 27.8% of requests, 4,873 tok, 76×
   replay) — and identical `(state, question)` pairs are exact-decision
   cache hits in OpenCodifier, microseconds. The requests Amortyx sees
   most often are the ones OpenCodifier answers cheapest.
2. **Escalation is confidence-gated** (§73): the expensive rung runs only
   when the cheap rungs abstain, and the reverse escalation added in
   Phase 16 re-decides on the full request view when a focused view
   looks weak — so a long hostile body costs extraction (~2.5 ms), not
   prefill.

Cache keys already fold graph/model/calibration/policy versions
(§64, D6): an Amortyx graph or calibration artifact update invalidates
cached routing decisions by construction.

## 6. Wire integration

Process boundary, loopback only:

```text
amortyx-router (axum, tokio)
   │  POST http://127.0.0.1:<port>/v1/decide   (native schema)
   ▼
opencodifier serve --focus-budget 512
```

- **Transport**: the native `/v1/decide` route
  (`opencodifier-http/src/routes.rs`), one call per decision batch —
  the endpoint already accepts multi-question requests, so the five
  decisions of §3 that share a state travel together.
- **Bind**: loopback by default per the standing serve rule; Amortyx and
  OpenCodifier on the same host, or an explicitly-flagged socket where
  they are not. No provider credentials ever transit this path.
- **Failure posture**: connection refused / timeout / 5xx ⇒ treat as
  abstention on every decision in the batch (§4). Amortyx's availability
  must never depend on the decision subsystem's availability.
- **Alternative surface**: `opencodifier-mcp` exists for agent-hosted
  integrations, but a router calling out per request wants the HTTP
  path; MCP's stdio shape fits tool-style use, not an inline lane.
- **Attribution**: outcomes land in the existing routing trail
  (`Selection` gains the decision ids + confidences), so every routed
  request keeps its why.

## 7. Shadow mode (§61) — rides the holdout

Amortyx already owns the instrument: `holdout.rs` allocates sessions
sticky-armed by SHA-256 and the usage ledger carries the arm tag. The
OpenCodifier shadow reuses exactly that allocation instead of inventing
a parallel one:

```text
request ──► existing Amortyx route  ──► actual provider/model
   │
   └──► OC shadow route (same ledger row, arm-tagged):
          decisions (§3 outcomes + confidences)  [prediction]
          actual selected model, latency, cost, success, quality [outcome]
```

Derived per decision id, control vs treatment:

- would OC have chosen the (same-class or better) model?
- would OC have saved cost / reduced latency?
- would verification have been necessary (how often did confidence land
  in the verify band)?
- abstention rate — an OC that abstains constantly is a failed
  integration regardless of its accuracy on the decisions it makes.

Promotion gate (§61): shadow evidence first; only a sustained
would-have-positive delta promotes a decision id from shadow to
advisory (trail records it, humans consult it), and only after that to
active routing. Each decision id promotes independently —
`task_complexity` may earn the hot path while `model_selection` stays
advisory.

## 8. The VIVERE path

VIVERE is a separate experimental project (Amortyx HANDOFF boundary:
"must not inherit Amortyx production claims"), and the same wall applies
here. Its role in this design is narrow and read-only:

- VIVERE corpus runs consume **shadow-ledger exports**, never live
  traffic. The corpus run is quota-shaped (flat subscription converts
  token savings into work-per-window), which is the same shape as
  OpenCodifier's "tokens avoided" metric (§59) — the ledger's savings
  rows and OC's decision rows join on request id.
- Model-path experiments (e.g. VIVERE-side decision models evaluated on
  the corpus) run through the same benchmark harness contract as the
  Phase 13 arms: pinned artifacts, SHA-256 manifest, replayed
  determinism, results as data. Nothing VIVERE-side is a routing
  authority; it is another arm under the same measurement discipline.
- Any production-sounding number out of the VIVERE path is labeled
  experimental in both trees. The promotion gate of §7 does not accept
  VIVERE evidence; only live shadow evidence.

## 9. Training from Amortyx (§62)

Optional, explicit, and versioned — never automatic:

```text
shadow ledger ─► privacy filter ─► dedup ─► label extraction
      ─► dataset (versioned, checksummed) ─► OpenCodifier training
      ─► Phase 13 benchmark harness ─► candidate deployment (D16 re-tier)
```

- Labels come from outcomes (actual model, actual success, actual
  latency), not from Amortyx's heuristics agreeing with themselves.
- Every dataset records source ledger window, privacy-filter version,
  and dedup parameters; training runs are reproducible from those
  pins, and the resulting candidate is benchmarked by the same harness
  that produced D16 before any tier claims.
- The candidate-deployment step re-uses the D16 tier mechanics: a new
  model enters a tier only by measured win, never by shipment.

## 10. Non-goals

- No OpenCodifier-in-Amortyx-process linkage: separate processes, loopback
  HTTP, independent lifecycles (§6).
- No provider keys, pricing tables, or quota state in OpenCodifier.
- No decision graph that encodes economics: candidate *eligibility* is
  Amortyx's input; the decision ranks among eligible candidates only.
- No automatic training from production (§9).
- No shared holdout semantics drift: the holdout allocator remains
  Amortyx's; OpenCodifier consumes arm tags, it never re-assigns arms.

## 11. Risks

| Risk | Mitigation |
|---|---|
| Added hot-path latency | §5 lane table: deterministic rungs only inline; escalation is off-path; measured 1.3 ms engine p50 as the budget anchor |
| OC unavailable | every failure mode maps to abstain ⇒ today's heuristics (§4); Amortyx availability unchanged |
| Heuristic-vs-OC disagreement noise | shadow lane measures would-have deltas before any promotion (§7); no behavior change until evidence |
| Long hostile request bodies | §45 focus extraction + hostile-input rule; §16 A/B: answer-identical on 120/120 padded items |
| Drift between the trees' expectations | the §3 registry entries are versioned artifacts; cache keys fold their versions, so a stale consumer gets invalidated decisions, not silently stale ones |

## 12. Shipped V1 (2026-10-03) — provider registration over the chat surface

The first integration running on this host is the **zero-Amortyx-code
path**: OpenCodifier registers in Amortyx's provider config, and the
semantic decision capability arrives as just another
OpenAI-compatible provider.

- **Runtime**: `opencodifier serve --focus-budget 512` as a systemd
  `--user` unit (`opencodifier.service`), loopback `127.0.0.1:8177`,
  engine identity `relational-v1|builtin-lexical-v1|focused-v1@512`.
- **Surface**: `POST /v1/chat/completions`
  ([INTEGRATIONS.md §2.5](INTEGRATIONS.md)) — the OpenAI-shaped
  decision projection. Amortyx's router speaks
  `POST {base_url}/chat/completions` to every `openai_compatible`
  provider, so the config-only registration needs no router changes:

  ```yaml
  # <amortyx config>.yaml → providers:
  - name: opencodifier
    provider_type: openai_compatible
    api_key_env: AMORTYX_OPENCODIFIER_API_KEY   # any non-empty value; OC does not check it
    base_url: "http://127.0.0.1:8177/v1"
    models:
      - name: relational-v1|builtin-lexical-v1|focused-v1@512   # a served id, echoed verbatim; not an LLM
  ```

  The `models[].name` must be a **served model id** (the engine
  identity string, as listed by `GET :8177/v1/models`). A placeholder
  name registers fine but every request 404s
  (`RouterError::NotFound("model not configured: …")`) — the name is
  matched exactly by `adapters_for_model`, not prefix-matched.

- **Contract on the wire**: strict `json_schema` in, structured-output
  JSON in `choices[0].message.content` out, outcome + calibrated
  confidence in the `opencodifier` extension object, abstention as
  `200` with `outcome: "abstain"`. A request without a strict schema is
  refused `schema.unsupported_generation_field` — the runtime decides,
  it does not generate prose, and a chat-shaped refusal is still a
  typed error Amortyx can route on. The envelope carries an honest
  zeroed `usage` object (`prompt_tokens`/`completion_tokens`/
  `total_tokens` = 0): the decision ran on the deterministic engine, no
  model tokens were consumed, and OpenAI-shaped clients (including
  Amortyx's response decoder) require the field.
- **Router version requirement (learned in E2E)**: released router
  builds drop the client's `response_format` at three ingress layers
  (`ChatCompletionsRequest` deserialization, `into_normalized`
  metadata hoisting, `parse_inbound`), so the strict schema never
  reaches OpenCodifier and every request is refused
  `schema.unsupported_generation_field` (surfaced by the router as a
  502 relay). The request-side passthrough that fixes this is Amortyx
  commit `38daffa9` (branch `feat/forward-client-response-format`,
  based on the deployed exact-cache line): `response_format` rides the
  metadata passthrough into `request_overrides` and is re-flattened
  onto the provider wire; packed/batch requests are excluded so their
  own synthesized `response_format` demux contract is untouched.
  Routers without this commit cannot front OpenCodifier's chat
  surface.
- **Boundary preserved**: this registration does NOT make Amortyx route
  user traffic to OpenCodifier as if it were an LLM. The provider is
  for decision-shaped requests (structured output, candidate lists in
  the schema); the economic routing plane stays Amortyx's (§1), and the
  routing-plane side-car of §6 remains the phase-2 path — native
  `/v1/decide`, one batched call per request's five decisions, wired at
  `chat_completions_route` (pre-`main.rs:1963`), with `abstain` ⇒
  byte-identical heuristic fallback (§4).
- **E2E validation**: complete (2026-10-04) — recorded in §13 below.

## 13. E2E validation record (2026-10-04) — V1 chat surface through the deployed router

Dual-arm against the live host: `POST :8177/v1/chat/completions`
(direct) vs the same body through the Amortyx router
`POST :8787/v1/chat/completions` (bearer auth), deterministic engine
served by `opencodifier.service`.

**Arms and results.**

- *Parity (fresh unique bodies, 5/5)*: content identical both arms
  (`{"policy":"fifo"}` for the eviction-policy probe; `{"backend":…}`
  for the storage probe). Latency: direct p50 ≈ 9 ms, through-router
  p50 ≈ 68 ms (min 26 / max 98) — the overhead is the router's
  hook chain (dedup, exact-cache keying, breaker consult) plus one
  HTTP hop; both arms deterministic.
- *Burst*: 12 concurrent identical requests through the router →
  12/12 HTTP 200, one distinct content (request coalescing +
  exact-match cache behave as designed under concurrency).
- *Abstention pass-through*: the no-metadata color probe returns
  `200` through the router with the honest uniform-distribution
  content and `outcome: "abstain"` semantics preserved on the direct
  arm; through-router responses strip the `opencodifier` extension
  object (finding F-1).
- *Circuit breaker, full lifecycle (unique bodies so the exact-match
  cache never masks it)*: with `failure_threshold: 5`,
  `recovery_timeout_ms: 30000` (default), `success_threshold: 3`:
  OC stopped → 5 real attempts fail (`502`, connection refused,
  ~30–60 ms each) → circuit opens → subsequent requests fast-fail
  `502 "opencodifier: circuit open"` → OC restarted (breaker does NOT
  reset on provider restart) → after the recovery timeout the next
  request is admitted half-open → 3 consecutive successes close the
  circuit → normal serving. Journal-correlated
  (`circuit_open`, `transitioned to half-open`, `closed (half-open
  success)`).
- *Exact-match cache vs outage*: identical bodies are served `200`
  from the router's cache (`x-amortyx-provider: cache`) while
  OpenCodifier is down. Correct cache semantics, operationally
  surprising during outage drills — breaker tests must use unique
  bodies (finding F-3).

**Findings (open, Amortyx-side unless noted).**

- **F-1 — response-side extension stripped.** The router's OpenAI
  adapter decodes the provider envelope into `NormalizedResponse` and
  re-serializes, dropping the `opencodifier` extension
  (`outcome`, `calibrated_confidence`). Direct responses carry it.
  Scoped fix: an extension bag on `NormalizedResponse` through
  adapter decode → normalized→chat projection → cache semantics
  (4 crates). Until then, clients needing the typed outcome must call
  OpenCodifier direct or read the native `/v1/decide` surface.
- **F-2 — intermittent silent empty-404 windows (environmental,
  unresolved).** During high-frequency service restart churn, some
  loopback requests to *both* `:8177` and `:8787` received empty
  `404`s that are absent from the router's journald while adjacent
  curl requests in the same minute are logged — i.e. proven **not**
  the router and not attributable to either service on current
  evidence (no nft/iptables redirects; single listener per port;
  self-healed; reproduced only while stop/start churn was in
  flight). Monitoring protocol if it recurs: capture both services'
  journals for the failing second, `ss -i` socket state, and one
  request from a second client host simultaneously.
- **F-3 — outage-drill cache masking.** Identical-body requests
  short-circuit at the exact-match cache and return `200` while the
  provider is down; breaker/failover drills must vary bodies
  per-request. Recorded here as an operational property, not a
  defect.
- **F-4 — doc drift fixed in this change (OpenCodifier-side,
  closed).** §12's example registered `opencodifier-decision`, which
  no served engine id matches; corrected to the real id and the
  exact-match behavior documented.

Fix-forward note: F-1 is the only code gap on the integration path;
the request-side half (F: `response_format` ingress) is already
landed as `38daffa9`.
