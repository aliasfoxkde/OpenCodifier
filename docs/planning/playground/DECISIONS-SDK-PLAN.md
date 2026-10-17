# The Playground & the Decisions SDK — plan of record

Status: **approved direction, build in progress** (2026-10-08)
Owner: maze-solver lane
Landing page: `site/playground.html` · games: `site/<game>.html` + `site/<game>-core.js` + `site/<game>.js`
SDK: `site/sdk/decisions-sdk.js` (global `DecisionsSDK`)

---

## 1. Vision

The maze solver proved the pattern: a small, honest game where the OpenCodifier
engine's role is **visible, auditable, and never faked**. The playground turns
that one-off into a **library** — racing, pong, pacman, life, a fighter, and a
quiz — all built on one shared runtime, the **Decisions SDK** (plural: it is a
kit for building decision-driven things, not one decision).

The SDK is a first-class deliverable, not scaffolding. It ships in the repo for
developers, and its usage stories intentionally reach beyond games:

| Surface | What the SDK gives it |
|---|---|
| **Games** (playground) | loop, RNG, config, scoring, engine bridge, UI shell, session board, live stats |
| **Testing** | seeded determinism — a fixed seed is a fixture; same seed, same game, same verdicts |
| **CI/CD** | headless node suites run the real SDK + engine per commit; a game run IS an integration test of the decision runtime |
| **Decision-making generally** | typed `choice`/`boolean`/`score` requests with calibrated confidence, deterministic traces, and abstention — the same contract the site's WASM module and the server runtime speak |

## 2. Non-negotiable principles (carried from the maze lane)

1. **Honest engine role.** Every game documents exactly what the engine decides
   and what plain code decides. Rung chips / trace panels name the mechanism
   per decision. If the engine is off or unavailable, the game never stalls and
   never pretends it asked.
2. **Deterministic-first.** Seeded RNG, fixed timestep. The engine is only ever
   consulted where a genuine tie or genuine uncertainty exists — never for
   decoration.
3. **Zero network, zero ML, zero server.** The WASM module is bundled under
   `site/wasm/`. Nothing a visitor does leaves the tab.
4. **Session-only by default.** Leaderboards and stats never persist. The one
   exception, user-authored quiz decks, is an explicit opt-in save
   (localStorage) with JSON export/import — user content, not telemetry, and
   clearly labelled.
5. **No dark patterns in the copy.** Cards say "in the workshop" until the game
   is real. Live pages claim only what the tests verify.
6. **Repo hygiene.** No versioned files, no placeholders, shared code extracted
   rather than duplicated, conventional commits on a branch (origin `main` is
   ruleset-protected).

## 3. Research summary (what this design is grounded in)

- **In-repo prior art.** `site/maze-core.js` (pure, DOM-free simulation:
  PRNG → map gen → FOV → decision ladder → scoring, 51 tests) and
  `site/maze.js` (DOM shell: accordion/tabs/overlay/quick controls, rAF loop,
  WASM bridge, 19 shell/link tests). The SDK extracts the *recurring* parts of
  both; the game-specific parts stay per-game.
- **Decision runtime contract.** The engine speaks `{ state, questions,
  policy, metadata }` → per-question typed decisions with confidence reports
  and a deterministic trace; abstention is a successful outcome. Games map
  their situations onto that contract; they do not invent side channels.
- **Genre mechanics** (standard, reimplemented cleanly rather than cloned):
  pong (paddle kinematics + reward shaping), cellular automata (B/S rule
  strings), arcade maze chase (scatter/chase/flee mode timers over a tile
  graph), pseudo-3D road racing (segment projection + curve accumulation —
  the classic OutRun-style renderer), fighting (move frames, hit/hurt boxes,
  habit histograms for an adaptive opponent), quiz (choice-over-options with
  confidence display). Nothing needs assets we cannot generate procedurally;
  sprites are drawn on canvas, keeping the zero-external-requests rule.
