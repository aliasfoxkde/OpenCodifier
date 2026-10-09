/* OpenCodifier — MicroWorld, core (DOM-free).
   The flagship: one tiny deterministic world where the creatures are the
   decision agents. Every creature re-scores ten candidate actions every
   ~0.75 s from its own meters and its local view — eat, drink, flee,
   fight, explore, rest, gather, go home, follow, ignore. Outside the
   close band the argmax rule decides; a genuine near-tie (|gap| ≤ 1.5)
   goes to the engine over just the tied candidates — if the colony-wide
   batching budget has a token.

   Division of labor, exactly as the card promises:
     - CREATURES decide. Ten scored actions, meters, local view, ties
       escalated on the record.
     - THE FOX is a rule, never a question: it hunts the nearest creature
       in sight and is driven off by a pack of two or more. Predation,
       starvation, thirst, births from colony stores — all rules.
     - The world is a seeded value lattice: grass regrows food, ponds
       quench thirst, the home tiles hold the colony's stores.

   Population, births, deaths, stores and the decision tallies make the
   sliders visible. The shell (microworld.js) renders; this file never
   touches the DOM. */

(function () {
  "use strict";
  const DK = globalThis.DecisionsSDK;

  const TICKS_PER_S = 60;
  const CELL = 20;
  const GW = 48, GH = 30;               /* 960 × 600, like traffic */
  const SIGHT = 8;                      /* creature view radius (cells) */
  const DECIDE_EVERY = 45;              /* ~0.75 s between decisions */
  const TIE_EPS = 1.5;                  /* genuine-tie band (score units) */
  const FOOD_MAX = 10;
  const STORE_CAP = 24;
  const BIRTH_COST = 6;                 /* stores per newborn */
  const BIRTH_EVERY = 300;              /* min ticks between births */
  const FOX_HUNT_R = 7;                 /* fox sight */
  const FOX_PACK_R = 2;                 /* pack-defense radius */
  const FOX_PACK_MIN = 2;               /* creatures needed to drive it off */
  const MOVE_EAT = 9, MOVE_WALK = 8, MOVE_FLEE = 4, MOVE_FIGHT = 6;

  const ACTIONS = ["eat", "drink", "flee", "fight", "explore", "rest",
    "gather", "gohome", "follow", "ignore"];
  const ACTION_DESC = {
    eat: "seek and eat the grass under or ahead — energy back up",
    drink: "walk to the nearest pond and drink — thirst back up",
    flee: "run from the fox — it is close and closing",
    fight: "face the fox down — a pack of two drives it off",
    explore: "push toward the least-visited ground in sight",
    rest: "hold still and recover a little energy — no threat near",
    gather: "strip a rich tile and carry the food home",
    gohome: "carry the food back to the colony stores",
    follow: "drift toward the nearest neighbour — safety in numbers",
    ignore: "do nothing this round — the world can wait",
  };
  const TILE = { GRASS: 0, WATER: 1, HOME: 2 };

  const DEFAULT_CONFIG = {
    seed: "oc-microworld",
    creatures: 34,       /* starting population */
    regrow: 1,           /* grass regrowth multiplier */
    budget: 2,           /* engine asks per second, colony-wide */
  };

  function normalizeConfig(raw) {
    const c = raw || {};
    const num = (v, d) => (typeof v === "number" && isFinite(v) ? v : d);
    const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
    const seed = c.seed === undefined || c.seed === null || c.seed === ""
      ? DEFAULT_CONFIG.seed : String(c.seed);
    return {
      seed,
      creatures: Math.round(clamp(num(c.creatures, DEFAULT_CONFIG.creatures), 8, 100)),
      regrow: clamp(num(c.regrow, DEFAULT_CONFIG.regrow), 0, 2),
      budget: Math.round(clamp(num(c.budget, DEFAULT_CONFIG.budget), 0, 10)),
    };
  }

  function microworldStamp(cfg) {
    const c = normalizeConfig(cfg);
    return "creatures " + c.creatures + " · regrow " + c.regrow.toFixed(2) +
      " · budget " + c.budget + "/s";
  }

  /* ---------- world ---------- */
  function idx(x, y) { return y * GW + x; }
  function inBounds(x, y) { return x >= 0 && y >= 0 && x < GW && y < GH; }
  function cheb(ax, ay, bx, by) {
    return Math.max(Math.abs(ax - bx), Math.abs(ay - by));
  }

  function createWorld(rawCfg) {
    const cfg = normalizeConfig(rawCfg);
    const rng = DK.makeRng("world:" + cfg.seed);
    const tiles = new Uint8Array(GW * GH);          /* TILE.* */
    const food = new Float32Array(GW * GH);
    const visits = new Float32Array(GW * GH);

    /* two seeded ponds */
    const ponds = [];
    for (let p = 0; p < 2; p++) {
      ponds.push({ x: 6 + Math.floor(rng() * (GW - 12)), y: 4 + Math.floor(rng() * (GH - 8)),
        r: 2.4 + rng() * 1.8 });
    }
    for (let y = 0; y < GH; y++) {
      for (let x = 0; x < GW; x++) {
        for (const pond of ponds) {
          const dx = (x - pond.x) * (1 + rng() * 0.12), dy = y - pond.y;
          if (dx * dx + dy * dy <= pond.r * pond.r) tiles[idx(x, y)] = TILE.WATER;
        }
      }
    }
    /* home: a 3×2 colony plot dead center, over any pond */
    const home = { x: 23, y: 14, w: 3, h: 2 };
    for (let y = home.y; y < home.y + home.h; y++) {
      for (let x = home.x; x < home.x + home.w; x++) tiles[idx(x, y)] = TILE.HOME;
    }
    /* initial food: grass everywhere, richer away from home */
    for (let i = 0; i < tiles.length; i++) {
      if (tiles[i] === TILE.GRASS) food[i] = 2 + rng() * (FOOD_MAX - 2);
    }

    const world = {
      cfg, tiles, food, visits, home,
      ticks: 0, tokens: cfg.budget,
      creatures: [], nextId: 1,
      fox: { x: 4 + Math.floor(rng() * (GW - 8)), y: 4 + Math.floor(rng() * (GH - 8)),
        dir: Math.floor(rng() * 4), cd: 120, cool: 0 },
      stores: 4, births: 0, deaths: 0, drives: 0,
      engineCalls: 0, ruleCalls: 0,
      lastBirth: 0, events: [], over: false,
      rng,
    };
    for (let i = 0; i < cfg.creatures; i++) spawnCreature(world);
    return world;
  }

  function spawnCreature(world) {
    const rng = world.rng;
    for (let tries = 0; tries < 200; tries++) {
      const x = 2 + Math.floor(rng() * (GW - 4));
      const y = 2 + Math.floor(rng() * (GH - 4));
      if (world.tiles[idx(x, y)] !== TILE.GRASS) continue;
      if (cheb(x, y, world.fox.x, world.fox.y) < 6) continue;
      world.creatures.push({
        id: world.nextId++,
        x, y,
        energy: 55 + rng() * 35,
        water: 55 + rng() * 35,
        carrying: 0,
        action: "explore", rung: "rule", scores: [],
        nextDecide: Math.floor(rng() * DECIDE_EVERY),
        moveAcc: 0,
      });
      return;
    }
  }

  /* ---------- local view ---------- */
  function scanNearest(world, x, y, pred) {
    let best = null, bestD = Infinity;
    const lo = Math.max(0, x - SIGHT), hi = Math.min(GW - 1, x + SIGHT);
    const loY = Math.max(0, y - SIGHT), hiY = Math.min(GH - 1, y + SIGHT);
    for (let ty = loY; ty <= hiY; ty++) {
      for (let tx = lo; tx <= hi; tx++) {
        if (!pred(tx, ty)) continue;
        const d = cheb(x, y, tx, ty);
        if (d < bestD) { bestD = d; best = { x: tx, y: ty, d }; }
      }
    }
    return best;
  }

  function nearestMate(world, c) {
    let best = null, bestD = Infinity;
    for (const o of world.creatures) {
      if (o === c) continue;
      const d = cheb(c.x, c.y, o.x, o.y);
      if (d < bestD) { bestD = d; best = o; }
    }
    return best ? { d: bestD, mate: best } : null;
  }

  function packSize(world, x, y) {
    let n = 0;
    for (const o of world.creatures) {
      if (cheb(o.x, o.y, x, y) <= FOX_PACK_R) n++;
    }
    return n;
  }

  /* ---------- decision: ten scored actions, ties to the engine ---------- */
  function scoreActions(world, c) {
    const t = world.tiles[idx(c.x, c.y)];
    const foodHere = t === TILE.GRASS ? world.food[idx(c.x, c.y)] : 0;
    const threat = cheb(c.x, c.y, world.fox.x, world.fox.y);
    const fd = scanNearest(world, c.x, c.y, (tx, ty) =>
      world.tiles[idx(tx, ty)] === TILE.GRASS && world.food[idx(tx, ty)] >= 1.5);
    const wd = scanNearest(world, c.x, c.y, (tx, ty) =>
      world.tiles[idx(tx, ty)] === TILE.WATER);
    const hd = cheb(c.x, c.y, world.home.x + 1, world.home.y);
    const pack = packSize(world, c.x, c.y);
    const mate = nearestMate(world, c);
    const visitsHere = world.visits[idx(c.x, c.y)];

    const s = {};
    s.flee = threat <= 7 ? 95 - threat * 9 : 0;
    s.fight = threat <= 5 && (pack >= FOX_PACK_MIN || c.energy > 75) ? 62 + pack * 4 : 0;
    s.drink = c.water < 60 && wd ? (60 - c.water) * 1.1 + 22 - wd.d * 2 : 0;
    s.eat = c.energy < 78 && fd ? (78 - c.energy) * 0.9 + 16 - fd.d * 1.6 : 0;
    s.gather = !c.carrying && foodHere >= 4 && c.energy > 35 && world.stores < STORE_CAP
      ? 28 + foodHere * 3 : 0;
    s.gohome = c.carrying ? 58 - hd * 1.2 : 0;
    s.rest = c.energy < 35 && threat > 7 ? 44 - c.energy : 0;
    s.follow = threat > 6 && c.energy > 40 && mate && mate.d <= 4 ? 24 : 0;
    s.explore = Math.max(6, 20 - visitsHere * 2.5);
    s.ignore = 6;
    for (const a of ACTIONS) s[a] = Math.max(0, s[a]);

    const facts = { threat, pack, foodHere, carrying: c.carrying,
      energy: Math.round(c.energy), water: Math.round(c.water),
      distFood: fd ? fd.d : -1, distWater: wd ? wd.d : -1,
      distHome: hd, stores: world.stores };
    return { scores: s, facts };
  }

  function decideCreature(world, c, engineChoose) {
    const { scores, facts } = scoreActions(world, c);
    const ranked = ACTIONS.map((a) => ({ id: a, score: scores[a] }))
      .sort((a, b) => b.score - a.score || ACTIONS.indexOf(a.id) - ACTIONS.indexOf(b.id));
    c.scores = ranked;

    let pick = ranked[0].id;
    let rung = "rule";
    world.ruleCalls += 1;

    const gap = ranked[0].score - (ranked[1] ? ranked[1].score : 0);
    const tied = ranked.filter((r) => r.score > 0 && ranked[0].score - r.score <= TIE_EPS);
    if (tied.length >= 2 && world.tokens >= 1 && engineChoose && !world.over) {
      world.tokens -= 1;
      world.engineCalls += 1;
      const req = {
        state: {
          text: "Creature #" + c.id + " at tick " + world.ticks +
            ". Energy " + facts.energy + "/100, water " + facts.water +
            "/100, carrying " + (facts.carrying ? "food" : "nothing") +
            ". Fox " + facts.threat + " cells away, pack " + facts.pack +
            ". Grass food here " + facts.foodHere.toFixed(1) +
            ", pond " + (facts.distWater < 0 ? "out of sight" : facts.distWater + " cells") +
            ", home " + facts.distHome + " cells, stores " + facts.stores +
            ". The top scored actions tie within " + TIE_EPS + ". Pick the next action.",
          facts: {},
        },
        questions: [{
          type: "choice", id: "creature-action",
          text: "What should creature #" + c.id + " do next?",
          candidates: tied.map((r) => ({ id: r.id, description: ACTION_DESC[r.id] })),
        }],
        policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" },
      };
      const answer = engineChoose(req);
      const ans = answer && answer.outcome === "accept" && Array.isArray(answer.answers)
        ? answer.answers.find((a) => a && typeof a.choice === "string") : null;
      if (ans && tied.some((r) => r.id === ans.choice)) {
        pick = ans.choice;
        rung = "engine";
        world.ruleCalls -= 1;
      }
    }
    c.action = pick;
    c.rung = rung;
    c.nextDecide = world.ticks + DECIDE_EVERY + Math.floor(world.rng() * 11);
    return { action: pick, rung, scores: ranked, facts };
  }

  /* ---------- movement ---------- */
  function stepToward(world, c, tx, ty, rate) {
    c.moveAcc += 1 / rate;
    if (c.moveAcc < 1) return;
    c.moveAcc -= 1;
    const dx = Math.sign(tx - c.x), dy = Math.sign(ty - c.y);
    const horizFirst = Math.abs(tx - c.x) >= Math.abs(ty - c.y);
    const tryOrder = horizFirst ? [[dx, 0], [0, dy]] : [[0, dy], [dx, 0]];
    for (const [mx, my] of tryOrder) {
      if (!mx && !my) continue;
      const nx = c.x + mx, ny = c.y + my;
      if (!inBounds(nx, ny) || world.tiles[idx(nx, ny)] === TILE.WATER) continue;
      c.x = nx; c.y = ny;
      return;
    }
  }

  function stepAway(world, c, fx, fy, rate) {
    c.moveAcc += 1 / rate;
    if (c.moveAcc < 1) return;
    c.moveAcc -= 1;
    const dx = Math.sign(c.x - fx), dy = Math.sign(c.y - fy);
    for (const [mx, my] of [[dx, 0], [0, dy], [-dx, 0], [0, -dy]]) {
      if (!mx && !my) continue;
      const nx = c.x + mx, ny = c.y + my;
      if (!inBounds(nx, ny) || world.tiles[idx(nx, ny)] === TILE.WATER) continue;
      c.x = nx; c.y = ny;
      return;
    }
  }

  /* ---------- one tick ---------- */
  function tickWorld(world, engineChoose) {
    if (world.over) return world;
    world.ticks += 1;

    /* budget: tokens refill every sim-second, colony-wide */
    if (world.ticks % TICKS_PER_S === 0) {
      world.tokens = Math.min(world.cfg.budget, world.tokens + world.cfg.budget);
    }
    /* grass regrows; memory of visited ground fades */
    const regrow = 0.0011 * world.cfg.regrow;
    for (let i = 0; i < world.tiles.length; i++) {
      if (world.tiles[i] === TILE.GRASS && world.food[i] < FOOD_MAX) {
        world.food[i] = Math.min(FOOD_MAX, world.food[i] + regrow);
      }
    }
    if (world.ticks % TICKS_PER_S === 0) {
      for (let i = 0; i < world.visits.length; i++) {
        if (world.visits[i] > 0) world.visits[i] *= 0.985;
      }
    }

    /* THE FOX — a rule, never a question */
    const fox = world.fox;
    if (fox.cool > 0) fox.cool -= 1;
    let prey = null, preyD = Infinity;
    for (const c of world.creatures) {
      const d = cheb(c.x, c.y, fox.x, fox.y);
      if (d <= FOX_HUNT_R && d < preyD) { preyD = d; prey = c; }
    }
    if (prey && fox.cool === 0) {
      stepTowardFox(world, fox, prey.x, prey.y);
      if (fox.x === prey.x && fox.y === prey.y) {
        const pack = packSize(world, prey.x, prey.y);
        if (pack >= FOX_PACK_MIN) {
          world.drives += 1;
          world.events.push({ tick: world.ticks, kind: "drive", id: prey.id });
          fox.cool = 300;
          bounceFox(world, fox, prey.x, prey.y);
        } else {
          killCreature(world, prey, "predation");
          world.events.push({ tick: world.ticks, kind: "kill", id: prey.id });
          fox.cool = 300;
        }
      }
    } else {
      /* wander: pick a new heading every ~2 s, drift, bounce off walls */
      if (world.ticks % 120 === 0) fox.dir = Math.floor(world.rng() * 4);
      const DX = [1, 0, -1, 0], DY = [0, 1, 0, -1];
      if (world.ticks % 3 === 0) {
        const nx = fox.x + DX[fox.dir], ny = fox.y + DY[fox.dir];
        if (inBounds(nx, ny)) { fox.x = nx; fox.y = ny; } else { fox.dir = (fox.dir + 2) % 4; }
      }
    }

    /* creatures: meters, decisions, actions */
    for (const c of world.creatures.slice()) {
      c.energy -= 0.018;
      c.water -= 0.026;
      if (c.energy <= 0) { killCreature(world, c, "starvation"); continue; }
      if (c.water <= 0) { killCreature(world, c, "thirst"); continue; }

      if (world.ticks >= c.nextDecide) decideCreature(world, c, engineChoose);
      world.visits[idx(c.x, c.y)] += 1;

      const ti = idx(c.x, c.y);
      const tile = world.tiles[ti];
      switch (c.action) {
        case "eat": {
          if (tile === TILE.GRASS && world.food[ti] >= 0.35) {
            world.food[ti] -= 0.35;
            c.energy = Math.min(100, c.energy + 0.55);
          } else {
            const fd = scanNearest(world, c.x, c.y, (tx, ty) =>
              world.tiles[idx(tx, ty)] === TILE.GRASS && world.food[idx(tx, ty)] >= 1.5);
            if (fd) stepToward(world, c, fd.x, fd.y, MOVE_EAT);
          }
          break;
        }
        case "drink": {
          const wd = scanNearest(world, c.x, c.y, (tx, ty) =>
            world.tiles[idx(tx, ty)] === TILE.WATER);
          if (wd && wd.d <= 1) c.water = Math.min(100, c.water + 1.2);
          else if (wd) stepToward(world, c, wd.x, wd.y, MOVE_WALK);
          break;
        }
        case "flee":
          stepAway(world, c, world.fox.x, world.fox.y, MOVE_FLEE);
          break;
        case "fight":
          stepTowardFox(world, c, world.fox.x, world.fox.y, MOVE_FIGHT);
          if (cheb(c.x, c.y, world.fox.x, world.fox.y) <= 1 &&
              packSize(world, world.fox.x, world.fox.y) >= FOX_PACK_MIN) {
            world.drives += 1;
            world.events.push({ tick: world.ticks, kind: "drive", id: c.id });
            world.fox.cool = 240;
            bounceFox(world, world.fox, c.x, c.y);
          }
          break;
        case "gather": {
          if (!c.carrying && tile === TILE.GRASS && world.food[ti] >= 4) {
            world.food[ti] -= 3;
            c.carrying = 1;
          } else if (!c.carrying) {
            const fd = scanNearest(world, c.x, c.y, (tx, ty) =>
              world.tiles[idx(tx, ty)] === TILE.GRASS && world.food[idx(tx, ty)] >= 4);
            if (fd) stepToward(world, c, fd.x, fd.y, MOVE_WALK);
          }
          break;
        }
        case "gohome": {
          const hx = world.home.x + 1, hy = world.home.y;
          if (cheb(c.x, c.y, hx, hy) <= 1 && c.carrying) {
            c.carrying = 0;
            world.stores += 1;
            world.events.push({ tick: world.ticks, kind: "deposit", id: c.id });
          } else if (c.carrying) {
            stepToward(world, c, hx, hy, MOVE_WALK);
          }
          break;
        }
        case "follow": {
          const m = nearestMate(world, c);
          if (m && m.d >= 2) stepToward(world, c, m.mate.x, m.mate.y, MOVE_WALK);
          break;
        }
        case "explore": {
          const xv = scanNearest(world, c.x, c.y, (tx, ty) =>
            world.tiles[idx(tx, ty)] === TILE.GRASS && world.visits[idx(tx, ty)] < 0.5);
          if (xv) stepToward(world, c, xv.x, xv.y, MOVE_WALK);
          else if (world.rng() < 0.1) stepToward(world, c,
            Math.floor(world.rng() * GW), Math.floor(world.rng() * GH), MOVE_WALK);
          break;
        }
        case "rest":
          c.energy = Math.min(100, c.energy + 0.022);   /* net gain while resting */
          break;
        default:
          break;   /* ignore */
      }
      world.visits[idx(c.x, c.y)] += 0;   /* visits recorded above */
    }

    /* births: the colony spends its stores */
    if (world.stores >= BIRTH_COST && world.creatures.length < 100 &&
        world.ticks - world.lastBirth >= BIRTH_EVERY) {
      world.stores -= BIRTH_COST;
      world.births += 1;
      world.lastBirth = world.ticks;
      spawnCreature(world);
      const newborn = world.creatures[world.creatures.length - 1];
      newborn.energy = 45; newborn.water = 45;
      world.events.push({ tick: world.ticks, kind: "birth", id: newborn.id });
    }

    /* extinction is a verdict, not an error */
    if (world.creatures.length === 0) world.over = true;
    return world;
  }

  /* the fox and the fight action move on the same rule: one cell toward,
     ignoring water (the fox is a shadow; a fighting creature holds ground) */
  function stepTowardFox(world, agent, tx, ty, rate) {
    if (rate !== undefined) {
      agent.moveAcc = agent.moveAcc || 0;
      agent.moveAcc += 1 / rate;
      if (agent.moveAcc < 1) return;
      agent.moveAcc -= 1;
    } else if (world.ticks % 2 !== 0) {
      return;   /* the fox covers a cell every other tick */
    }
    const dx = Math.sign(tx - agent.x), dy = Math.sign(ty - agent.y);
    const horizFirst = Math.abs(tx - agent.x) >= Math.abs(ty - agent.y);
    const tryOrder = horizFirst ? [[dx, 0], [0, dy]] : [[0, dy], [dx, 0]];
    for (const [mx, my] of tryOrder) {
      if (!mx && !my) continue;
      const nx = agent.x + mx, ny = agent.y + my;
      if (!inBounds(nx, ny)) continue;
      if (agent !== world.fox && world.tiles[idx(nx, ny)] === TILE.WATER) continue;
      agent.x = nx; agent.y = ny;
      return;
    }
  }

  function bounceFox(world, fox, x, y) {
    const dx = Math.sign(fox.x - x), dy = Math.sign(fox.y - y);
    for (const [mx, my] of [[dx * 5, dy * 5], [dx * 5, 0], [0, dy * 5]]) {
      const nx = fox.x + mx, ny = fox.y + my;
      if (inBounds(nx, ny)) { fox.x = nx; fox.y = ny; return; }
    }
  }

  function killCreature(world, c, why) {
    const i = world.creatures.indexOf(c);
    if (i >= 0) world.creatures.splice(i, 1);
    world.deaths += 1;
    world.events.push({ tick: world.ticks, kind: why, id: c.id });
  }

  function simulateTicks(world, ticks, engineChoose) {
    for (let i = 0; i < ticks; i++) {
      if (world.over) break;
      tickWorld(world, engineChoose);
    }
    return world;
  }

  function runSummary(world) {
    let e = 0;
    for (const c of world.creatures) e += c.energy;
    return {
      ticks: world.ticks,
      seconds: world.ticks / TICKS_PER_S,
      pop: world.creatures.length,
      avgEnergy: world.creatures.length ? e / world.creatures.length : 0,
      stores: world.stores,
      births: world.births,
      deaths: world.deaths,
      drives: world.drives,
      engineCalls: world.engineCalls,
      ruleCalls: world.ruleCalls,
      carrying: world.creatures.reduce((n, c) => n + c.carrying, 0),
      over: world.over,
    };
  }

  globalThis.MicroWorld = {
    TICKS_PER_S, CELL, GW, GH, SIGHT, DECIDE_EVERY, TIE_EPS,
    FOOD_MAX, STORE_CAP, BIRTH_COST, BIRTH_EVERY, FOX_HUNT_R, FOX_PACK_R, FOX_PACK_MIN,
    TILE, ACTIONS, ACTION_DESC, DEFAULT_CONFIG,
    normalizeConfig, microworldStamp, createWorld, tickWorld, simulateTicks,
    decideCreature, scoreActions, runSummary, spawnCreature,
  };
})();
