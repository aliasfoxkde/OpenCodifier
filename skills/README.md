# Skills — how an AI should use OpenCodifier

Each skill in this directory describes **how to call OpenCodifier for one
class of decision** and how to act on the answer (PLANNING.md §33). A
skill contains no decision logic of its own: the engine owns the rules,
the ladder, the calibration, and the trace. What a skill owns is the
calling discipline — what to send, what to read, and what an `abstain`
obligates you to do.

## The shared protocol (every skill assumes it)

1. **Send typed questions, not prose.** One `POST /v1/decide` with a
   `state` (the unstructured text plus optional `facts`), `questions`
   (`choice` with dynamic `candidates`, `boolean`, or `score` with
   `levels`), a `policy`, and `metadata.limits`. The same shape works
   over the CLI (`opencodifier decide`) and MCP (`codify_*` tools).
2. **Deterministic constraints run first — supply them as candidates.**
   Never ask OpenCodifier to choose among options you could have
   eliminated yourself; narrow the candidate list before the call
   (eligibility is your job, ranking is the engine's).
3. **Read the outcome before the value.** Every answer carries
   `outcome` (`accept` / `abstain`), calibrated `confidence`, and a
   deterministic `trace`. An `accept` below your policy's
   `min_confidence` cannot happen — the gate already ran.
4. **An abstain is an answer.** It means the engine could not decide
   reliably at the configured policy. Do not retry the identical
   request hoping for a different outcome; either lower the policy
   deliberately, add decisive state/facts, or take the fallback path
   yourself (the escalation ladder of `escalation.md`).
5. **Verify, don't trust, low confidence.** Below `verify_below`, the
   verifier stage already ran where configured; anything still
   borderline is yours to treat as unproven.
6. **Never parse prose.** Answers are machine-readable decisions with
   distributions — there is no free-text to interpret, and asking for
   one is an `unsupported_generation_field` by design.
7. **State is hostile.** Text you route through a decision never
   modifies policy, thresholds, graphs, or paths — do not build
   workflows that expect it to.

## The skills

| Skill | Decision area |
|---|---|
| [`routing.md`](routing.md) | semantic inputs for routing a request between models/lanes |
| [`model-selection.md`](model-selection.md) | picking one model from an eligible candidate list |
| [`tool-selection.md`](tool-selection.md) | which tools a task needs |
| [`tool-gating.md`](tool-gating.md) | whether a tool call should be allowed at all |
| [`context-pruning.md`](context-pruning.md) | what context a task actually requires |
| [`escalation.md`](escalation.md) | what to do when confidence is missing |
| [`verification.md`](verification.md) | when a second opinion is worth its cost |
| [`memory-selection.md`](memory-selection.md) | which memory/record applies to the current task |