- **Ecosystem note.** The deterministic-core + shell split matches how the
  wider harness tests things (headless node suites for logic; browser drives
  for wiring). The SDK keeps that seam deliberate: **every game must be
  playable headless**, because that is what makes a game run usable as a CI
  test of the runtime.

## 4. SDK architecture (`site/sdk/decisions-sdk.js`)

One IIFE, one global `DecisionsSDK`, DOM-free except the explicitly-marked
`shell` section. Extraction order keeps the maze green at every commit.

### 4.1 Core (DOM-free — moves from maze-core)

    DecisionsSDK.rng(seed)            // → () => float [0,1)   (mulberry32, as today)
    DecisionsSDK.hashSeed(str)        // stable string → uint32
    DecisionsSDK.clamp(v, lo, hi)     // config clamping
    DecisionsSDK.normalize(cfg, spec) // spec-driven: {key: {type, min, max, def, bool}}
    DecisionsSDK.rubric               // normalizeRubric, scoreRun, resolveWinner (moved)
    DecisionsSDK.sessionBoard()       // { record(row), rows(), filtered({who,result,sort}),
                                      //   clear() } — memory only, dies with the tab

### 4.2 Sim (DOM-free)

    DecisionsSDK.Sim                  // fixed-timestep accumulator driven by the host's rAF
    DecisionsSDK.graph                // bfsDistances-style helpers generalized per game

### 4.3 Engine bridge (the one sanctioned WASM call path)

    DecisionsSDK.engineBridge({ toggleEl, statusEl, importBase })
      .ensure()        // lazy dynamic import of ./wasm/opencodifier_wasm.js
      .choose(request) // → { decision, confidence, trace } | null (any failure = null)
      .status          // 'off' | 'loading' | 'ready' | 'unavailable'

Rules: one shared module instance per page; every call site records the ask in
the game's trace; 5 consecutive failures flip the toggle off (the maze's
proven behavior); abstention/failure always has a local fallback decided at
the call site. The bridge also meters every call (count, wall time) into the
game's stats overlay.

### 4.4 Shell (DOM-building helpers — moves from maze.js)

    DecisionsSDK.shell.h(tag, attrs, ...kids)         // the h() element factory
    DecisionsSDK.shell.accordion(items, {openFirst})  // single-open group (as built)
    DecisionsSDK.shell.tabs(panes, {first})           // notes/engine/rubric pattern
    DecisionsSDK.shell.overlayBoard(board, filters)   // transparent leaderboard over the canvas
    DecisionsSDK.shell.statsOverlay(meters)           // latency / decision-time panel
    DecisionsSDK.shell.quickControls(defs)            // mode/reset/new/pause row
    DecisionsSDK.shell.kvRow(pairs)                   // status readouts

The maze migrates onto the SDK in the same series (maze-core re-exports from
the SDK where a function moved verbatim; behavior-identical, suites must stay
green before and after each move). No big-bang rewrite.

### 4.5 Stats & metering (cross-game, per operator request)

A stats button on every game opens a transparent overlay of **measured**
numbers, never asserted ones: FPS (avg/1% worst), frame time, bot decision
time per tick (avg/max, samples), engine request count and total/avg wall
time, vision/sim step time, render time, and steps-per-second. Metering lives
in the SDK (`DecisionsSDK.meters()` — monotonic clock, rolling windows) so
every game reports the same categories; the maze adopts it first.

## 5. Game specs

Every game: page `site/<game>.html`, logic `site/<game>-core.js` (headless),
shell `site/<game>.js`, tests `site/tests/<game>_test.mjs` (+ shell suites on
the shared DOM stub), leaderboard session-only, options in the accordion,
info in tabs, stats overlay, honest rung chips throughout.

### 5.1 Pong — `pong.html` (engine paddles, user-written rewards)

- **Mechanics:** classic pong; ball speeds up per rally; paddle AI tracks
  with capped speed so misses stay possible.
