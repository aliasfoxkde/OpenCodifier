# Program Handoff — 2026-10-15

**Status:** handoff document. It organizes the state of record and names the
next steps with their stopping conditions. Every factual claim here is a
pointer to a record of record (listed per item); nothing in this document is
itself primary evidence. Times are US Central.

**Branch of note:** `site/games-audit-2026-10` (pushed to GitForge and
origin) carries the 2026-10-15 site work: cube overhaul (`ce9d65a`) and the
docs freshness sweep (`553cd28`). Site flow: edits on branch → Cloudflare
Pages preview → human merges the PR.

---

## 1. State of record

**Release.** `v0.6.1` (2026-10-15): selective prediction end to end,
`opencodifier-itemgen` as a crate, corpus-composition question closed, first
gated weights on HuggingFace. `v0.6.0` is an unpublished tombstone (tag
pattern retired); `.github/CHANGELOG.md` says so explicitly. Coverage 98.39 %
in-image DA; CI of record is GitForge on the fedora remote.

**Model program** (`docs/TRAINING.md`, all verdicts recorded there):

| Arm | Verdict | Numbers of record |
|---|---|---|
| E1-E (2B, iterb scaffold 178k) | **GATE_PASS ×2 — release candidate** | suite 0.8083, JevBench 0.6840, ECE 0.127 (T=3.362) |
| t33p (2B, merged-v6-full 307k) | GATE_PASS ×2 — evidence, not candidate | suite 0.6917, board 0.6580, lane drop −3.4 (vs E1-E's −12.4) |
| T88 imatrix Q4_K_M quant | **FAIL all three gates** | recorded 2026-10-15, 10:20 AM |
| MiniCPM5 GGUF rung | CLOSED no-go | adapter lane-bound; size/speed legs win |
| Corpus-volume lever | **CLOSED** | v5 catastrophic → v6 gate-passing → scaffold alone best; "gated volume ≠ transfer" |

The two legs of the release gate: suite ≥ 0.68 AND JevBench ≥ 0.6494
(`docs/RELEASE_POLICY.md`; [[Models]] on the wiki mirrors it). Weights
publish only gate-gated to HuggingFace (`TaskWizerAI`), open Apache-2.0.

**Site / playground** (`site/`, branch `site/games-audit-2026-10`):

- **Twisty Cubes 2×2–21×21** (`cube.html`, `cube.js`, `cube-core.js`,
  `cube-n-core.js`): two verified cores under one facade. 3×3 keeps the
  battle-tested LBL core; 2×2 has its own coach (layer build → (R′D′RD)
  orientation → BFS over the 24 top permutations); 4×4–21×21 are validated
  free play with piece-law checks on every move and SiGN wide turns.
  Renderers: CSS-3D orbit at n ≤ 3, canvas net at n ≥ 4. Mirror/ghost/
  sudokube skins at 3×3. The long-standing T-perm typo (15 tokens) is fixed,
  and the greedy phase-3 stall is replaced by exhaustive BFS. Tests:
  cube-n-core 13, cube-core 10, playwright smoke 25/25.
- **Rubik's Clock** (`clock.html`, `clock.js`, `clock-core.js`): TNoodle-pinned
  move table, rank-14 basis over Z₁₂, 12¹⁴ reachable states, **unique
  solution** — solved by exact elimination, replay-verified; engine-free on
  purpose (no ties exist to arbitrate). Tests: core 12/12, smoke 13/13.
  Two shell bugs found by smoke and fixed: plan moves no longer invalidate
  their own plan mid-step, and the boot chip now renders (and truthfully
  reads "scrambled" — the page boots paused per the site convention).
- **Playground hub** (`playground.html`): truthful card badges — playable vs
  "verified core is in test" (Pyraminx, Skewb) vs "designed, not faked"
  (Megaminx, Square-1, Fisher, Windmill).
- **Docs sweep** (`553cd28`): SECURITY supported-matrix → 0.6.x; D11 axum pin
  corrected to 0.8; PLANNING §36 amendment (no `/v1/systemone` route — Jev
  compat ships as a schema adapter); itemgen added to ARCHITECTURE; site
  counts corrected (27 test suites); PLAN phase 22 notes the sweep;
  CHANGELOG explains the 0.6.0 tombstone.
- **Wiki** (GitHub wiki, pushed `ae89b4e`): 17 pages + footer/sidebar;
  Roadmap now carries the vision rung (§3 below); Playground lists the cube
  family and Clock truthfully.

**Compute posture** (memory + `docs/handoffs/PLATFORM_DECISION_RUNTIME_ADOPTION_HANDOFF_2026-10-12.md`):
the T5500 is down; fedora carries the eval/training load through harness-jobs;
local hosts serve models; training is free-tier-first (Kaggle T4) with a ~$50
Modal burst budget authorized. CI runs on GitForge, never local cargo.

## 2. Immediate next steps (in order)

1. **Integrate Pyraminx + Skewb** — cores are being built by the in-flight
   agent (site/pyraminx-core.js exists; Skewb pending). Stopping condition:
   both cores pass node suites (TNoodle/`jaap`-pinned move tables, seeded
   solve sweeps), pages ship (`pyraminx.html`, `skewb.html`), playground
   "soon" badges flip to playable. Do NOT fake a solver: if the generator
   search can't prove a basis, the page ships free-play with laws validation,
   like the big cubes.
2. **Games-audit PR through review** — push any remaining branch work, open
   the CF Pages preview, and hand the operator the PR. Stopping condition:
   human merge to main + production re-deploy.
3. **Board submission** — `docs/board-submission-draft.md` + 8 operator
   questions are the blockers; the dominance verdict (composite plausible,
   every-axis not yet) is recorded in `docs/BENCHMARKS.md`. Stopping
   condition: operator answers → submission filed.
4. **#150 eval lane v2** — batching + sdpa with anchor-drift check. t33p's
   −3.4 lane agreement (vs E1-E's −12.4) makes the lane-honesty question
   sharper: the suite leg may be partially lane-specific. Stopping condition:
   load-pinned determinism replay or an agreed tolerance, recorded in
   `docs/TRAINING.md`.

## 3. Areas of improvement (ranked)

1. **Vision support — the operator requirement.** "It can be optional and a
   separate rung/lever that routes only when needed, but we need vision
   support to compete… And it needs to be comprehensive and fast." Design
   shape (recorded on the wiki Roadmap, mirrors the binding rules):
   - `ImageBackend` seam beside `InferenceBackend`/`EmbeddingBackend` in
     `opencodifier-runtime`; ONNX behind the feature gate (D11); no Python
     at runtime (§73).
   - Rung position: above lexical/embedding, below the text decision model;
     fires **only on image-bearing requests** — the default text path never
     pays for it.
   - Outputs are candidate-conditioned typed decisions (choice/boolean/
     score), never free-form generation; confidence passes the mandatory
     calibration gate; cache keys carry vision-model + calibration versions.
   - Models: small quantized vision encoders (SigLIP-class) on the ONNX CPU
     path, explicit opt-in pull (release policy), verifier contract mirrored
     from the text ladder (agree → accept, disagree → abstain).
   - Open work: sizing + first parity fixtures (fixture-first, per the
     model-import parity discipline in quality rules).
2. **Big-cube solver honesty.** 4×4–21×21 are free play by design. A
   reduction solver is a real project; if wanted, size it as its own phase
   (parity handling is the hard part) rather than diluting the coach.
3. **Megaminx / Square-1 / Fisher / Windmill.** Designed-but-not-faked on
   the playground. Megaminx needs a dodecahedral core (12-color laws); 
   Square-1 needs a non-group-regular move algebra (shape-dependent). Both
   deserve their own verified cores before any page ships.
4. **Per-rung calibration artifacts** (D25/F24 seam exists). E1-E's per-kind
   T fit is the working pattern; ship rung artifacts only where the ECE gate
   passes.
5. **Docs phase 22 residue.** The wiki cross-checks ride the same phase;
   re-run the stale-doc probe after each release cut, not ad hoc.

## 4. Roadmap

- **Near (this cycle):** Pyraminx + Skewb pages; games-audit PR merged; board
  submission filed; #150 lane v2 spec'd; vision rung sized with parity
  fixtures named.
- **Mid:** vision rung implementation behind its feature gate; consensus-flip
  patch / strands consensus pass (quality-first trainer lever); T3.4 4B
  contingency if the 2B plateau holds; per-rung calibration artifacts where
  gates pass; Megaminx core.
- **Far:** big-cube reduction solver; hosted tier per
  `docs/HOSTED_TIER.md`; registry publication (explicit operator go, D24);
  Square-1 core.

**Deliberately not scheduled** (unchanged): default-on escalation ladder
(per-domain calibration precedes it), browser model path (D23), any
cloud/API-key dependency (the default posture is fully local and offline).

## 5. Standing constraints (unchanged, binding)

- Push order: `git push gitforge` first, then `origin`; pushed history is
  immutable — fix forward, never force-push; never `git stash`.
- CI/CD through GitForge on fedora; a red GitHub Actions run is not a code
  signal. Heavyweight checks via harness-jobs profiles.
- Never echo credentials (gitforge/Kaggle/HF/env files); `gitforge auth
  --login` is the operator's credential path.
- Conventional commits, `Co-Authored-By: Claude` trailer; aegis baseline
  regen rides the same commit as source edits.
- Site conventions: boot-paused games with truthful play buttons; accordion
  exclusivity site-wide; stats overlay on every game; responsive rule for
  every canvas; edits on branch → CF preview → human merge.
- Findings land in repo docs + commit + push before the turn ends; memory
  holds pointers only.
- Reports in AM/PM 12-hour US Central.

## 6. Where everything lives

| Record | Path |
|---|---|
| Live plan + phase table | `docs/PLAN.md` |
| Binding decisions | `docs/DECISIONS.md` (D1–D37+) |
| Founding spec | `docs/PLANNING.md` (§§1–80) |
| Training record + verdicts | `docs/TRAINING.md` |
| Benchmark page of record | `docs/BENCHMARKS.md` |
| Release policy + notes flow | `docs/RELEASE_POLICY.md`, `.github/CHANGELOG.md` |
| Site release flow | `docs/SITE_RELEASE_FLOW.md` |
| Wiki | GitHub wiki (17 pages; clone at `/var/tmp/wiki-oc`) |
| Prior handoff (runtime adoption) | `docs/handoffs/PLATFORM_DECISION_RUNTIME_ADOPTION_HANDOFF_2026-10-12.md` |
