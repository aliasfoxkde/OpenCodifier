# Fusion study — confidence-gated escalation ladder (post-hoc)

Simulated offline over measured per-item arm rows; every probability,
prediction and latency is a harness measurement, nothing re-inferred.
This bounds a live ladder before one is wired into the policy.

Joined 120 suite items across `engine`, `embed`, `llm`.

## Single arms (reference, joined set)

- **engine**: acc 0.683, ECE 0.094
- **embed**: acc 0.575, ECE 0.330
- **llm**: acc 0.725, ECE 0.062

Oracle (any rung correct): **0.975** — no routing policy can exceed this on the joined set.

## Cascade results (thresholds swept)

Accept a rung when its winner probability clears the threshold;
final rung always answers. Cost = rungs actually incurred.

| engine t | fallback t | acc | ECE | mean ms | routing |
|---|---|---|---|---|---|
| 0.35 | 0.30 | 0.800 | 0.105 | 409.9 | engine 71%, llm 29% |
| 0.50 | 0.50 | 0.842 | 0.114 | 713.8 | engine 57%, llm 42% |
| 0.55 | 0.30 | 0.867 | 0.127 | 752.7 | embed 1%, engine 54%, llm 45% |
| 0.60 | 0.50 | 0.833 | 0.108 | 928.0 | engine 46%, llm 54% |

Best accuracy: 0.867 at engine t=0.55, fallback t=0.30 (embed 1%, engine 54%, llm 45%; mean 752.7 ms).
Best under 497 ms mean: 0.800 at engine t=0.35, fallback t=0.30 (engine 71%, llm 29%; mean 409.9 ms).

Best operating point by class: lexical_semantic 0.72, metadata_match 1.00, relational_compositional 0.88.

## Live-wiring design (task #59 step 2 — drafted 2026-09-30; wiring LANDED same day, see "Wired" below; margin study DONE 2026-09-30, d15 refit LANDED same day — exact-fit 2B artifact in-tree, `CALIBRATION.md`)

The thresholds above are **post-hoc sweeps over raw winner probabilities**.
Wiring them into the engine must go through the seams that already exist,
without re-opening the IR:

1. **Per-rung policy, not one policy.** `DecisionPolicy`
   (`opencodifier-core/src/policy.rs`) is the gate type; the ladder needs
   one per rung because the rungs' confidence semantics differ (exact
   proofs are p = 1.0 by construction; the embedding rung has no
   calibrated probability at all — CALIBRATION finding 3; the model rung
   carries a fitted temperature). Concretely: engine configuration gains
   a `LadderPolicy` — optional overrides keyed per node id, then per
   node kind, resolved at the existing confidence gate (the rung that
   decides is the node that decides). No IR change, no wire change —
   the ladder is engine-internal composition of an existing, validated
   type.

2. **Per-rung profiles implied by this study + CALIBRATION:**
   - `rule/relational` (proofs): accept at p ≥ 1.0 — the proofs gate
     themselves; abstain gate irrelevant (never fires). Matches
     relational-v1's no-ship decision (identity calibration).
   - `embedding`: **never accept on probability** — gate by rank/margin
     (the rank/margin study, CALIBRATION finding 3; measured 2026-09-30,
     `embed-margin-study.md`). `min_margin` is the §19 hook for this;
     the probability gates stay at accept-never (`min_confidence: 1.0`)
     and the margin gate takes the measured operating point
     `min_margin ≈ 0.0183` — coverage 0.208 at accepted accuracy 0.880,
     escalating everything wider-than-flat to the model rung.
   - `model` (calibrated): thresholds re-derived on **calibrated**
     confidence. The study's t = 0.55/0.30 are raw-prob numbers (arm ECE
     0.094); after the shipped temperature artifacts the same operating
     point lives at different thresholds — the margin study re-sweeps on
     calibrated + margin-gated rows before any number ships.
   - `verifier/external`: unchanged (always runs below the model rung's
     `verify_below`).