- **Engine role:** each paddle tick the *paddle policy* scores candidate
  target positions; a genuine tie between reachable intercepts (or
  reward weights making two options equal) escalates to `score` requests.
  With engine off, the same candidates are scored by the plain weighted
  formula — identical arithmetic, visible rung chip.
- **Reward functions (the point):** user-editable weights per event —
  `pursue`, `center_after_hit`, `risk_near_wall`, `bounce_setup`,
  `taunt_distance` — plus per-bot aggression. Two bots playing each other
  under different weights makes incentive design *watchable*.
- **Options:** sides (human/bot per side), ball speed, paddle size, win
  score, reward weights per side, engine toggle.
- **Scoring:** matches won → session board row (weights in the stamp so
  verdicts only compare like-for-like).

### 5.2 Game of Life — `life.html` (rules + engine flavor)

- **Mechanics:** toroidal or bounded grid; rule strings (`B3/S23` classic,
  `B36/S23` highlife, custom B/S entry); speed, cell size, paint tools.
- **Engine role (honest, small):** a classifier rung — every N generations
  the engine receives population/birth/death time-series and returns a
  `choice` among labelled trajectories (still / oscillator / glider /
  exploding / dying) shown as a chip; ties between candidate "interesting
  next seeds" (density/entropy scored) may be engine-broken. Life itself is
  pure rules — never engine-decided.
- **Options:** rule preset/custom, grid size, speed, wrap, pattern library
  (glider, pulsar, r-pentomino, acorn, gun), random density, engine toggle.
- **Scoring:** none (sandbox) — the board records seed → verdict experiments.

### 5.3 Pac-maze — `pacman.html` (engine ghost logic)

- **Mechanics:** the SDK map generator produces the board; pellets, 4 power
  pellets, frightened mode on power, lives, score.
- **Engine role:** each ghost's mode at a genuine decision point asks
  `choice` over personality candidates (direct pursuit, cut-ahead intercept,
  patrol, retreat) scored by ghost-specific reward weights. Frightened
  retreat is a rule, not a choice — honest labelling per move in the trace.
- **Options:** ghost count/speeds, fright duration, pellet fill, maze seed/
  size/braid, human or bot player, engine toggle.
- **Scoring:** pellets + ghosts eaten + level; session rows stamped
  seed|size|ghosts|speeds|fright.

### 5.4 Apex Line — `racing.html` (2.5D sprite racer)

- **Mechanics:** segmented road with curves/hills projected
  pseudo-perspectively (segment scale + curve accumulation; sprite-scaled
  roadside objects and opponent cars, all procedurally drawn); laps, lap
  timer, off-road slowdown, collisions.
- **Engine role:** AI racers pick a *line* (apex / inside / outside /
  slipstream) each stretch via candidate scoring; overtakes are genuine
  `choice` points when gap and speed make two lines equal. The player's
  suggested-line chip uses the same call — visible either way.
- **Options:** track seed/length/curviness/hills, laps, opponents count/
  skill, traffic density, difficulty (grip/top speed), engine toggle.
- **Scoring:** lap + total time, DNF on quit; session board with track stamp.

### 5.5 Sparring Partner — `fighter.html` (opponent that learns you)

- **Mechanics:** side-view 1v1; move list (jab, hook, block, duck, dash,
  special with meter); hit/hurt windows; rounds, timer, health.
- **Engine role (the learning):** the opponent keeps a decayed histogram of
  the player's recent moves by situation (range band × player state). At
  each decision tick the histogram generates candidate responses with
  expected values; close EVs go to the engine as `choice` with the histogram
  in `state` — the engine reasons over *your recorded habits*. A
  reset-learning button exposes/erases the table (auditability).
- **Options:** opponent adaptivity (histogram decay), aggression, difficulty
  (reaction delay), rounds, engine toggle.
- **Scoring:** rounds won, damage; session rows stamped with adaptivity.

### 5.6 Ask the Engine — `quiz.html` (the engine answers, really)

