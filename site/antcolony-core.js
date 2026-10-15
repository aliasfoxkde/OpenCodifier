/* OpenCodifier — Ant Colony, core (DOM-free).
   §5.12 of the Decisions-SDK plan: emergence from simple decisions. A
   pheromone grid, a nest, food piles, one wandering spider, and up to
   1000 ants each deciding at ~1 Hz:

   SEARCH / GATHER / RETURN / FOLLOW_TRAIL / EXPLORE / DEFEND — with rule
   prefilters, exactly as specified:
     - carrying food  → RETURN is a RULE, never a question
     - food underfoot → GATHER is a RULE, never a question
     - otherwise the contested set is scored on the pheromone snapshot:
       follow-trail (steep gradient), search (food sense), explore
       (novelty), or defend (when the spider is close). Genuine near-ties
       within TIE_EPS go to the engine — but only up to the BATCHING
       BUDGET: a colony of 1000 ants cannot ask 1000 times a second, so
       the colony holds ask tokens that refill at cfg.askBudget/s. A
       contested ant with no token falls back to the argmax rule and the
       rung says so.
   The trail network is emergent: returning ants deposit, evaporation
   decays, diffusion spreads, and followers steer up the gradient. There
   is no pathfinding code anywhere in this file.

   The shell (antcolony.js) renders; this file never touches the DOM. */

