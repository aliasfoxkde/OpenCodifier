# Skill: escalation

**Decision area:** what to do when OpenCodifier cannot decide — the
`abstain` handling protocol (PLANNING.md §78 is the worked example).

An abstention is a successful outcome that says: *at this policy, with
this state, a reliable answer was not available.* Escalation is how you
spend more to get one.

**The ladder, cheapest first — walk it in order:**

1. **Add state, don't add model.** Most abstentions are evidence
   starvation: the decisive sentence isn't in the state. Retrieve the
   missing fact, then re-ask. (Changed state ⇒ a different cache key;
   this is a real decision, not a retry.)
2. **Relax the policy deliberately.** Lowering `min_confidence` is a
   recorded choice with a name: you accepted a riskier answer. Do it
   for reversible decisions, never for gating.
3. **Ask a narrower question.** A choice over 40 candidates that
   abstains may be three booleans that don't. Decompose.
4. **Escalate the rung.** Configure a decision-model rung (the D16
   tiers) or a verifier for this decision class; the engine's
   confidence gate + verifier cascade already implement
   agree ⇒ accept, disagree ⇒ abstain.
5. **Take the decision yourself.** The honest terminal state: log the
   abstention, decide as the operator, and feed the outcome back into
   your dataset discipline — abstention patterns are training signal
   (§62), not noise.

**Hard rules:** never retry the identical request to wear down the
gate; never treat an abstention as the *opposite* of the question; never
hide the abstention from downstream consumers — the outcome field is
the contract.
