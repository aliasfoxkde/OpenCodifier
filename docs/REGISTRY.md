# Decision Registry format (§63, D20)

A decision definition is a JSON document that names a reusable decision:
the question to ask, where its candidates come from, and the policy that
gates the answer. Definitions are artifacts — versioned, hashed,
loadable — but they are bookkeeping, never a second decision path:
instantiation produces an ordinary canonical request through the same
validating constructors every other ingress uses.

## The document

```json
{
  "id": "amortyx.model_selection",
  "version": 1,
  "question": {
    "type": "choice",
    "text": "Which model should process the request?"
  },
  "candidates": {
    "static": [
      { "id": "local-qwen", "description": "fast local coding model" },
      { "id": "cloud-large", "description": "long-context cloud service" }
    ]
  },
  "policy": {
    "min_confidence": 0.80,
    "verify_below": 0.65,
    "abstain_below": 0.50,
    "risk": "low"
  }
}
```

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | Dotted artifact name; also the instantiated question's id. Non-empty. |
| `version` | yes | Label version, ≥ 1. Human bookkeeping — not the artifact's identity. |
| `question` | yes | `{ "type": "choice" \| "boolean" \| "score", "text": ... }`; score questions add `"levels": [...]` (≥ 2, lowest first). |
| `candidates` | choice only | Either `{"dynamic": true}` (the caller supplies candidates per call) or `{"static": [...]}` (embedded, non-empty). Exactly one — never both. Forbidden on boolean/score. |
| `policy` | no | A complete native policy document (see `DecisionPolicy`); omitted means the engine default. |

Candidate documents are `{ "id", "description" }` pairs — the same shape
the native wire format accepts, validated by the same constructor.

## Identity: content, not labels

A definition's identity is the **SHA-256 hex of its canonical JSON
serialization** (`DecisionDefinition::content_hash()`). `id` and
`version` are labels: bumping `version` without editing anything else is
still a content change and moves the hash; two byte-identical documents
are the same artifact whatever their labels claim. This is the cache
key's discipline (D6) applied one level up.

Definition identity **never enters decision cache keys.** The
instantiated request is canonical and cache keys fold request content
plus engine identity only — two definitions that instantiate to
byte-identical requests legitimately share a cached decision, because
the decision depends on the request, not on which artifact produced it.

## Instantiation

```rust
use opencodifier_schema::registry::{DecisionDefinition, Registry};
use opencodifier_core::Candidate;

let document = serde_json::from_value(definition_json)?;
let definition = DecisionDefinition::decode(&document)?;
let request = match definition.is_dynamic() {
    true => definition.instantiate(state_text, &caller_candidates)?,
    false => definition.instantiate(state_text, &[])?,
};
// `request` is an ordinary DecisionRequest: same IR, same engine, same
// cache discipline as a hand-written wire request.
```

A dynamic choice definition refuses an empty candidate list; a static
one refuses extra candidates rather than silently merging. Decode
failure is the earliest possible refusal: text, levels, candidates, and
policy are validated at `decode`, so a registry cannot hold a definition
the engine would later reject.

## The registry

`Registry::new(definitions)` indexes by `id` and refuses duplicates —
lookup, not discovery. No filesystem, no network, no reload: loading
documents from disk is the caller's job, which keeps the crate sync and
local-first.

## Example: routing recipe

The `router.target` definition in [`../recipes/registry/`](../recipes/registry/)
shows the full path — definition → instantiate → decide through a
committed graph — with its captured response.
