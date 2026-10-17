# Skill: tool-gating

**Decision area:** whether a *specific* tool call should proceed — the
pre-execution check (PLANNING.md §75's gate half).

This is a risk decision, not a classification. Shape the policy to the
blast radius of the tool:

- Reversible/read-only tools: default policy (`min_confidence` 0.80).
- State-changing tools: raise `min_confidence` (0.90+) and set
  `risk: "high"` in the policy block.
- Destructive tools: do not ask a model class to gate these at all —
  gate by deterministic rule (allowlist paths, explicit confirmation),
  and use OpenCodifier only for the narrow "does this match the
  allowed pattern" boolean.

**Call shape:** `state` = the proposed call (command, path, arguments)
plus the surrounding task context; one `boolean` ("Should this call
proceed?"); high-bar policy.

**Reading the answer:** on `accept`, proceed and record the decision id
in your audit trail (the `trace` is your why). On `abstain` at a high
bar, **fail closed** — an ungated destructive call is never the
recovery path. Ask a human, not a lower threshold.

**Anti-patterns:** gating with a low bar to reduce friction (then it
is decoration, not gating); treating `abstain` as "yes by default";
letting the tool's own output text adjust the gate — state never
modifies policy.
