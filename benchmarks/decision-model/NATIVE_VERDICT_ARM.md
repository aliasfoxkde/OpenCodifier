# Native verdict-slot readout arm (Jev-Style) — design of record

Status: designed 2026-09-28; runner shipped (`runner/run_jev_native.py`);
first run waits for a quiet host (the params A/B owns the cores). This is
task #25 ("MBLI / masked-likelihood arm") resolved to its concrete form:
the *masked-likelihood readout* is not a hypothetical arm type, it is the
published, sha256-pinned interface of Jev-Style-Decision-v3, and the
reference implementation ships beside the weights.

## Why this arm exists

D16 measured Jev-Style-0.8B-Decision-v3 through our fork's tree mode and
recorded 0.217 accuracy (ECE 0.408); JSON-writing chat scores 0.0. Both are
*interface-mismatch* rows: the model was never trained to emit candidate-id
tokens or JSON answers. Its trained readout (model card + shipped source) is:

1. **Render** the input as raw text — no chat template, no BOS/EOS, no
   system message, special tokens disabled in user text:

   ```
   State:\n<state>\n\nQuestion [<type>]: <question>\nOptions:\n
   - <option 1>\n ... - <option K>\n
   Judge each option:\n
   <option 1> ->\n ... <option K> ->\n
   ```

   (`macjev-render-v1`; options are `name: description` for choice,
   `level i: description` for score, `false:/true:` with held/not-held
   criteria for noul.)

2. **Read** one verdict slot per option: at the k-th ` ->` token position,
   score_k = logit(" yes") − logit(" no"), taken from the one fused decode's
   prompt-position logits (the scorer sets `batch.logits = 1` only at slot
   positions — no generation happens at all).

3. **Calibrate**: probabilities = softmax(scores / T) with T from the
   shipped `readout_config.json` (macjev-temperatures-v1): global T 0.880 +
   20 fitted `family|qtype|option_bucket` groups, shrinkage_k 100, clamp
   [0.3, 5], fit quality carried inside the artifact (NLL 0.3775→0.3667,
   ECE 0.0329→0.0114 on 15,655 rows, pool content-hashed).

Because attention is causal, a slot's hidden state depends only on the
tokens up to it — so the native readout is *exactly* reproducible without
their C++ scorer by teacher-forcing the render and truncating after each
` ->`; their scorer just does all K slots in one pass.

## Why not through our fork

The fork's `/v1/decision` cannot express this interface today:

- `render_prompt` (`decision-engine.cpp:666`) wraps every request in a
  system catalog + the model's chat template + a `{\n` continuation —
  byte-incompatible with the trained layout. Running the Jev model through
  it is precisely what produced the 0.217 transfer row.
- The readout is logits at *prompt* positions; tree mode scores generated
  token paths. Adding a `mode: "readout"` to the fork is possible future
  work (and `jev_score.cpp`, 367 lines, is a working blueprint — it shares
  the same libllama our fork builds), but the arm does not need it: the
  authors' runtime is a JSON-lines subprocess over the same libllama,
  zero fork changes, byte-exact, and `verify=True` re-hashes weights,
  tokenizer and readout config against their `manifest.json` on load.

## The arm

`runner/run_jev_native.py` drives the reference runtime
(`jev_style_decision_gguf.py`, imported from the model dir, manifest
verified) over our 120-item suite, mapping each item natively:

| suite item | Jev-Style protocol |
|---|---|
| `context` | `state` (string passes through) |
| `question` | choice question text |
| `candidates [{id, description}]` | `options = {id: description}` |
| *(nothing)* | `category=None` → shipped global T |
| `instructions` (suite header) | **unused** — the protocol has no instruction channel (recorded in the result's `caveat`) |

Phases: one single pass, then a full replay for determinism (greedy logits,
fixed math ⇒ exact match expected). No batched phase (single-question
states share no prefix) and no chat phase (the 0.0 chat row already exists
from D16 as the mismatch anchor). Output schema matches the other arms
(`arm: jev_native_verdict_slot`, rows with pred/prob, `metrics` with the
shared `ece`/`metrics` helpers) so `summarize.py` picks it up unchanged;
latency is client wall per item, labelled as such in `config`.

## Cost

120 items × 4–6 candidates, one fused decode per item (~300–500 tokens
each) on an 0.8B Q4_K_M ≈ minutes of CPU, plus a one-file scorer build
against the fork's existing `libllama.so` (their script links, does not
reconfigure — the fork's build tree already has it).

## Acceptance criteria

1. **Integrity**: `verify=True` passes — weights, tokenizer, readout config
   re-hashed against their manifest (all seven files verified OK at
   setup, 2026-09-28).
2. **Interface fidelity**: render layout and readout come from their code,
   not ours; the result records `readout: macjev-readout-v1`, `template:
   macjev-render-v1`, shipped `global_temperature`, and the runner pins
   provenance `chaoliangUNSW/Jev-Style-0.8B-Decision-v3-GGUF`.
3. **Determinism**: replay matches exactly (`predictions_match`, Δprob 0).
4. **The hypothesis under test**: native-interface accuracy ≫ 0.217 (the
   transfer row). Not comparable to their published JevBench numbers
   (different suite); the comparison that matters is *same weights, our
   suite, their interface vs our tree mode*. Whatever the number is, it
   lands in REPORT.md as the interface-sensitivity measurement.

## What OpenCodifier buys from it

- **Interface sensitivity, quantified**: same weights, three interfaces
  (0.0 / 0.217 / native) — the cleanest evidence for the spec's claim that
  the *decision interface* is the product, not the model (§3, §13).
- **A readout design for §15/§16**: if we train an in-house decision model
  (the planned §15 path), the verdict-slot readout is the reference
  architecture — prompt-position logits, one decode per state, calibration
  shipped as a versioned artifact with fit quality inside.
- **A D15 calibration reference**: their `readout_config.json` is the
  most complete temperature artifact we have seen (per-group shrinkage,
  clamp, content-hashed pool, in-artifact fit quality) — the bar our D15
  artifact format should meet as it grows beyond a global temperature.
