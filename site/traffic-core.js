/* OpenCodifier — Traffic Simulator, core (DOM-free).
   §5.11 of the Decisions-SDK plan: cars AND lights decide.

   Division of labor, exactly as specced:
     - CARS are rules, never questions: STOP (car ahead), STOP (red
       light), GO, WAIT (stopped at red ≥ 3 s), TURN (argmax over
       exit-arm queues at every intersection entry) and REROUTE (the
       argmax flips to a side street when the straight queue is long).
       Local state only — light phase, gap, queue.
     - LIGHTS are the decision agents: each re-scores every sim-second
       from queue counts on both axes. Outside the close band the
       argmax rule decides; a genuine near-tie (|gap| ≤ 120) goes to
       the engine over {keep, switch} — if the city-wide batching
       budget has a token. Rules first: under minimum phase → keep,
       past maximum phase → switch, emergency approaching on a red
       arm → switch immediately. Preemption is a RULE, never a
       question — a test scans every escalated request for it.

   Throughput (cars/min), wait percentiles (p50/p90 of completed
   trips) and the decision tallies make the congestion slider visible.
   The shell (traffic.js) renders; this file never touches the DOM. */

(function () {
  "use strict";
  const DK = globalThis.DecisionsSDK;

  const TICKS_PER_S = 60;
  const CELL = 20;
  const GW = 48, GH = 30;                 /* 960 × 600 */
  const VX = [6, 18, 30, 42];             /* vertical roads (lanes vx, vx+1) */
  const HY = [5, 15, 25];                 /* horizontal roads (lanes hy, hy+1) */
  const TIE_EPS = 120;
  const MIN_PHASE = 6 * TICKS_PER_S;
  const MAX_PHASE = 24 * TICKS_PER_S;
  const LIGHT_DECIDE = TICKS_PER_S;       /* each light re-scores every sim-second */
  const ARM_CELLS = 8;                    /* queue counting horizon */
  const EXIT_ARM = 8;                     /* turn-choice queue horizon */
  const REROUTE_QUEUE = 5;                /* straight queue that flips the argmax */
  const WAIT_STATE_TICKS = 3 * TICKS_PER_S;
  const MOVE_BASE = 10;                   /* ticks per cell at congestion 0 */
  const MOVE_CONGESTION = 8;              /* extra ticks per cell at congestion 1 */
  const EMERGENCY_MOVE = 5;               /* emergency cruise: ticks per cell */
  const HISTORY_EVERY = 5 * TICKS_PER_S;
  const WAIT_SAMPLES_MAX = 400;

  const DIRS = [[0, -1], [1, 0], [0, 1], [-1, 0]];   /* N E S W */
  const DIR_NAME = ["north", "east", "south", "west"];
  const CAR_STATES = ["cruise", "stop", "wait", "cross"];
  const TURNS = ["straight", "left", "right"];
  const DEFAULT_CONFIG = {
    seed: "oc-traffic", spawnRate: 36, congestion: 0.3, askBudget: 2,
  };

  /* ---------- config + stamp ---------- */

  function normalizeConfig(raw) {
    const c = raw || {};
    /* 0 is meaningful for spawnRate and askBudget: explicit checks, no `||` */
    const num = (v, lo, hi, dflt) => (v === undefined || v === null
      || Number.isNaN(Number(v))) ? dflt : DK.clampf(Number(v), lo, hi, dflt);
    return {
      seed: c.seed === undefined || c.seed === null ? DEFAULT_CONFIG.seed : String(c.seed),
      spawnRate: num(c.spawnRate, 0, 90, DEFAULT_CONFIG.spawnRate),
      congestion: num(c.congestion, 0, 1, DEFAULT_CONFIG.congestion),
      askBudget: num(c.askBudget, 0, 10, DEFAULT_CONFIG.askBudget),
    };
  }

  const trafficStamp = (cfg) =>
    "traffic|" + cfg.seed + "|" + cfg.spawnRate + "|" + cfg.congestion +
    "|" + cfg.askBudget;

  /* ---------- geometry ----------
     lanes: southbound column vx / northbound vx+1; westbound row hy /
     eastbound hy+1. An intersection is the 2×2 cell block at (vx..vx+1,
     hy..hy+1); crossing it straight is two cells, and the exit snaps to
     the chosen arm's lane — that snap IS the turn. */

  const moveEvery = (cfg) => Math.round(MOVE_BASE + cfg.congestion * MOVE_CONGESTION);

  function isRoad(x, y) {
    for (const vx of VX) if (x === vx || x === vx + 1) return true;
    for (const hy of HY) if (y === hy || y === hy + 1) return true;
    return false;
  }

  const SPAWN_LANES = [];
  for (const vx of VX) {
    SPAWN_LANES.push({ x: vx, y: 0, dir: 2 });              /* southbound from top */
    SPAWN_LANES.push({ x: vx + 1, y: GH - 1, dir: 0 });     /* northbound from bottom */
  }
  for (const hy of HY) {
    SPAWN_LANES.push({ x: 0, y: hy + 1, dir: 1 });          /* eastbound from left */
    SPAWN_LANES.push({ x: GW - 1, y: hy, dir: 3 });         /* westbound from right */
  }

  function lightIndexAt(x, y) {
    for (let i = 0; i < VX.length; i++) {
      for (let j = 0; j < HY.length; j++) {
        if (x >= VX[i] && x <= VX[i] + 1 && y >= HY[j] && y <= HY[j] + 1) {
          return j * VX.length + i;
        }
      }
    }
    return -1;
  }

  /* first intersection block entry strictly ahead of (x, y) on dir within
     `horizon` cells: {dist, li} or null */
  function nextLightAhead(x, y, dir, horizon) {
    const [dx, dy] = DIRS[dir];
    for (let d = 1; d <= horizon; d++) {
      const nx = x + dx * d, ny = y + dy * d;
      if (nx < 0 || ny < 0 || nx >= GW || ny >= GH) return null;
      const li = lightIndexAt(nx, ny);
      if (li >= 0) return { dist: d, li };
    }
    return null;
  }

  /* first cell outside the block on the given exit's lane */
  function exitCell(light, dir) {
    if (dir === 0) return { x: light.vx + 1, y: light.hy - 1 };       /* north */
    if (dir === 1) return { x: light.vx + 2, y: light.hy + 1 };       /* east */
    if (dir === 2) return { x: light.vx, y: light.hy + 2 };           /* south */
    return { x: light.vx - 1, y: light.hy };                          /* west */
  }

  /* ---------- world ---------- */

  function makeLight(id, vx, hy) {
    return {
      id: "light-" + id, vx, hy,
      phase: id % 2 === 0 ? "NS" : "EW",     /* staggered starts, no sweep */
      t: (id % 3) * 90,                      /* de-synchronized but < MIN_PHASE */
      lastSwitch: 0,
    };
  }

  function createGame(raw) {
    const cfg = normalizeConfig(raw);
    const rng = DK.makeRng("traffic:" + cfg.seed);
    const lights = [];
    let id = 0;
    for (const hy of HY) for (const vx of VX) lights.push(makeLight(id++, vx, hy));
    return {
      cfg, stamp: trafficStamp(cfg), rng,
      lights, cars: [], nextId: 0,
      occ: new Uint8Array(GW * GH),
      ticks: 0, spawnAcc: 0,
      decisions: 0, rungs: { rule: 0, engine: 0 },
      engineCalls: 0, tokens: cfg.askBudget,
      tally: {},
      completed: 0, travelTicks: 0,
      waitSamples: [],
      emergency: null,
      log: [],
      history: [],
      over: false, lastDecision: null,
    };
  }

  const key = (x, y) => y * GW + x;

  /* ---------- spawning ---------- */

  function spawnCar(g, opts) {
    const emergency = !!(opts && opts.emergency);
    /* seeded lane pick; walk the rotation until a free edge cell */
    const start = Math.floor(g.rng() * SPAWN_LANES.length);
    for (let i = 0; i < SPAWN_LANES.length; i++) {
      const lane = SPAWN_LANES[(start + i) % SPAWN_LANES.length];
      if (g.occ[key(lane.x, lane.y)] === 0) {
        const car = {
          id: "c" + g.nextId++,
          x: lane.x, y: lane.y, dir: lane.dir,
          exitDir: lane.dir, plannedFor: -1,
          state: "cruise", waitT: 0, age: 0, waitLogged: false,
          emergency,
        };
        g.cars.push(car);
        g.occ[key(car.x, car.y)] += 1;
        if (emergency) {
          g.emergency = car.id;
          g.log.push({ tick: g.ticks, kind: "emergency",
            note: "emergency vehicle " + car.id + " inbound from the "
              + DIR_NAME[lane.dir] + " edge" });
        }
        return car;
      }
    }
    return null;
  }

  function injectEmergency(g) {
    if (g.emergency || g.over) return null;
    const car = spawnCar(g, { emergency: true });
    return car ? car.id : null;
  }

  /* ---------- shared counters ---------- */

  const isNS = (dir) => dir === 0 || dir === 2;
  const lightGreenFor = (light, dir) => (light.phase === "NS") === isNS(dir);

  function carsOnArm(g, x, y, dir, horizon) {
    /* cars on the arm AHEAD of (x, y), driving dir — the queue on the
       street this exit enters. Downstream, not the upstream approach. */
    const [dx, dy] = DIRS[dir];
    let n = 0;
    for (let d = 0; d <= horizon; d++) {
      const cx = x + dx * d, cy = y + dy * d;
      if (cx < 0 || cy < 0 || cx >= GW || cy >= GH) break;
      if (g.occ[key(cx, cy)] > 0) n += g.occ[key(cx, cy)];
    }
    return n;
  }

  /* ---------- car decisions: TURN and REROUTE (argmax, never asked) ---------- */

  function decideTurn(g, car, li) {
    const light = g.lights[li];
    const exits = TURNS.map((id) => {
      const dir = id === "straight" ? car.dir
        : id === "left" ? (car.dir + 3) % 4 : (car.dir + 1) % 4;
      const anchor = exitCell(light, dir);
      return { id, dir, ev: 300 - 60 * carsOnArm(g, anchor.x, anchor.y, dir, EXIT_ARM) };
    });
    exits.sort((a, b) => b.ev - a.ev || TURNS.indexOf(a.id) - TURNS.indexOf(b.id));
    let pick = exits[0];
    let reroute = false;
    if (pick.id === "straight") {
      const anchor = exitCell(light, car.dir);
      const straightQ = carsOnArm(g, anchor.x, anchor.y, car.dir, EXIT_ARM);
      if (straightQ >= REROUTE_QUEUE) {
        pick = exits[1];                     /* the REROUTE: side street wins */
        reroute = true;
      }
    }
    car.exitDir = pick.dir;
    settle(g, reroute ? "reroute" : "turn-" + pick.id, "rule",
      { id: car.id, pick: pick.id, gap: exits.length > 1 ? exits[0].ev - exits[1].ev : 0 });
  }

  /* RULE: emergency approaching on a red arm → preempt, never a question.
     Scanned EVERY tick, not at the decide cadence: an emergency at 5
     ticks/cell crosses the whole 12-cell horizon inside one decide
     period, so a cadence-gated check loses the race with the vehicle it
     exists for. A test asserts the preempt log entry appears. */
  function preemptLights(g) {
    if (!g.emergency) return;
    const ec = g.cars.find((c) => c.id === g.emergency);
    if (!ec) return;
    for (const light of g.lights) {
      if (lightGreenFor(light, ec.dir)) continue;    /* already green for it */
      const near = nextLightAhead(ec.x, ec.y, ec.dir, ARM_CELLS + 4);
      if (near && g.lights[near.li] === light) {
        switchPhase(g, light, "preempt");
      }
    }
  }

  /* ---------- light decision ladder ---------- */

  function queueCounts(g, light) {
    let onGreen = 0, onRed = 0, inX = 0, redWait = 0;
    for (const c of g.cars) {
      if (lightIndexAt(c.x, c.y) >= 0) { inX += 1; continue; }
      const onV = c.x === light.vx || c.x === light.vx + 1;
      const onH = c.y === light.hy || c.y === light.hy + 1;
      if (!onV && !onH) continue;
      const near = onV
        ? Math.abs(c.y - (light.hy + 1)) <= ARM_CELLS + 1
        : Math.abs(c.x - (light.vx + 1)) <= ARM_CELLS + 1;
      if (!near) continue;
      if (lightGreenFor(light, c.dir)) onGreen += 1;
      else { onRed += 1; redWait += c.waitT; }
    }
    return { onGreen, onRed, inX, redWait };
  }

  function decideLight(g, light) {
    light.t += LIGHT_DECIDE;

    /* RULE: minimum phase → keep, never a question */
    if (light.t < MIN_PHASE) {
      settleLight(g, light, "keep", "rule", null);
      return;
    }
    /* preemption for emergencies is handled by preemptLights() every
       tick — an emergency here is already green for it or not yet near */
    /* RULE: maximum phase → switch, never a question */
    if (light.t >= MAX_PHASE) {
      switchPhase(g, light, "max");
      settleLight(g, light, "switch", "rule", null);
      return;
    }

    /* scored: keep vs switch from the queues */
    const q = queueCounts(g, light);
    const keepEv = Math.round(300 + 40 * q.onGreen + 200 * q.inX);
    const switchEv = Math.round(300 + 40 * q.onRed + Math.min(200, q.redWait / 30));
    const gap = Math.abs(keepEv - switchEv);
    let pick = keepEv >= switchEv ? "keep" : "switch";
    let rung = "rule";

    if (gap <= TIE_EPS && g.tokens >= 1 && g.engineChoose && !g.over) {
      g.tokens -= 1;
      g.engineCalls += 1;
      const req = {
        state: {
          text: "Intersection " + light.id + " at tick " + g.ticks +
            ". Phase " + light.phase + " green for " + (light.t / TICKS_PER_S).toFixed(0) +
            "s (min " + (MIN_PHASE / TICKS_PER_S) + ", max " + (MAX_PHASE / TICKS_PER_S) +
            "). Queues: " + q.onGreen + " cars on green, " + q.onRed +
            " on red (waiting " + (q.redWait / TICKS_PER_S).toFixed(0) +
            "s total), " + q.inX + " inside the box. Pick keep or switch.",
          facts: {},
        },
        questions: [{
          type: "choice", id: "light-phase",
          text: "Should " + light.id + " keep its phase or switch?",
          candidates: [
            { id: "keep", description: "hold the current green — the moving axis keeps flowing" },
            { id: "switch", description: "swap the phase — the red axis has earned the green" },
          ],
        }],
        policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" },
      };
      const answer = g.engineChoose ? g.engineChoose(req) : null;
      const ans = answer && answer.outcome === "accept" && Array.isArray(answer.answers)
        ? answer.answers.find((a) => a && typeof a.choice === "string") : null;
      if (ans && (ans.choice === "keep" || ans.choice === "switch")) {
        pick = ans.choice;
        rung = "engine";
      }
    }
    if (pick === "switch") switchPhase(g, light, rung === "engine" ? "engine" : "scored");
    settleLight(g, light, pick, rung, gap);
  }

  function switchPhase(g, light, why) {
    light.phase = light.phase === "NS" ? "EW" : "NS";
    light.t = 0;
    light.lastSwitch = g.ticks;
    g.log.push({ tick: g.ticks, kind: "light",
      note: light.id + " → " + light.phase + " (" + why + ")" });
  }

  /* ---------- settle ---------- */

  function settle(g, choice, rung, at) {
    g.tally[choice] = (g.tally[choice] || 0) + 1;
    g.decisions += 1;
    g.rungs[rung] += 1;
    g.lastDecision = { id: at.id, choice, rung, at };
    return choice;
  }

  function settleLight(g, light, choice, rung, gap) {
    g.tally[choice] = (g.tally[choice] || 0) + 1;
    g.decisions += 1;
    g.rungs[rung] += 1;
    g.lastDecision = {
      id: light.id, choice, rung,
      at: { phase: light.phase, ageS: +(light.t / TICKS_PER_S).toFixed(1),
        gap: gap === null ? null : gap },
    };
    return choice;
  }

  /* ---------- car physics + rule transitions ---------- */

  function complete(g, car) {
    g.completed += 1;
    g.travelTicks += car.age;
    g.waitSamples.push(car.waitT);
    if (g.waitSamples.length > WAIT_SAMPLES_MAX) g.waitSamples.shift();
    if (car.emergency) g.emergency = null;
    removeCar(g, car);
  }

  function markStopped(g, car, every) {
    if (car.state === "cruise") {
      settle(g, "stop", "rule", { id: car.id });
      car.waitLogged = false;
    }
    car.state = "stop";
    car.waitT += every;
    if (!car.waitLogged && car.waitT >= WAIT_STATE_TICKS) {
      settle(g, "wait", "rule", { id: car.id, waitedS: +(car.waitT / TICKS_PER_S).toFixed(1) });
      car.waitLogged = true;
      car.state = "wait";
    }
  }

  function carTick(g, car, every) {
    car.age += 1;
    if (car.age % every !== 0) return;       /* cruise cadence */

    const inBox = lightIndexAt(car.x, car.y) >= 0;
    const [dx, dy] = DIRS[car.dir];
    const nx = car.x + dx, ny = car.y + dy;

    /* off the map → the trip is complete */
    if (nx < 0 || ny < 0 || nx >= GW || ny >= GH) { complete(g, car); return; }

    const nl = lightIndexAt(nx, ny);

    /* entering a new box: TURN / REROUTE (argmax, rule) */
    if (!inBox && nl >= 0 && car.plannedFor !== nl) {
      decideTurn(g, car, nl);
      car.plannedFor = nl;
    }

    /* leaving the box: snap to the chosen arm's lane — that snap IS the
       turn; if the exit is blocked the car waits inside (cleared by the
       phase rules, reroutes, and the emergency) */
    if (inBox && nl < 0) {
      const light = g.lights[car.plannedFor];
      const ex = exitCell(light, car.exitDir);
      if (g.occ[key(ex.x, ex.y)] > 0) { markStopped(g, car, every); return; }
      g.occ[key(car.x, car.y)] -= 1;
      car.x = ex.x; car.y = ex.y;
      car.dir = car.exitDir; car.plannedFor = -1;
      g.occ[key(ex.x, ex.y)] += 1;
      if (car.state !== "cruise") settle(g, "go", "rule", { id: car.id });
      car.state = "cruise";
      car.waitLogged = false;
      return;
    }

    /* RULE: car ahead → STOP, never asked */
    if (g.occ[key(nx, ny)] > 0) { markStopped(g, car, every); return; }

    /* RULE: red light at the boundary → STOP, never asked (the emergency
       vehicle excepted — it runs the light by design) */
    if (!inBox && nl >= 0 && !car.emergency
      && !lightGreenFor(g.lights[nl], car.dir)) {
      markStopped(g, car, every);
      return;
    }

    /* GO */
    if (car.state !== "cruise") settle(g, "go", "rule", { id: car.id });
    g.occ[key(car.x, car.y)] -= 1;
    car.x = nx; car.y = ny;
    g.occ[key(nx, ny)] += 1;
    car.state = "cruise";
    car.waitLogged = false;
  }

  function removeCar(g, car) {
    const i = g.cars.indexOf(car);
    if (i !== -1) g.cars.splice(i, 1);
    g.occ[key(car.x, car.y)] = Math.max(0, g.occ[key(car.x, car.y)] - 1);
  }

  /* ---------- tick ---------- */

  function tickGame(g, engineChoose) {
    if (g.over) return;
    g.ticks += 1;
    g.engineChoose = engineChoose || null;
    g.tokens = Math.min(g.cfg.askBudget, g.tokens + g.cfg.askBudget / TICKS_PER_S);

    /* spawns */
    g.spawnAcc += g.cfg.spawnRate / 3600;
    while (g.spawnAcc >= 1) {
      g.spawnAcc -= 1;
      spawnCar(g, null);
    }

    const every = moveEvery(g.cfg);
    preemptLights(g);
    for (const c of g.cars.slice()) {
      if (g.cars.indexOf(c) === -1) continue;    /* completed this tick */
      carTick(g, c, c.emergency ? EMERGENCY_MOVE : every);
    }

    /* each light re-scores on its own second-tick, de-synchronized */
    for (const light of g.lights) {
      if ((g.ticks + light.vx * 7 + light.hy * 3) % LIGHT_DECIDE === 0) {
        decideLight(g, light);
      }
    }

    if (g.ticks % HISTORY_EVERY === 0) {
      const moving = g.cars.filter((c) => c.state === "cruise").length;
      g.history.push({ t: Math.round(g.ticks / TICKS_PER_S),
        cars: g.cars.length, completed: g.completed, moving,
        p50: waitPercentile(g, 50) });
      if (g.history.length > 400) g.history.shift();
    }
  }

  function waitPercentile(g, p) {
    if (!g.waitSamples.length) return 0;
    const sorted = g.waitSamples.slice().sort((a, b) => a - b);
    const i = Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length));
    return +(sorted[i] / TICKS_PER_S).toFixed(1);
  }

  /* ---------- summary ---------- */

  function runSummary(g) {
    const simS = g.ticks / TICKS_PER_S;
    const moving = g.cars.filter((c) => c.state === "cruise").length;
    const waiting = g.cars.filter((c) => c.state === "stop" || c.state === "wait").length;
    return {
      ticks: g.ticks, simSeconds: +simS.toFixed(1),
      decisions: g.decisions,
      perSecond: simS ? +(g.decisions / simS).toFixed(2) : 0,
      rungs: Object.assign({}, g.rungs),
      engineCalls: g.engineCalls,
      tally: Object.assign({}, g.tally),
      cars: g.cars.length, moving, waiting,
      completed: g.completed,
      throughput: simS ? +(g.completed / (simS / 60)).toFixed(2) : 0,
      avgTripS: g.completed ? +(g.travelTicks / g.completed / TICKS_PER_S).toFixed(1) : 0,
      waitP50: waitPercentile(g, 50),
      waitP90: waitPercentile(g, 90),
      emergency: g.emergency,
      lights: g.lights.length,
      logEntries: g.log.length,
      over: g.over,
      history: g.history.length,
    };
  }

  function simulateTicks(g, ticks, engineChoose) {
    for (let i = 0; i < ticks; i++) {
      if (g.over) break;
      tickGame(g, engineChoose);
    }
    return g;
  }

  globalThis.TrafficCore = {
    TICKS_PER_S, CELL, GW, GH, VX, HY, TIE_EPS,
    MIN_PHASE, MAX_PHASE, LIGHT_DECIDE, ARM_CELLS, REROUTE_QUEUE,
    WAIT_STATE_TICKS, EMERGENCY_MOVE,
    DIRS, DIR_NAME, CAR_STATES, TURNS, DEFAULT_CONFIG, SPAWN_LANES,
    normalizeConfig, trafficStamp, createGame, tickGame, simulateTicks,
    runSummary, injectEmergency, decideLight, decideTurn, queueCounts,
    preemptLights,
    waitPercentile, lightIndexAt, nextLightAhead, isRoad, moveEvery,
    lightGreenFor, carsOnArm, exitCell,
  };
})();
