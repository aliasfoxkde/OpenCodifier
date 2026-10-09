# Playground — engine-wire findings (2026-10-08, Pong integration)

All facts below were measured in Chromium 141 against the real
`sdk/../wasm/opencodifier_wasm.js` module served from `site/` — not read
from source and assumed. Repro pages: `http://127.0.0.1:8777/pong.html`.

## Wire contract (measured)

- `WasmEngine.decide()` takes a **JSON string** and returns a **JSON
  string**. Request shape that works:
  `{ state: { text, facts: {} }, questions: [{ type: "choice", id, text,
     candidates: [{ id, description }] }],
     policy: { min_confidence, verify_below, abstain_below, risk } }`
- Response: `{ answers: [{ type: "choice", question_id, choice, confidence,
  distribution }], confidence: { calibrated_confidence, entropy, margin,
  ood_score, top_probability }, metrics, outcome: "accept", trace }`.

## Defects found & fixed

1. **`decide(plain object)` HANGS the engine** — a JS object passed
   straight to `decide()` freezes the main thread >20 s (page killed by the
   probe; no error, no return). Only strings are safe. The maze never hit
   this because it stringifies by hand. **Fix:** the SDK bridge
   (`engineBridge.choose`) stringifies on the way in and parses on the way
   out — the single choke point every game goes through.
2. **Pong tie-break used the wrong question type.** `type: "score"` in this
   engine is an ordered-severity scale (`levels: [{label}, …]`, ≥2, answer
   is a distribution over levels) — NOT a candidate ranking. Sending
   `options` fails with `schema.invalid_value: missing field 'levels'`.
   **Fix:** ties ask `type: "choice"` with `candidates`, read
   `answers[0].choice` (same shape as the maze tiebreak).
