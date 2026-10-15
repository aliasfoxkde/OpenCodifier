/* OpenCodifier — Spacecraft Emergency, core (DOM-free).
   §5.15 of the Decisions-SDK plan: the abstention showcase.

   A crossing ship decides every 2 sim-seconds over six actions:
   CONTINUE / REDUCE POWER / CHANGE COURSE / SHUTDOWN ENGINE /
   RETURN HOME / EMERGENCY. Most situations resolve deterministically —
   the rule gates fire first (hull critical, thermal limit, no line to
   the destination while home is still reachable) and a calm crossing
   scores CONTINUE far above the rest. The designed-in drama is the
   ladder's honest bottom rung:

     confidence = (score gap) × (telemetry agreement)

   When the dual sensors of a channel disagree (instrument fault, drift),
   agreement collapses — the computer's confidence drops below the
   abstain line and it REFUSES TO GUESS: the sim freezes, the conflict
   is shown (both readings), and the captain (the player) resolves the
   loop. The resolution is logged on the trace with the "player" rung.
   Genuine-but-mild uncertainty (0.30 ≤ conf < 0.55) goes to the engine
   as usual, under the batching budget; an engine ABSTAIN escalates to
   the captain too — refusal is a first-class outcome, never an error.

   The shell (spacecraft.js) renders; this file never touches the DOM. */

