# Cube Puzzle Implementation Audit
**Date:** 2026-10-18
**Branch:** `vision/vision-rung-2026-10` (c932e8e)

---

## Summary

The NxN + non-cubic + shape-mods overhaul is **not shipped**. The memory record is stale. The site has: (1) a complete 3x3x3 LBL solver with shape-mods support, (2) Pyraminx and Skewb **shells** that reference absent cores, (3) no Clock, (4) no NxN cores.

---

## What Is Shipped

### cube-core.js — FULLY SHIPPED
- **3x3x3 only.** Move permutations derived from cubie geometry, not tables.
- 5-law validator: shape, color counts, corner-piece identity, edge-piece identity, corner-twist sum, edge-flip sum, permutation parity.
- Deterministic LBL solver: cross → F2L corners → middle-layer edges → OLL → PLL → AUF.
- CO distance table (BFS from all 24 targets over sune/anti-sune × 4 U-alignments).
- Shape mods: `classic`, `mirror`, `ghost` — same algebra, same solver, different cubie `scale3d` geometry only.
- **No NxN support.** No even/odd layer semantics. No 4x4+ parity handling.

### cube.js — FULLY SHIPPED
- CSS 3D cube (26 cubie divs, 54 live sticker divs).
- Drag-orbit with pointer capture.
- Step-through coach with rung chips (rule / roster / engine).
- Engine bridge for genuine tie-breaking.
- Session board, stats overlay.
- Shape-mod switcher (no-rescramble: applies `scale3d` to existing geometry).
- Boot paused (newGame on build).

### pyraminx.js — SHELL ONLY
- References `globalThis.PyraminxCore` which **does not exist** in the tree.
- Line 9: `const PC = globalThis.PyraminxCore;` — null guard on line 11 catches this, so the game silently does nothing if the core is missing.
- Scramble/generation stubs present (`PC.createGame`, `PC.normalizeConfig`).
- `solve()` says "solver not yet implemented".
- Move buttons wired (U, U', L, L', R, R', B, B').
- Canvas 2D rendering stub (triangular grid drawing).

### skewb.js — SHELL ONLY
- References `globalThis.SkewbCore` which **does not exist** in the tree.
- Line 9: `const SC = globalThis.SkewbCore;` — same null-guard silent failure.
- Scramble/generation stubs present.
- `solve()` says "solver not yet implemented".
- Move buttons wired (axes × clockwise + prime).
- Canvas 2D rendering stub (diamond/pentagon drawing).

### Clock — NOT PRESENT
- No `clock.js` or `clock-core.js` anywhere in `site/`.
- TNoodle table mentioned in memory does not exist.

### NxN (cube-n-core.js) — NOT PRESENT
- No `cube-n-core.js` or any NxN solver in `site/`.
- Memory says 2×2 BFS coach + 4–21 free play + laws were on `site/games-audit-2026-10` branch. That branch was not merged to `main` or `vision/vision-rung-2026-10`. The current branch's pyraminx/skewb addition is commit `5251e52`.

---

## Memory Claim vs Actual State

| Claim (memory, 2026-10-15) | Actual |
|---|---|
| "cube 2×2–21×21 overhaul + Rubik's Clock shipped on branch site/games-audit-2026-10 (commits ce9d65a, 553cd28, 4f32536)" | Those commits are not in the current tree. The branch was not merged. |
| "Two verified cores under one facade in site/cube.js (makeK): cube-core (3×3 LBL) and cube-n-core (2×2 BFS coach, 4–21 free play + laws)" | cube-n-core.js does not exist. |
| "Clock = clock-core.js (TNoodle-pinned table, Z₁₂ rank-14, unique solution, engine-free on purpose)" | clock-core.js does not exist. |
| "Pyraminx/Skewb cores = background agent lane" | Only the shells exist; no cores shipped. |
| "Megaminx/Square-1/Fisher/Windmill deliberately designed, not faked" | These are not in the site directory at all. |

---

## Scope for the NxN + Non-Cubic + Shape Mods Overhaul

### Tier 1 — Minimum Viable (get every shell functional)

1. **Pyraminx core** (`pyraminx-core.js`)
   - Tetrahedron facelet model (4 faces × 9 stickers = 36 state elements)
   - Face turn permutations (U, L, R, B axes, each with tip and base rotations)
   - Validator (color counts, piece identity, orientation sum)
   - Scrambler (seeding, move generation)
   - Solver: trivial 1-layer approach or BFS for small puzzles; the shell is already wired

2. **Skewb core** (`skewb-core.js`)
   - 6 faces × 5 stickers = 30 state elements
   - 4-axis quarter-turn model (corner-cutting, not layer-based)
   - Validator, scrambler, solver stubs
   - Shell already wired

### Tier 2 — Complete WCA Non-Cubic

3. **Clock core** (`clock-core.js`)
   - 9 clock faces × 4 pins = 36 state elements
   - TNoodle's Jaumann/Wethiell algorithms (rank-14 Z₂₈ solution space, unique per configuration)
   - 14 moves (UR, DR, DL, UL, U, R, D, L, ALL, y2, U', R', D', L')
   - Engine-free by design: the solution is deterministic
   - TNoodle table must be committed or generated at init

### Tier 3 — NxN Family

4. **cube-n-core.js** — 2×2 through 7×7
   - BFS for 2×2 (6-move solver, ~28k state space)
   - Reduction method for 4×2 through 7×7 (solve centers → pair edges → solve as 3×3)
   - Parity handling (single-slice flip for even cubes, corner-twist for odd)
   - Separate state layout from 3×3 (facelet count = 6×n²)
   - Laws differ per n: for even n, no fixed centers; for odd n, center orientation matters

### Tier 4 — Shape Mods Extension

5. **Mirror/ghost** already done in cube.js.
6. **Future shape mods** (Fisher, Windmill, etc.) require separate cores — they break the 3×3 move algebra and must not share facelet layout assumptions.

### What Does NOT Need Work

- **cube-core.js** — complete, no changes needed for NxN work
- **cube.js shell** — works for 3×3; NxN needs a new shell (different DOM layout for 4×4+, different move model)
- **Shape mod geometry** — already correct in cube.js

### Key Constraints from the Architecture

- Each core is a separate JS module (not merged into one file)
- Facelet layout is unique per puzzle family — no shared "makeK" facade without explicit adapter
- Shell → core interface: `createGame(cfg)`, `applyMove(state, tok)`, `applyAlg(state, alg)`, `validateState(state)`, `verifySolved(state)`, `normalizeConfig(raw)`, `{stamp,seed}` config
- Solver is engine-free for Clock; engine-bridge tie-breaking for NxN where reduction has multiple equivalent moves
- Memory notes: `validateState` shape differs per core (cube-core uses `{ok, checks:[{name,ok}]}`, cube-n-core would use `{legal, issues}`) — normalize at the shell layer via `S.K.laws(state)`
