# Skill: verification

**Decision area:** when a second opinion is worth its cost
(PLANNING.md §19/§50, worked example §78).

**The engine's contract:** verification is confidence-gated. The
verifier runs only when the first decision lands in the verify band
(below `verify_below`, above `abstain_below`), and never on every
request — two classifiers on everything is the cost discipline the
architecture exists to enforce. Agree ⇒ accept; disagree ⇒
abstain/escalate.

**Your part, configuring it:**

- Set `verify_below` where a wrong answer is expensive and a second
  decision is cheap: gating, tool use, model selection for long jobs.
- Leave it unset for cheap reversible classifications where the
  distribution's spread already tells you what you need.
- The verifier must be *independent* of the first mechanism: a
  different rung of the ladder (lexical vs decision model), not the
  same model asked twice.

**Reading the answer:** when verification ran, the report says so and
the confidence you see is post-verification. If you find yourself
wanting to verify `accept`s above the band, that is a policy
calibration problem — lower `verify_below` explicitly and pay for it
on every request, instead of verifying ad hoc.

**Anti-patterns:** verifying with the same mechanism (agreement is
guaranteed, the cost is not); using a *weaker* verifier to overrule a
*stronger* first decision; sampling your own confidence by re-asking —
the exact-decision cache will serve your "verification" from the first
answer.
