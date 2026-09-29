# Skill: model-selection

**Decision area:** choosing one model from a list of *eligible* models.

**The division of labor (PLANNING.md §74):** you compute eligibility
(price, privacy lane, context window, quota, availability) and send the
survivors as `candidates`; OpenCodifier ranks them semantically against
the request. OpenCodifier did not make the economic decision — it
supplied the semantic decision information.

**Call shape:**

- `state` — the request text plus anything describing difficulty
  ("refactor this parser", "42K tokens of log context").
- One `choice` question, `candidates` = eligible model ids with
  descriptions carrying their capability shape ("long-context local
  model, weak at proofs").
- Policy: `min_confidence` high (a model pick is cheap to act on but
  expensive to get wrong); `verify_below` set if a verifier model is
  available.

**Reading the answer:** the winner's `confidence` is calibrated — treat
0.55 as "barely preferred", not "certain". If the top two are within a
few points, the distribution is telling you the request is
indifferent to the choice: pick on economics (cheaper/faster) and move
on. That is not abstention; it is the distribution doing its job.

**On abstain:** the candidate descriptions did not discriminate. Fix
the descriptions (say what each model is *for*), not the policy.

**Anti-patterns:** sending ineligible models "so the engine can
reject them" (filtering is deterministic and yours); encoding prices
into candidate descriptions (the engine ranks semantics, not dollars);
re-asking per request with an unchanged state — same state, same
candidates ⇒ the exact-decision cache answers in microseconds.