- **Mechanics:** multiple-choice questions; every answer is a **real engine
  `choice` request** over the option candidates with the question text in
  `state` — the confidence report and trace are shown per answer. No local
  simulation dressed up as engine output; the deck carries the keyed
  answers, the engine's pick is displayed next to them, and agreement is
  scored honestly (the engine can be wrong and the scoreboard shows it).
- **Modes:** *speed-run* (engine answers stream, live scoring), *audit*
  (step through: question → engine answer + confidence + trace → keyed
  answer → agree/disagree), *compete* (human first, then engine on the same
  question; accuracy + median speed on the scoreboard).
- **User questions:** add/edit/delete in a JSON deck editor (validated:
  2–6 options, exactly one keyed, non-empty text); decks export/import as
  JSON; explicit opt-in save to localStorage, labelled as local-only.
- **Built-in deck:** general knowledge + logic puzzles sized for the
  engine's lexical rungs (the honest framing: rungs 1–4, zero-ML — the
  engine's accuracy ceiling is part of the demo).
- **Scoring:** accuracy, streak, median answer time; you-vs-engine rows.

### 5.7 MicroWorld — `microworld.html` (the tiny AI that runs the world)

The strongest demo of the set: **one small world, many deciders** — instead of
ten disconnected games. A 2D top-down, fully deterministic micro-world with
20–100 autonomous creatures, food, water, enemies, shelter, gatherable
resources, a day/night cycle, weather, limited energy, and simple inherited
personalities (bold/shy, hoarder/sampler). The player can drop food, scare,
feed, or follow creatures. The simulation is plain code; **the decision
visualization is the game**: every creature continuously asks "what should I
do next?" and the runtime answers over the fixed verb set — `EAT`, `DRINK`,
`FLEE`, `FIGHT`, `EXPLORE`, `REST`, `GATHER`, `RETURN_HOME`, `FOLLOW`,
`IGNORE` — with the chosen verb drawn over the creature and a click-through
trace for any single one.

- **Decision budget (honest by design):** creatures re-decide at ~1 Hz (not
  per frame), urgency rules pre-filter candidates (a creature adjacent to a
  predator gets a 2-candidate question, not 10), and creatures are batched
  into the runtime's native multi-question requests (up to 32 questions per
  WASM call). The stats overlay shows calls/s and latency so the cost is
  never hidden. With the engine off, the same normalized candidate scores
  from the creature's own needs vector decide — same arithmetic, labelled
  `needs` instead of `engine` on the chip.
- **World:** seeded tile map (SDK mapgen), hunger/thirst/energy/rest drives,
  personality multipliers on candidate scores, day/night light scalar,
  weather effects on move cost, shelter tiles, enemy wander/chase.
- **Engine role:** per-creature `choice` (batched); population-level
  `score` requests for the "what should the world throw at them next"
  narrator events (weather shift, predator arrival).
- **Options:** creature count, spawn traits, day length, weather rate,
  resource density, predator count, player interference tools, decide-rate,
  engine toggle, focus-follow (pin the trace to one creature).
- **Scoring:** sandbox like Life — but the board records "colony
  survival time" experiments per seed/config stamp.
- **Why it matters:** it demonstrates the runtime at its actual job — many
  small typed decisions with calibrated confidence, batched, auditable,
  deterministic — at a scale a single maze scout cannot show.

### 5.8 Choose Your Door — `door.html` (one decision, fully shown)

The "can it decide?" rung of the suite (brainstorm §4): three doors, one
character, revealed reasoning. The character's health, weapon, knowledge,
inventory, objective and risk tolerance form the state; the doors are the
candidates; the decision, confidence, candidate scores, and full trace are
the UI. **Personalities are policy, not code paths** — coward/greedy/brave
differ only in the weights the same request carries, which is the point the
demo makes: `decision policy + state → decision`. Abstention is reachable
(equiprobable doors + no objective → the engine declines and the UI says
why). Options: personality, state knobs, door count (3–5), series play.
Tests: same state+policy → same door (determinism); different policies
diverge on the fixed state; abstention case fires; no engine when toggled
off (ladder scores only).

