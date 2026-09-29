# Skill: context-pruning

**Decision area:** what context a task actually requires — before you
pay to carry the rest (PLANNING.md §76 is the worked example).

**Call shape:**

- `state` — the candidate context material (or a structured summary of
  its sections, each as a sentence or fact).
- `boolean` questions per section class ("Does the answer need the
  full ledger?", "Does the answer need the error history?"), or one
  `score` ("How much of the attached context is relevant?" over
  `none/some/most/all`).
- Set `metadata.limits.max_input_bytes` to your real budget — the
  engine refuses oversized states instead of silently truncating.

**Reading the answer:** use the score distribution to size the carry:
"mostly relevant" at 0.7 with "all" at 0.2 means carry the large
context but expect to reference, not quote. Booleans that abstain are
telling you the state text lacks the discriminating vocabulary — send
section summaries rather than a bigger blob.

**The engine helps here too:** with a focus policy configured, the
engine's own extraction decides long states on the evidence-bearing
sentences (≤ a token budget) and escalates to the full state only when
the focused view is weak — you do not need to pre-prune for the
decision to be cheap.

**Anti-patterns:** pruning by character count (the ladder already
prefers cheap mechanisms); discarding context because one boolean
said "no" for the whole blob; feeding secrets as state to decide what
to prune — decide on shape, then handle the material under your own
redaction rules.
