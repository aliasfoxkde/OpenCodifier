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

## Live-wiring design (task #59 step 2 — drafted 2026-09-30, gated on the margin study + d15 refit)

The thresholds above are **post-hoc sweeps over raw winner probabilities**.
Wiring them into the engine must go through the seams that already exist,
without re-opening the IR:

1. **Per-rung policy, not one policy.** `DecisionPolicy`
   (`opencodifier-core/src/policy.rs`) is the gate type; the ladder needs
   one per rung because the rungs' confidence semantics differ (exact
   proofs are p = 1.0 by construction; the embedding rung has no
   calibrated probability at all — CALIBRATION finding 3; the model rung
   carries a fitted temperature). Concretely: engine configuration gains
   a `ladder: Vec<(RungId, DecisionPolicy)>`; the executor consults the
   current rung's policy at the existing confidence gate. No IR change,
   no wire change — the ladder is engine-internal composition of an
   existing, validated type.

2. **Per-rung profiles implied by this study + CALIBRATION:**
   - `rule/relational` (proofs): accept at p ≥ 1.0 — the proofs gate
     themselves; abstain gate irrelevant (never fires). Matches
     relational-v1's no-ship decision (identity calibration).
   - `embedding`: **never accept on probability** — gate by rank/margin
     (the rank/margin study, CALIBRATION finding 3). `min_margin` is the
     §19 hook for this; the probability gates stay at accept-never
     (`min_confidence: 1.0`).
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
- **Cache interaction is safe by construction** (verify with a test
  anyway): the cache stores pre-gate decisions; `resolve_threshold`
  re-applies gates after a cache hit, so a changed ladder takes effect
  without invalidating cached decisions — but the engine-config identity
  already rides every cache key, so a *shipped* ladder still shifts keys
  exactly when it should.
- Tests to add: per-kind override selects the rung policy; empty ladder
  byte-identical to current behavior (wire fixtures stay locked);
  embedding rung with `min_confidence: 1.0` + `min_margin` set never
  accepts on probability; trace records the rung source; cache-hit path
  re-applies the ladder gates.

