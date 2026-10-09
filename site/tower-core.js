/* OpenCodifier — Tower Defense, core (DOM-free).
   §5.14 of the Decisions-SDK plan: continuous decision-making at volume,
   BOTH sides deciding on the ladder:

   - ENEMIES decide every DECIDE_EVERY ticks from their local situation
     (hp ratio, tower pressure on the route ahead, diversion availability,
     allies nearby): ADVANCE / TARGET TOWER (saboteurs) / RETREAT / REGROUP
     / DIVERT onto the alternate route. These are labelled RULE decisions.
   - TOWERS decide their targeting policy per shot: nearest / weakest /
     strongest / fastest / save-special / use-special — each candidate is
     SCORED on the frame-tick snapshot (threat-weighted expected damage,
     overkill penalty, kill value, blast value), the argmax wins, and a
     NEAR-TIE within TIE_EPS escalates to the engine as a real choice with
     the snapshot in the state (the full wire contract — a bare
     {questions} request throws in the WASM; see FINDINGS.md).
   - diversion routes unlock under pressure (tower damage this wave);
     enemies divert only when the route exists (test-pinned).
   - seeded wave composition, gold/bounties, lives, and a decisions/s
     counter measured in SIM time.

   The shell (tower.js) renders; this file never touches the DOM. */

