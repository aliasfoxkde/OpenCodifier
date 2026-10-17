# Skill: routing

**Decision area:** producing the semantic inputs a router needs to send a
request to the right lane (PLANNING.md §74's example is this skill).

Routing economics — price, latency targets, provider health, quota — are
the router's job. OpenCodifier answers the *semantic* half:

1. **Task class** — a `choice` question over your task taxonomy
   (e.g. `coding`, `retrieval`, `summarization`, `chat`).
2. **Complexity** — a `score` question with ordered levels
   (`simple` … `expert`). Read the full distribution, not just the
   winner: a 0.62 complex / 0.30 medium split justifies soft behavior
   (a larger completion budget) without a second call.
3. **Requirements** — `boolean` questions in the same request:
   needs tools, needs long context, needs fresh data.

Send all of these as **one request** (one `state`, several
`questions`) — they share the state and the answer arrives together.

**Call shape:** state = the request text (or its focused view; the
engine extracts evidence-bearing sentences itself when you configure a
focus budget). Candidates only on the class question. Policy defaults
are sane; raise `min_confidence` if a misroute is expensive.

**Reading the answer:** attach `outcome` + `confidence` per question to
the routing record. An abstained complexity question is a signal the
state is insufficient to classify — route conservatively (your safest
lane), do not guess from character counts.

**Anti-patterns:** asking OpenCodifier *which provider to use* (that is
policy + eligibility, yours); feeding raw provider payloads with
credentials as state; re-deciding per retry — a retry storm is a cache
hit, not a new decision.
