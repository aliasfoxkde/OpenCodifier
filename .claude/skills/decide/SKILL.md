---
name: decide
description: Turn unstructured state into a typed, machine-actionable decision (choice, boolean, or score) with calibrated confidence via the local OpenCodifier runtime. Use when facing a classification or routing choice that should be decided deterministically — CI failure triage, flaky-vs-broken adjudication, candidate selection, escalation gating — instead of guessing or hand-writing heuristics.
---

Decide with the **local-first decision runtime** on `http://127.0.0.1:8177`
(start it with `opencodifier serve` if `/v1/healthz` does not answer; the
equivalent MCP tools are `codify_decide` / `codify_explain` when the
`opencodifier` MCP server is registered).

## 1. Shape the request — structure is the fuel

The engine's cheapest rungs (exact rules, lexical match) decide from
**structured facts in `state`**, not prose. Extract the signal first —
job names, error classes, counts, timestamps, repeat behavior — and put
it in the state as compact declarative text. Raw log spew earns an
`abstain`, which is the runtime working correctly, not a failure.

Questions are typed. `choice` needs runtime candidates (never baked in
elsewhere); `boolean` and `score` take no candidates.

## 2. Call it — POST /v1/decide

```bash
curl -s http://127.0.0.1:8177/v1/decide -H 'content-type: application/json' -d '{
  "state": {"text": "CI failure triage: job integration-test failed on main. Signature: 3 of 47 tests failed with connection refused to test database; the same job passed on the previous commit; failure repeated on retry."},
  "questions": [{
    "type": "choice",
    "id": "triage_action",
    "text": "How should the pipeline triage this failed job?",
    "candidates": [
      {"id": "retry",  "description": "rerun the job; likely transient environment"},
      {"id": "bisect", "description": "hunt the introducing commit; real regression"},
      {"id": "block",  "description": "fail the pipeline and page a human"}
    ]
  }]
}'
```

Read the response in this order:

1. `outcome` — the confidence-gate verdict: `accept` / `verified` are
   decisive; `verify` means the engine wants the verifier rung;
   `abstain` / `escalate` / `no_valid_candidate` are refusals. **Every
   outcome arrives as HTTP 200** — route on the field, not the status.
2. `answers[].choice` + `answers[].distribution` — the selected
   candidate and the full probability distribution.
3. `confidence.calibrated_confidence` — gate-calibrated, never raw
   softmax. Compare against your action's risk threshold, not a
   universal one.
4. `trace.entries` — the deterministic execution trace (this is the
   *only* explanation surface; there is no chain-of-thought). Ask for
   it when the decision will gate something expensive.

The error envelope is always `{"error": {"code": "...", "message": "…"}}`
with stable namespaces: `schema.*` / `ir.*` mean *your payload is
wrong*; `engine.*` means *the runtime could not decide*.

## 3. Route on the outcome — abstention is an answer

Wire the branch before you call: `accept`/`verified` → act on
`answers[].choice`; `verify` → re-ask via `codify_verify` or treat as
non-decisive; `abstain`/`escalate` → your named fallback (human review,
conservative default, or richer state extraction and one retry). Never
treat an abstain as a zero-confidence guess for the top candidate.

If the decision matters repeatedly (a gate in a loop), tighten behavior
with `policy` (`min_confidence` / `verify_below` / `abstain_below`)
in the request rather than by re-interpreting confidence ad hoc —
policy is validated input, threshold-free-styling afterwards is drift.

## 4. Honesty rules

- State quality is decision quality: an `abstain` on messy input is a
  correct result. Report it as such; do not retry the same state and
  present the top candidate as decided.
- One call decides one question set over one state. Independent
  questions belong in one request; independent *states* belong in
  separate calls (or `/v1/batch`).
- The runtime decides; it never generates prose. A schema with
  free-form generation fields is refused (`schema.unsupported_generation_field`).
