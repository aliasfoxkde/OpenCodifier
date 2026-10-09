/* Apex Line core — pure simulation for the 2.5D racer. No DOM, no canvas,
   no timers: deterministic given (seed, config, input script), unit-testable
   under `node --test` like pac-core.js (import for side effect, read the
   global; sdk/decisions-sdk.js must load first — the seeded RNG and clamps
   are the SDK's, not reimplemented here).

   Design contract (docs/planning/playground/DECISIONS-SDK-PLAN.md §5.4):
   - the track is a loop of segments with eased curves and hills; the shell
     projects them pseudo-perspectively — the core only carries the numbers;
   - AI racers pick a LINE every stretch (apex / inside / outside /
     slipstream) by scoring candidates against a personality weight vector —
     the pong mechanism driving a race line instead of a chase;
   - a genuine tie between the top two lines escalates to the engine as a
     real choice question (id "racer-line"); the player's suggested-line
     chip runs the same scoring with neutral weights and escalates the same
     way (id "player-line");
   - off-road slowdown, bump collisions, laps with per-lap timing, and a bot
     player are all in the core so tests can drive whole races. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;

  const TICKS_PER_S = 60;
  const SEG_LEN = 200;            /* world units per segment */
  const ROAD_HALF = 1;            /* road spans x in [-1, 1] (normalized) */
  const OFF_ROAD_X = 0.92;        /* beyond this you're in the dirt */
  const SLIP_DIST = 6 * SEG_LEN;  /* slipstream window, world units */
  const BUMP_DIST = 1.2 * SEG_LEN;
  const CAR_HALF_W = 0.14;        /* normalized half-width of a car */
  const LINE_STRETCH = 6;         /* re-decide the line every N segments */

  const LINES = ["apex", "inside", "outside", "slipstream"];
  const LINE_OF = {};             /* per-racer current line target */
  const LINE_NAMES = {
    apex: "apex — hug the inside of the bend",
    inside: "inside — hold the inner lane",
    outside: "outside — take the wide safe line",
    slipstream: "slipstream — tuck behind the car ahead",
  };

  /* racer personalities — the incentive surface, pong-style weights */
  const PERSONALITIES = [
    { name: "hauler", color: "#ff4d4d", weights: { apex: 5, inside: 1, outside: 0, slipstream: 0 } },
    { name: "shadow", color: "#b8ff4d", weights: { apex: 1, inside: 0, outside: 1, slipstream: 5 } },
    { name: "wall", color: "#4de3ff", weights: { apex: 1, inside: 0, outside: 5, slipstream: 1 } },
    { name: "rookie", color: "#ff9ad5", weights: { apex: 0, inside: 1, outside: 2, slipstream: 2 } },
    { name: "ace", color: "#ffb84d", weights: { apex: 4, inside: 2, outside: 1, slipstream: 2 } },
    { name: "ghost", color: "#c94dff", weights: { apex: 2, inside: 3, outside: 0, slipstream: 1 } },
  ];

  const DEFAULT_CONFIG = {
    seed: "oc-line",
    lengthSegs: 500,     /* segments per lap */
    curviness: 0.5,      /* 0..1 — how much the track bends */
    hilliness: 0.5,      /* 0..1 — how much it climbs */
    laps: 2,
    opponents: 3,
    skill: 3,            /* 1..5 — AI pace */
    traffic: 0.3,        /* 0..1 — slow non-deciding cars */
    topSpeedPct: 1,      /* difficulty: player top speed multiplier */
    grip: 3,             /* 1..5 — resistance to centrifugal drift */
    botPlayer: false,
  };

  function normalizeConfig(raw) {
    const c = Object.assign({}, DEFAULT_CONFIG, raw && typeof raw === "object" ? raw : {});
    c.seed = String(c.seed == null ? DEFAULT_CONFIG.seed : c.seed);
    c.lengthSegs = Math.round(DK.clampf(+c.lengthSegs, 200, 1600, DEFAULT_CONFIG.lengthSegs));
    c.curviness = DK.clampf(+c.curviness, 0, 1, DEFAULT_CONFIG.curviness);
    c.hilliness = DK.clampf(+c.hilliness, 0, 1, DEFAULT_CONFIG.hilliness);
    c.laps = Math.round(DK.clampf(+c.laps, 1, 9, DEFAULT_CONFIG.laps));
    c.opponents = Math.round(DK.clampf(+c.opponents, 0, PERSONALITIES.length, DEFAULT_CONFIG.opponents));
    c.skill = Math.round(DK.clampf(+c.skill, 1, 5, DEFAULT_CONFIG.skill));
    c.traffic = DK.clampf(+c.traffic, 0, 1, DEFAULT_CONFIG.traffic);
    c.topSpeedPct = DK.clampf(+c.topSpeedPct, 0.5, 1.5, DEFAULT_CONFIG.topSpeedPct);
    c.grip = Math.round(DK.clampf(+c.grip, 1, 5, DEFAULT_CONFIG.grip));
    c.botPlayer = c.botPlayer === true;
    return c;
  }

  /* the stamp separates seed, shape, laps, field, and difficulty */
  function racingStamp(cfg) {
    const c = normalizeConfig(cfg);
    return ["racing", c.seed, c.lengthSegs, c.curviness, c.hilliness,
      c.laps, c.opponents, c.skill, c.traffic, c.topSpeedPct, c.grip].join("|");
  }

  /* ---------- track ---------- */

  const easeIn = (a, b, p) => a + (b - a) * Math.pow(p, 2);
  const easeInOut = (a, b, p) => a + (b - a) * (-Math.cos(p * Math.PI) / 2 + 0.5);

  /* Build a loop of segments: alternating straights, eased bends, and
     rolling hills. Deterministic under the SDK RNG. Returns
     { segs: [{curve, y}], totalLen } — y is the elevation at each segment
     END; the loop closes by easing the last hill back to 0. */
  function buildTrack(cfg) {
    const c = normalizeConfig(cfg);
    const rng = DK.makeRng("track:" + racingStamp(c));
    const segs = [];
    let y = 0;
    const addSeg = (curve, endY) => { segs.push({ curve, y: endY }); };
    const addSection = (enter, hold, leave, curve, dy) => {
      const startY = y;
      const endY = startY + dy;
      const n = enter + hold + leave;
      for (let i = 0; i < enter; i++) addSeg(curve * easeIn(0, 1, i / enter), easeInOut(startY, endY, (i + 1) / n));
      for (let i = 0; i < hold; i++) addSeg(curve, easeInOut(startY, endY, (enter + i + 1) / n));
      for (let i = 0; i < leave; i++) addSeg(curve * easeInOut(1, 0, i / leave), easeInOut(startY, endY, (enter + hold + i + 1) / n));
      y = endY;
    };
    const bend = () => {
      const dir = rng() < 0.5 ? -1 : 1;
      const strength = (0.3 + rng() * 0.7) * c.curviness * 4;
      const dy = Math.round((rng() * 2 - 1) * c.hilliness * 4) * SEG_LEN;
      addSection(18, 12 + Math.floor(rng() * 26), 18, dir * strength, dir * dy);
    };
    /* close the elevation loop: ease whatever height we're at back to 0 */
    while (segs.length < c.lengthSegs - 40) {
      if (rng() < 0.35) {
        const n = 10 + Math.floor(rng() * 20);
        for (let i = 0; i < n; i++) addSeg(0, y);
      } else {
        bend();
      }
    }
    const drop = y;
    addSection(14, 12, 14, 0, -drop);
    while (segs.length < c.lengthSegs) addSeg(0, 0);
    return { segs, totalLen: segs.length * SEG_LEN };
  }

  const segAt = (track, z) => track.segs[Math.floor(((z % track.totalLen) + track.totalLen) % track.totalLen / SEG_LEN) % track.segs.length];
  const curveAt = (track, z) => segAt(track, z).curve;

  /* ---------- cars ---------- */

  function makeCar(kind, name, color, z, x, weights, topSpeed) {
    return {
      kind, name, color, z, x,
      speed: 0, topSpeed,
      px: x, pz: z,
      weights,                     /* null for traffic */
      lap: 1, lapStart: 0, lapTimes: [], bestLap: null,
      line: "inside", lineAt: 0,   /* current line + tick it was chosen */
      finished: null,              /* total time once done */
      position: null,
    };
  }

  function createGame(raw) {
    const cfg = normalizeConfig(raw);
    const track = buildTrack(cfg);
    const maxSpeed = SEG_LEN * TICKS_PER_S * 0.6 * cfg.topSpeedPct;   /* 7200 u/s at 1.0 */
    const rng = DK.makeRng("grid:" + racingStamp(cfg));
    const cars = [];
    /* standing grid: AI ahead, player at the back */
    for (let i = 0; i < cfg.opponents; i++) {
      const p = PERSONALITIES[i % PERSONALITIES.length];
      const pace = 0.78 + 0.055 * cfg.skill + 0.02 * (cfg.opponents - i);
      cars.push(makeCar("racer", p.name, p.color,
        (cfg.opponents - i) * 3.2 * SEG_LEN, (i % 2 === 0 ? -0.35 : 0.35),
        Object.assign({}, p.weights), maxSpeed * Math.min(0.99, pace)));
    }
    const nTraffic = Math.round(cfg.traffic * 8);
    for (let i = 0; i < nTraffic; i++) {
      cars.push(makeCar("traffic", "traffic", "#9aa7b0",
        ((4 + i * 5.5 + rng() * 3) % 30) * SEG_LEN, (rng() * 1.2 - 0.6),
        null, maxSpeed * (0.42 + rng() * 0.18)));
    }
    const player = makeCar("player", "you", "#fde047", 0, 0, null, maxSpeed);
    cars.push(player);
    return {
      cfg, track, cars, player,
      maxSpeed,
      t: 0, ticks: 0,
      startedAt: 0,
      over: false, result: null,     /* "finished" | "dnf" */
      finalTime: null,
      rungs: { line: 0, engine: 0, traffic: 0 },
      engineCalls: 0,
      lastDecision: null,
      suggest: { line: "inside", rung: null, at: null },
    };
  }

  const ahead = (g, car) => g.cars.filter((o) => o !== car && o.finished === null).map((o) => {
    let dz = o.z - car.z;
    if (dz < -g.track.totalLen / 2) dz += g.track.totalLen;
    if (dz > g.track.totalLen / 2) dz -= g.track.totalLen;
    return { o, dz, dx: o.x - car.x };
  }).sort((a, b) => a.dz - b.dz);

  /* candidate lines with tactical adjustments — integer-weighted like pac
     so genuine ties are possible and meaningful */
  function lineCandidates(g, car) {
    const curve = curveAt(g.track, car.z + 10 * SEG_LEN);   /* look a stretch ahead */
    const near = ahead(g, car).filter((a) => a.dz > 0 && a.dz < SLIP_DIST);
    const blocking = near.find((a) => a.dz < BUMP_DIST && Math.abs(a.dx) < CAR_HALF_W * 2);
    const slipping = near.find((a) => a.dz < SLIP_DIST && Math.abs(a.dx) < 0.3);
    const curveTerm = Math.round(Math.abs(curve) * 2);      /* 0..? halves */
    return LINES.map((id) => {
      let score = car.weights ? car.weights[id] : 1;
      if (id === "apex") score += curveTerm;
      if (id === "slipstream") {
        if (slipping) score += 2;
        if (near.length === 0) score -= 1;                    /* nobody to tuck behind */
      }
      if (blocking) {
        const away = (id === "apex" && curve * blocking.dx <= 0) ||
          (id === "outside" && curve * blocking.dx >= 0) ? 1 : 0;
        score += away;
      }
      return {
        id,
        score,
        description: LINE_NAMES[id] + (blocking ? " (car in the bumper zone)" : ""),
      };
    });
  }

  /* pick a line: weights argmax with stable tie-break, engine on genuine
     ties — the pac mechanism, applied to racing lines */
  function decideLine(g, car, questionId, engineChoose) {
    const cands = lineCandidates(g, car);
    const sorted = [...cands].sort((a, b) => b.score - a.score ||
      LINES.indexOf(a.id) - LINES.indexOf(b.id));
    const tie = sorted.length > 1 && Math.abs(sorted[0].score - sorted[1].score) < 1e-9;
    let pick = sorted[0].id;
    let rung = "line";
    if (tie && engineChoose) {
      /* the full wire contract — state + question text + policy. A bare
         {questions} request makes the WASM decide() throw (measured: the
         bridge burned its fail limit and went unavailable mid-race) */
      const w = car.weights;
      const wtxt = w
        ? "apex " + w.apex + ", inside " + w.inside + ", outside " + w.outside +
          ", slipstream " + w.slipstream
        : "neutral";
      const answer = engineChoose({
        state: {
          text: "Racing line tie: " + car.name + " at z " + Math.round(car.z) +
            " (speed " + Math.round(car.speed) + ", curve " +
            +curveAt(g.track, car.z + 10 * SEG_LEN).toFixed(3) + ") scores two lines" +
            " exactly equal under its reward weights (" + wtxt + "). Pick a line.",
          facts: {},
        },
        questions: [{
          type: "choice", id: questionId,
          text: "Which line should " + car.name + " take?",
          candidates: sorted.map((c) => ({ id: c.id, description: c.description })),
        }],
        policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" },
      });
      g.engineCalls += 1;
      const ans = answer && answer.outcome === "accept" && Array.isArray(answer.answers)
        ? answer.answers.find((a) => a && typeof a.choice === "string") : null;
      const choice = ans ? ans.choice : null;
      if (choice && sorted.some((c) => c.id === choice)) {
        pick = choice;
        rung = "engine";
      }
    }
    car.line = pick;
    car.lineAt = g.ticks;
    if (car.kind === "racer") {
      g.rungs[rung] += 1;
      g.lastDecision = {
        who: car.name, at: { x: +car.x.toFixed(2), z: Math.round(car.z) },
        dir: pick, rung, tie,
      };
    }
    return { pick, rung, tie };
  }

  /* target x for a line at this z: apex/inside pull toward the inside of
     the upcoming bend; outside pushes wide; slipstream mirrors the car ahead */
  function lineTargetX(g, car) {
    const look = segAt(g.track, car.z + 10 * SEG_LEN);
    const curve = look.curve;
    const insideSign = curve >= 0 ? 1 : -1;      /* curve>0 bends right: inside is +x */
    switch (car.line) {
      case "apex": return insideSign * Math.min(0.8, 0.35 + Math.abs(curve) * 0.5);
      case "inside": return insideSign * 0.35;
      case "outside": return -insideSign * 0.45;
      case "slipstream": {
        const near = ahead(g, car).find((a) => a.dz > 0 && a.dz < SLIP_DIST);
        return near ? near.o.x : 0;
      }
      default: return 0;
    }
  }

  /* the bot player: neutral-weight line choice + steer/pace control — a
     readable baseline, honest about being one */
  function botDrive(g) {
    const p = g.player;
    decideLine(g, p, "player-line", g.engineChoose);
    const target = lineTargetX(g, p);
    const steer = DK.clampf((target - p.x) * 2.2, -1, 1);
    const curveAhead = Math.abs(curveAt(g.track, p.z + 8 * SEG_LEN));
    const braking = curveAhead > 0.9 && p.speed > p.topSpeed * 0.8;
    return { steer, accel: !braking, brake: braking };
  }

  /* ---------- physics ---------- */

  function moveCar(g, car, dt) {
    const seg = segAt(g.track, car.z);
    const offRoad = Math.abs(car.x) > OFF_ROAD_X;
    const speedPct = car.speed / g.maxSpeed;

    /* steering + centrifugal pull toward the outside of the bend */
    const gripFactor = 1 - (g.cfg.grip - 1) * 0.11;
    car.x += (car.steer || 0) * dt * 2.1 * Math.min(1, speedPct * 2);
    car.x -= seg.curve * speedPct * speedPct * 0.42 * gripFactor;

    /* pace */
    if (car.kind === "player" && !g.cfg.botPlayer) {
      const input = car.input || {};
      if (input.brake) car.speed -= g.maxSpeed * 1.6 * dt;
      else if (input.accel) car.speed += g.maxSpeed * 0.42 * dt;
      else car.speed -= g.maxSpeed * 0.12 * dt;
    } else {
      /* AI pace: target speed shaped by skill, curviness caution, line */
      const caution = 1 - Math.min(0.3, Math.abs(seg.curve) * (car.kind === "traffic" ? 0.18 : 0.09));
      const lineBonus = car.line === "slipstream" && Math.abs(car.x) < 0.5 ? 1.04 : 1;
      const want = car.topSpeed * Math.max(0.55, caution) * lineBonus;
      car.speed += DK.clampf(want - car.speed, -g.maxSpeed * 1.2 * dt, g.maxSpeed * 0.4 * dt);
    }
    if (offRoad) car.speed = Math.min(car.speed, g.maxSpeed * 0.35);
    car.speed = DK.clampf(car.speed, 0, car.topSpeed);

    /* bumps: run into a slower car and you lose the difference */
    const blocking = ahead(g, car).find((a) =>
      a.dz > 0 && a.dz < BUMP_DIST && Math.abs(a.dx) < CAR_HALF_W * 2);
    if (blocking && car.speed > blocking.o.speed) {
      car.speed = blocking.o.speed * 0.92;
      car.x += (car.x < blocking.o.x ? -1 : 1) * 0.16 * dt * 60 * dt;
    }

    car.x = DK.clampf(car.x, -1.6, 1.6);

    /* advance + laps */
    car.z += car.speed * dt;
    if (car.z >= g.track.totalLen) {
      car.z -= g.track.totalLen;
      const lapTime = (g.t - car.lapStart);
      car.lapStart = g.t;
      car.lapTimes.push(lapTime);
      if (car.bestLap === null || lapTime < car.bestLap) car.bestLap = lapTime;
      car.lap += 1;
      if (car.kind !== "traffic" && car.lap > g.cfg.laps && car.finished === null) {
        car.finished = g.t;
        car.lap = car.lapTimes.length;   /* laps actually run, not the counter */
        const doneCount = g.cars.filter((o) => o.finished !== null && o.kind !== "traffic").length;
        car.position = doneCount;
        if (car === g.player) {
          g.over = true;
          g.result = "finished";
          g.finalTime = car.finished;
        }
      }
    }
  }

  function tickGame(g, input, engineChoose) {
    if (g.over) return g;
    g.engineChoose = engineChoose || null;
    const dt = 1 / TICKS_PER_S;
    g.t += dt;
    g.ticks += 1;

    /* line decisions on the stretch boundary */
    for (const car of g.cars) {
      if (car.finished !== null) continue;
      const segIdx = Math.floor(car.z / SEG_LEN);
      if (segIdx / LINE_STRETCH >= (car.nextDecide || 0)) {
        car.nextDecide = (Math.floor(segIdx / LINE_STRETCH) + 1);
        if (car.kind === "racer") decideLine(g, car, "racer-line", engineChoose);
        else if (car.kind === "traffic") g.rungs.traffic += 1;
      }
      car.steer = null;
      car.input = null;
    }

    /* drive each car */
    for (const car of g.cars) {
      if (car.finished !== null) continue;
      if (car === g.player) {
        if (g.cfg.botPlayer) {
          const d = botDrive(g);
          car.steer = d.steer;
          car.input = { accel: d.accel, brake: d.brake };
        } else {
          car.steer = input && typeof input.steer === "number" ? DK.clampf(input.steer, -1, 1) : 0;
          car.input = {
            accel: !!(input && input.accel),
            brake: !!(input && input.brake),
          };
        }
      } else {
        car.steer = DK.clampf((lineTargetX(g, car) - car.x) * 2.0, -1, 1);
      }
      moveCar(g, car, dt);
    }

    /* player suggestion chip: neutral weights, escalates on ties too */
    if (g.ticks % 30 === 0) {
      const p = g.player;
      const cands = lineCandidates(g, p);
      const sorted = [...cands].sort((a, b) => b.score - a.score ||
        LINES.indexOf(a.id) - LINES.indexOf(b.id));
      const tie = sorted.length > 1 && Math.abs(sorted[0].score - sorted[1].score) < 1e-9;
      let line = sorted[0].id;
      let rung = "line";
      if (tie && engineChoose) {
        const res = decideLine(g, p, "player-line", engineChoose);
        line = res.pick;
        rung = res.rung;
      } else {
        p.line = line;
      }
      g.suggest = { line, rung, at: { x: +p.x.toFixed(2), z: Math.round(p.z) } };
    }
    return g;
  }

  function runSummary(g) {
    return {
      seed: g.cfg.seed,
      stamp: racingStamp(g.cfg),
      result: g.result,
      over: g.over,
      ticks: g.ticks,
      time: g.finalTime,
      lapTimes: g.player.lapTimes.slice(),
      bestLap: g.player.bestLap,
      lap: g.player.lap,
      position: g.player.position,
      finishers: g.cars.filter((c) => c.finished !== null)
        .sort((a, b) => a.finished - b.finished).map((c) => c.name),
      rungs: Object.assign({}, g.rungs),
      engineCalls: g.engineCalls,
      lastDecision: g.lastDecision ? Object.assign({}, g.lastDecision) : null,
    };
  }

  /* whole-race bot simulation — the deterministic drive the tests use */
  function simulateGame(cfg, engineChoose) {
    const g = createGame(cfg);
    g.cfg.botPlayer = true;
    while (!g.over && g.ticks < TICKS_PER_S * 60 * 20) tickGame(g, null, engineChoose);
    return g;
  }

  globalThis.RacingCore = {
    TICKS_PER_S, SEG_LEN, LINES, PERSONALITIES, LINE_STRETCH, OFF_ROAD_X,
    DEFAULT_CONFIG, normalizeConfig, racingStamp, buildTrack, curveAt, segAt,
    createGame, tickGame, runSummary, simulateGame, botDrive, lineCandidates,
    decideLine, lineTargetX, ahead,
  };
})();
