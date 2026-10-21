# Prose Fact Extractor — Design

**Status:** Design only. Not started. Blocked on: (a) choice of extraction
model, (b) measurement protocol against the 2B model's accuracy.

## Problem

`facts.rs` has four exact grammar patterns. JevBench states are free prose:
0 of 231 items match any pattern. The relational solver cannot prove anything
about JevBench states until prose can be extracted.

The `temporal_numeric` family (JevBench accuracy 0.333, the hardest family)
exemplifies what's needed: date entities, temporal relationships, named-entity
comparison.

## What extraction means

Two sub-problems:

1. **Named-entity recognition (NER)** — extract typed entities from prose:
   - Dates: "31 August 2026", "18 months later"
   - Named entities: "certificate EW-5540981", "claim CL-28-00471"
   - Numeric values: thresholds, durations

2. **Temporal relationship extraction** — given entities, extract relations:
   - "period starts on delivery date, ends 18 months later"
   - "claim reported on [date]"
   - "before/after/during" comparisons

## Candidate approaches

### 1. VLM pipeline (most general)

A VLM that returns structured JSON:
```
Input: image or prose state text
Output: {"dates": [...], "entities": [...], "relations": [...]}}
```

A captioning VLM piped through a regex post-processor is the simplest version.
SigLIP2 cannot do this (dual encoder, not a VLM).

### 2. Small NER model + rule post-processor

A distilled NER model (e.g., `dslim/bert-base-NER` or a smaller distilled variant)
plus hand-written temporal relationship rules over the extracted entities.

### 3. Focus module + structured extraction

The existing `focus` module extracts decision-relevant text spans. Extend it with
a small LM head for entity tagging within the extracted view.

## Integration with existing IR

The extractor produces `RelationalFact` variants — the same enum `facts.rs`
uses. Add new variants:

```rust
pub enum RelationalFact {
    // ... existing 4 variants ...

    /// "date_X is before date_Y" or "date_X is after date_Y"
    TemporalBefore { earlier: String, later: String },
    /// "entity_X amount threshold compared to Y: > < ="
    NumericThreshold { entity: String, comparator: Comparator, value: f64 },
    /// "entity named X" (for later comparison)
    NamedEntity { role: String, entity_id: String },
}
```

The relational solver then proves over these facts, just like it does over
`DependsOn` and `RestoresAfter`.

## Verification requirement

Any extractor must be measured against the 2B model's accuracy on the same
items before integration. An extractor that fires wrongly is worse than none
(§23.6 consequence c): it silently discards candidates on weak evidence.

Measurement protocol:
1. Run the extractor on JevBench temporal_numeric items
2. Run the 2B model on the same items
3. If extractor is wrong on an item where 2B is right → do not integrate
4. Only integrate pairs where extractor is right OR both are wrong

## Rejected approaches

- **Pattern matching over raw prose**: already rejected — 0/231 match
- **LLM API calls at runtime**: violates local-first, no-API-keys posture
- **Diffusion models**: no decision benchmark evidence (§23)