3. **`facts` values are tagged enums** (`RawFactValue`): a bare integer in
   `facts` is rejected ("invalid type: integer `4`, expected adjacently
   tagged enum RawFactValue"). Games send `facts: {}` — text carries the
   context.
4. **`import()` inside the SDK resolves against the SDK file's URL, not the
   document's** — measured 404 at `/sdk/wasm/opencodifier_wasm.js` when the
   base was `./wasm/`. A bare `"wasm/"` never resolves at all (module
   specifier rules). The bridge's default `../wasm/` is correct for pages
   at the site root: games should pass NO `importBase` until a page lives
   in a subdirectory.

## Verification

- `tests/pong_test.mjs` (11 tests): tie request is a `choice` with the tied
  candidate ids; abstain / unknown-id / bridge-null all fall back to the
  local formula and the match still finishes.
- Live page: all reward weights set to 0 → every think tick is a genuine
  tie → 8 s of play produced 934 engine-tie escalations, under-readout
  "last tie: engine", no console errors.

## Apex Line integration (2026-10-08, racing)

5. **A bare `{questions:[…]}` request THROWS inside WASM `decide()`** —
   missing `state`, question `text`, or `policy` rejects the request. One
   throw is survivable, but the SDK bridge's fail limit is 5 consecutive
   throws: the bridge flipped ready → "unavailable" ~30 s into the first
   race while the game kept running on local weights only. Pac-maze never
   hit this because it has sent the full contract from the start; racing
   was the first game to send a truncated one. **Fix:** every engine call
   carries the full pac-style contract — `state: {text, facts:{}}`,
   question `text`, and `policy: {min_confidence, verify_below,
   abstain_below, risk}` — and the wire shape is pinned in
   `tests/racing_test.mjs`. **Rule for every future game:** never send a
   bare `{questions}`.

## Verification (racing)

- `tests/racing_test.mjs` (13 tests): track determinism + closed elevation
  loop; standing grid; off-road/bump physics; personality divergence;
  traffic never finishes; and the tie path asserts the full wire contract
  (non-trivial `state.text`, `facts` deep-equals `{}`, question `text`,
  `policy.risk === "low"`), engine rung credited on accept, weights-argmax
  fallback on abstain/unknown-id/null.
- Live page (seed "ties", bot driver): full race to P3, 1:09.7, best lap
  33.0 s, **15 engine-answered ties**, chip stayed "engine: ready"
  throughout, scoreboard row logged, zero console errors.

## Sparring Partner integration (2026-10-08, fighter)

- **Browser-only defect: `g.prng` existed only in `simulateMatch`.** The
  shell's "driver: you" button routes the player through `botInput`, which
  dereferences `g.prng` — undefined in a normal `createGame` game, so the
  console filled with "rng is not a function" the moment the driver was on.
  Node tests never saw it because they always drive through
  `simulateMatch`. Fix: `createGame` seeds the prng itself
  (`"player:" + stamp`, deterministic), `simulateMatch` no longer
  duplicates the assignment. Lesson: any input path a shell can switch on
  must work on the object the shell actually holds, not the one tests
  build.
- **Dead-on-the-shelf threshold: `TIE_EPS = 120` produced ZERO natural
  engine escalations in a live 1132-tick browser bout** (engine calls 0
  with the engine ready and on) — the headline mechanism was unfalsifiable
  in product. Calibrated on 520 bot-vs-bot decisions: top-two EV gaps min
  6 / p25 664 / median 1579 milli-points; **TIE_EPS = 300** puts 8.8% of
  decisions in the contested window (~4–5 real escalations per 3-round
  bout). Copy updated everywhere (core header, accordion, page hero/card),
  the question state wording now derives from TIE_EPS instead of a
  hardcoded "0.12", and `lastDecision.evGap` is exposed so the next
  calibration is a replay away.
- **Verification (fighter):** 138/138 node tests (14 fighter core, incl.
  the two learning-convergence proofs and the pinned full wire contract);
  browser smoke extended to fighter.html (playground card "playable",
  canvas, non-loading chips, accordion ≥ 3, sliders ≥ 4, scoreboard/stats
  open-close-Escape, learning accordion reset click, driver on, trace +
  under-row populated) — SMOKE OK with zero console errors. Live 1311-tick
  bout: 225 moves observed, 40 habit-table decisions + **2 engine
  near-ties both answered and honored** (decisions — engine 2), engine
  chip ready, habit table 6 rows in the stats overlay, ~50 fps. Screenshots: dark, dark+stats, light, mobile 390px.

## Ask the Engine integration (2026-10-08, quiz)

- **SDK defect (all games): `engineBridge.ensure()` resolved `null` while a
  load was in flight.** `if (engine || busy) return Promise.resolve(engine)`
  handed concurrent callers an already-resolved null instead of the
  in-flight load — any game that asks the engine at page load (quiz's
  speed-run answers question 1 the moment the page renders) graded it
  "engine unavailable" forever. Fix: `ensure()` stores and returns the
  in-flight `pending` promise, cleared on settle so a failed load still
  retries on the next call. Backward compatible; every game benefits.
- **Related shell defect (compete): the human's answer rendered the
  "shown" panel before the engine's row existed** — the engine's turn is
  async, so the panel read `results[len-1]` of an empty array
  ("Cannot read properties of undefined (reading 'pick')"). Fix: render an
  "asking the engine…" phase first, flip to "shown" after the answer
  lands; the shown branch also guards missing rows now.
- **The engine's ceiling on the built-in deck is MEASURED, not asserted**
  (real WASM, in-tab, seed oc-quiz): 12 questions → **10 abstained, 2
  answered — both wrong, both cognitive-reflection traps** (bat-and-ball
  "$1.10" at 97% confidence; bloops/razzies → "razzies" at 92%). The two
  systematic human heuristic traps are exactly where the lexical rungs
  guess confidently wrong; the confidence report sits beside each wrong
  answer, which is the demo. Median engine answer 1–2 ms after warm-up.
- **Verification (quiz):** 16 quiz-core node tests (154 suite total) —
  deck validation naming every editor failure, seeded order, the FULL wire
  contract pinned, grading (accept right/wrong, abstain, unavailable,
  non-candidate choice), scoring (accuracy over ANSWERED, abstain rate,
  streaks, median), compete isolation, audit judgment scoring, JSON
  round-trip, opt-in save/load with invalid-deck rejection. Browser smoke:
  full speed-run click-through to "run complete", board row recorded,
  stats overlay, audit ask → key → verdict, deck editor
  validate/template/load, compete (human click + engine follow), Esc
  closes — SMOKE OK, zero console errors. Live: engine chip ready, real
  requests 1–70 ms in-tab. Screenshots: dark, dark+audit, dark+deck
  editor, light, mobile 390px.

## Tower Defense integration (2026-10-08, tower)

- **Same-target policies are fake choices (measured, then fixed).** The first
  escalation measurement showed 75% of tower decisions at evGap exactly 0 —
  with one enemy in range, nearest/weakest/strongest/fastest all pick the SAME
  target, so the "choice" was an illusion and 15.8% of all decisions went to
  the engine with nothing to decide. Fix: candidates dedupe by target (same
  target = same action, keep the first in POLICIES order), and a
  single-action snapshot is never escalated (the self-gap `cands[0]-cands[0]`
  read as a perfect tie — a second fake). Honest rate after: 5.5-7.3% of
  decisions, ~0.7 asks/sim-second.
- **Saboteurs were dead on the shelf (measured, like the fighter's TIE_EPS).**
  Unarmored, a 40 hp saboteur next to the split dies to flanking fire at
  ~t31 (two towers in range, 12 dmg / 30 ticks each) while breaking a
  100 hp tower needs 50 ticks at 2 dmg/tick — the TARGET TOWER decision could
  never pay off. Fix: saboteurs take half damage (armored), which is also why
  the tower ladder's scoreTarget already prioritizes them (+200).
- **The first alternate route was a shortcut.** ALT totaled 1080 px against
  MAIN's 1440 px — "divert under pressure" would have been a reward. Redesigned
  as a 2160 px southern detour (1.5x the main road); the geometry test pins
  ALT.total > MAIN.total. Also pinned: enemies divert only near the junction
  (a mid-field divert teleports them back to the split).
- **Small core bug the tests caught:** `damageEnemy` flipped `diversionUnlocked`
  on accumulated damage even when `cfg.diversion` was off — gating moved inside.
- **Live WASM verification (25 s run):** 204 decisions total, 7.85 decisions/s
  of sim time, 21 near-tie asks, 10 answered by the engine (the runtime
  abstains on the rest — its prerogative, the rung records what happened),
  2 diversions taken, engine latency avg 5.6 ms / worst 36 ms, zero console
  errors. Node-side across seeds: escalation 5.5-7.3% of decisions.
- Verification: 18 core tests including the spec's four (seeded waves
  clearable by the default loadout; forced policy changes outcomes; no divert
  without the route; determinism), the full wire contract on the escalated
  request, and the no-bridge/broken-bridge degradation pair. Site suite 172
  green; smoke green including the tower block; screenshots dark/light/mobile.


## Ant Colony integration (2026-10-08, antcolony)

Spec: plan §5.12 — pheromone grid, nest, food piles; six decisions per ant
at ~1 Hz with rule prefilters (carrying food -> RETURN is a rule, not a
question); trail decays and diffuses; the network is emergent, no
pathfinding code; decisions/s counter; batching budget respected.

### The colony was dead on arrival (calibration lesson #3)
First full probe: **0 deliveries in 150 sim-s**, 3 gathers in 300 s, 118/120
ants parked in `explore`. Three mechanical causes, each invisible until
measured:
1. `explore` scored 450 flat — above `search`'s 300 baseline — so ants
   explored right past food they could smell; and explore's turn noise was
   +/-1.4 rad/tick, which at 60 Hz is pure rotation: ants jittered in place
   and never translated.
2. `search` steering (+/-0.15 rad/tick) was weaker than its own wander
   (+/-0.3) — even "searching" ants could not actually converge on a pile.
3. Pickup required stepping on the exact 10 px pile cell.
Fixes: correlated walks (explore +/-0.5, search +/-0.3), search pull
(+/-0.25, angle-normalized) > wander, pickup radius 12 px, search ev
300+500*food so the smell actually flips the argmax, deposit 2.0 / evap 0.4
chemistry. After: 56 gathers + 42 delivered per 150 s, trailMax 1.29.
The unit suites passed while the product was dead — the "trail forms" spec
test is what actually falsifies; everything else was green around it.

### engineCalls counted asks that never happened
`decide()` incremented the counter before checking whether a bridge
existed, so a no-bridge run reported asks. Gate: ask only when
`g.engineChoose` exists. The tower rule ("a no-bridge run never asks")
transfers; caught by test, not by eye.

### The batching budget is the game's headline mechanic
A 1000-ant colony at 1 Hz would be 1000 questions a second. The colony
holds ask-tokens that refill at `cfg.askBudget`/s capped at one second's
worth; a contested ant with no token takes the argmax rule and the rung
says so. Budget 0 = pure rules colony. Live default run (120 ants,
191 sim-s): 22,887 decisions at 119.98/s (== ants x rate), **322 asks vs
the 382 budget cap**, 80 engine-honored, avg engine latency 1.04 ms, tick
cost 0.16 ms, zero console errors.

### The engine refuses flat contests — measured, live
Of 322 live WASM asks only 80 were honored (25%): the runtime returns
non-accept outcomes when the distribution sits between `abstain_below`
(0.30) and `min_confidence` (0.55) — a flat 3-way behavior contest is
exactly that. So the engine arbitrates only the ties it can actually
distinguish, and every declined ask falls back to the argmax rule with the
rung reporting `rule`. This is the abstention story working in production,
not a defect.

### Discovery window shape
First ~50 s of a seeded run delivers ~0 (random discovery), then the
trail-fed flow dominates (0 -> 24 -> 18 per 50-s window on the calibration
seed; late > first window is the pinned assertion). "Visit rate rises" is
pinned as first-window vs last-window, not a monotonic slope — delivery
windows fluctuate.

### Shell traps hit again
Single-open accordion: `#antcolony-release` (Follow one ant section) and
`#antcolony-obstacles` (The colony section) both time out unless their
section head is opened first; opening one section closes the other, so the
smoke reopens before each. Follow-clicks must be element-relative
(`locator.click(position=...)`) — viewport coords go stale after the page
scrolls and one stray click landed on a nav link mid-loop.

### Verification
17 core tests (189 suite-wide, 0 fail) including the four spec-pinned:
trail forms (delivery rate rises), RETURN/GATHER never candidates across a
live recorded run, batching budget (400 ants, 1/s budget -> 61 asks max;
0 budget -> 0 asks), determinism. Smoke green through the ant block
(playground card, chips, engine ready, 4x speed, asks > 0, follow pin,
board/stats overlays, wall restart). Screenshots: dark / stats / follow /
canvas zoom (trail network plainly visible) / light / mobile — light and
mobile fine, canvas stays dark in both themes.

## AI Civilization in 60 Seconds integration (2026-10-08, civ60)

**The boom was a silent runaway.** First live run: default city hit the
POP_CAP 200 in ~150 sim-seconds (188 births, food 1304, wood 2347,
decisions/s 129) while all 19 core tests stayed green — the same
suite-green/product-dead lesson as the ant colony's dead pheromone field,
inverted: this time the product was too alive. Root cause chain, fixed in
three moves:

1. `HOME_COST` 5 → 12. A home is a project; wood becomes the rate-limiter
   (births gate on `pop < homes + 4`, so growth is gated on BUILD, which
   is gated on WOOD, which competes with FARM — the cascade is the game).
2. Birth-gate slack 8 → 4 and the housing loop re-checked. With slack 8 and
   cheap homes the gate never actually closed.
3. The rebalance exposed a **deadlock**: with `woodNeed = 1` under
   homelessness, work (ev 550) outscores build (ev 546) at the 5-homeless
   mark, so the city works forever, homes freeze at 7, the gate never
   opens, births = 0 across 150 s. Fix: build base 400 → 500 — build
   outranks work while homelessness is material, loses to it at a 1/12
   share, so the last home waits for the next birth: a self-balancing
   loop. Result: founding → pop 23 @ 60 s → stable plateau 28 (rules-only).

**The engine is a growth multiplier.** The rules-only plateau is 28; the
live WASM run at the same seed and window reaches pop 132 — 298 near-tie
asks, **100% honored** (the ant colony honored 25%): a civ's flat contests
are two-way ties the runtime can order, unlike the ant's true three-way
flats that land in the abstain band. Arbitrating farm/work ties toward
work compounds: more wood → more homes → more births. Recorded as an
honest edge, not a bug — and it forced the renderer honest: 64 grid
slots cannot hold 132 citizens, so the town square packs extra rings per
slot with shrinking dots. The canvas must hold everyone the city
actually has.

**Fixture traps worth remembering:**
- `STARVE_TICKS` was not exported; the test set `c.starve = CC.STARVE_TICKS
  − 30` = NaN, `NaN >= STARVE_TICKS` is false, nobody ever died. Export the
  constants the tests need to pin.
- `createGame({ citizens: 1 })` clamps to the floor of 4, and a seed-only
  config means the default 12 — fixtures must be explicit about which city
  they are starving.
- The `select()` shell helper never wired its change listener: antcolony's
  obstacles select **silently never restarted the colony** (the smoke
  assert passed vacuously). Fixed in both shells; the civ smoke now
  asserts the events-off select actually disables the injection buttons.
- `askBudget: 0` fell through `Number(0) || default` to 2 — a rules-only
  city is a real configuration, so normalization checks
  undefined/null/NaN explicitly, never falsiness.

**Verification.** 19/19 core tests, 208/208 full suite, smoke block green
(card, canvas, engine-ready, shortage → chronicle, 4× asks > 0, follow at
fixed canvas coords, release, board, stats incl. coherence row, events-off
disables buttons). Live WASM run, 149 sim-s: 8,953 decisions (60/s cum,
pop-weighted), 298 asks / 298 honored, coherent pop 132 = 12 + 120 births
− 0 − 0, engine avg 1.12 ms / worst 14.1 ms, tick 0.03 ms avg, zero
console errors. The chart shows the injected shortage as the food series
V-dip; screenshots dark/stats/follow/zoom/light/mobile verified. The
saturation steady state (build/work mono-culture once the city fills)
is visible in the zoom shot and recorded here as the honest edge it is.


## Traffic Simulator integration (2026-10-08, traffic)

Cars-rules / lights-decide split per plan §5.11. CARS: STOP (ahead), STOP
(red), GO, WAIT (>=3s), TURN (argmax over exit-arm queues), REROUTE — never
asked; the suite scans every escalated request and asserts only keep/switch
candidates appear. LIGHTS: 1 Hz re-score, rules first (min 6s / max 24s),
near-tie |gap| <= 120 to the engine under the city-wide budget.

Three core defects the suite/probes caught before the page ever shipped:

1. **Preemption lost races with the vehicle it exists for.** The emergency
   check lived inside the light's 1 Hz decide ladder, but the emergency at
   5 ticks/cell crosses the whole 12-cell horizon inside one decide period —
   a seeded probe run produced ZERO preemptions. Fixed: `preemptLights(g)`
   scans every tick (tickGame, before car ticks); the ladder notes it and
   moves on. A test now bounds the first preempt to the approach window.
2. **carsOnArm walked upstream.** It counted cars BEHIND the exit anchor —
   the approach queue — so the downstream jam the turn choice exists to
   avoid was invisible to the argmax (a probe parked 6 cars on the straight
   exit and the car still scored straight as equal-best). Fixed: forward
   walk; "the queue on the street this exit enters".
3. **Wire fixture parked a ghost inside the box.** Eastbound car at
   (vx+2, hy+1) is INSIDE the 2x2 intersection -> queueCounts scored it
   inX (+200 to keep), gap 280, no ask. Park arms from vx+3 outward.

Reroute semantics pinned honestly: the REROUTE label fires only when the
straight arm wins the argmax AND its queue >= 5 — i.e. all arms jammed and
straight is least-bad. A jammed straight with open sides is an ordinary
turn (argmax already refuses it); all three fixtures pinned.

Smoke traps (recurring): the emergency button lives in a CLOSED accordion
section (single-open — reopen the head first); inserting the traffic card
made `.pg-where a` first = traffic, breaking the pong assert (scope card
asserts to their card); click-to-follow with a 14 px pixel hit-test is
unusable on sparse traffic — pickCar is now CELL-SNAPPED (nearest car-cell
within 2.5 cells), a product fix a human also needed.

Honest edges (in the page's Honest-edges card): honored rate 100% again
(115/115) — the runtime answers every keep/switch ask at confidence 0.7;
and QUIET boxes tie 300-vs-300 constantly, eating the ask budget (~1.35
asks/s of the 2/s default at the default spawn) — the ledger shows the
budget going to empty intersections, which is the truth of the scoring.

Default spawn bumped 18 -> 36 cars/min after the canvas shot: 18/min x
~11 s trips = 3 cars on a 12-box grid reads as barren; 36 = 7-car steady
state. moveEvery/tests unaffected (no test pins 18).

Verification: traffic core 16/16 (determinism, no-deadlock over 120 s,
min-phase gap scan excluding preempts, emergency window, budget<=1/s+2,
full wire contract, abstain/non-candidate fallback, 3-scenario reroute,
stamp, percentiles); full suite 224/224; SMOKE OK; live WASM: engine
ready, asks 115 / honored 115, 2 preemptions + vehicle cleared, 31.8
cars/min at 4x, tick 0.03 ms avg / 8.08 worst, engine 3.43 ms avg / 61.8
worst (first WASM call), zero console errors; screenshots dark/canvas/
stats/follow/light/mobile verified.


## Spacecraft Emergency integration (2026-10-08, spacecraft)

Core defects found by the tests themselves:

1. **Empty tank still delivered full thrust** — `speed = power*0.2` read
   `g.power` with no fuel gate, so a dry ship coasted home at cruise and the
   `adrift` outcome was unreachable (dry fixture ended `returned`). Fix:
   physics cuts the engines the tick fuel hits 0 (`power = 0` + "fuel
   exhausted — engines cut" log); `adrift` then fires naturally on the
   `fuel <= 0 && power <= 1` endGame.
2. **Garbage engine answer was treated as a refusal** — a malformed band
   answer (accept + non-candidate choice) routed into the "engine abstained"
   player escalation. House rule everywhere else: garbage is ignored, the
   argmax holds. Now only `outcome === "abstain"` escalates; a malformed
   answer falls through to the argmax rule rung.

Test-fixture lessons (not core bugs):

3. **The freeze pauses physics** — `tickGame` early-returns while a pending
   is live, so any long-run fixture must captain through escalations
   (`resolvePending` with candidates[0]) or the sim stands still forever.
   `drive(g, seconds)` helper does exactly that; the escalation SPEC walk
   iterates on `g.ticks`, not loop count (resolve iterations don't tick).
4. **An abort needs fuel-margin arithmetic, not vibes** — abort fixture:
   `dist=600, fuel=30` (home 400 away needs ~27.6 with the 1.15 safety
   factor; destination needs 41.4 → correctly unreachable).

Shell defects:

5. **Inject buttons disabled forever** — `wire()` runs before `newGame()`,
   so the one-time `setInjectEnabled()` saw `S.game == null` and disabled
   the buttons permanently. Fix: render() calls `setInjectEnabled()` every
   frame (the state it depends on — pending/over — changes under the loop).
6. **Panel rebuilt under the cursor** — the loop keeps rendering while
   frozen, and rebuilding the console buttons every frame races clicks.
   Fix: rebuild gated on a `reason|text` dataset key.

Smoke traps (all recurring): single-open accordion heads are TOGGLES —
guard the head click on the target's visibility or you close the section
you meant to open; a pending disables inject buttons BY DESIGN, so the
smoke captains through organically-raised pendings (up to 3 faults live at
once, re-escalating every 2 sim-s) before injecting; a difficulty-0.5
gauntlet at 4x can honestly END the ship — inject fire on a freshly
restarted flight; the stats regex must allow 1-3 chars between label words
("decisions — captain" is space+em-dash+space, `..` matches only 2);
latency rows render only after an uncertain-band ask, so assert them
conditionally; `python3 x.py | tail` reports TAIL's exit code — redirect to
a log and echo `$?`.

Honest edges (documented, not hidden):

- **The engine band can honestly see zero asks.** With two sensor faults
  live, agreement = 0.25^2 = 0.0625 collapses conf straight past the
  0.30-0.55 engine band into captain escalation (live run: 15 decisions,
  13 rule, 0 engine, 2 captain). The engine hears single-conflict or
  clean-telemetry narrow gaps only.
- **Rendering never touches g.rng** — the sensor strip shows the readings
  the ladder last decided on (a snapshot), because `readSensors()` consumes
  the RNG stream and a repaint that called it would desync the sim.
- **The freeze is total by design**: physics stops while the console waits.

Verification: 20/20 core tests in tests/spacecraft_test.mjs; full suite
244/244 (`npm test`); SMOKE OK rc=0 (full playground walk incl. fault →
CAPTAIN'S CALL → captain click → trace shows captain rung, stats counters,
fire injected + extinguished, board checkpoint, drift restart); live WASM
verify: engine ready, escalation panel shows reason + conf + both readings
of each disputed gauge, resolution on the captain rung, tick 0.00 ms avg /
0.09 ms worst, 0 console errors; screenshots dark/escalation/stats/fire/
canvas/light/mobile reviewed.


## The Prisoner's Dilemma integration (2026-10-08, dilemma)

Core defects found by the tests (both fixed):
- `tournament()` referenced `series` without declaring it — the series
  aggregation rewrite dropped the `const`. Syntax-check passes, first
  tournament call ReferenceErrors. Lesson: `node --check` catches
  syntax, not free variables; the test suite catches the rest.
- History tail sample was gated on `g.over`, but `g.over` is set AFTER
  the history block in `tickGame` — the tail sample could never fire.
  Keyed it on `g.round === g.cfg.rounds` instead, with a
  no-duplicate-tail guard (`rounds % 10 !== 0`) and a test for both.

Test-side traps (core was right every time):
- Grim trigger: moves are SIMULTANEOUS — grim decides before seeing
  the opponent's same-round move, so it cooperates THROUGH the round
  containing the first seen defection and defects from the next. The
  first probe said "wrong" and the second said the assertion was
  off-by-one (`slice(firstD)` → `slice(firstD + 1)`).
- Rungs count BOTH sides of every round: an engine-agent round-1
  refusal is rung rule ×2 (mood rule + the suspicious opponent), not
  ×1. Budget math: 80 rounds engine-vs-roster with budget 3 →
  rungs.rule = 2×80 − 3 = 157.
- Tokens are ONE per-match pool, not per-side: engine-vs-engine with
  budget 2 spends it on round 1 (one ask each side) and both finish on
  the mood rule. Documented in the honest-edges card, not "fixed" —
  per-match is the pinned design (it makes the budget a strategic
  resource, and engine-vs-engine sharing is the honest edge case).
- The test file has no `DK` binding — the SDK is read off globalThis
  (`globalThis.DecisionsSDK.makeRng`), unlike the cores which alias it
  at load.
- Mood rule round 1: empty seen window → cooperate. Refusal tests that
  asserted a round-1 defect were testing a mood rule that doesn't
  exist.

Shell: no new traps — spacecraft's lessons (engine-ready poll,
visibility-guarded accordion heads, pipe exit codes) transferred
whole. The tournament button runs synchronously (~ms for 28 pairings
at ≤100 rounds); no busy guard needed at this scale.

Live verification (WASM engine ready): default engine-vs-titfortat
spent all 4 asks into full cooperation (123/123 at r41, tokens 0);
tournament crowns GRIM TRIGGER 1866 over TIT-FOR-TAT 1810 across 28
pairings / 2800 interaction rounds / 28 engine asks — the classic
Axelrod ordering, emergent from the seeded round-robin; noisy
tft-vs-tft at 0.25 bleeds to ~48% cooperation with 38 logged flips,
visible in the ribbon garble ticks and the collapse-recovery chart.
0 console errors. Full suite 262/262, SMOKE OK rc=0.

Design note for the batch: this is the deliberate CONTRAST to
Spacecraft Emergency — there the engine abstains and a human captain
rescues it; here the engine agent IS the player and refusal falls to
a (deliberately stub) mood rule on the record. Between them the two
games demonstrate both refusal postures the SDK supports.

## RPSLS++ integration (2026-10-08, rpsls)

Core design flaws caught before they shipped:
- **A fair add is a REORIENTATION, not an append.** Adding a vertex to
  a tournament graph orients ALL its edges: `water beats rock+paper`
  is only valid if scissors/lizard/spock all gain water in their own
  beats. The first cut appended the newcomer and let every other pair
  fall unresolved — `validateGraph` correctly refused every add, which
  would have killed the headline feature. `checkAdd` now validates the
  rewritten graph (`reoriented()`), and the shell's live add works
  mid-match.
- **Strict regularity makes even rosters impossible.** A regular
  tournament (wins == losses for every move) only exists on odd
  counts, so with the classic fairness rule the add feature dies at
  6 moves. Fairness is now parity-aware: exact regularity on odd
  counts, |wins − losses| ≤ 1 on even ones; the add rule is "beat
  exactly floor(n/2) of the existing n".
- **The seed was cosmetically deterministic but causally inert.** Both
  duels with different seeds produced IDENTICAL summaries: the EV
  argmax never consulted the rng, so play locked into rng-free
  rhythms. Real pattern-hunters explore — a seeded 5% deviation from
  the argmax (recorded on the rung as "explored") makes seeds matter
  and stops the bot being counter-looped. The determinism test then
  became meaningful instead of vacuous.
- `predictorVotes` read `pred.freq.markov` — a move named "markov" —
  because `predict()` returns `{freq, markov, mirror, random}`. The
  markov vote and the blend were silently dead. Also: the in-place
  best/second scan never advanced `second` while moves[0] led; replaced
  with a sort by (−ev, roster order).
- `predictorVotes` called `rng()` out of scope (ReferenceError on the
  first bot decision) — the tests caught it on the very first run.

Shell: the arena labeled both duel sides "you"/"bot" — side names now
follow the mode (you/the-bot vs left/right), including the verdict
verb ("YOU WIN" vs "LEFT WINS") and the strip legend. Smoke lesson:
after a successful add the form resets, so the post-add refusal
assertion must re-enter an id before expecting the even-roster note.

Live verification (WASM engine ready): an all-rock human lost 1–4
with 7 ties — the bot's log spams "lizard beats rock", frequency
6.86 / markov-1 6.13 trust earned, 6 engine asks on the close calls
(a one-trick human keeps EVs near-tied). Water joined mid-match
("r12 water joined the graph, beating rock, paper", moves 6, graph
verdict "every pair resolves, all moves fair"). Bot duel: 0 asks,
tokens untouched. 0 console errors. Full suite 272/272, SMOKE OK
rc=0.
## Choose Your Door integration (2026-10-08, door)

Task #37, plan §5.8 — the "can it decide?" rung: one room, 3–5 doors,
one character, the whole decision on screen.

**Shipped.** `door-core.js` (globalThis.DoorCore): personality = POLICY
(four weight vectors over the same scorer — coward danger×3, greedy
reward×3, brave seeks danger, steady balanced), no personality code
paths. State modifiers scale the SAME weights (hurt/unarmed fear danger
more, knowledge shrinks mystery's pull, torches soften effort).
Confidence = softmax(scores, T=0.8). Full wire contract (id
"door-pick", candidates = live doors, policy risk field rides the
personality: coward low / brave high). Balanced rooms (identical
traits) + no objective = the canonical dead-heat refusal. `series()`
plays headless rooms; `divergence(a,b)` counts per-room disagreements.
`door.js` shell: DOM-first (no canvas — the trace IS the UI), door
cards with trait bars + score/p, verdict panel with rung badge + why,
the exact wire request printed in a details panel, series panel with
per-personality bars + divergence line, session scoreboard + stats.
14 core tests; suite 286/286; smoke rc=0.

**Defects found and fixed during integration:**

1. *Series tally poisoned by tie-breaks.* Every 4th series room is the
   balanced dead-heat setup, and the ladder's door-order tie-break gave
   door-1 ~50% of wins under EVERY personality — the aggregate bars
   looked identical across policies and the core demo point
   (policy → behavior) appeared dead. Room-level truth: coward vs
   greedy diverge on 90% of decided rooms. Fix: `series()` counts
   ladder-decided dead heats as `tieBreaks`, not tally wins; the shell
   shows both plus the divergence metric. Lesson: aggregate tallies
   hide per-room divergence when any deterministic tie-break fires at
   volume — report the divergence, not just the marginals.

2. *Engine answered "verify" on every default room → the engine lane
   never showed an honored decision.* The runtime's confidence on the
   plain state text sat below verify_below (0.4), so the demo only ever
   showed refusals. Fix: ship the ladder's softmax probabilities in the
   state text (same pattern as rpsls shipping EVs) — the default room
   now honors ENGINE door-3 live. Seeds with razor-thin gaps still
   verify → nobody moves, which is the honest runtime posture, and the
   why echoes the outcome either way.

3. *Refusal why mislabeled non-accept outcomes.* A "verify" outcome
   landed in the generic "named nothing live" branch. The why now
   echoes the actual outcome JSON and, on a dead heat, says so
   ("equiprobable, no argmax").

4. *Shell bugs:* engineBridge starts "off" — `setEnabled(true)` at
   build was missed (chip sat at "off" for 30s in smoke); smoke hit the
   single-open accordion twice (series button hidden; then a guarded
   head-click still toggled the open section closed — guard on
   `is_visible()` of the target control, per the recurring trap).

5. *Test-side:* 3-door uniform softmax is exactly 1/3 — ABOVE the 0.30
   abstain line — so the canonical 3-door refusal fires on the dead
   heat, not the policy line (5-door variant lands at 0.2 < 0.30 and
   exercises the line too). `Object.assign({health,weapon}, fresh)`
   argument order silently clobbered the overrides under test.

**Live verification (review server :8777, WASM engine ready):** default
room → ENGINE rung, STEADY takes door-3 (ladder p 0.378), wire panel
shows state text + 3 candidates + policy; balanced + no objective →
ABSTAIN, "Nobody moves.", dead-heat why, no chosen card; series 200
rooms × 4 personalities ladder-only, coward/greedy disagree in 131/150
(87.3%), 50 tie-breaks counted separately; engine latency avg ~67–80
ms; scoreboard rows recorded; 0 console errors; light + mobile shots
clean.

**Design note.** This game is the suite's pure decision surface — no
canvas, no tick loop; the decision, confidence, scores, and trace are
the UI. It is also the contrast arm to Spacecraft Emergency: there the
engine abstains and a human captain rescues it; here abstention is the
product itself, and the demo metric is that policy (not code paths)
diverges behavior at 90% of rooms.

### 2026-10-08 — Rubik's Cube (§5.16): solver correct, the bug was a double-apply

**Core (`cube-core.js`).** 54-facelet URFDLB model; deterministic LBL
solver (cross → corners → middle → EO → CO → CP → EP → AUF). The gate is
**300/300 seeded scrambles solved** (+ determinism, engine-last-option
choose, refusal-falls-back, scrambleLen 1–40, solved-input-0-moves).
Solver phases are empirically complete, each verified by full state-space
enumeration of its phase: EO exact 192 configs (two algs × U-alignments,
0 stuck), CO 648 intrinsic-coordinate configs (multi-source BFS from all
24 orientation-solved targets, diameter 3, κ twist-offset table derived
by simulation and consistency-asserted), CP 23 states (A/A⁻¹/T/Y
two-sided conjugates — bare corner transpositions are ODD, unreachable by
A-perm 3-cycles alone), EP all 12 A4 states (Ua/Ub/H two-sided
conjugates, 0 stuck).

**The one bug worth recording.** The whole gate was red for sessions
because of a single structural defect: `push()` (the step recorder)
already advances `cur = compose(cur, toks)`, and six call sites
*additionally* pre-composed the same moves manually — corners/middle
ejects, EO, CO, CP, EP. Every greedy alg was applied twice. The CO
"anomaly" (rounds continuing at dist 0, exit state ≠ last round state)
was exactly this: the post-compose debug print read the single-applied
state, the loop condition read the double-applied one. Rule for any
solver with a record-and-advance helper: **one owner of state
advancement, ever** — either the helper composes or the caller does,
never both.

**Two more caught by the spec tests, not the gate.** (1) pll-edges
candidates `U^j ∘ alg` with a prefix-only alignment: the alignment
rotates the (already home) corners away and the edge-only alg never
restores them — `edgesPlaced` is corner-blind so the loop exited
"victorious" on a broken cube, and the silent AUF loop returned NOT
SOLVED without a throw. Fix: two-sided conjugates `U^j ∘ alg ∘ U^(4−j)`
(corners invariant), verified 0-stuck over all 12 A4 states, plus a loud
`last-layer alignment failed` guard after AUF. (2) `Z_PERM =
Ua ∘ Ub` was **identity** (Ub is Ua's inverse) — a mislabeled no-op
candidate that had been along for the ride; adjacent double swaps are
two conjugated U-perms composed, which the greedy reaches in two rounds
anyway, so Z was removed rather than faked. (3) The greedy tie paths
recorded `source: "asked"` even when the engine refused, and oll-edges
*recorded* the engine's choice but played `good[0]` — cosmetic choice.
All four greedy phases now share `pickTied()` with tryAll's
engine/refused/roster trichotomy and honor the chosen index.

**Shell (`cube.js` + `cube.html` + CSS).** CSS-3D cube: 26 cubie divs ×
6 box faces, every face at ±HALF along its normal (coincident interior
faces identically dark — no visible z-fight), facelet binding is purely
coordinate-keyed (`faceletAt(face, x, y, z)` through the cubie grid, no
row/col table to get wrong). Drag-orbit, 18 move buttons + undo, turns
are instant with a face flash (no tween drift between shown and solved
state — honest-edges note in the page copy). A hand move mid-plan drops
the rest of the plan on purpose. Step-through coach: per-step rung chips
(rule / roster / engine — greedy steps now carry their tie through
`push`), click-to-jump, auto-solve. **Seed UX (task #43 pattern):**
numeric seed input, empty = fresh `DK.randomSeed()` each game, rolled
value written back into the field so every run stays reproducible; dice
button; deep-link `?seed=`. **Fullscreen (task #44 pattern):**
`DK.shell.fullscreen(el, btn)` helper added to the SDK shell kit
(requestFullscreen + label flip + fullscreenchange listener) — cube uses
it; the per-game pass rolls it out everywhere. Reset-params button
restores `DEFAULT_CONFIG` (#42 pattern).

**Verification.** `tests/cube_test.mjs` 9 tests (pinned LL algebras by
simulation: Ua/Ub/H F2L+corner purity, T/Y transpositions, A-perm edge
neutrality, inverse pairs; 30 seeded solves with phase-order assertion;
choose/refusal/determinism/config clamps). Full suite **295/295**. Live
probe on :8777: 26/54 counts, orbit drag, hand moves, 23-step plan,
auto-solve → solved chip + uniform faces in screenshot (yellow U, green
F, red R), rung chips {rule: 16, roster: 4}, board row recorded,
fullscreen enter/exit, 0 console errors, 390px layout wraps. Smoke block
added to pg_smoke.py and passing.

## 2026-10-09 — Library redesign + chrome rollout (task #45/#42/#43/#44)

**Audit first, then fix.** A button-walk probe (load page, capture
pageerror/console-error, `toDataURL` twice 700 ms apart for canvas
liveness, then click EVERY button once and re-check errors) over all 16
game pages found exactly one shell defect: **life.js** called
`S.board.add(...)` on a `sessionBoard()` that only exposes
`record/all/clear/filtered` — and once fixed, a second latent bug
surfaced: `recordExperiment` stored no `ts`, so `renderBoard` crashed on
`r.ts.toLocaleTimeString()`. Two defects, one page, found only because
the probe exercises every button. The probe itself (game_audit.py
pattern) is the cheap regression net for this whole site class.

**Flip cards.** Rebuilt the playground library as 17 CSS-3D flip cards
(badge + title + one-liner on the front; full description + play button
on the back), a category chip row (decisions-on-display / living worlds /
arcade & action / puzzles & classics / workshop) and a search box,
combined in one filter pass. Three lessons:

- The grid-stack flip (both faces `grid-area: 1/1` + preserve-3d)
  double-paints in Chromium even when computed styles are perfect
  (checked headed under xvfb, not just headless SwiftShader). The classic
  absolute-face structure (`position: absolute; inset: 0` + explicit
  container height) renders clean. Correctness also never rides on
  backface-visibility: a `visibility` swap timed to pass 90°
  (`transition: visibility 0s 0.45s`) guarantees one face paints.
- Fixed card heights clip; JS sizes each card to
  `max(front.scrollHeight, back.scrollHeight) + border` (the +border is
  the `.card` 1px box — scrollHeight/clientHeight differ by exactly 2px).
  Zero overflow at 1600/1024/768/390.
- The fixed left toc-rail (labels expand to ~14rem + always-visible
  quick links) overlaps the first column on any `page-wide` page — the
  1120px landing container only clears it by centering. Fix:
  `.page-wide .section { padding-left: 16.5rem }` at >1200px, verified
  no rail/card intersection at 1600/1366/1280.

**Chrome rollout pattern.** Two new SDK helpers, one wiring site per game:

- `DK.shell.numericSeed(input, diceBtn, onChange)` — forces
  `type="number"`, guarantees the box always carries a numeric seed
  (fresh page → random 6-digit; emptied → repaired on next `get()`),
  dice rerolls + fires onChange. `get()` is the only read path.
- `DK.shell.resetParams(btn, root, apply)` — snapshots every form
  control at wiring time, restores on click, re-fires input/change so
  the game's own listeners apply the values.
- `DK.shell.fullscreen(el, btn)` now tags the target `.game-fs`, so one
  CSS rule gives every game the same viewport ownership.

pong + life converted as the reference (dice reroll verified live,
fullscreen enter/exit verified, zero page errors). Tests:
`tests/sdk_shell_test.mjs` pins the helper contracts with minimal
element stubs (7 tests; note node ≥18 has a real `Event` global — stub
`dispatchEvent` must tolerate non-writable `target`).

## 2026-10-09 — Pac-Man arcade rework + chrome rollout complete (task #45 close-out)

**Pac-Man themed after the real game** (user: "the pacman clone is not very good …
it should be themed after the actual pacman game"). Two boards now:

- **Classic (default)** — `pac-core.js`: the real 28×31 arcade layout as
  `CLASSIC_ROWS` ('#'/./o/-/space; door tiles are grid code 2, `passableC`
  wraps x on the row-14 tunnel and keeps doors solid). Ghost FSM
  house→leaving(scripted to the door column)→active→eyes(BFS to the door-out
  tile (13,11))→entering(scripted) with staggered releases 0/1s/4s/7s;
  scatter/chase clock (7-20-7-20-5-20-5 s, shorter at level ≥2, permanent
  chase after); classic targeting (blinky=pac, pinky=4 ahead, inky=2-ahead
  vector doubled from blinky, clyde shy within 8); fright with the
  turnabout + 200-400-800-1600 doubling chain; fruit at 70/170 pellets;
  READY gate (`readyUntil`), lives, level clear with fresh pellets.
- **Generated** — the previous SDK-maze mode, unchanged; weighted-intent
  personalities live there. Existing tests pin `board: 'generated'`.
- The decision layer survives on the arcade board: at every junction the
  open directions are scored by squared distance to the ghost's target and
  a *genuine* equidistant tie escalates to the engine (test: blinky on the
  ring midpoint vs the player straight below → left/right both d²=37).
- Shell: board select dims generated-only sliders (`.is-off`), canvas
  672×840 with 3 HUD rows (1UP/HIGH SCORE), lives+LEVEL footer, READY!/
  GAME OVER under the house, walls drawn as connected blue outline strokes
  (edge-trace with neighbor-extension joins — inset segments extended to
  tile ends when the neighbor wall's matching edge is also exposed),
  pink gate across the door tiles, cherries at (13,17).
- Bot fix: `botIntent` read `gh.state`; classic ghosts carry `phase` —
  threats were invisible and the bot walked into hunters (score 70 → 6320
  after `phase === "active" || state === "active"`).
- Classic tick lesson: the generated tick writes `p.want`; my first classic
  tick only passed `want` to reversal — intent was silently dropped and the
  player wall-held. Mirror the generated line (`classicReverse; p.want =
  want`) whenever adding a second tick path.
- Tests: 21 in pacman_test.mjs (14 generated pinned + 7 classic: board
  shape/tunnel wrap/house stagger/scatter clock/fright+combo/engine
  tie/bot determinism). Suite 302 → 309, all green.
- Probe gotchas: `#pac-lives` is inside the closed Run accordion (open it
  before `fill`); tile-locked ghosts decide only at tile arrivals (~15
  ticks at 4.2 t/s) — loop until the observable fires; a roaming ghost
  teleported onto the player carries `prog` remainder and steps off before
  collide — reset `gh.prog = 0` in the fixture.

**Chrome rollout final** — all 16 games have numeric-seed kit (random on
boot, dice reroll, repair-on-garbage), reset-params, and tagged fullscreen.
Third agent (spacecraft/dilemma/rpsls/door) verified live; door targets
`root` directly (no `.game-wrap`), matching life.js. `npm test` 309/309.

## 2026-10-09 — chrome overhaul round (user rejection fixes)

User verdict: "everything is a little janky" — games not running with the bot,
logo blurry/off-center, no visible scroll-following nav, blank space under
How it works, zero padding on ecosystem flip cards, plus three feature asks
(Benchmarks rename, hosted-API pricing/waitlist, Decide on its own page).

Root causes found by browser audit (python playwright, 1600px + 390px):

1. **Duplicate `id="ladder"`** (section anchor AND inner rung container).
   `site.js` `getElementById("ladder")` returned the SECTION, so all 7 rung
   buttons were appended after `.ladder-wrap` — the left ladder column stayed
   empty (the "excess blank space under How it works"). Fix: rung container
   renamed `#ladder-rungs`; section keeps `#ladder` for anchors/toc.
   Lesson: getElementById silently returns the first match — duplicate ids
   fail at runtime, not at parse.
2. **Unlayered `*` reset nuked tw.css paddings.** styles.css carried
   `* { margin:0; padding:0 }` unlayered; tw.css ships `.eco-face` etc. in
   `@layer components`, and unlayered beats layered at ANY specificity —
   every Tailwind padding/margin on the page was 0 (flip cards, mobile nav
   menu). Fix: reset moved into `@layer base`. Lesson: mixing a Tailwind
   output with a hand CSS file requires layer discipline, not load order.
3. **The toc rail was hover-revealed dots** (labels `max-width:0`) with no
   global gutter — invisible as a menu, painting over hero/ladder/eco on
   non-`.page-wide` pages, and reduced-motion users got no rail at all.
   Fix: always-visible 13.5rem panel rail (progress hairline, scrollspy,
   quick links), reduced-motion users included (it is navigation), and a
   global `body { padding-left: 16rem }` gutter at ≥1201px that replaced the
   `.page-wide` special case.
4. **Grid tracks with auto minimums blow out on phones.** `1fr` tracks
   (hero mobile, `.grid-2`, `.ladder-wrap`, `.pg-grid`, `.cloud-wrap`) and
   `.hero-visual`'s auto margins (fit-content sizing) let `pre`/`code`/
   table min-content push the page to 126px overflow at 390px. Fix:
   `minmax(0, 1fr)` for stacked tracks + explicit `width:100%` on
   `.hero-visual` + `.mz-scorebar { flex-wrap: wrap }` + hide the nav CTA
   under 480px (menu panel carries those links).
5. **Dead-on-arrival games.** Liveness probe (canvas/DOM signature at t0 vs
   t0+2.5s) found: pacman/racing/fighter waited for a human (bot toggle
   existed but defaulted off), rpsls defaulted "you vs bot", life booted an
   empty paused court, cube booted scrambled and idle, door booted undecided,
   quiz waited. Fix: bot/driver/mode defaults flipped (`!== false` reads),
   life seeds a boot soup (unless `?pattern=`), cube `startAuto()` after the
   opening scramble, door `decideNow()` on boot. quiz stays turn-based (the
   engine's verdict shows next to the dealt question).

Feature adds this round: nav label "The board" → "Benchmarks" (all pages),
hosted-API section on index + api.html ($0.50 per 1M decisions, coming-soon
badge, waitlist = mailto until signup logic exists), `decide.html` carved out
of playground.html (console + demo.js + live chips moved; api.html handoff and
legacy try.html redirect rewired; `?run=` deep links preserved), playground
reordered games → library → decide-link with the maze wall-of-text collapsed
into `<details>`.

Verification receipts: 310/310 node tests; 20-page audit clean (zero console
errors, zero h-overflow, rail present iff ≥2 sections, no rail overlap);
390px sweep zero overflow on all probed pages; 16/16 games open alive or
intentionally turn-based.

## Round 3 — paused boots, real pause, MicroWorld, civ60 legibility (2026-10-09)

User report: "most of the games are still non-functional… need bots by default,
start in a paused state, with a clear play button"; MicroWorld's flipped card
had no way to open it; civ60 "makes no sense"; only Life and Maze "work".

6. **Cosmetic pause was the whole bug class.** `S.running` existed in state,
   readout and button handlers, but the rAF loop never gated on it — pause
   buttons did nothing in 10 of 12 loop shells (life was the only real one,
   matching the user's "works" report). Fix: uniform loop guard
   `if (!S.running) { requestAnimationFrame(loop); return; }` at loop top in
   every shell. Life needed the second look: its step was gated but
   `render()+renderUnder()` ran every frame (60 fps redraw of a static board);
   gating the whole loop then required explicit renders on the paused mutation
   paths (step, loadSeed, boot).
7. **The second `S.running = true` trap.** First patch replaced the object-
   literal `running: true,` in the SHELLS dict of 10 files; six shells
   (spacecraft/dilemma/tower/traffic/antcolony/civ60) had a SECOND
   `S.running = true` inside `newGame()` that survived, so restarts un-paused.
   Life's object literal was hit by the wrong patch form. Lesson: grep for
   every assignment form before believing a state fix; behavioral
   verification (wrapped canvas draw-call counter) caught all of them.
8. **Supersedes round-2 finding 5.** The bot-default + auto-boot behavior
   ("life seeds a boot soup", "cube startAuto()", "door decideNow() on boot")
   is REPLACED by the paused-boot convention: every loop game boots with
   `running:false`, a truthful "▶ play" initial label, and restarts land
   paused too. Bots stay the only player — pressing play starts the bot.
   cube's "▶ auto-solve" carries the mz-play styling instead.
9. **CSS hover-flip makes fronts unclickable.** `.pg-flip:hover` rotated the
   card away under the cursor; the front's small h3 link could not be
   clicked and there was no obvious launcher. Fix: flip only via `.is-flipped`
   (click / Enter / Space on a focusable card, `aria-pressed` tracked,
   links/buttons exempt from the flip handler); every playable card got a
   teal `▶ Play` pill on the front; hint copy says "click the card".
   Probe caveat: a Play click DURING the flip animation lands on rotated
   content — settle before clicking (test artifact, not a product bug).
10. **Per-series chart normalization reads as nonsense.** civ60's resource
    chart scaled each of food/wood/pop/homes to its own max — line heights
    were incomparable. Now one shared max with a "scale 0–N" label.
11. **A sim without a stated goal is "makes no sense".** civ60 gained a
    deterministic verdict line ("10 without a roof — BUILD is score-favored…",
    raid/hunger/festival states) + a "your move:" guidance line, both derived
    only from the snapshot; needs (hungry/roofless) joined the under-row.
12. **MicroWorld shipped** (microworld-core.js + shell + page + 11 tests +
    library card + sitemap): 34 creatures, 10 actions, ties→engine under a
    per-second budget, fox pack ecology, births/stores as rules. `#mw-canvas`
    initially had no responsive CSS rule (586px overflow at 390px) — every
    game canvas needs its `width:100%` + aspect-ratio rule.
13. **Library filter orphans.** Converting the last "in the workshop" card to
    playable left the `data-cat="soon"` filter chip targeting an empty set
    (chip removed); "flip a card" copy updated to "click a card".

Verification receipts: 320/320 node tests (11 new microworld); behavioral
paused-probe over all 13 loop games — paused boot, play runs, pause stops,
0 draw-calls while paused; full-site sweep 28 page/viewport combos, 0 console
errors, 0 h-overflow (390/768/1440); life paused-path redraws verified
(step/seed/soup); civ60 shortage-cascade probe clean; screenshots in
/var/tmp/game-audit/ (ephemeral).

## Round 4 — 2026-10-09 — vision sweep + Try a Decision

14. **Try a Decision builder (decide.html).** Form (category / state /
    question / optional criteria) compiles into the existing request editor
    and fires the same decide path — one dispatch, JSON view shows the wire
    verbatim. Boolean criteria has NO wire field, so it is composed into the
    question text with a negator-free template ("satisfied when: … fail
    when: …") — see 15. Score note states the zero-ML tab's honest limit
    (overlap or rule, else abstain). Tests: tests/try_decision_test.mjs (10).
15. **"No when:" flipped the answer.** The template's literal "No" injected
    exactly one NEGATOR token; the lexical boolean rung parity-counts
    negators over question text (classifier.rs decide_boolean_with_support)
    and an odd count reverses the read — a TRUE question answered FALSE at
    82–90%. Fix: negator-free template + live parity warning when user text
    makes the count odd. Lesson: any copy composed INTO a question string
    must be checked against engine/lexical.rs NEGATORS.
16. **Boolean answers displayed "undefined".** demo.js answersTable read
    `a.choice`; boolean answers carry `value`. Fallback added.
17. **Library card fronts rendered browser-default blue.** styles.css's
    `.pg-flip` block styled faces/back/pill but never `h3`, `h3 a`, front
    `p`, `.pg-where`; playground.html's local `.pg-card h3 a` rule targeted
    a class the cards don't have. Fronts now pin palette ink/dim, kill the
    underline, teal hover, green `.pg-badge.playable`. Verified by computed
    style + screenshot at 1440/1024/390.
18. **"try.html" naming drift.** try.html is a live 21-line redirect stub
    (?run=…→decide.html, else→playground.html) — kept as-is. But index.html
    fineprint labeled a playground.html link "try.html", and demo.js /
    maze-core.js header comments named pages they don't run in (demo.js is
    shared by decide/playground/api; maze-core by maze+pacman). Label now
    matches its href; comments name their real loader pages, and the
    decide.html?run= comment notes the legacy try.html?run= redirect.

Verification receipts: 331/331 node tests; r4 re-sweep playground+index at
1440/1024/390 — 0 h-overflow, palette-confirmed card fronts; sweep shots in
/var/tmp/site-sweep/ (ephemeral).

## Round 5 — 2026-10-09 — maze objectives, cube shapes, ratings, chrome (#77-#80)

1. **Beat-the-clock objective (maze).** `fastMode` config + stamp suffix
   (`120/fast`); at half-time the bot drops loot/chest rungs (`pressing`
   gate) and prefers informative frontiers
   (`score = useD - (unknownBorders-1) * (pressing ? 1.5 : 0.5)`; plain
   runs keep pure nearest-frontier for replay compat). Bot speed raised
   2-90 steps/s; committed whole-route plans (`botPlan` {kind,x,y,path})
   followed without re-deliberation, cleared on serve/interrupt. BFS
   route-builder walks a SOURCE-rooted distance map downward from the
   target with negated directions.
2. **Route-efficiency stats.** runSummary gains plans/planSteps/
   planReplans/stepEfficiency; stats overlay shows route efficiency
   (optimal/taken %) and beat-the-clock rows (plans/steps-on-plan/
   replans/% predicted). Probe: plans ~240-306, ~98% of steps on-plan,
   0 replans on seeds fast1-fast4.
3. **Cube shape mods.** `cubeType` classic/mirror/ghost is geometry-only
   (excluded from cubeStamp); per-cubie `scale3d` from fixed signature
   tables; `rebuildGeometry()` retypes the live cube without rescramble.
   Movelog rail: newest-first chips, `.latest` highlight, 120-chip
   window. Pyraminx/megaminx honestly refused in copy (different puzzle
   families).
4. **Ratings + feedback loop (#78).** `oc-rating:{slug}` in localStorage;
   game-page strip (stars, feedback, save) with a prefilled GitHub issue
   link; hub badges + sort/filter select. The issue link is the bridge to
   the scheduled improvement pass.
5. **Site release flow documented (#79).** `docs/SITE_RELEASE_FLOW.md`:
   ratings -> `game-feedback` issues -> `site/improve-pass-<YYYY-MM>`
   branch -> CF Pages branch preview = staging -> human PR merge =
   promotion. GitForge CI image has no Node, so the JS suite is not a CI
   lane yet (surface, not faked).
6. **Hosted-telemetry copy (#80).** api.html + index.html: decision
   telemetry (latency percentiles, confidence distributions, refusal
   codes, abstain rate), alerting hooks, usage export, deterministic
   replays; comparison table row (hosted included vs self-host BYO).
7. **Game switcher + seed dice.** site-core.js injects a switch bar on
   every game page (hub gets search over `.pg-flip` cards with live
   count); every game gets a dice button next to seed
   (`DK.shell.numericSeed`). TOC filter only when a page has >= 8
   sections.
8. **Mojibake vector identified + fixed.** Landing page itself was clean;
   the real vector was direct-view .md/.txt assets served without a
   charset (http.server/browser default windows-1252). ASCII-fied
   README/AGENTS/robots.txt + `_headers` for robots.txt. CF Pages
   `_headers` does not support `/*.md` globs, so assets stay ASCII.

## Round 5 addendum — GridWars (#77) + wiki Docs page (#70), agent-built, verified

9. **GridWars shipped** (gridwars-core.js 1,037 / gridwars.js 867 /
   gridwars.html 386 / tests 543 lines, 30/30 green). DOM-free core: 8
   geometry types with a tagged decision ladder (11 tags), 4 tiers, 4
   weapons (numpad 1-4), splitting children, spring-mesh grid, swept
   collisions, engine wave-lead questions with seeded fallback. Real bug
   found en route: 13 px/tick bullets vs ~12 px hit window tunneled
   targets — updateBullets now sweeps the step as a segment through the
   core's segCircle. Enemy AI is honestly none; scoreboard session-only.
10. **Docs page shipped** (docs.html, 10 sections + sidebar + filter).
    Honesty softenings during authorship: hosted = waitlist, WASM scope
    = rungs 1-4 + graph, `--policy` validated-then-inapplicable, no
    /metrics claimed, calibration_version 0 flagged un-calibrated, the
    "17 of 18" abstention figure attributed to the calibration survey.
11. **Four crate-vs-site-copy contradictions fixed at the source** (all
    re-verified against the crates before patching):
    decide.html refusal card named `engine.zero_candidates` (real:
    `ir.empty_candidates`), `schema.invalid_request` (real family:
    schema.invalid_json/missing_field/invalid_type/invalid_value/
    limit_exceeded — now shown as `schema.*`), and `gate.below_threshold`
    (no gate.* codes exist; below-gate is a 200 abstain, now labeled
    "no code"). `graph.cycle` and `engine.missing_backend` were real.
    api.html + index.html documented `serve --addr` (real flag: `--bind`)
    and claimed `/metrics` ships in the binary (no scrape endpoint; real
    surface: per-response metrics/trace, /v1/healthz, /v1/capabilities).
12. **Chrome integration**: Docs nav link (desktop + mobile) added to all
    20 non-docs pages; docs.html + gridwars.html added to sitemap.xml and
    the AGENTS.md page map. Browser-verified: 2 docs links per page
    (desktop/mobile), docs sidebar 14 links / 0 dead anchors, filter
    narrows to Graph mode on "graph" + empty-note on misses + Esc resets,
    0 console errors desktop+mobile.

## Round 5 close-out — civ60/antcolony/microworld liveness (#75) + sweep fixes

13. **Game liveness + civ60 rewrite (#75), agent-built, verified.** civ60
    got a full presentation pass (11-chip resource bar, drawn seeded town
    scene with zero image files, newest-first chronicle, verdict panel);
    Ant Colony got an on-canvas legend + brighter ant strokes; MicroWorld
    got boot veil + "watch for:" hints + terrain retune so rich grass is
    the one loud signal. Real defect fixed en route: the new styles block
    hardcoded light-on-dark text, invisible in the light theme — alert
    red now `--oc-bad` for both themes, text uses ink vars. 47/47 tests,
    0 console noise, veil→play→pause loop verified.
14. **Sweep false alarm decoded.** `scrollWidth` overflow reported
    uniformly (23/54/84/133 px) on every page at every width — that is
    clipped decorative bleed (bg-mesh, section aura) behind
    `body { overflow-x: hidden }`; `scrollTo` provably cannot move X.
    Content-level probing is the real signal.
15. **Two real mobile overflows fixed (both this session's features).**
    `.game-switch-bar` had no `flex-wrap`, so the rating strip riding in
    it pushed 133 px past a 375 px viewport and the hub's rating select
    414 px. Bar now wraps; strip takes its own full row
    (`flex: 1 1 100%`); the duplicate bare `.rating-hub` rule was merged
    into the new one.
16. **grid-2/grid-3 minmax trap at 881-1000 px.** `1fr 1fr` floors at the
    cell's max-content, so a long token in an index build card pushed the
    grid 98 px past a 900 px viewport (width class never swept before —
    prior rounds used 1024). Both grids floored at
    `repeat(N, minmax(0, 1fr))`; wide tables already scroll inside
    `.table-wrap` by design. Final sweep: 24 pages x 3 widths, 0 content
    overflows, 0 console errors.