3. **F24 (gates inherit calibration) is the release gate for the
   wiring**: per-domain (suite-class) calibration precedes enabling the
   ladder by default. Until then the ladder ships **opt-in** — default
   engine behavior stays single-rung, exactly like the disabled §19
   uncertainty gates, so wire fixtures and cache keys do not shift.

4. **ONNX substitution note:** the ladder's model rung is defined by the
   `InferenceBackend` trait, not by llama.cpp; the q4/int8 ONNX probe
   (REPORT F-row, task #56) showed the 2B model runs through ORT with
   the same suite accuracy, so a ladder deployment without the llama.cpp
   server is config, not code.

## Wiring seam, verified in code (2026-09-30)

- Policy enters the executor **per request**:
  `Executor::resolve_threshold` (`crates/opencodifier-engine/src/
  executor.rs:924`) reads `self.request.policy()` and applies
  `ConfidenceReport::outcome_for` once per decided question;
  `escalation_warranted` (line 914) takes the same single policy.
- The deciding node's identity is **not carried** on
  `QuestionDecision` (executor.rs:117) — the one structural gap. Fix:
  add `decided_by: NodeId` (plus the `NodeKind`), stamped in
  `Executor::merge` (`NodeOutput::Decided` arm, line 1203) where
  `spec: &NodeSpec` is already in scope. `pub(crate)` type — no wire or
  IR change.
- Gate resolution becomes:
  `ladder.policy_for(kind).unwrap_or_else(|| request.policy())` at both
  `resolve_threshold` and `escalation_warranted`; the trace entry gains
  the policy source (which rung's gate fired) so an execution trace
  explains the gate, per the no-hidden-thresholds rule.

### Wired (2026-09-30) — final shape, one correction

Implemented as above with two deviations the code forced, plus one
correction to this document's own earlier claim:

- `decided_by` is stamped at **construction**
  (`decide_question`, called from `decide_questions` where the deciding
  node's id and kind are already in scope), not in `merge`'s
  `NodeOutput::Decided` arm — `merge` re-orders nothing and needs no
  change. It is a plain `(String, NodeKind)`, so ladder resolution at
  the threshold node re-derives the same rung the decision node used,
  independent of trace order.
- `escalation_warranted` (focused-extraction reverse escalation) also
  reads the rung policy — the escalation decision is part of the same
  gate semantics.
- **Cache correction:** an earlier draft of this section claimed the
  cache stores pre-gate decisions and that `resolve_threshold`
  re-applies gates after a hit. False in the implementation: the cache
  stores **completed (gated) responses**, and a hit returns the stored
  response wholesale (prepending only its own `hit=true` trace entry).
  The stale-gate hazard is therefore real, and the protection is the
  identity decoration: a configured ladder decorates the engine's
  model id as `|ladder-v1@<id>` (same rule as focused extraction's
  `|focused-v1@<budget>`), which rides every cache key — a changed
  ladder re-keys instead of replaying an old gate. An empty ladder is
  byte-identical to no ladder: no decoration, no `policy_source` trace
  facts, unchanged fixtures.
- Assembly refuses a non-empty ladder whose `id` is empty or `"none"`
  (`EngineError::InvalidConfig`): the id is the cache-key
  discriminator, so an anonymous ladder is a correctness bug, not a
  style issue.
- **Latent gap noted, not fixed here:** `NodeSpec::threshold` is
  declared and validated on the graph and folded into node
  fingerprinting (`optimize.rs`), but no executor code consumes it —
  the gate has always read the request policy alone. The ladder is
  engine-configuration-level and composes alongside that knob; wiring
  the graph scalar to the gate is a separate, deliberate change.

Surface: `opencodifier_engine::LadderPolicy` (`per_node` → `per_kind`
→ request policy), `EngineConfig::with_ladder`,
`EngineHandle::with_ladder`. Tests: `src/ladder.rs` (resolution order,
identity validation) and `tests/ladder.rs` (empty-ladder byte
identity, per-kind tightening with `policy_source` in the trace,
per-node precedence, the accept-never rung `min_confidence: 1.0`,
`InvalidConfig` on an anonymous ladder, cache-key shift on a changed
ladder id, rung outcome riding a cache hit).

