# Token-reduction mechanisms for the router stack — design doc (2026-10-26)

**Task of record:** PLAN task #21. **Governing directive:** classify requests
*before* they hit the Amortyx router so traffic routes to the right model and
trivial/repeat requests never cost a provider call at all — for Amortyx, the
Harness (backend-fixed), and GitForge.

**Method of record:** OpenCodifier decides each classification; `abstain`
degrades byte-identically to today's heuristic (INTEGRATION_AMORTYX.md §1/§4).
Every mechanism below splits its savings into **billable tokens** vs
**latency**, and names its validation arm.

---

## 0. What is already live (receipt, 2026-10-26)

The phase-2 consult is **live in shadow**: `routing.opencodifier` in
`/var/tmp/amortyx-router-dev.toml` (see INTEGRATION_AMORTYX.md §14). Measured
on first burst through the router:

- consult overhead **p50 ~3 ms, max 9 ms** (hot-path budget §5 is single-digit
  ms — met);
- 22/22 `complexity_agreed: Some(false)` on trivial probes: engine says
  `medium`, builtin says `complex` (the builtin's 3-chars/token estimate
  inflates). This divergence is exactly what shadow mode collects before any
  enforce promotion.

## 1. Mechanisms, ranked

### M1. Pre-router classification (SHIPPED, shadow)
One batched `/v1/decide` per request: complexity score + context/tool/freshness
booleans (+ provider ranking at `opencodifier_ordered_chain`). Savings: only
via enforce-mode routing — small model when `complexity=simple` wins. Tokens:
**indirect but compounding** (a simple request on a small/cheap model instead
of the default chain). Latency: +1–4 ms. Validation: shadow agreement
counters → per-decision promote (§7). **Next step:** accumulate ≥1 week of
shadow divergence data, then enforce `task_complexity` first.

### M2. Short-circuit trivial/repeat requests before ANY provider call (the real token killer)
Today Amortyx has exact-cache + semantic-cache + dedup, but nothing decides
"does this request need an LLM at all?" before the chain build. OC's boolean
ladder with `codify_gate` semantics (measured: engine_wrong 0.000, saved_calls
0.5036 on the gate suite) is exactly this. Wire: a `HookResult::Cached`-style
return next to the hook short-circuit at `main.rs ~2160`, gated on an OC
boolean answer with high confidence; abstain ⇒ normal path. Tokens: **100% of
the request** for every gate-pass (no prompt, no completion). Latency: ~2 ms
replaces a multi-second call. Risk: false "no-LLM-needed" — mitigated by the
abstain posture and risk policy (`DecisionPolicy.risk`). *Build item: new
Amortyx hook; no OC change.*

### M3. Consult-verdict cache (decision-level)
OC's exact-decision cache already absorbs byte-identical repeats, but
semantically-equal requests with different text re-decide. A router-side
consult cache (request-hash → verdict, short TTL) skips the 1–4 ms consult on
bursts. Small latency win only — tokens unaffected. Low priority unless M2
ships (then the M2 verdict is the thing to cache).

### M4. GitForge AI review: per-file gating + tiering (BIGGEST billable-token sink in the fleet)
`gitforge-ai/src/anthropic.rs:139` inlines the **entire diff into one Sonnet
prompt — no truncation, no filtering, no budget ledger**. Replace the
`contains("test")` complexity heuristic (`gitforge-review/src/lib.rs:347`)
with per-file OC decisions: "does this file need AI review, at which tier?"
(haiku / local / opus / skip). Abstain ⇒ current behavior. Tokens: potentially
50–80% of review spend. Also: CI minutes — `detect-changes.sh` regex →
decision (wrongly-run clippy+test+coverage+fuzz chain ≈ 5 h of CI), and
`step_failed_by_infrastructure` → flaky-vs-real-vs-infra classification
(prevents a full agentic fix-it loop per false red X).

### M5. Harness job dispatch: task→profile inference
`harness_job_manager.py` requires the exact profile string (38 profiles); an
agent burns a context turn reading the table before every dispatch. Add an OC
decision (task text → profile + resource class) as a suggestion tool
(`job_suggest`), abstain ⇒ "read the table". Tokens: one agent turn per
dispatch; also fewer misrouted jobs (each misroute = a whole agentic loop).
*Build item: one MCP tool in backend-fixed/scripts; no manager change.*

### M6. Semantic cache in Amortyx (exists, evidence needed)
`SemanticCacheHook` ships with `Suggest` mode. The OC shadow data (what
fraction of traffic is semantically repeat?) sizes the enforce decision. No
new code; run Suggest mode during the M1 shadow window and read the counters.

### M7. Prompt compression / headroom
Amortyx `ccr.rs` (content handles) + Headroom already compress bulky content
zero-token-style. Complementary to everything above; not OC's lane. Keep as-is.

### M8. No-go notes
- zero-token-bridge's askonce fan-out (all providers concurrently) is the
  *opposite* of token-budget discipline — do not replicate.
- Prompt-cache-aware routing: current chain is latency_first with small
  providers; caching benefit requires provider affinity, which Amortyx
  affinity mode already offers (disabled enforce). Revisit only after M1
  enforce produces measurable provider stickiness.

## 2. Validation protocol (per quality.md dual-arm rule)

For every mechanism that changes dispatch (M1 enforce, M2, M4, M5):
same corpus direct-to-backend vs through-the-mechanism; provider-reported
usage as the only token truth; abstention rate reported as a first-class
metric; findings stored in docs/ before the thread closes. The M1 shadow
window IS the first dual-arm (dispatch unchanged = arm parity by construction,
with the divergence counters as the recorded evidence).

## 3. Build queue (derived)

| # | Item | Repo | Type |
|---|---|---|---|
| 1 | M1: accumulate shadow evidence → enforce `task_complexity` | Amortyx (config) | after 1 week |
| 2 | M2: no-LLM-needed short-circuit hook | Amortyx (new hook) | code |
| 3 | M4: per-file review gating + detect-changes decision | GitForge | code |
| 4 | M5: `job_suggest` MCP tool | backend-fixed | code |
| 5 | M6: read SemanticCache Suggest counters during M1 window | Amortyx (config) | measurement |
