# Skill: tool-selection

**Decision area:** which of the available tools a task actually needs
(PLANNING.md §75 is the worked example).

**Call shape:**

- `state` — the task text.
- Per tool family, a `boolean` question ("Does this task need shell
  execution?", "Does this task need filesystem access?") — booleans are
  cheaper than one choice over tool bundles and answer independently.
- If the toolset is small and mutually exclusive, one `choice` over
  tool ids with descriptions of when each applies.

**Reading the answer:** a `boolean` answer carries its probability —
0.85 "needs tools" is a planning input, not a gate; you decide the
action threshold. Combined with `difficulty` (a `score` question) you
get the shape of the execution plan without any prose.

**On abstain:** the task text does not reveal tool needs. Either
gather more state (read the file list, check the environment) before
deciding, or provision conservatively.

**Anti-patterns:** listing every tool you have as candidates and
asking "which one" when the honest question is "which ones" (use
booleans); deciding tool need *after* execution failed; encoding
permission levels into the decision (that is `tool-gating`).
