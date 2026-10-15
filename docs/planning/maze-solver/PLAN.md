# Maze Solver with Dynamic Fog of War — plan

Status: **landed — P0–P10 complete (2026-10-08)** ·
P10 verification record: 58/58 node --test (board page, maze core 42,
shell 9, deep link 2); Playwright drive on the served page 10/10 checks
(boot, canvas, bot escape + leaderboard row + rung chip `bfs.exit`, god
toggle + assisted log, keyboard move `x 17 · y 17 → x 17 · y 23`, induced
fight via slow-bot/fast-monster config (status chip `fighting 2.0s`), rubric
click-add `steps` 6→7);
dark + light section screenshots inspected. Worktree: was
`.claude/worktrees/maze-solver` (branch `worktree-maze-solver`) —
removed after the merge.
Target: `site/try.html` (the playground), new section `#maze`, fully self-contained JS.
Shipped scope grew past the original plan per operator requests: versus ghost
mode, optional timer, chests (coins 0–5 or forced monster + humor lines),
difficulty presets, deep links (`?maze=<seed>`), session leaderboard with
explored %, drag-and-drop rubric editor. Standalone-game outlook:
`STANDALONE-GAME.md` next to this file.

## 1. What this is (and refuses to be)

A birds-eye-view retro rogue-like demo: a procedurally generated maze, a character
spawned at the center, and fog of war driven by **generalized Bresenham line-of-sight**.
Two runners: the **bot** (the demo's decision ladder — including real junction
tie-breaks routed through the WASM engine already on the page) and the **human**
(keyboard play). A session-only leaderboard compares them under a user-editable
scoring rubric.