(function () {
  "use strict";
  const DK = globalThis.DecisionsSDK;

  const TICKS_PER_S = 60;
  const DT = 1 / TICKS_PER_S;
  const DECIDE_EVERY = 2 * TICKS_PER_S;     /* a decision every 2 sim-seconds */
  const START_DIST = 1000;                  /* units to the destination */
  const FUEL_PER_UNIT = 0.06;               /* at cruise: burn/speed */
  const SAFETY = 1.15;                      /* range-planning margin */
  const FAULT_TICKS = 15 * TICKS_PER_S;     /* an instrument fault self-heals */
  const HISTORY_EVERY = 5 * TICKS_PER_S;
  const LOG_MAX = 200;

  const ACTIONS = ["continue", "reduce", "course", "shutdown", "home", "emergency"];
  const ACTION_NAME = {
    continue: "CONTINUE", reduce: "REDUCE POWER", course: "CHANGE COURSE",
    shutdown: "SHUTDOWN ENGINE", home: "RETURN HOME", emergency: "EMERGENCY",
  };
  const ACTION_DESC = {
    continue: "hold course and cruise power — the burn continues, the clock runs",
    reduce: "cut engine output to cool the plant and stretch the fuel",
    course: "steer around what is ahead — costs fuel, clears the hazard",
    shutdown: "engines to zero: the plant cools, the ship drifts",
    home: "abort the crossing — turn and make for Earth while the fuel lasts",
    emergency: "damage control: extinguish, patch, steady the crew",
  };

  const CHANNELS = ["temp", "fuel", "hull"];
  const CHANNEL_TOL = { temp: 8, fuel: 6, hull: 10 };
  const CHANNEL_LABEL = { temp: "engine temp", fuel: "fuel", hull: "hull" };

  const DEFAULT_CONFIG = {
    seed: "oc-voyager", drift: 0.15, failures: 0.2, difficulty: 0.5,
    askBudget: 2,
  };

  /* ---------- config + stamp ---------- */

  function normalizeConfig(raw) {
    const c = raw || {};
    /* 0 is a real value for every knob: explicit checks, no `||` */
    const num = (v, lo, hi, dflt) => (v === undefined || v === null
      || Number.isNaN(Number(v))) ? dflt : DK.clampf(Number(v), lo, hi, dflt);
    return {
      seed: c.seed === undefined || c.seed === null ? DEFAULT_CONFIG.seed : String(c.seed),
      drift: num(c.drift, 0, 1, DEFAULT_CONFIG.drift),
      failures: num(c.failures, 0, 1, DEFAULT_CONFIG.failures),
      difficulty: num(c.difficulty, 0, 1, DEFAULT_CONFIG.difficulty),
      askBudget: num(c.askBudget, 0, 10, DEFAULT_CONFIG.askBudget),
    };
  }

  const shipStamp = (cfg) =>
    "ship|" + cfg.seed + "|" + cfg.drift + "|" + cfg.failures +
    "|" + cfg.difficulty + "|" + cfg.askBudget;

  /* ---------- world ---------- */

  function createGame(raw) {
    const cfg = normalizeConfig(raw);
    const rng = DK.makeRng("ship:" + cfg.seed);
    return {
      cfg, stamp: shipStamp(cfg), rng,
      ticks: 0, nextDecisionAt: DECIDE_EVERY,
      dist: START_DIST, dist0: START_DIST,
      fuel: 120, hull: 100, temp: 55, power: 70, crew: 100,
      ahead: null,                 /* hazard name when something is ahead */
      fire: false, engineFault: false,
      aborting: false, homeDist: 0,
      faults: {},                  /* channel -> {b: offset, until: ticks} */
      eventsFired: 0, nextEventAt: firstEventAt(cfg, rng),
      tokens: cfg.askBudget,
      decisions: 0, rungs: { rule: 0, engine: 0, player: 0 },
      engineCalls: 0, playerCalls: 0,
      tally: {},
      pending: null,
      over: false, outcome: null,
      log: [], history: [],
      lastDecision: null,
    };
  }

  function firstEventAt(cfg, rng) {
    /* difficulty 0 → a calm crossing with no events at all */
    if (cfg.difficulty <= 0) return Infinity;
    return Math.round((18 + rng() * 12) / cfg.difficulty) * TICKS_PER_S;
  }

  const key = (x) => x;

  function log(g, kind, note) {
    g.log.push({ tick: g.ticks, kind, note });
    if (g.log.length > LOG_MAX) g.log.shift();
  }

  /* ---------- sensors ---------- */

  /* true value ± drift noise; the backup additionally carries a stuck
     offset while a fault is live. Conflict = |a − b| > channel tolerance */
  function readSensors(g) {
    const out = {};
    for (const ch of CHANNELS) {
      const truth = ch === "temp" ? g.temp : ch === "fuel" ? g.fuel : g.hull;
      const noise = 3 * g.cfg.drift;
      const a = truth + (g.rng() * 2 - 1) * noise;
      const f = g.faults[ch];
      const b = f && g.ticks < f.until
        ? truth + f.b
        : truth + (g.rng() * 2 - 1) * noise;
      out[ch] = { a, b, conflict: Math.abs(a - b) > CHANNEL_TOL[ch] };
    }
    return out;
  }

  function injectSensorFault(g, channel) {
    if (!CHANNELS.includes(channel)) return null;
    const offset = (g.rng() < 0.5 ? -1 : 1) * (20 + g.rng() * 15);
    g.faults[channel] = { b: offset, until: g.ticks + FAULT_TICKS };
    log(g, "fault", CHANNEL_LABEL[channel] + " backup sensor stuck "
      + (offset < 0 ? "low" : "high") + " (" + (offset > 0 ? "+" : "") + offset.toFixed(0) + ")");
    return channel;
  }

  function conflictChannels(sensors) {
    return CHANNELS.filter((ch) => sensors[ch].conflict);
  }

  /* ---------- range planning ---------- */

  const canFinish = (g) => g.fuel >= g.dist * FUEL_PER_UNIT * SAFETY;
  const canReturn = (g) => g.fuel >= (g.dist0 - g.dist) * FUEL_PER_UNIT * SAFETY;

  /* ---------- scoring ---------- */

  function scoreActions(g) {
    const fuel = g.fuel, hull = g.hull, temp = g.temp, power = g.power;
    return {
      continue: 300 + fuel * 0.4 - Math.max(0, temp - 70) * 3
        - Math.max(0, 60 - hull) * 2,
      reduce: 240 + Math.max(0, temp - 75) * 6 + Math.max(0, power - 85) * 2
        + (g.engineFault ? 120 : 0),
      course: 280 + (g.ahead ? 400 : 0) - (hull < 50 ? 60 : 0) - fuel * 0.1,
      shutdown: 160 + Math.max(0, temp - 85) * 7 + (g.engineFault ? 260 : 0),
      home: 240 + (!canFinish(g) && canReturn(g) ? 400 : 0)
        + Math.max(0, 50 - fuel) * 2,
      emergency: 100 + Math.max(0, 45 - hull) * 8 + (g.fire ? 300 : 0)
        + (g.crew < 40 ? 150 : 0),
    };
  }

  /* ---------- rule gates (never asked) ---------- */

  function ruleGate(g) {
    if (g.hull <= 15) return { action: "emergency", why: "hull critical" };
    if (g.fire && g.crew < 30) return { action: "emergency", why: "fire, crew failing" };
    if (g.temp >= 95) return { action: "shutdown", why: "thermal limit" };
    if (!canFinish(g) && canReturn(g)) return { action: "home", why: "the crossing is out of range; Earth is not" };
    return null;
  }

  /* ---------- decision ladder ---------- */

  function decide(g, engineChoose) {
    if (g.over || g.pending) return null;
    const t0 = Date.now();
    const sensors = readSensors(g);
    const conflicts = conflictChannels(sensors);

    /* 1. rules fire first, whatever the scores say */
    const gate = ruleGate(g);
    if (gate) {
      return execute(g, gate.action, "rule", {
        why: gate.why, gap: null, conflicts,
        sensors, ms: Date.now() - t0,
      });
    }

    /* 2. score, sort, measure honest confidence */
    const scores = scoreActions(g);
    const ranked = ACTIONS.map((id) => ({ id, ev: scores[id] }))
      .sort((a, b) => b.ev - a.ev || ACTIONS.indexOf(a.id) - ACTIONS.indexOf(b.id));
    const gap = ranked[0].ev - ranked[1].ev;
    const agreement = Math.max(0.1, Math.pow(0.25, conflicts.length));
    const conf = DK.clampf(gap / 100, 0.02, 0.98) * agreement;

    const candidates = ranked.slice(0, 2).map((r) => ({
      id: r.id, description: ACTION_DESC[r.id],
    }));
    const stateText = "Ship " + (g.aborting ? "returning" : "outbound")
      + ": fuel " + g.fuel.toFixed(0)
      + " (sensors " + sensors.fuel.a.toFixed(0) + "/" + sensors.fuel.b.toFixed(0)
      + (sensors.fuel.conflict ? " CONFLICT" : "") + "), hull " + g.hull.toFixed(0)
      + " (" + sensors.hull.a.toFixed(0) + "/" + sensors.hull.b.toFixed(0)
      + (sensors.hull.conflict ? " CONFLICT" : "") + "), engine temp "
      + g.temp.toFixed(0) + " C (" + sensors.temp.a.toFixed(0) + "/"
      + sensors.temp.b.toFixed(0) + (sensors.temp.conflict ? " CONFLICT" : "")
      + "), power " + g.power.toFixed(0) + "%, " + g.dist.toFixed(0)
      + " to the destination, crew " + g.crew.toFixed(0) + "%."
      + (g.ahead ? " " + g.ahead + " ahead." : "")
      + (g.fire ? " Fire aboard." : "")
      + (g.engineFault ? " Engine fault." : "")
      + " Pick " + ACTION_NAME[candidates[0].id] + " or "
      + ACTION_NAME[candidates[1].id] + ".";

    /* 3. the abstain line: conflicting telemetry (or a dead-heat score)
       means the computer refuses to guess — the captain resolves it */
    if (conf < 0.3) {
      g.pending = {
        reason: conflicts.length ? "conflicting telemetry: "
          + conflicts.map((c) => CHANNEL_LABEL[c]).join(", ")
          : "no confident action",
        sensors, conflicts,
        question: {
          type: "choice", id: "ship-action",
          text: "The flight computer abstained (" + conf.toFixed(2)
            + " < 0.30). Captain's call:",
          candidates,
        },
      };
      log(g, "abstain", "computer abstained at conf " + conf.toFixed(2)
        + (conflicts.length ? " — " + conflicts.map((c) => CHANNEL_LABEL[c]).join(" + ")
          + " disagree" : "") + " — escalated to the captain");
      g.lastDecision = { action: null, rung: "player", conf,
        gap, conflicts, ms: Date.now() - t0, pending: true };
      return g.lastDecision;
    }

    /* 4. the uncertain band: the engine arbitrates, if it has a token */
    let pick = ranked[0].id;
    let rung = "rule";
    if (conf < 0.55) {
      if (g.tokens >= 1 && engineChoose && !g.over) {
        g.tokens -= 1;
        g.engineCalls += 1;
        const req = {
          state: { text: stateText, facts: {} },
          questions: [{
            type: "choice", id: "ship-action",
            text: ACTION_NAME[candidates[0].id] + " or "
              + ACTION_NAME[candidates[1].id] + "?",
            candidates,
          }],
          policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" },
        };
        const answer = engineChoose(req);
        const ans = answer && answer.outcome === "accept" && Array.isArray(answer.answers)
          ? answer.answers.find((a) => a && typeof a.choice === "string") : null;
        if (ans && candidates.some((c) => c.id === ans.choice)) {
          pick = ans.choice;
          rung = "engine";
        } else if (answer && answer.outcome === "abstain") {
          /* the engine ALSO refused → the captain decides; a malformed
             answer is NOT a refusal: the argmax holds, house rule */
          g.pending = {
            reason: "the engine abstained at conf " + conf.toFixed(2),
            sensors, conflicts,
            question: {
              type: "choice", id: "ship-action",
              text: "The engine refused the call. Captain's decision:",
              candidates,
            },
          };
          log(g, "abstain", "engine abstained at conf " + conf.toFixed(2)
            + " — escalated to the captain");
          g.lastDecision = { action: null, rung: "player", conf,
            gap, conflicts, ms: Date.now() - t0, pending: true };
          return g.lastDecision;
        }
      }
      /* no token → the argmax decides and the rung says so */
    }

    return execute(g, pick, rung, {
      why: "conf " + conf.toFixed(2) + (conflicts.length ? " (degraded)" : ""),
      gap: +gap.toFixed(1), conflicts, sensors, ms: Date.now() - t0,
    });
  }

  /* ---------- executing an action ---------- */

  function execute(g, action, rung, at) {
    g.tally[action] = (g.tally[action] || 0) + 1;
    g.decisions += 1;
    g.rungs[rung] += 1;
    applyAction(g, action);
    log(g, "action", ACTION_NAME[action] + " — " + rung
      + (at && at.why ? " (" + at.why + ")" : ""));
    g.lastDecision = Object.assign({ action, rung, pending: false }, at);
    g.nextDecisionAt = g.ticks + DECIDE_EVERY;
    return g.lastDecision;
  }

  function applyAction(g, action) {
    if (action === "continue") {
      if (g.power === 0) log(g, "action", "engines relit — power back to cruise");
      g.power = 70;
    } else if (action === "reduce") {
      g.power = 45;
    } else if (action === "course") {
      if (g.ahead) {
        log(g, "event", "steered around the " + g.ahead);
        g.ahead = null;
        g.fuel = Math.max(0, g.fuel - 2);
      }
    } else if (action === "shutdown") {
      g.power = 0;
    } else if (action === "home") {
      if (!g.aborting) {
        g.aborting = true;
        g.homeDist = g.dist0 - g.dist;
        g.ahead = null;
        log(g, "event", "course reversed — " + g.homeDist.toFixed(0) + " home");
      }
    } else if (action === "emergency") {
      if (g.fire) {
        g.fire = false;
        log(g, "event", "fire extinguished");
      }
      g.hull = Math.min(100, g.hull + 5);
      g.crew = Math.min(100, g.crew + 10);
      g.power = Math.min(g.power, 40);
      for (const ch of CHANNELS) delete g.faults[ch];   /* the crew recalibrates */
    }
  }

  /* the captain has spoken: resolve the pending loop */
  function resolvePending(g, choice) {
    if (!g.pending || g.over) return null;
    const cands = g.pending.question.candidates;
    if (!cands.some((c) => c.id === choice)) return null;
    const reason = g.pending.reason;
    g.pending = null;
    g.playerCalls += 1;
    return execute(g, choice, "player", { why: "captain — " + reason, gap: null, conflicts: [], sensors: null, ms: 0 });
  }

  /* ---------- events ---------- */

  const EVENT_KINDS = ["debris", "storm", "fault", "fire"];
  function injectEvent(g, kind) {
    if (g.over || g.pending) return null;
    if (!EVENT_KINDS.includes(kind)) return null;
    if (kind === "debris") {
      g.ahead = "a debris field";
    } else if (kind === "storm") {
      g.temp += 25;
    } else if (kind === "fault") {
      injectSensorFault(g, CHANNELS[Math.floor(g.rng() * CHANNELS.length)]);
    } else if (kind === "fire") {
      g.fire = true;
    }
    g.eventsFired += 1;
    log(g, "event", ({
      debris: "long-range scan: a debris field directly ahead",
      storm: "solar storm — plant temperature spiking",
      fire: "FIRE aboard — hull burning",
    })[kind] || "sensor fault reported");
    g.nextEventAt = g.ticks + Math.round((25 + g.rng() * 20) / Math.max(0.05, g.cfg.difficulty)) * TICKS_PER_S;
    return kind;
  }

  /* ---------- tick ---------- */

  function tickGame(g, engineChoose) {
    if (g.over || g.pending) return;      /* the computer will not act on a guess */
    g.ticks += 1;
    const dt = DT;
    g.tokens = Math.min(g.cfg.askBudget, g.tokens + g.cfg.askBudget * dt);

    /* physics — an empty tank cuts the engines; the ship drifts silent */
    if (g.fuel <= 0 && g.power > 0) {
      g.power = 0;
      log(g, "event", "fuel exhausted — engines cut");
    }
    const speed = g.power * 0.2;
    const burn = g.power * 0.012;
    g.fuel = Math.max(0, g.fuel - burn * dt);
    if (!g.aborting) g.dist = Math.max(0, g.dist - speed * dt);
    else g.homeDist = Math.max(0, g.homeDist - speed * dt);
    g.temp += ((g.power - 60) * 0.06 - (g.power === 0 ? 1.5 : 0.35)) * dt;
    if (g.fire) { g.hull -= 3 * dt; g.crew -= 0.4 * dt; }
    g.crew = Math.max(0, g.crew - 0.05 * dt);
    if (g.engineFault && g.temp <= 60) {
      g.engineFault = false;
      log(g, "event", "engine fault cleared after the plant cooled");
    }

    /* scheduled events */
    if (g.ticks >= g.nextEventAt) {
      const kind = EVENT_KINDS[Math.floor(g.rng() * EVENT_KINDS.length)];
      injectEvent(g, kind);
    }
    /* debris not steered around in time grazes the hull */
    if (g.ahead && g.rng() < 0.02) {
      g.hull -= 4;
      log(g, "event", "hull grazed by the " + g.ahead);
    }

    /* the flight computer speaks on its cadence */
    if (g.ticks >= g.nextDecisionAt) decide(g, engineChoose);

    /* outcomes */
    if (!g.over) {
      if (g.dist <= 0 && !g.aborting) endGame(g, "docked", "destination reached — docking clamps engaged");
      else if (g.aborting && g.homeDist <= 0) endGame(g, "returned", "safely home — the abort was the right call");
      else if (g.fuel <= 0 && g.power <= 1) endGame(g, "adrift", "fuel exhausted — the ship drifts silent");
      else if (g.hull <= 0) endGame(g, "destroyed", "hull failure — all lost");
      else if (g.crew <= 0) endGame(g, "lost", "the crew did not make it");
    }

    if (g.ticks % HISTORY_EVERY === 0) {
      g.history.push({ t: Math.round(g.ticks / TICKS_PER_S),
        dist: Math.round(g.dist), fuel: +g.fuel.toFixed(1),
        hull: Math.round(g.hull), temp: Math.round(g.temp),
        power: Math.round(g.power) });
      if (g.history.length > 400) g.history.shift();
    }
  }

  function endGame(g, outcome, note) {
    g.over = true;
    g.outcome = outcome;
    log(g, "end", note);
  }

  /* ---------- summary ---------- */

  function runSummary(g) {
    const simS = g.ticks / TICKS_PER_S;
    return {
      ticks: g.ticks, simSeconds: +simS.toFixed(1),
      decisions: g.decisions,
      perSecond: simS ? +(g.decisions / simS).toFixed(2) : 0,
      rungs: Object.assign({}, g.rungs),
      engineCalls: g.engineCalls,
      playerCalls: g.playerCalls,
      tally: Object.assign({}, g.tally),
      dist: +g.dist.toFixed(1), fuel: +g.fuel.toFixed(2),
      hull: +g.hull.toFixed(1), temp: +g.temp.toFixed(1),
      power: Math.round(g.power), crew: +g.crew.toFixed(1),
      aborting: g.aborting,
      faults: Object.keys(g.faults).filter(
        (ch) => g.faults[ch] && g.ticks < g.faults[ch].until).length,
      eventsFired: g.eventsFired,
      pending: g.pending ? g.pending.reason : null,
      outcome: g.outcome, over: g.over,
      logEntries: g.log.length, history: g.history.length,
    };
  }

  function simulateTicks(g, ticks, engineChoose) {
    for (let i = 0; i < ticks; i++) {
      if (g.over || g.pending) break;
      tickGame(g, engineChoose);
    }
    return g;
  }

  globalThis.ShipCore = {
    TICKS_PER_S, DECIDE_EVERY, START_DIST, FUEL_PER_UNIT, SAFETY, FAULT_TICKS,
    ACTIONS, ACTION_NAME, ACTION_DESC, CHANNELS, CHANNEL_TOL, CHANNEL_LABEL,
    EVENT_KINDS, DEFAULT_CONFIG,
    normalizeConfig, shipStamp, createGame, tickGame, simulateTicks,
    runSummary, decide, execute, resolvePending, injectSensorFault,
    injectEvent, readSensors, conflictChannels, scoreActions, ruleGate,
    canFinish, canReturn,
  };
})();
