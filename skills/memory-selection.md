# Skill: memory-selection

**Decision area:** which stored record/memory/note applies to the
current task — retrieval as a typed decision instead of a similarity
dump.

**Call shape:**

- `state` — the current task text plus its key entities.
- One `choice` question, `candidates` = the retrieved record ids with
  one-line descriptions. Candidates are *your* retrieval candidate
  set (the engine is deliberately not a vector store — §25); the
  decision is which of them actually applies.
- For unbounded collections, ask the cheaper question first — a
  `boolean` per top-k hit ("Relevant to this task?") — and only
  escalate to a choice when several booleans are true.

**Reading the answer:** the distribution over record ids is the
relevance ranking, calibrated. A near-tie at the top means the records
overlap: use both rather than pretending the 0.51 is a real
preference. `abstain` means none of the candidates matches the task —
which is exactly the "no relevant memory" answer a similarity search
cannot give you honestly.

**Anti-patterns:** treating top-1 as "the relevant memory" regardless
of confidence; asking with empty candidates (there is nothing to
decide); stuffing full record bodies into `state` — describe them in
the candidate descriptions and keep the state about the task.