(function () {
  "use strict";
  const DK = globalThis.DecisionsSDK;

  const TICKS_PER_S = 60;
  const CELL = 30;
  const W = 960, H = 480;                 /* canvas px (32 × 16 cells) */
  const DECIDE_EVERY = 30;                /* enemy decision cadence (ticks) */
  const TIE_EPS = 120;                    /* milli-value near-tie → engine */
  const BASE_LIVES = 20;
  const START_GOLD = 100;
  const TOWER_HP = 100;
  const DISABLE_TICKS = 300;              /* a destroyed tower is down 5s */
  const SPECIAL_CHARGE = 100;             /* shots to arm the special */
  const BLAST_RADIUS = 90;
  const DIVERT_DAMAGE = 250;              /* tower damage this wave that unlocks the alt route */

  const POLICIES = ["nearest", "weakest", "strongest", "fastest", "save-special", "use-special"];
  const POLICY_DESC = {
    "nearest": "shoot the enemy closest to this tower",
    "weakest": "shoot the lowest-hp enemy in range",
    "strongest": "shoot the highest-hp enemy in range",
    "fastest": "shoot the fastest-moving enemy in range",
    "save-special": "hold the special charge for a denser moment",
    "use-special": "detonate the special now if it is armed",
  };

  /* main route (cell coords, ×CELL+CELL/2 → px) and the alternate route
     that splits at SPLIT_AT and rejoins at JOIN_AT. The alt route is a
     deliberate long-way-round southern detour (2160 px vs 1440 px) —
     diverting buys safety and pays in travel time. */
  const MAIN_PTS = [[0, 5], [7, 5], [7, 10], [16, 10], [16, 4], [25, 4], [25, 9], [32, 9]];
  const ALT_PTS = [[7, 5], [7, 1], [2, 1], [2, 13], [10, 13], [10, 6], [30, 6], [30, 13], [25, 13], [25, 9]];
  const SPLIT_AT = 1;                     /* main waypoint index where they split */
  const JOIN_AT = 6;                      /* main waypoint index where they rejoin */

  const TOWER_SLOTS = [[5, 7], [9, 7], [12, 8], [14, 6], [18, 7], [22, 6], [14, 12], [27, 7]];
  const LOADOUTS = {
    standard: { slots: [0, 1, 2, 3, 4], dmg: 12, rate: 30, specialSlot: 2 },
    artillery: { slots: [1, 4, 7], dmg: 30, rate: 60, specialSlot: 4 },
    screen: { slots: [0, 1, 2, 3, 4, 5], dmg: 8, rate: 20, specialSlot: -1 },
    solo: { slots: [4], dmg: 18, rate: 25, specialSlot: -1 },
  };

  const ENEMY_TYPES = {
    grunt: { hp: 34, speed: 45, bounty: 5, leaks: 1, saboteur: false },
    runner: { hp: 18, speed: 85, bounty: 4, leaks: 1, saboteur: false },
    brute: { hp: 110, speed: 26, bounty: 14, leaks: 2, saboteur: false },
    saboteur: { hp: 40, speed: 50, bounty: 8, leaks: 1, saboteur: true },
  };
  const DEFAULT_CONFIG = {
    seed: "oc-tower", waves: 5, diversion: true, loadout: "standard", speed: 1,
  };

  /* ---------- geometry ---------- */

  const px = (c) => c * CELL + CELL / 2;
  function buildRoute(pts) {
    const wps = pts.map(([cx, cy]) => ({ x: px(cx), y: px(cy) }));
    const segs = [];
    let total = 0;
    for (let i = 0; i < wps.length - 1; i++) {
      const len = Math.hypot(wps[i + 1].x - wps[i].x, wps[i + 1].y - wps[i].y);
      segs.push({ a: wps[i], b: wps[i + 1], len, start: total });
      total += len;
    }
    return { wps, segs, total };
  }
  const MAIN = buildRoute(MAIN_PTS);
  const ALT = buildRoute(ALT_PTS);
  const SPLIT_T = MAIN.segs[SPLIT_AT].start;      /* px where the split begins */
  const JOIN_T = MAIN.segs[JOIN_AT].start;        /* px where they rejoin */

  function posOn(route, t) {
    for (const s of route.segs) {
      if (t <= s.start + s.len || s === route.segs[route.segs.length - 1]) {
        const k = s.len === 0 ? 0 : Math.min(1, Math.max(0, (t - s.start) / s.len));
        return { x: s.a.x + (s.b.x - s.a.x) * k, y: s.a.y + (s.b.y - s.a.y) * k };
      }
    }
    const last = route.wps[route.wps.length - 1];
    return { x: last.x, y: last.y };
  }
  const distToBase = (t) => Math.max(0, MAIN.total - t);   /* main-route progress ≈ threat */

  /* ---------- config + stamp ---------- */

  function normalizeConfig(raw) {
    const c = raw || {};
    const loadout = LOADOUTS[c.loadout] ? c.loadout : "standard";
    return {
      seed: c.seed === undefined || c.seed === null ? DEFAULT_CONFIG.seed : String(c.seed),
      waves: DK.clamp(Math.round(Number(c.waves) || DEFAULT_CONFIG.waves), 1, 20),
      diversion: c.diversion === undefined ? true : !!c.diversion,
      loadout,
      speed: DK.clampf(Number(c.speed) || 1, 0.5, 8, 1),
    };
  }

  const towerStamp = (cfg) =>
    "tower|" + cfg.seed + "|" + cfg.waves + "|" + (cfg.diversion ? "d" : "n") +
    "|" + cfg.loadout;

  /* ---------- game ---------- */

  function makeTower(slotIdx, spec, special) {
    const [cx, cy] = TOWER_SLOTS[slotIdx];
    return {
      id: "t" + slotIdx, x: px(cx), y: px(cy),
      range: 105, dmg: spec.dmg, rate: spec.rate,
      special, charge: 0, cooldown: 0,
      hp: TOWER_HP, disabledUntil: 0,
      policy: "auto", forced: 0, shots: 0, kills: 0,
    };
  }

  function createGame(raw) {
    const cfg = normalizeConfig(raw);
    const spec = LOADOUTS[cfg.loadout];
    const g = {
      cfg, stamp: towerStamp(cfg),
      ticks: 0, wave: 0,                       /* wave 0 = nothing spawned yet */
      lives: BASE_LIVES, gold: START_GOLD,
      towers: spec.slots.map((i) => makeTower(i, spec, i === spec.specialSlot)),
      enemies: [], shots: [], blasts: [],
      spawnQueue: [], spawnEvery: 45,
      waveDamage: 0, diversionUnlocked: false,
      kills: 0, leaked: 0, diverted: 0,
      decisions: 0, rungs: { rule: 0, engine: 0 },
      engineCalls: 0, waveDamageDealtTotal: 0,
      over: false, victory: false, lastDecision: null,
    };
    return g;
  }

  /* ---------- waves (seeded composition) ---------- */

  function waveComposition(wave, seed) {
    const rng = DK.makeRng("wave:" + seed + ":" + wave);
    const count = 6 + 2 * wave;
    const wBrute = Math.min(0.3, 0.04 + 0.04 * wave);
    const wRunner = 0.15 + 0.02 * wave;
    const wSaboteur = wave >= 3 ? 0.1 : 0;
    const list = [];
    for (let i = 0; i < count; i++) {
      const r = rng();
      const bruteAt = wBrute;
      const runnerAt = bruteAt + wRunner;
      const saboteurAt = runnerAt + wSaboteur;
      const type = r < bruteAt ? "brute" : r < runnerAt ? "runner"
        : r < saboteurAt ? "saboteur" : "grunt";
      list.push(type);
    }
    return list;
  }

  function startWave(g) {
    g.wave += 1;
    g.waveDamage = 0;
    g.diversionUnlocked = false;               /* re-locks each wave */
    g.spawnQueue = waveComposition(g.wave, g.cfg.seed);
    g.spawnEvery = Math.max(20, 45 - 2 * g.wave);
    g.spawnIn = 30;
  }

  function spawnEnemy(g, type) {
    const t = ENEMY_TYPES[type];
    const e = {
      id: "e" + g.ticks + "_" + g.enemies.length,
      type, ...t,
      maxHp: t.hp, hp: t.hp,
      route: "main", t: 0,                     /* px along the route */
      state: "advance", stateUntil: 0, decideIn: DECIDE_EVERY,
      target: null, diverted: false,
    };
    g.enemies.push(e);
    return e;
  }

  /* ---------- the enemy ladder (rules, labelled) ---------- */

  /* tower pressure = dps in range of the enemy's position ahead on its route */
  function towerPressure(g, e) {
    const p = posOn(e.route === "alt" ? ALT : MAIN, e.t);
    let dps = 0;
    for (const tw of g.towers) {
      if (g.ticks < tw.disabledUntil) continue;
      if (Math.hypot(tw.x - p.x, tw.y - p.y) <= tw.range) dps += tw.dmg * (TICKS_PER_S / tw.rate);
    }
    return dps;
  }

  function alliesNear(g, e) {
    const p = posOn(e.route === "alt" ? ALT : MAIN, e.t);
    let n = 0;
    for (const o of g.enemies) {
      if (o === e || o.hp <= 0) continue;
      const q = posOn(o.route === "alt" ? ALT : MAIN, o.t);
      if (Math.hypot(q.x - p.x, q.y - p.y) < 60) n += 1;
    }
    return n;
  }

  function decideEnemy(g, e) {
    e.decideIn = DECIDE_EVERY;
    /* divert only near the junction — the enemy walks BACK to the split
       point, so a mid-field divert would read as a teleport */
    const canDivert = g.diversionUnlocked && e.route === "main"
      && e.t >= SPLIT_T && e.t < SPLIT_T + 300;
    const pressure = towerPressure(g, e);
    const ratio = e.hp / e.maxHp;
    let choice = "advance";
    if (canDivert && pressure >= 20 && ratio > 0.35) {
      choice = "divert";
    } else if (ratio < 0.3 && pressure >= 20) {
      choice = "retreat";
    } else if (ratio < 0.55 && ratio >= 0.3 && alliesNear(g, e) >= 2) {
      choice = "regroup";
    } else if (e.saboteur) {
      const tw = nearestTower(g, posOn(e.route === "alt" ? ALT : MAIN, e.t));
      if (tw) choice = "target-tower";
    }
    applyEnemyChoice(g, e, choice);
    g.decisions += 1;
    g.rungs.rule += 1;
    g.lastDecision = {
      who: "enemy", id: e.id, type: e.type, sit: e.state,
      choice, at: { hp: Math.round(e.hp), pressure: Math.round(pressure) },
    };
  }

  function nearestTower(g, p) {
    let best = null, bd = Infinity;
    for (const tw of g.towers) {
      if (g.ticks < tw.disabledUntil) continue;
      const d = Math.hypot(tw.x - p.x, tw.y - p.y);
      if (d < 70 && d < bd) { bd = d; best = tw; }
    }
    return best;
  }

  function applyEnemyChoice(g, e, choice) {
    if (choice === "divert") {
      /* switch to the alternate route at the same split point */
      e.route = "alt"; e.diverted = true; e.t = 0;
      g.diverted += 1;
      e.state = "advance";
    } else if (choice === "retreat") {
      e.state = "retreat"; e.stateUntil = g.ticks + 90;
    } else if (choice === "regroup") {
      e.state = "regroup"; e.stateUntil = g.ticks + 120;
    } else if (choice === "target-tower") {
      e.state = "target-tower"; e.stateUntil = g.ticks + 150;
      e.target = nearestTower(g, posOn(e.route === "alt" ? ALT : MAIN, e.t));
    } else {
      e.state = "advance";
    }
  }

  /* ---------- the tower ladder (scored candidates + engine near-ties) ---------- */

  function inRange(g, tw) {
    return g.enemies.filter((e) => {
      if (e.hp <= 0) return false;
      const p = posOn(e.route === "alt" ? ALT : MAIN, e.t);
      return Math.hypot(tw.x - p.x, tw.y - p.y) <= tw.range;
    });
  }

  const policyTarget = (policy, es, tw) => {
    if (!es.length) return null;
    const sorted = es.slice().sort((a, b) => {
      const pa = posOn(a.route === "alt" ? ALT : MAIN, a.t);
      const pb = posOn(b.route === "alt" ? ALT : MAIN, b.t);
      const da = Math.hypot(tw.x - pa.x, tw.y - pa.y);
      const db = Math.hypot(tw.x - pb.x, tw.y - pb.y);
      if (policy === "nearest") return da - db;
      if (policy === "weakest") return a.hp - b.hp;
      if (policy === "strongest") return b.hp - a.hp;
      return b.speed - a.speed;                       /* fastest */
    });
    return sorted[0];
  };

  /* score one target-picking policy on the snapshot: threat-weighted
     damage minus overkill, plus a kill bonus (integers, milli-value) */
  function scoreTarget(tw, e) {
    const dmg = Math.min(tw.dmg, e.hp);
    const over = Math.max(0, tw.dmg - e.hp);
    const threat = 1 + Math.round(1000 * (1 - Math.min(1, distToBase(e.t) / MAIN.total)));
    let v = 600 * (dmg / e.maxHp) * threat / 1000 + (tw.dmg >= e.hp ? 400 : 0) - over * 8;
    if (e.type === "saboteur") v += 200;              /* protecting towers matters */
    return Math.round(v);
  }

  function decideTower(g, tw, engineChoose) {
    const es = inRange(g, tw);
    tw.cooldown = tw.rate;
    if (!es.length) return null;                       /* holding fire is free */
    g.decisions += 1;
    let pick, rung = "rule", evGap = 0, req = null;

    if (tw.policy !== "auto") {
      tw.forced += 1;
      /* the shell only offers target-picking policies as overrides */
      pick = POLICIES.slice(0, 4).indexOf(tw.policy) !== -1 ? tw.policy : "nearest";
    } else {
      const cands = [];
      /* policies that pick the SAME target are the same action — keep the
         first in POLICIES order, so the engine is only asked when distinct
         actions genuinely contest (measured: without this, 75% of tower
         decisions are single-target gap-0 fakes) */
      const byTarget = new Map();
      for (const p of ["nearest", "weakest", "strongest", "fastest"]) {
        const t = policyTarget(p, es, tw);
        if (!t || byTarget.has(t)) continue;
        byTarget.set(t, p);
        cands.push({ id: p, ev: scoreTarget(tw, t) });
      }
      if (tw.special) {
        const inBlast = es.filter((e) => {
          const q = posOn(e.route === "alt" ? ALT : MAIN, e.t);
          return Math.hypot(tw.x - q.x, tw.y - q.y) <= BLAST_RADIUS;
        }).length;
        cands.push({ id: "save-special", ev: tw.charge >= SPECIAL_CHARGE ? 200 : 400 + tw.charge * 3 });
        cands.push({ id: "use-special", ev: tw.charge >= SPECIAL_CHARGE
          ? 700 + inBlast * 250 : -1000 });
      }
      if (!cands.length) return null;
      cands.sort((a, b) => b.ev - a.ev || POLICIES.indexOf(a.id) - POLICIES.indexOf(b.id));
      /* a single action is not a contest — never escalated */
      evGap = cands.length > 1 ? cands[0].ev - cands[1].ev : Infinity;
      pick = cands[0].id;
      if (cands.length > 1 && evGap <= TIE_EPS && engineChoose) {
        /* the full wire contract — snapshot in the state, policies as
           candidates. A bare {questions} request throws in the WASM. */
        req = {
          state: {
            text: "Tower " + tw.id + " snapshot at tick " + g.ticks + ", wave " + g.wave +
              ": " + es.length + " enemies in range (" +
              es.map((e) => e.type + " hp " + Math.round(e.hp) + " speed " + e.speed).join("; ") +
              "). Scores (milli-value): " + cands.map((c) => c.id + " " + c.ev).join(", ") +
              ". Top two within " + (TIE_EPS / 1000).toFixed(2) + ". Tower special charge " +
              tw.charge + "/" + SPECIAL_CHARGE + ". Pick a targeting policy.",
            facts: {},
          },
          questions: [{
            type: "choice", id: "tower-policy",
            text: "What should tower " + tw.id + " do this shot?",
            candidates: cands.map((c) => ({ id: c.id, description: POLICY_DESC[c.id] })),
          }],
          policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" },
        };
        g.engineCalls += 1;
        const answer = engineChoose(req);
        const ans = answer && answer.outcome === "accept" && Array.isArray(answer.answers)
          ? answer.answers.find((a) => a && typeof a.choice === "string") : null;
        if (ans && cands.some((c) => c.id === ans.choice)) {
          pick = ans.choice;
          rung = "engine";
        }
      }
      g.lastDecision = {
        who: "tower", id: tw.id, sit: "wave" + g.wave + "×" + es.length,
        choice: pick, rung, evGap,
        at: { inRange: es.length, charge: tw.charge },
      };
    }

    if (pick === "use-special") {
      if (tw.charge >= SPECIAL_CHARGE) fireSpecial(g, tw, es);
      else tw.charge = Math.min(SPECIAL_CHARGE, tw.charge + 2);   /* unarmed: hold */
    } else if (pick === "save-special") {
      tw.charge = Math.min(SPECIAL_CHARGE, tw.charge + 2);
    } else {
      const t = policyTarget(pick, es, tw);
      if (t) fireShot(g, tw, t);
    }
    g.rungs[rung] += 1;
    return { pick, rung, evGap };
  }

  function fireShot(g, tw, e) {
    const p = posOn(e.route === "alt" ? ALT : MAIN, e.t);
    g.shots.push({ x1: tw.x, y1: tw.y, x2: p.x, y2: p.y, life: 6 });
    tw.shots += 1;
    tw.charge = Math.min(SPECIAL_CHARGE, tw.charge + 6);
    damageEnemy(g, e, tw.dmg, tw);
  }

  function fireSpecial(g, tw, es) {
    tw.charge = 0;
    tw.shots += 1;
    for (const e of es.slice()) {
      const q = posOn(e.route === "alt" ? ALT : MAIN, e.t);
      if (Math.hypot(tw.x - q.x, tw.y - q.y) <= BLAST_RADIUS) {
        damageEnemy(g, e, tw.dmg * 3, tw);
      }
    }
    g.blasts.push({ x: tw.x, y: tw.y, r: BLAST_RADIUS, life: 12 });
  }

  function damageEnemy(g, e, dmg, tw) {
    /* saboteurs are armored: they must survive long enough next to a
       tower to break it (measured: unarmored, flanking fire kills them
       at ~t31 while a tower needs 50 ticks to break — dead mechanic) */
    if (e.saboteur) dmg = Math.ceil(dmg / 2);
    const dealt = Math.min(e.hp, dmg);
    e.hp -= dmg;
    g.waveDamage += dealt;
    g.waveDamageDealtTotal += dealt;
    if (e.hp <= 0) {
      g.kills += 1;
      g.gold += e.bounty;
      if (tw) tw.kills += 1;
    }
    if (g.cfg.diversion && g.waveDamage >= DIVERT_DAMAGE) g.diversionUnlocked = true;
  }

  /* ---------- tick ---------- */

  function tickGame(g, engineChoose) {
    if (g.over) return;
    g.ticks += 1;

    /* spawn */
    if (g.spawnQueue.length) {
      g.spawnIn -= 1;
      if (g.spawnIn <= 0) {
        spawnEnemy(g, g.spawnQueue.shift());
        g.spawnIn = g.spawnEvery;
      }
    } else if (g.enemies.length === 0 && !g.over) {
      if (g.wave >= g.cfg.waves) {
        g.over = true;
        g.victory = g.lives > 0;
        return;
      }
      startWave(g);
    }

    /* enemies */
    for (const e of g.enemies) {
      if (e.hp <= 0) continue;
      if (e.decideIn > 0) e.decideIn -= 1;
      if (e.decideIn === 0 && e.state !== "retreat" && e.state !== "regroup") {
        decideEnemy(g, e);
      } else if (e.decideIn === 0) {
        e.decideIn = DECIDE_EVERY;                     /* mid-state: finish it first */
        if (g.ticks >= e.stateUntil) { e.state = "advance"; }
      }
      if (g.ticks >= e.stateUntil && e.state !== "advance" && e.state !== "target-tower") {
        e.state = "advance";
      }
      const v = e.speed / TICKS_PER_S;
      if (e.state === "advance") {
        const route = e.route === "alt" ? ALT : MAIN;
        e.t += v;
        if (e.route === "main" && e.t >= MAIN.total) { leak(g, e); continue; }
        if (e.route === "alt" && e.t >= ALT.total) {
          /* rejoined: continue on main past the join point */
          e.route = "main"; e.t = JOIN_T;
        }
      } else if (e.state === "retreat") {
        e.t = Math.max(0, e.t - v * 0.6);
        e.hp = Math.min(e.maxHp, e.hp + 0.15);
      } else if (e.state === "regroup") {
        e.hp = Math.min(e.maxHp, e.hp + 0.25);
      } else if (e.state === "target-tower") {
        const alive = e.target && e.target.hp > 0 && g.ticks >= e.target.disabledUntil;
        if (alive) {
          e.target.hp -= 2;
          if (e.target.hp <= 0) {
            e.target.disabledUntil = g.ticks + DISABLE_TICKS;
            e.target.hp = TOWER_HP;                    /* repaired after the outage */
            e.state = "advance"; e.stateUntil = g.ticks;
          }
        } else {
          e.state = "advance";
        }
      }
    }
    g.enemies = g.enemies.filter((e) => e.hp > 0 && !e.done);

    /* towers */
    for (const tw of g.towers) {
      if (g.ticks < tw.disabledUntil) continue;
      if (tw.cooldown > 0) { tw.cooldown -= 1; continue; }
      decideTower(g, tw, engineChoose);
    }

    /* fx decay */
    for (const s of g.shots) s.life -= 1;
    g.shots = g.shots.filter((s) => s.life > 0);
    for (const b of g.blasts) b.life -= 1;
    g.blasts = g.blasts.filter((b) => b.life > 0);

    if (g.lives <= 0 && !g.over) {
      g.over = true;
      g.victory = false;
    }
  }

  function leak(g, e) {
    e.done = true;
    g.lives -= e.leaks;
    g.leaked += 1;
  }

  /* ---------- summary ---------- */

  function runSummary(g) {
    return {
      over: g.over, victory: g.victory,
      wave: g.wave, waves: g.cfg.waves,
      lives: g.lives, gold: g.gold,
      kills: g.kills, leaked: g.leaked, diverted: g.diverted,
      decisions: g.decisions,
      perSecond: g.ticks ? +(g.decisions / (g.ticks / TICKS_PER_S)).toFixed(2) : 0,
      rungs: Object.assign({}, g.rungs),
      engineCalls: g.engineCalls,
      ticks: g.ticks,
      diversionUnlocked: g.diversionUnlocked,
    };
  }

  /* whole-run simulation at 1× — the deterministic drive the tests use */
  function simulateRun(cfg, engineChoose) {
    const g = createGame(cfg);
    let guard = TICKS_PER_S * 60 * 20;
    while (!g.over && guard-- > 0) tickGame(g, engineChoose);
    return g;
  }

  globalThis.TowerCore = {
    TICKS_PER_S, CELL, W, H, DECIDE_EVERY, TIE_EPS, BASE_LIVES, START_GOLD,
    SPECIAL_CHARGE, BLAST_RADIUS, DIVERT_DAMAGE,
    POLICIES, POLICY_DESC, LOADOUTS, ENEMY_TYPES, DEFAULT_CONFIG,
    MAIN_PTS, ALT_PTS, TOWER_SLOTS,
    MAIN, ALT, SPLIT_T, JOIN_T, posOn, distToBase,
    normalizeConfig, towerStamp,
    createGame, startWave, waveComposition, spawnEnemy,
    towerPressure, alliesNear, decideEnemy, inRange, policyTarget, scoreTarget,
    decideTower, tickGame, leak, runSummary, simulateRun,
  };
})();