(function () {
  "use strict";
  const DK = globalThis.DecisionsSDK;

  const TICKS_PER_S = 60;
  const CELL = 10;
  const GW = 96, GH = 60;                 /* grid cells (960 × 600 px) */
  const W = GW * CELL, H = GH * CELL;
  const TIE_EPS = 120;                    /* milli-value near-tie → engine */
  const ANT_SPEED = 30;                   /* px/s */
  const PROBE_DIST = 14;                  /* px ahead of the ant */
  const PROBE_ANGLE = 0.55;               /* rad for the left/right probes */
  const NEST = { x: W / 2, y: H - 40 };
  const NEST_RADIUS = 14;
  const SPIDER_RADIUS = 9;
  const THREAT_RADIUS = 70;               /* ant feels the spider inside this */
  const DEFEND_QUORUM = 3;                /* defenders that make it flee */
  const SPIDER_FLEE_TICKS = 6 * TICKS_PER_S;
  const FOOD_SENSE = 130;                 /* px at which search smells a pile */
  const PICKUP_RADIUS = 12;               /* px — the pile cell plus a step */

  const BEHAVIORS = ["follow-trail", "search", "explore", "defend"];
  const BEHAVIOR_DESC = {
    "follow-trail": "steer up the strongest nearby pheromone gradient",
    "search": "scan locally for food piles and glean their edge",
    "explore": "walk a fresh heading into low-trail space",
    "defend": "hold ground facing the spider until the group rallies",
  };
  const DEFAULT_CONFIG = {
    seed: "oc-ants", ants: 120, evaporation: 0.4, deposit: 2.0,
    foodPiles: 3, obstacles: "none", decideRate: 1, askBudget: 2,
  };

  /* ---------- config + stamp ---------- */

  function normalizeConfig(raw) {
    const c = raw || {};
    return {
      seed: c.seed === undefined || c.seed === null ? DEFAULT_CONFIG.seed : String(c.seed),
      ants: DK.clamp(Math.round(Number(c.ants) || DEFAULT_CONFIG.ants), 10, 1000),
      /* fraction of trail strength lost per second */
      evaporation: DK.clampf(Number(c.evaporation) || DEFAULT_CONFIG.evaporation, 0, 0.95, DEFAULT_CONFIG.evaporation),
      deposit: DK.clampf(Number(c.deposit) || DEFAULT_CONFIG.deposit, 0.1, 8, DEFAULT_CONFIG.deposit),
      foodPiles: DK.clamp(Math.round(Number(c.foodPiles) || DEFAULT_CONFIG.foodPiles), 1, 8),
      obstacles: c.obstacles === "wall" ? "wall" : "none",
      decideRate: DK.clampf(Number(c.decideRate) || DEFAULT_CONFIG.decideRate, 0.5, 4, DEFAULT_CONFIG.decideRate),
      askBudget: DK.clampf(Number(c.askBudget) || DEFAULT_CONFIG.askBudget, 0, 10, DEFAULT_CONFIG.askBudget),
    };
  }

  const colonyStamp = (cfg) =>
    "ants|" + cfg.seed + "|" + cfg.ants + "|" + cfg.evaporation.toFixed(2) +
    "|" + cfg.deposit.toFixed(1) + "|" + cfg.foodPiles + "|" + cfg.obstacles +
    "|" + cfg.decideRate + "|" + cfg.askBudget;

  /* ---------- world ---------- */

  const idx = (cx, cy) => cy * GW + cx;
  const cellOf = (x, y) => [Math.floor(x / CELL), Math.floor(y / CELL)];

  function buildObstacles(kind) {
    const blocked = new Uint8Array(GW * GH);
    if (kind === "wall") {
      /* a vertical wall with one gap, between the nest and the south field */
      const wallX = Math.floor(GW / 2);
      for (let cy = 8; cy < GH - 8; cy++) {
        if (cy >= 26 && cy <= 33) continue;          /* the gap */
        blocked[idx(wallX, cy)] = 1;
        blocked[idx(wallX + 1, cy)] = 1;
      }
    }
    return blocked;
  }

  function makeFoodPiles(count, seed) {
    const rng = DK.makeRng("food:" + seed);
    const piles = [];
    for (let i = 0; i < count; i++) {
      const cx = 6 + Math.floor(rng() * (GW - 12));
      const cy = 4 + Math.floor(rng() * Math.floor(GH * 0.55));   /* mostly north of the nest */
      piles.push({ cx, cy, x: cx * CELL + CELL / 2, y: cy * CELL + CELL / 2, amount: 400 });
    }
    return piles;
  }

  function createGame(raw) {
    const cfg = normalizeConfig(raw);
    const rng = DK.makeRng("colony:" + cfg.seed);
    const ants = [];
    for (let i = 0; i < cfg.ants; i++) {
      const a = rng() * Math.PI * 2;
      ants.push({
        id: "a" + i,
        x: NEST.x + Math.cos(a) * NEST_RADIUS,
        y: NEST.y + Math.sin(a) * NEST_RADIUS,
        heading: rng() * Math.PI * 2,
        carrying: false, behavior: "search",
        decideIn: Math.floor(rng() * TICKS_PER_S / cfg.decideRate),
      });
    }
    return {
      cfg, stamp: colonyStamp(cfg),
      pher: new Float32Array(GW * GH),
      blocked: buildObstacles(cfg.obstacles),
      piles: makeFoodPiles(cfg.foodPiles, cfg.seed),
      ants, rng,
      spider: { x: 40, y: 40, heading: rng() * Math.PI * 2, fleeUntil: 0 },
      ticks: 0,
      decisions: 0, rungs: { rule: 0, engine: 0 },
      engineCalls: 0, tokens: cfg.askBudget,
      gathers: 0, delivered: 0, milestones: [], spiderFled: 0, spiderFed: 0,
      over: false, lastDecision: null, recordedMilestones: {},
    };
  }

  /* ---------- sensing ---------- */

  const trailAt = (g, x, y) => {
    const [cx, cy] = cellOf(x, y);
    if (cx < 0 || cy < 0 || cx >= GW || cy >= GH) return 0;
    return g.pher[idx(cx, cy)];
  };

  /* three-probe gradient: returns {ahead, left, right} trail strengths */
  function probes(g, ant) {
    const p = (ang) => trailAt(g,
      ant.x + Math.cos(ang) * PROBE_DIST,
      ant.y + Math.sin(ang) * PROBE_DIST);
    return {
      ahead: p(ant.heading),
      left: p(ant.heading - PROBE_ANGLE),
      right: p(ant.heading + PROBE_ANGLE),
    };
  }

  function foodSense(g, ant) {
    let best = Infinity;
    for (const pile of g.piles) {
      if (pile.amount <= 0) continue;
      const d = Math.hypot(pile.x - ant.x, pile.y - ant.y);
      if (d < best) best = d;
    }
    return best === Infinity ? 0 : Math.max(0, 1 - best / FOOD_SENSE);
  }

  function pileAt(g, ant) {
    for (const pile of g.piles) {
      if (pile.amount > 0
        && Math.hypot(pile.x - ant.x, pile.y - ant.y) <= PICKUP_RADIUS) return pile;
    }
    return null;
  }

  function spiderThreat(g, ant) {
    return Math.hypot(g.spider.x - ant.x, g.spider.y - ant.y) <= THREAT_RADIUS;
  }

  const distToNest = (ant) => Math.hypot(NEST.x - ant.x, NEST.y - ant.y);

  /* ---------- the decision ladder ---------- */

  function decide(g, ant) {
    ant.decideIn = Math.max(1, Math.round(TICKS_PER_S / g.cfg.decideRate));

    /* RULE PREFILTERS — the spec pins these as never-asked */
    if (ant.carrying) return settle(g, ant, "return", "rule", 0, []);
    const pile = pileAt(g, ant);
    if (pile) return settle(g, ant, "gather", "rule", 0, []);

    const threat = spiderThreat(g, ant);
    const pr = probes(g, ant);
    const food = foodSense(g, ant);
    const localTrail = Math.max(pr.ahead, pr.left, pr.right);

    /* scored candidates on the snapshot */
    const cands = [];
    if (localTrail > 0.01) {
      cands.push({ id: "follow-trail", ev: Math.round(800 * localTrail) });
    }
    cands.push({ id: "search", ev: Math.round(300 + 500 * food) });
    cands.push({ id: "explore", ev: Math.round(250 + 200 * (1 - Math.min(1, localTrail))) });
    if (threat) {
      const allies = g.ants.filter((o) => o !== ant && !o.carrying
        && Math.hypot(o.x - g.spider.x, o.y - g.spider.y) <= THREAT_RADIUS).length;
      const nearNest = Math.max(0, 1 - distToNest(ant) / (W / 3));
      cands.push({ id: "defend", ev: Math.round(500 * nearNest + 150 * Math.min(3, allies)) });
    }
    cands.sort((a, b) => b.ev - a.ev || BEHAVIORS.indexOf(a.id) - BEHAVIORS.indexOf(b.id));

    const evGap = cands.length > 1 ? cands[0].ev - cands[1].ev : Infinity;
    let pick = cands[0].id;
    let rung = "rule";

    /* genuine contest + a token in the batch budget + a live bridge */
    if (cands.length > 1 && evGap <= TIE_EPS && g.tokens >= 1 && g.engineChoose) {
      g.tokens -= 1;
      g.engineCalls += 1;
      const req = {
        state: {
          text: "Ant " + ant.id + " of the colony at tick " + g.ticks +
            ", " + Math.round(distToNest(ant)) + " px from the nest, carrying " +
            (ant.carrying ? "food" : "nothing") + ". Pheromone probes: ahead " +
            pr.ahead.toFixed(2) + ", left " + pr.left.toFixed(2) + ", right " +
            pr.right.toFixed(2) + ". Food sense " + food.toFixed(2) +
            ". Spider " + (threat ? "NEARBY" : "far") +
            ". Colony has delivered " + g.delivered + " so far. Pick the ant's next behavior.",
          facts: {},
        },
        questions: [{
          type: "choice", id: "ant-behavior",
          text: "What should ant " + ant.id + " do next?",
          candidates: cands.map((c) => ({ id: c.id, description: BEHAVIOR_DESC[c.id] })),
        }],
        policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" },
      };
      const answer = g.engineChoose ? g.engineChoose(req) : null;
      const ans = answer && answer.outcome === "accept" && Array.isArray(answer.answers)
        ? answer.answers.find((a) => a && typeof a.choice === "string") : null;
      if (ans && cands.some((c) => c.id === ans.choice)) {
        pick = ans.choice;
        rung = "engine";
      }
    }
    return settle(g, ant, pick, rung, evGap, cands.map((c) => c.id));
  }

  function settle(g, ant, behavior, rung, evGap, asked) {
    ant.behavior = behavior;
    if (behavior === "gather") doGather(g, ant);
    g.decisions += 1;
    g.rungs[rung] += 1;
    g.lastDecision = {
      id: ant.id, choice: behavior, rung,
      evGap: rung === "rule" && asked.length === 0 ? null : evGap,
      at: {
        carrying: ant.carrying,
        nest: Math.round(distToNest(ant)),
      },
    };
    return { behavior, rung, evGap };
  }

  function doGather(g, ant) {
    const pile = pileAt(g, ant);
    if (!pile) return;
    pile.amount -= 1;
    g.gathers += 1;
    ant.carrying = true;
    ant.behavior = "return";
    /* face home: the ant's path integration — the ONE piece of navigation
       knowledge in the colony (real ants do exactly this) */
    ant.heading = Math.atan2(NEST.y - ant.y, NEST.x - ant.x);
  }

  /* ---------- pheromone field ---------- */

  function deposit(g, ant, dt) {
    const [cx, cy] = cellOf(ant.x, ant.y);
    if (cx < 0 || cy < 0 || cx >= GW || cy >= GH) return;
    const i = idx(cx, cy);
    g.pher[i] = Math.min(4, g.pher[i] + g.cfg.deposit * dt);
  }

  function ageField(g, dt) {
    const keep = 1 - g.cfg.evaporation * dt;
    const pher = g.pher;
    for (let i = 0; i < pher.length; i++) {
      if (pher[i] > 0) pher[i] *= keep;
      if (pher[i] < 0.001) pher[i] = 0;
    }
    /* cheap diffusion: every other call, share 8% with the 4-neighbours */
    g.diffuseIn = (g.diffuseIn || 0) - dt;
    if (g.diffuseIn <= 0) {
      g.diffuseIn = 0.5;
      const snap = pher.slice();
      for (let cy = 1; cy < GH - 1; cy++) {
        for (let cx = 1; cx < GW - 1; cx++) {
          const i = idx(cx, cy);
          const v = snap[i];
          if (v <= 0.001) continue;
          const give = v * 0.02;                       /* 8% split four ways */
          pher[i] -= give * 4;
          pher[idx(cx - 1, cy)] += give;
          pher[idx(cx + 1, cy)] += give;
          pher[idx(cx, cy - 1)] += give;
          pher[idx(cx, cy + 1)] += give;
        }
      }
    }
  }

  function trailMax(g) {
    let m = 0;
    for (let i = 0; i < g.pher.length; i++) if (g.pher[i] > m) m = g.pher[i];
    return m;
  }

  /* ---------- movement + tick ---------- */

  function bounce(g, ant, nx, ny) {
    const [cx, cy] = cellOf(nx, ny);
    const inside = nx >= 0 && ny >= 0 && nx < W && ny < H
      && !(cx >= 0 && cy >= 0 && cx < GW && cy < GH && g.blocked[idx(cx, cy)]);
    if (inside) { ant.x = nx; ant.y = ny; return true; }
    ant.heading += Math.PI * (0.5 + 0.5 * g.rng());   /* turn away, seeded */
    return false;
  }

  function steerByProbes(g, ant) {
    const pr = probes(g, ant);
    if (pr.ahead >= pr.left && pr.ahead >= pr.right) return;
    ant.heading += pr.left > pr.right ? -PROBE_ANGLE * 0.6 : PROBE_ANGLE * 0.6;
  }

  function moveAnt(g, ant, dt) {
    const b = ant.behavior;
    if (b === "return") {
      const want = Math.atan2(NEST.y - ant.y, NEST.x - ant.x);
      let diff = want - ant.heading;
      while (diff > Math.PI) diff -= Math.PI * 2;
      while (diff < -Math.PI) diff += Math.PI * 2;
      ant.heading += Math.max(-0.2, Math.min(0.2, diff));
      deposit(g, ant, dt);                       /* the trail, laid */
    } else if (b === "follow-trail") {
      steerByProbes(g, ant);
    } else if (b === "search") {
      /* correlated walk with a food pull: the pull must out-muscle the
         wander or "search" is a lie — measured (±0.6 noise vs ±0.15 steer
         meant 3 gathers in 300 s). */
      ant.heading += (g.rng() - 0.5) * 0.3;
      const f = foodSense(g, ant);
      if (f > 0) {
        let best = null, bd = Infinity;
        for (const pile of g.piles) {
          if (pile.amount <= 0) continue;
          const d = Math.hypot(pile.x - ant.x, pile.y - ant.y);
          if (d < bd) { bd = d; best = pile; }
        }
        if (best) {
          let diff = Math.atan2(best.y - ant.y, best.x - ant.x) - ant.heading;
          while (diff > Math.PI) diff -= Math.PI * 2;
          while (diff < -Math.PI) diff += Math.PI * 2;
          ant.heading += Math.max(-0.25, Math.min(0.25, diff));
        }
      }
    } else if (b === "explore") {
      /* long correlated walks, not in-place jitter */
      ant.heading += (g.rng() - 0.5) * 0.5;
    } else if (b === "defend") {
      const want = Math.atan2(g.spider.y - ant.y, g.spider.x - ant.x);
      ant.heading += Math.max(-0.2, Math.min(0.2, want - ant.heading)) * 0.5;
    }
    bounce(g, ant,
      ant.x + Math.cos(ant.heading) * ANT_SPEED * dt,
      ant.y + Math.sin(ant.heading) * ANT_SPEED * dt);

    if (b === "return" && distToNest(ant) <= NEST_RADIUS) {
      ant.carrying = false;
      g.delivered += 1;
      ant.behavior = "search";
      milestone(g);
    }
  }

  function milestone(g) {
    const step = 100;
    const m = Math.floor(g.delivered / step);
    if (m > 0 && !g.recordedMilestones[m]) {
      g.recordedMilestones[m] = true;
      g.milestones.push({ delivered: g.delivered, ticks: g.ticks });
    }
  }

  function moveSpider(g, dt) {
    const s = g.spider;
    if (g.ticks < s.fleeUntil) {
      const want = Math.atan2(s.y - NEST.y, s.x - NEST.x);   /* flee outward */
      s.heading += Math.max(-0.3, Math.min(0.3, want - s.heading));
    } else {
      /* the quorum check: enough defenders nearby and it bolts */
      let defenders = 0;
      for (const a of g.ants) {
        if (a.behavior === "defend"
          && Math.hypot(a.x - s.x, a.y - s.y) <= THREAT_RADIUS) defenders += 1;
      }
      if (defenders >= DEFEND_QUORUM) {
        s.fleeUntil = g.ticks + SPIDER_FLEE_TICKS;
        g.spiderFled += 1;
      }
      s.heading += (g.rng() - 0.5) * 0.5;
    }
    const sp = (g.ticks < s.fleeUntil ? 90 : 18) * dt;
    const nx = s.x + Math.cos(s.heading) * sp;
    const ny = s.y + Math.sin(s.heading) * sp;
    if (nx > 10 && nx < W - 10 && ny > 10 && ny < H - 10) { s.x = nx; s.y = ny; }
    else s.heading += Math.PI;
  }

  function tickGame(g, engineChoose) {
    if (g.over) return;
    const dt = 1 / TICKS_PER_S;
    g.ticks += 1;
    g.engineChoose = engineChoose || null;

    /* the batching budget refills continuously, capped at one second's worth */
    g.tokens = Math.min(g.cfg.askBudget, g.tokens + g.cfg.askBudget * dt);

    ageField(g, dt);
    moveSpider(g, dt);

    for (const ant of g.ants) {
      ant.decideIn -= 1;
      if (ant.decideIn <= 0) decide(g, ant);
      moveAnt(g, ant, dt);
    }
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
      gathers: g.gathers, delivered: g.delivered,
      spiderFled: g.spiderFled,
      trailMax: +trailMax(g).toFixed(3),
      milestones: g.milestones.length,
    };
  }

  /* fixed-length simulation drive (tests) */
  function simulateTicks(g, ticks, engineChoose) {
    for (let i = 0; i < ticks; i++) {
      if (g.over) break;
      tickGame(g, engineChoose);
    }
    return g;
  }

  globalThis.AntCore = {
    TICKS_PER_S, CELL, GW, GH, W, H, TIE_EPS,
    ANT_SPEED, PROBE_DIST, PROBE_ANGLE, NEST, NEST_RADIUS,
    THREAT_RADIUS, DEFEND_QUORUM, SPIDER_FLEE_TICKS, FOOD_SENSE, PICKUP_RADIUS,
    BEHAVIORS, BEHAVIOR_DESC, DEFAULT_CONFIG,
    normalizeConfig, colonyStamp,
    createGame, decide, tickGame, simulateTicks, moveAnt,
    trailAt, probes, foodSense, pileAt, spiderThreat, distToNest,
    deposit, ageField, trailMax, runSummary,
  };
})();
