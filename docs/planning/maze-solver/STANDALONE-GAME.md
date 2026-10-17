# STANDALONE-GAME.md — "Codifier: Deep Maze" (future separate project)

Status: **spec only — nothing here is built.** This is the documented answer to
"what would the full standalone game look like, with the demo as the starting
point?" It exists so the ideas survive the demo thread; do not treat any of it
as a promise on the site or in the repo's plans.

## 1. What carries over from the demo, as-is

| Demo artifact | Fate in the standalone game |
|---|---|
| `site/maze-core.js` | The seed. Pure logic, zero DOM, 42 node tests: seeded RNG (FNV-1a → mulberry32), walls-as-cells generation + braiding, generalized Bresenham FOV, BFS, the decision ladder, entity/combat tick, rubric scoring. A standalone game promotes this to a real module (ES module or WASM) with the same test discipline. |
| Fog model (unknown / remembered / visible, wall-stop rays, corner-block) | Unchanged — it is the game's core verb. |
| Versus mode (two run states over one maze, zero information leak) | Becomes the *actual* multiplayer model (§4), not just a ghost. |
| Rubric JSON + drag-and-drop editor | Becomes the "league rules" system: community-hosted rule sets, verifiable score claims. |
| Chests / monsters / fights placeholders | Real subsystems (§3). |
| Honesty posture (session-only scores, assisted runs score nothing, trace of every decision) | Becomes anti-cheat design (§5). |

## 2. Game structure

- **Runs → expeditions → campaigns.** A run is one maze. An expedition is a
  chain of 3–5 mazes with carried-over resources; dying ends it. A campaign is
  the meta-progression shell (unlockable biomes, objectives, bot rivals).
- **Biomes** = generation parameter sets + palettes: Vault (the demo's clean
  teal grid), Overgrowth (braided, vine mazes that slowly re-wall corridors),
  Deep Dark (vision 5, monsters aggro on sound), Clockworks (timed doors,
  the timer mechanic promoted to a puzzle verb).
- **Objectives** graduate from toggles to level grammar: "exit in N steps",
  "collect ≥ k of j loot", "open the sealed chest (two monsters chase)",
  "never be seen" (ghost mode), "escape before the timer".
- **Progression**: 5–8 hand-tuned intro levels with fixed seeds (teach verbs:
  fog, chests, fights, the timer), then infinite seeded runs with daily/weekly
  seeds (same seed worldwide, leaderboard per seed — deterministic, therefore
  honestly comparable).

## 3. Combat and monsters (the demo's explicit placeholder)

The demo freezes both parties for 2 s with animations only. The standalone
game replaces the freeze with a **static-swap combat mini-decision**:

- Combat is itself a ladder decision, on brand: each round (0.5 s) the player
  picks from ≤ 4 tactical options (strike / guard / feint / flee); the monster
  picks from its own policy table. Resolution is a small deterministic matrix
  with per-monster weights — no RNG hidden from the player; the matrix is
  shown in the bestiary.
- **Injury, not death**: losing a fight costs carried loot and adds a limp
  (speed 70% for the rest of the floor). Death is a campaign-level event only.
- **Monster variety** = parameter + one behavior flag each, all reusing the
  demo's FSM hooks: `aggroRange/speed/wander` (the demo's dunes), `sentinel`
  (patrols a loop, never de-aggros), `mimic` (renders as a chest), `swarm`
  (low aggro, mutual call-in), `wraith` (ignores walls for LOS, blocked by
  lit cells).
- **Stealth as a first verb**: monsters hear sprinting and chest-opening
  (the demo's chest pin becomes a risk window, not just a cost).

## 4. Multiplayer

The demo's versus ghost already proves the hard part: **the maze is shared,
the simulations are not** — each runner owns its own monster/fight/chest
state over the same grid, so there is no information leak to police.

- **Race (async, shipped first)**: weekly seed; your ghost is recorded as a
  decision trace (rung + direction per step — a few hundred bytes) and
  replayed locally on other players' clients. No server simulation at all.
- **Duel (sync, later)**: both runners live in one maze; seeing the other
  runner's glow (not fog) is the demo's rule. Monsters track whichever
  runner they can see. First to exit; loot is contested by presence.
- **Co-op (latest)**: shared fog — either runner's revealed cells are
  visible to both; fights can be tag-teamed (second player joining a fight
  halves its duration). Requires the same no-server trick: each client
  simulates both runners from exchanged move traces.

## 5. Anti-cheat by construction

Deterministic seeds + pure-logic core means a score claim is a **trace**:
(seed, config stamp, move list). Anyone can re-simulate the trace locally and
verify the score — the rubric engine already scores a run from its state, so
verification is `simulate(trace) → score`. God-view runs are marked and
excluded (the demo's `assisted` flag), timers are simulated inside the core,
and the engine's rung trace doubles as a plausibility fingerprint.

## 6. Engine hooks (the OpenCodifier tie-in, kept honest)

The demo's honesty line stays: the maze bot is a hand-written ladder; the
WASM engine only breaks genuine equal-cost ties. The standalone game keeps
that shape but widens the engine's real surface:

- **Difficulty as policy objects**: the runtime's policy schema
  (`min_confidence`, `verify_below`, `abstain_below`) maps naturally onto bot
  personas — a "cautious scout" is `abstain_below: 0.4` on frontier commitment
  plus wider threat margins; a "speedrunner" accepts ties locally.
- **Rubric = decision**: score evaluation can be expressed as the same
  request document the playground uses, so "which run wins under my rules"
  is itself an auditable, trace-producing decision.
- **Never**: neural-net pathfinding, cloud calls, or pretending the WASM
  engine plays the maze. The ladder is the product; the engine is the referee.

## 7. Tech shape

- Same constraint as the site: static files, zero network at rest, all
  computation in-tab. Packaging: PWA first (installable, offline), Electron/
  Tauri shell later if storefronts matter.
- `maze-core.js` → `src/core/` with the test suite carried over and grown;
  renderer stays canvas 2D (the demo's renderer already handles 59×59 at
  60 fps; biomes are palette + generation swaps, not renderer rewrites).
- Audio: the demo's lazy WebAudio beeps become a real bus with a music mute
  default (SFX opt-in stays a good default).
- Save data: `localStorage` for campaign progress (fine — it is not scoring
  data); leaderboard claims stay session/trace-based per §5.

## 8. Rough build order (if this ever becomes a project)

1. Promote `maze-core.js` to the standalone repo with its tests (day one).
2. Combat matrix + bestiary (replaces the freeze), injury rules.
3. Campaign shell + 6 tutorial levels (fixed seeds).
4. Async race multiplayer + weekly seeds + trace verification.
5. Biomes 2–3, sync duel, co-op.
6. League rules (community rubrics) + verified-score board.