### 5.9 RPSLS++ — `rpsls.html` (pattern-hunting opponent)

Rock-paper-scissors-lizard-spock with an extendable move graph (brainstorm
§5): each move beats a set, loses to a set, cycles must stay balanced when
the user adds moves. The opponent builds a frequency/markov table of your
history and decides per round: counter-the-frequency, counter-the-markov,
random-mix, or mirror — close expected values escalate to `choice` with the
history table in `state`; with engine off the same EVs decide. Player modes:
you-vs-bot, bot-vs-bot (watch two predictors duel), streak/counter stats,
history strip. Tests: move-graph closure (every pair resolves), predictor
beats a fixed-cycle bot above chance, determinism per seed, EV-tie path.

### 5.10 The Prisoner's Dilemma — `dilemma.html` (decisions inside a system)

Iterated prisoner's dilemma (brainstorm §11): COOPERATE/DEFECT, editable
payoff matrix, and a strategy roster — trusting, suspicious, retaliatory,
forgiving, greedy, random, tit-for-tat variants — plus the **engine agent**,
which decides per round from the interaction history via `choice` over
{cooperate, defect} with the recent history in `state`. Headless tournament
mode runs N×N round-robins (100k interactions) and charts cooperation rate
over time — the demo of decisions whose future state depends on previous
decisions. Options: rounds, payoff matrix, roster selection, noise
(misheard moves), engine toggle for the engine agent. Tests: classic
results reproduce (defect-vs-always-cooperate exploits; tit-for-tat ties
itself at full cooperation), matrix edits change outcomes, engine agent
stays within the roster contract.

### 5.11 Traffic Simulator — `traffic.html` (cars AND lights decide)

Grid roads, intersections, and queues (brainstorm §9): cars decide
STOP/GO/TURN/WAIT/REROUTE from local state (light phase, gap, queue);
lights are autonomous agents deciding their phase from queue counts and an
emergency-vehicle flag — `choice` over {keep, switch} when counts are
close, rules when they are not. Congestion and spawn-rate sliders make the
decisions visibly change; throughput (cars/min, wait percentiles) in the
stats/readout. Tests: no gridlock deadlock under seeded spawns, emergency
preemption fires, determinism, lights respect minimum-phase.

### 5.12 Ant Colony — `antcolony.html` (emergence from simple decisions)

Pheromone grid, nest, food piles (brainstorm §7): each ant decides
SEARCH/GATHER/RETURN/FOLLOW_TRAIL/EXPLORE/DEFEND at ~1 Hz with rule
prefilters (carrying food → RETURN is a rule, not a question); trail
strength decays and diffuses; the collective trail network is emergent —
no pathfinding code. Decision counter (decisions/s, total, avg latency)
makes the scale claim tangible. Options: ant count (50–1000), evaporation,
deposit rate, food piles, obstacles, decide-rate. Tests: seeded run forms
a trail (food-to-nest visit rate rises), no ant asks about RETURN when
empty-handed, batching budget respected, determinism.

### 5.13 AI Civilization in 60 Seconds — `civ60.html` (decision propagation)

A tiny civilization sim (brainstorm §8): citizens decide WORK/EAT/SLEEP/
BUILD/FARM/FIGHT/TRADE/EXPLORE/HAVE_CHILD/FLEE from needs and events; the
player injects events (food shortage, raid, harvest) and watches the
decision cascade the brainstorm describes — hunger ↑ → farming ↑ → wood
production ↓ → construction slows. Event log + per-resource time series;
decisions batched like MicroWorld. Options: citizen count, event injection,
speed (the "60 seconds" is a fast-forward scale), engine toggle. Tests:
shortage event shifts the decide distribution toward FARM/EAT, population
recovers or collapses coherently, determinism, batching budget.

### 5.14 Tower Defense — `tower.html` (both sides decide)