Honesty constraints (mirroring the site's "claims we refuse to make" card):

- The **maze solver** is a deterministic JS decision ladder shaped like the runtime's
  cheapest-reliable-first ladder (rule → BFS → engine). It is *not* the Rust engine
  doing spatial reasoning.
- The **WASM engine** (`site/wasm/`, already loaded by the page) is used where it
  genuinely decides: junction tie-breaks between *equal-cost* frontier choices, as a
  `choice` request over direction candidates with knowledge-derived evidence
  descriptions. Engine unavailable / abstain / low confidence → local tie-break. The
  rung that decided every step is visible in the trace panel.
- The **scoring rubric** is plain JSON, evaluated by a pure deterministic function in
  `maze-core`. The page does not claim the WASM engine computes the arithmetic.
- No network requests. No persistence: leaderboard lives in page memory only
  (operator requirement). No localStorage for scores.

## 2. Architecture

Two browser-facing files, one pure-logic file, one test file. The pure logic file is
a classic script assigning `globalThis.MazeCore` (same pattern as `benchmarks.js`),
so `node --test` can import and execute it with no DOM and no build step.

```
site/
  maze-core.js        pure logic: RNG, maze gen, Bresenham/FOV, bot ladder,
                      entities/combat, rubric scoring   (globalThis.MazeCore)
  maze.js             shell: canvas render, input, game loop, UI wiring  (IIFE)
  tests/maze_test.mjs node --test suite over MazeCore (DOM-free)
  try.html            new #maze section + honest-notes card
  styles.css          .mz-* styles from existing theme tokens only
```

Data flow (per tick): `config + seed → maze → FOV(bot/player) → bot ladder OR
human input → movement → entities (loot/monsters/combat) → metrics → rubric score`.

### 2.1 Core model

- **Grid**: odd dimensions `(2c+1)×(2c+1)`; walls are full cells (not thin edges) so
  Bresenham rays, fog, and movement share one representation.
- **Generation**: iterative recursive-backtracker carve from the center cell; then
  *braiding* (removing a % of dead ends with a loop carve) so monster dodging is
  possible when monsters are enabled.
- **Start**: nearest odd floor cell to the exact center. **Exit**: the border floor
  cell with maximum BFS distance from start (farthest point → hardest race).
- **RNG**: string seed → FNV-1a hash → mulberry32. Same seed + dims ⇒ identical
  maze, entities, and (local-mode) bot path. Monster wander RNG is a separate stream
  (`seed+":monsters"`) — positions may vary with real-time tick interleaving; the
  page says so.
- **Fog**: per-cell state `unknown | remembered | visible`. Vision radius R ∈ 5..15
  (default 9), recomputed on every position change: for every cell in the disc, walk
  the generalized Bresenham line from the player; mark cells revealed; **stop the ray
  at the first wall** (the wall cell itself becomes visible; nothing behind it).
  Diagonal step with both orthogonal neighbors walled = corner-blocked (no leaks).
- **God mode**: renders the full map + all entities regardless of fog; a run finished
  with god mode on is marked `assisted` and excluded from the winner verdict.

### 2.2 Bot decision ladder (the demo of the engine pattern)

Evaluated top-down each step; the first rung that fires decides:

1. `fight` — mid-combat: hold position (2.0 s static count).
2. `rule.exit` — standing on exit → step out, run complete.
3. `bfs.exit` — exit cell revealed → shortest known path to it.
4. `rule.flee` — monster threat on/near the chosen path → safe path or retreat.
5. `bfs.loot` — revealed loot remains (objective on) → nearest safe loot.
6. `bfs.frontier` — nearest revealed-but-unexplored walkable boundary cell.
7. `abstain` — no reachable frontier (never expected on a connected maze; reported,
   never guessed).

Tie-break policy: direction order N,E,S,W, then lowest BFS cost, then (x,y). When the
top two candidates are **equal-cost at a junction** and the WASM engine is enabled,
the choice goes to the engine as a real `choice` request; rung shows `engine.tiebreak`.

### 2.3 Entities & objectives (optional, parametric)

- **Loot** (toggle, count 0–16, default 8): on floor cells ≥ 3 from start, dead-end
  preferred. Picked up by stepping on. Disabled ⇒ metric 0 everywhere.
- **Monsters** (toggle, count 0–10, default 3; speed 1–5 cells/s, default 2; aggro
  range 2–10 cells, default 5; wander randomness): idle = random walk (bias against
  reversing); player within aggro range **with Bresenham LOS** ⇒ chase via BFS.
  Collision (same cell or cell swap) ⇒ **fight**: both frozen, 2.0 s static countdown
  with flash/shake animation (respecting `prefers-reduced-motion`), then the monster
  is driven off (removed). Fights cost time, never lives.
- **Bot avoidance**: threat cells (aggro'd monster + neighbors) get prohibitive BFS
  cost; fights happen only when unavoidable.

### 2.4 Scoring rubric (user-editable JSON)

```json
{ "version": 1,
  "criteria": [ { "tag": "time", "weight": 3 }, { "tag": "explored", "weight": 2 },
                { "tag": "loot", "weight": 1 }, { "tag": "fights", "weight": -2 } ],
  "tiebreakers": ["time", "steps"] }
```

- Tags: `time, steps, explored, loot, fights`. Weight −3..5 (0 = ignored; negative =
  penalty axis). Editor: drag palette chips into the rubric list, per-tag weight
  steppers, live JSON view, reset-to-default. Disabled objectives appear locked.
- Normalization per map (pars known at generation): `time`/`steps` = `1 − min(m, 4·opt)/ (4·opt)`;
  `explored` = revealed floor ÷ total floor; `loot` = collected ÷ total;
  `fights` = `1/(1+fights)`. Aggregate = `round(100 · Σ wᵢ·nᵢ / Σ|wᵢ|)`.
- **Leaderboard (session-only)**: rows tagged `BOT` / `YOU`, columns result, time,
  steps, explored %, loot x/y, fights, score. Disabled objectives render `0`.
  Winner verdict only when both runners share seed+config stamp.

## 3. Controls panel

Canvas centered (default 600×600; board 480/600/720; maze cells 13/17/21/25/29 →
grid 2c+1, cell px derived). Panel: player coordinates + status + rung chip; seed
(shown, editable); Generate / Restart same seed / Run bot / Play yourself / Pause /
God mode toggle; vision radius slider 5–15 (default 9); bot speed 2–60 steps/s;
objectives group (loot + count, monsters + count/speed/aggro + wander randomness,
braid %); rubric builder; leaderboard; decision trace.

## 4. Phases & atomic tasks

**P0 — plan** (this doc; task #1).

**P1 — maze gen** (task #2): RNG; grid model + validate config; backtracker carve;
braid; start/exit. ✚ same seed ⇒ identical grid; all floors reachable; border intact
except exit; start is center-area floor; braid % reduces dead-ends monotonically-ish.

**P2 — Bresenham + FOV** (task #3): all-octant integer line; ray walk w/ wall-stop +
corner-block; disc iteration; fog state transition rules. ✚ octant sweep vs reference;
wall blocks behind-wall cell; wall itself revealed; radius bounds; revealed set only
grows; radius 5/9/15 behave.

**P3 — bot ladder** (task #4): known-map builder; BFS; frontier set; rung functions;
decision record {rung, dir, note}; headless `solveMaze()` loop. ✚ property: bot
escapes on every seed×size×braid sample; steps < 6× optimal; identical seed+config ⇒
identical local-mode path; rung order respected.

**P4 — entities & combat** (task #5): placement; pickup; monster FSM (wander/aggro/
chase); fight resolution (enter, be-entered, swap); threat map. ✚ aggro needs range
AND LOS; chase reduces distance; all three collision shapes trigger exactly one
fight; fight completes at 2.0 s and removes monster; loot pickup increments.

**P5 — rubric + metrics** (task #6): schema validation + clamp; per-tag transforms;
aggregate; winner + tiebreakers; disabled-objective zeroing. ✚ golden-value tests;
weight-0 exclusion; negative weights penalize; tiebreakers order respected; malformed
rubric → default rubric (documented, never thrown).

**P6 — tests green** (task #7): `tests/maze_test.mjs`; `just site-test` extended to
run both files; all green.

**P7 — shell** (task #8): DPR canvas; fog/god renderer; entity sprites (shapes, no
assets); fight animation; keyboard + on-screen dpad; rAF loop; lifecycle + timer;
bot tick. ✚ manual review + Playwright.

**P8 — UI wiring** (task #9): panel bindings; modes; leaderboard render; drag-drop
rubric editor + JSON view; WASM import + engine tie-break + fallback + trace panel.
✚ every control affects the sim; verdict appears for same-config pairs.

**P9 — site integration** (task #10): try.html section, honest-notes card, styles
(dark + light), AGENTS.md page-map line, PROJECT_STRUCTURE site tree.

**P10 — verification & ship** (task #11): full test run; Playwright drive (bot
escape, god toggle, manual move, induced fight, rubric drop, leaderboard rows);
screenshots dark+light; requirements checklist audit; conventional commit; push
branch to `gitforge` + `origin` (never main directly).

## 5. Risk register (honest assessment)

| Risk | Mitigation |
|---|---|
| Bresenham corner leaks look buggy | corner-block rule on diagonal steps, tested |
| Bot looks dumb if engine picks badly | engine only breaks *equal-cost* ties — never affects efficiency |
| WASM fails to load | maze works 100% local; engine checkbox disabled with note |
| Rubric editor fiddly on mobile | chips also clickable (drop = click-to-append); JSON view editable path stays |
| Perf at 59×59 grid + 15 vision | FOV only on move (O(R³) worst ≈ 30k cell-visits, trivial); full redraw per rAF is ≤ ~3.5k rects |
| Score formula feels arbitrary | formula + normalization pars printed under the editor |
| Fairness bot vs human | same seed, same fog, same vision; god-mode runs marked assisted and excluded from verdict |
| Reduced motion / a11y | matchMedia gate on shake/pulse; canvas has role/aria-label + aria-live status; full keyboard play |

## 6. Decisions log

- D-MS-1: walls-as-cells grid (odd-grid backtracker), not thin-edge maze — one
  representation for rays, fog, movement.
- D-MS-2: per-destination-cell Bresenham rays (literal reading of the vision spec),
  not DDA shadowcasting — the algorithm is the requirement.
- D-MS-3: engine scope = equal-cost junction tie-breaks only (honest, zero
  efficiency risk, real decisions).
- D-MS-4: leaderboard session-only per operator instruction — no localStorage, no
  persistence.
- D-MS-5: rubric arithmetic stays in tested pure JS; no claim the WASM engine scores.