Grid paths and waves (brainstorm §6): enemies decide ADVANCE/TARGET
TOWER/RETREAT/REGROUP/alternate-route (diversion paths unlock under
pressure); towers decide targeting policy — nearest/weakest/strongest/
fastest/save-special/use-special — as candidates scored per frame-tick
snapshot, with genuine ties escalated. Wave composition, tower loadouts,
and speed controls; the commercial-looking demo of continuous
decision-making at volume (decisions/s counter). Tests: seeded waves are
clearable by the default loadout, tower policy changes change outcomes,
enemies divert only when the alternate route exists, determinism.

### 5.15 Spacecraft Emergency — `spacecraft.html` (the abstention showcase)

Telemetry-driven crisis decisions (brainstorm §10): fuel/hull/engine/
temperature/distance/crew feed CONTINUE/REDUCE POWER/CHANGE COURSE/
SHUTDOWN ENGINE/RETURN HOME/EMERGENCY. Most situations resolve
deterministically; the designed-in drama is **conflicting telemetry →
low confidence → ABSTAIN → escalate to the player**, whose resolution is
logged into the trace. Scenario seeds, drift/instrument-failure rates,
difficulty; every decision shows rung + confidence + trace. Tests:
injected sensor conflict produces abstention (not a guess), escalation
resolves the loop, deterministic per seed, decision latency shown.

### 5.16 Rubik's Cube Solver — `cube.html` (the solver is code; the coaching is the engine)

- **Mechanics:** a 3×3×3 cube in CSS 3D (27 cubie divs, face-layer turns via
  transform transitions, drag-to-orbit, no WebGL). Standard notation
  (U D L R F B, prime via shift/right-click), free play with a move-history
  list + undo, scramble (seeded, SDK rng), and a step-through solver with an
  annotated coach panel. Facelet-level state lives in `cube-core.js` with a
  validator that refuses impossible states WITH the reason (color counts,
  permutation parity, corner/edge orientation sums) — the validator is a
  deterministic gate, shown, not hidden.
- **Solver ladder (deterministic, in-repo, no vendored solver):**
  1. **Layer-by-layer** (shipped first): white cross → F2L corners →
     second layer → 4-look last layer. ~100–120 moves, but every step is
     human-followable and gets a named algorithm + a "why this case" note —
     that readability is the product.
  2. **Old Pochmann** (second): piece-by-piece buffer cycling — the most
     "watchable" solve, one decision per piece.
  3. **Kociemba two-phase** (stretch, cite cube.js ldez/cubejs as prior art):
     ≤22 moves but opaque and needs a few seconds of table-building at load;
     only worth it as a "machine line" comparison column, never the default.
- **Engine role (honest, small — the solver itself is NEVER engine-decided):**
  - **Equal-path tie-breaks:** LBL regularly offers two equally short
    inserts/OLL variants; when the code's cost table ties, that exact
    decision is a real `choice` question (candidates = the tied algorithms,
    with their case descriptions) — rung chip per step.
  - **Scramble curation:** candidate scrambles scored on measured features
    (faces touched, solved-piece count after N moves, cross disruption);
    genuine ties engine-broken.
  - **Redundant verifier:** a boolean "does this state satisfy the solved
    invariant?" cross-check beside the code's own check — labeled as a
    redundancy demo, never authoritative.
- **Options:** solver method, turn animation speed, color scheme, scramble
  length, coach detail (none/names/full explanations), engine toggle.
- **Scoring:** human solves timed with TPS; code solves scored as move count
  vs the method's baseline (LBL ~110, OP ~180, Kociemba ~22); session rows
  stamped `method|scramble-seed|move-count|engine-ties`.
- **Honest edges:** the engine never invents moves — it only ranks among
  algorithms the deterministic layer already proved legal and equal-cost;
  invalid cube states are refused with the validator's reason, not fixed.
- **Tests:** validator rejects each broken invariant class; LBL solves all
  of N seeded scrambles and every intermediate state stays legal; tie
  escalation hits the engine only on equal-cost ties; determinism per seed.

## 6. Testing & CI

- **Headless-first.** `<game>-core.js` must not touch the DOM. Node suites
  drive full games via seeded sims, exactly as `maze_test.mjs` does today:
  property tests (map validity, reachability), determinism (same seed → same
  run), ladders (bot escapes/wins within N× optimal), honesty invariants (no
  engine call when the toggle is off; every trace line names a rung;
  abstain/failure falls back).
- **Shell tests** reuse the DOM stub (`tests/maze_dom_stub.mjs`, to be
  promoted to `tests/dom_stub.mjs` with maze suites importing that path).
- **Drives.** Playwright (Python) walks each live page on the LAN review
  server: boot, one full bot game, options round-trip, overlays, zero
  console errors.
- **CI/CD.** The site suite runs in the GitForge pipeline (`.gitforge.yml`
  node job) on every push — a green pipeline means every shipped game *played
  itself to a legal finish* on this commit. The SDK's decision contract is
  thus integration-tested by its own games; the release gate stays the repo's
  standard gates (never a game score).
- **Docs gate.** `site/AGENTS.md`, `docs/PROJECT_STRUCTURE.md`, and this
  directory update in the same series as the code they describe.

## 7. Phasing (systematic order, each phase shippable)

| Phase | Contents | Accept when |
|---|---|---|
| **P1 — SDK extraction** | `sdk/decisions-sdk.js` core+rubric+board+engine bridge+shell+meters; maze consumes it | all maze tests green before AND after; no duplicated implementations left |
| **P2 — Playground landing** | `playground.html` + nav/sitemap cross-links | cards honest; deep links resolve (DONE for the landing; cards flip per phase) |
| **P3 — Pong + Life** | cheapest full games prove the SDK end-to-end | both suites green; drives pass; cards flip to "playable" |
| **P4 — Pac-maze** | reuses SDK mapgen + board | bot player completes a level headless |
| **P5 — Apex Line** | renderer + track gen + racer ladder | headless bot finishes N laps; smooth-render smoke in drive |
| **P6 — Sparring Partner** | histogram + engine-over-habits | learning measurable headless (adaptivity on blocks more jabs) |
| **P7 — Ask the Engine** | real-answer quiz + deck editor + compete | every displayed answer traces to a logged request; deck validation tests |
| **P8 — MicroWorld** | the many-decider world (batched requests, needs-vector fallback) | 100 creatures decide at 1 Hz headless within the latency budget; trace per creature |
| **P10 — Cabinet games** | Door → RPSLS++ → Dilemma → Traffic → Ant Colony → Tower Defense → Spacecraft → Civ-60 (ascending size; brainstorm `GAME_IDEA_BRAINSTORM.md`) | each: page + headless core + suite + card flipped; cabinet trio (Door/RPSLS/Dilemma) first |
| **P11 — Sweep** | stats overlay in every game, docs, drives refresh, audit, branch commit | review server verdict from the operator |

## 8. Risks & standing decisions

- **Scope discipline:** six games is a program, not a batch. Phases land
  independently; the playground advertises only what exists.
- **Engine-cost honesty:** WASM calls stay sub-millisecond and sparse
  (decision points, not per-frame). Per-game budgets noted in each spec; the
  stats overlay makes the real numbers visible to any visitor. MicroWorld is
  the stress case — its batching + decide-rate budget (§5.7) is what keeps it
  inside that rule at 100 creatures.
- **Sprite work:** everything procedural on canvas; if a game needs real
  sprites later, they are generated at build time into `site/assets/` —
  never hotlinked.
- **Quiz epistemics:** the deck's keyed answers are the ground truth the
  scoreboard scores against; the engine's answer is *displayed as the
  engine's*, with confidence — agreement is a measured quantity, never
  assumed.
- **Persistence:** nothing new persists without the explicit opt-in rule
  (§2.4). The quiz deck save is the only planned exception.
