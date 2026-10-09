/* OpenCodifier — GridWars, core (DOM-free).
   Twin-stick arena in the Geometry Wars / GridWars lineage (the freeware
   clone by Marco Incitti, itself a homage to Bizarre Creations'): a neon
   vector arena on a warping spring grid, geometry that hunts you, and a
   score multiplier built out of the geoms the dead drop.

   Everything here is deterministic given (seed, config, input): the same
   seed replays the same spawn sequence, the same enemy decisions, the same
   particles. No DOM, no canvas, no timers — the shell (gridwars.js) renders
   and feeds `g.input`; node --test drives the rest (see tests/gridwars_test.mjs).

   Decisions (the house ladder, cheapest rung first):
   - EVERY ENEMY decides on a cheap rule ladder every DECIDE_EVERY ticks and
     on every phase transition, from its own local situation: chase / wander /
     charge / brake / aim / flee / jink / trail / pull / burst / split. Each
     pick is counted per tag so the UI can show "what the enemies decided".
     All of it lands on the "rule" rung and says so.
   - THE ENGINE gets exactly one kind of question: which geometry leads the
     next wave (and which newly-unlocked geometry enters first on a tier-up).
     That is a real choice over the tier-valid roster — nothing else is a
     genuine tie. With the WASM module absent, or on any abstain/refusal, the
     pick falls back to the seeded roster weighting and says "rule".
     Bounded: at most one ask per wave block plus one per tier-up.

   Roster and behavior follow the clones' personalities:
   grunt (diamond)   chases, jinks off your fire line
   drifter (pinwheel)drifts, only leans toward you when already close
   cube (square)     wanders, splits into 3 cubelets when shot
   rocket            aims, charges a straight line, brakes, re-aims
   snake (pinwheel)  eats your trail; only the head takes damage
   weaver (cube)     weaves in, bursts into 4 weavlings
   well (black hole) pulls everything, swallows, then bursts */

"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  if (!DK) throw new Error("gridwars-core.js needs sdk/decisions-sdk.js loaded first");
  const hashSeed = DK.hashSeed;
  const mulberry32 = DK.mulberry32;
  const makeRng = DK.makeRng;

  const TICKS_PER_S = 60;
  const W = 960, H = 600;              /* arena px, 16:10 like the other games */
  const DECIDE_EVERY = 12;             /* enemy decision cadence — 5 Hz */
  const WAVE_LEN = 900;                /* 15 s of sim per wave block */
  const GEOM_STEP = 5;                 /* geoms per +1 multiplier */
  const MULT_CAP = 20;
  const MULT_MAGNET = 92, MULT_PICKUP = 18;
  const BOMB_SCORE_STEP = 25000;       /* +1 bomb */
  const LIFE_SCORE_STEP = 75000;       /* +1 life */
  const SPAWN_BASE = 104;              /* ticks between spawns at wave 1 */
  const SPAWN_MIN = 14;
  const PARTICLE_CAP = 900;
  const THREAT_R = 130;                /* "a bullet is coming at me" radius */
  const TRAIL_SENSE = 150;             /* snake: how far it smells the trail */
  const WELL_BURST = 8;                /* swallows before a well bursts */
  const ROCKET_ALIGN = 0.18;           /* rad of alignment before a charge */
  const ROCKET_CHARGE_MAX = 46;

  /* decision vocabulary — the stats panel counts every tag */
  const TAGS = ["chase", "wander", "charge", "brake", "aim", "flee", "jink",
    "trail", "split", "pull", "burst"];

  /* ---------- difficulty: tiers and scaling ---------- */
  const DIFFICULTIES = {
    chill: { label: "chill", tierScale: 1.6, spawnScale: 0.8, hpScale: 0.8, speedScale: 0.85 },
    standard: { label: "standard", tierScale: 1.0, spawnScale: 1.0, hpScale: 1.0, speedScale: 1.0 },
    overdrive: { label: "overdrive", tierScale: 0.6, spawnScale: 1.35, hpScale: 1.15, speedScale: 1.1 },
  };

  /* tier thresholds are on effective score = score + survival bonus
     (25/s), divided by the difficulty's tierScale — score and time both
     buy complexity, and new geometry arrives with each tier. */
  const TIERS = [
    { id: 1, name: "tier 1 — diamonds", score: 0, unlocks: ["grunt", "drifter"] },
    { id: 2, name: "tier 2 — squares", score: 2000, unlocks: ["cube", "rocket"] },
    { id: 3, name: "tier 3 — snakes", score: 6000, unlocks: ["snake"] },
    { id: 4, name: "tier 4 — weavers", score: 12000, unlocks: ["weaver", "well"] },
  ];
  const SURVIVAL_PER_S = 25;

  /* ---------- geometry ---------- */
  const ENEMY_TYPES = {
    grunt: { label: "Diamond", color: "#c084fc", hp: 1, size: 9, speed: 112, points: 100,
      tier: 1, geoms: 2, debris: 18, shape: "diamond", desc: "chases you, jinks off your fire line" },
    drifter: { label: "Pinwheel", color: "#f0abfc", hp: 1, size: 10, speed: 66, points: 50,
      tier: 1, geoms: 1, debris: 14, shape: "pinwheel", desc: "drifts; only leans toward you when close" },
    cube: { label: "Cube", color: "#60a5fa", hp: 2, size: 13, speed: 62, points: 150,
      tier: 2, geoms: 2, debris: 24, shape: "square", desc: "wanders, splits into 3 cubelets when shot" },
    rocket: { label: "Rocket", color: "#fb923c", hp: 2, size: 11, speed: 168, points: 200,
      tier: 2, geoms: 2, debris: 24, shape: "rocket", desc: "aims, charges a straight line, brakes" },
    snake: { label: "Snake", color: "#4ade80", hp: 3, size: 10, speed: 108, points: 300,
      tier: 3, geoms: 3, debris: 28, shape: "snake", desc: "eats your trail; only the head takes damage" },
    weaver: { label: "Weaver", color: "#f472b6", hp: 2, size: 14, speed: 126, points: 250,
      tier: 4, geoms: 3, debris: 28, shape: "cube", desc: "weaves in, bursts into 4 weavlings" },
    well: { label: "Well", color: "#22d3ee", hp: 1, size: 18, speed: 24, points: 150,
      tier: 4, geoms: 6, debris: 40, shape: "well", desc: "pulls everything; feed it 8 and it bursts" },
    /* children — never spawned by a wave, only by a splitting parent */
    cubelet: { label: "Cubelet", color: "#93c5fd", hp: 1, size: 7, speed: 152, points: 25,
      tier: 2, geoms: 0, debris: 9, shape: "square", child: true, desc: "split shard — chases" },
    weavling: { label: "Weavling", color: "#fbcfe8", hp: 1, size: 7, speed: 158, points: 25,
      tier: 4, geoms: 0, debris: 9, shape: "diamond", child: true, desc: "split shard — chases" },
  };
  const SPLIT_CHILDREN = { cube: { type: "cubelet", n: 3 }, weaver: { type: "weavling", n: 4 } };

  /* ---------- weapons ---------- */
  const WEAPONS = {
    pulse: { label: "Pulse", key: "1", color: "#67e8f9", cooldown: 8, dmg: 1,
      speed: 13, count: 1, spread: 0, life: 58, desc: "fast single shot" },
    scatter: { label: "Scatter", key: "2", color: "#fde047", cooldown: 18, dmg: 1,
      speed: 11, count: 3, spread: 0.38, life: 34, desc: "3-way spread" },
    laser: { label: "Laser", key: "3", color: "#f87171", cooldown: 42, dmg: 3,
      beam: true, range: 1200, desc: "piercing beam, long cooldown" },
    wave: { label: "Wave", key: "4", color: "#a78bfa", cooldown: 50, dmg: 3,
      ring: true, speed: 6.5, r0: 10, r1: 80, desc: "slow expanding ring, hits a group" },
  };
  const WEAPON_ORDER = ["pulse", "scatter", "laser", "wave"];

  /* which weapon params the options panel may touch, and how far */
  const WEAPON_LIMITS = {
    pulse: { cooldown: [3, 30], dmg: [1, 4] },
    scatter: { cooldown: [6, 45], dmg: [1, 4], count: [2, 5], spread: [0.1, 0.9] },
    laser: { cooldown: [15, 90], dmg: [1, 6] },
    wave: { cooldown: [25, 120], dmg: [1, 5], r1: [40, 160] },
  };

  function normalizeWeapons(raw) {
    const out = {};
    for (const key of WEAPON_ORDER) {
      const spec = WEAPON_LIMITS[key];
      const src = raw && typeof raw === "object" ? raw[key] : null;
      const row = {};
      for (const k of Object.keys(spec)) {
        const v = src && src[k] !== undefined ? Number(src[k]) : WEAPONS[key][k];
        row[k] = k === "spread" ? DK.clampf(v, spec[k][0], spec[k][1], WEAPONS[key][k])
          : Math.round(DK.clamp(v, spec[k][0], spec[k][1], WEAPONS[key][k]));
      }
      out[key] = row;
    }
    return out;
  }

  /* the live weapon: the built-in tuned by the panel's mods */
  function weaponOf(g, key) {
    const base = WEAPONS[key];
    const mods = g.cfg.weaponMods[key] || {};
    return Object.assign({}, base, mods);
  }

  /* ---------- config ---------- */
  const DEFAULT_CONFIG = {
    seed: "oc-gridwars",
    difficulty: "standard",
    spawnRate: 1,      /* 0.4 .. 2 — scales the spawn interval */
    maxEnemies: 60,    /* 8 .. 120 */
    shipSpeed: 270,    /* px/s top speed, 140 .. 460 */
    inertia: 0.5,      /* 0 = stops on a dime, 1 = ice rink */
    lives: 3,          /* 1 .. 9 */
    invuln: 2.5,       /* respawn shield, seconds, 0 .. 6 */
    warp: 0.6,         /* grid spring response, 0 .. 1 */
    density: 40,       /* grid line spacing, 24 .. 64 px */
    bombs: 3,          /* 0 .. 6 */
    weaponMods: normalizeWeapons(null),  /* per-weapon tuning from the options panel */
  };

  function normalizeConfig(raw) {
    const src = raw && typeof raw === "object" ? raw : {};
    const c = {};
    c.seed = src.seed === undefined || src.seed === null ? DEFAULT_CONFIG.seed : String(src.seed);
    c.difficulty = DIFFICULTIES[src.difficulty] ? src.difficulty : DEFAULT_CONFIG.difficulty;
    c.spawnRate = DK.clampf(src.spawnRate, 0.4, 2, DEFAULT_CONFIG.spawnRate);
    c.maxEnemies = DK.clamp(src.maxEnemies, 8, 120, DEFAULT_CONFIG.maxEnemies);
    c.shipSpeed = DK.clamp(src.shipSpeed, 140, 460, DEFAULT_CONFIG.shipSpeed);
    c.inertia = DK.clampf(src.inertia, 0, 1, DEFAULT_CONFIG.inertia);
    c.lives = DK.clamp(src.lives, 1, 9, DEFAULT_CONFIG.lives);
    c.invuln = DK.clampf(src.invuln, 0, 6, DEFAULT_CONFIG.invuln);
    c.warp = DK.clampf(src.warp, 0, 1, DEFAULT_CONFIG.warp);
    c.density = DK.clamp(src.density, 24, 64, DEFAULT_CONFIG.density);
    c.bombs = DK.clamp(src.bombs, 0, 6, DEFAULT_CONFIG.bombs);
    c.weaponMods = normalizeWeapons(src.weaponMods);
    return c;
  }

  /* ---------- the spring grid (warps, then settles) ---------- */
  function makeGrid(density) {
    const cols = Math.ceil(W / density) + 1;
    const rows = Math.ceil(H / density) + 1;
    const nodes = new Array(cols * rows);
    for (let r = 0; r < rows; r++) {
      for (let col = 0; col < cols; col++) {
        const x = Math.min(col * density, W);
        const y = Math.min(r * density, H);
        nodes[r * cols + col] = { x, y, ox: x, oy: y, vx: 0, vy: 0 };
      }
    }
    return { density, cols, rows, nodes };
  }

  function pushGrid(g, x, y, radius, force) {
    const nodes = g.grid.nodes;
    for (let i = 0; i < nodes.length; i++) {
      const n = nodes[i];
      const dx = n.x - x, dy = n.y - y;
      const d = Math.hypot(dx, dy);
      if (d > radius || d < 0.0001) continue;
      const f = (1 - d / radius) * force;
      n.vx += (dx / d) * f;
      n.vy += (dy / d) * f;
    }
  }

  function updateGrid(g) {
    const k = 0.018 + 0.05 * g.cfg.warp;
    const damp = 0.88;
    const nodes = g.grid.nodes;
    for (let i = 0; i < nodes.length; i++) {
      const n = nodes[i];
      n.vx = (n.vx + (n.ox - n.x) * k) * damp;
      n.vy = (n.vy + (n.oy - n.y) * k) * damp;
      n.x += n.vx;
      n.y += n.vy;
    }
  }

  /* ---------- particles (pooled by cap, deterministic per event) ---------- */
  function debris(g, x, y, color, count, force) {
    for (let i = 0; i < count; i++) {
      if (g.particles.length >= PARTICLE_CAP) return;
      const a = g.rng() * Math.PI * 2;
      const s = (0.35 + g.rng() * 0.65) * force;
      g.particles.push({
        x, y, vx: Math.cos(a) * s, vy: Math.sin(a) * s,
        life: 1, decay: 0.018 + g.rng() * 0.03, size: 1 + g.rng() * 2.2, color,
      });
    }
  }

  function thrustTrail(g, p) {
    if (g.particles.length >= PARTICLE_CAP) return;
    const a = Math.atan2(-p.vy, -p.vx) + (g.rng() - 0.5) * 0.7;
    g.particles.push({
      x: p.x, y: p.y, vx: Math.cos(a) * 2.4, vy: Math.sin(a) * 2.4,
      life: 0.5, decay: 0.06, size: 1.6, color: "#7dd3fc",
    });
  }

  function updateParticles(g) {
    const out = g.particles;
    for (let i = out.length - 1; i >= 0; i--) {
      const q = out[i];
      q.x += q.vx; q.y += q.vy;
      q.vx *= 0.96; q.vy *= 0.96;
      q.life -= q.decay;
      if (q.life <= 0) out.splice(i, 1);
    }
  }

  /* ---------- scoring ---------- */
  function award(g, base) { g.score += Math.round(base * g.mult); }

  function record(g, tag) {
    g.decisions.total += 1;
    g.decisions.tags[tag] = (g.decisions.tags[tag] || 0) + 1;
    g.rungs.rule += 1;
  }

  /* ---------- tiers ---------- */
  function effectiveScore(g) {
    return g.score + (g.ticks / TICKS_PER_S) * SURVIVAL_PER_S;
  }
  function tierFor(score, ticksS, difficulty) {
    const diff = DIFFICULTIES[difficulty] || DIFFICULTIES.standard;
    const eff = (Number(score) || 0) + (Number(ticksS) || 0) * SURVIVAL_PER_S;
    let tier = 1;
    for (const t of TIERS) if (eff >= t.score * diff.tierScale) tier = t.id;
    return tier;
  }
  /* the roster a tier may draw from — children are split-only */
  function typesForTier(tier) {
    return Object.keys(ENEMY_TYPES).filter((k) =>
      !ENEMY_TYPES[k].child && ENEMY_TYPES[k].tier <= tier);
  }
  function unlocksForTier(tier) {
    const t = TIERS[Math.max(0, Math.min(TIERS.length, tier) - 1)];
    return t ? t.unlocks.slice() : [];
  }

  /* ---------- waves: seeded composition + the one engine question ---------- */
  function pickWeighted(rng, pool) {
    let total = 0;
    const weights = pool.map((k) => {
      const t = ENEMY_TYPES[k];
      const w = (t.tier === Math.max.apply(null, pool.map((p) => ENEMY_TYPES[p].tier)) ? 2 : 1) *
        (k === "well" ? 0.5 : 1);
      total += w;
      return w;
    });
    let roll = rng() * total;
    for (let i = 0; i < pool.length; i++) {
      roll -= weights[i];
      if (roll <= 0) return pool[i];
    }
    return pool[pool.length - 1];
  }

  function waveComposition(wave, tier, seedStr) {
    const rng = makeRng(String(seedStr) + ":wave" + wave);
    const pool = typesForTier(tier);
    const size = Math.min(6 + 2 * wave, 60);
    const out = [];
    for (let i = 0; i < size; i++) out.push(pickWeighted(rng, pool));
    return out;
  }

  /* The engine's single job: name the geometry that enters next. Two or
     more tier-valid candidates is a genuine choice; one candidate is not a
     question. Any abstain/refusal/absence falls back to the seeded pick. */
  function nextWave(g, engineAsk) {
    g.wave += 1;
    g.waveTick = 0;
    g.tier = tierFor(g.score, g.ticks / TICKS_PER_S, g.cfg.difficulty);
    g.queue = waveComposition(g.wave, g.tier, g.cfg.seed);
    waveLead(g, engineAsk, "wave " + g.wave);
    return g.wave;
  }

  function waveLead(g, engineAsk, why, poolOverride) {
    const pool = Array.isArray(poolOverride) && poolOverride.length ? poolOverride
      : typesForTier(g.tier);
    if (pool.length < 2 || typeof engineAsk !== "function") {
      const pick = pickWeighted(g.rng, pool);
      g.lastEntered = pick;
      g.rungs.rule += 1;
      g.lastEngine = { why, rung: "rule", pick, candidates: pool.length };
      return pick;
    }
    const req = {
      state: {
        text: "GridWars wave lead. Tick " + g.ticks + ", wave " + g.wave + ", tier " + g.tier +
          ". Player score " + g.score + ", multiplier x" + g.mult + ", " +
          g.enemies.length + " geometry on the field. Tier-valid geometry: " +
          pool.map((k) => k + " (" + ENEMY_TYPES[k].desc + ")").join("; ") +
          ". Pick which geometry enters the field next.",
        facts: {},
      },
      questions: [{
        type: "choice", id: "gridwars-wave-lead",
        text: "Which geometry enters next at " + why + "?",
        candidates: pool.map((k) => ({ id: k, description: ENEMY_TYPES[k].desc })),
      }],
      policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" },
      metadata: { request_id: "gridwars-wave-lead" },
    };
    g.engineCalls += 1;
    let pick = null;
    try {
      const answer = engineAsk(req);
      const ans = answer && answer.outcome === "accept" && Array.isArray(answer.answers)
        ? answer.answers.find((a) => a && typeof a.choice === "string") : null;
      if (ans && pool.indexOf(ans.choice) >= 0) pick = ans.choice;
    } catch (err) { pick = null; }
    if (pick) {
      g.rungs.engine += 1;
      g.lastEngine = { why, rung: "engine", pick, candidates: pool.length };
    } else {
      pick = pickWeighted(g.rng, pool);
      g.rungs.rule += 1;
      g.lastEngine = { why, rung: "rule", pick, candidates: pool.length };
    }
    g.lastEntered = pick;
    g.queue[0] = pick;          /* the lead entrant is the engine's pick */
    return pick;
  }

  /* ---------- spawning ---------- */
  function spawnInterval(g) {
    const diff = DIFFICULTIES[g.cfg.difficulty];
    const t = SPAWN_BASE - (g.wave - 1) * 6 - Math.min(24, g.score / 900);
    return Math.max(SPAWN_MIN, t / (g.cfg.spawnRate * diff.spawnScale));
  }

  function spawnEnemy(g, type, x, y) {
    const t = ENEMY_TYPES[type];
    if (!t) return null;
    if (g.enemies.length >= g.cfg.maxEnemies) return null;
    const diff = DIFFICULTIES[g.cfg.difficulty];
    const e = {
      id: g.nextId++, type,
      x: x === undefined ? spawnEdgeX(g) : x, y: y === undefined ? spawnEdgeY(g) : y,
      vx: 0, vy: 0, h: g.rng() * Math.PI * 2,
      hp: Math.max(1, Math.round(t.hp * diff.hpScale)),
      maxHp: Math.max(1, Math.round(t.hp * diff.hpScale)),
      size: t.size,
      speed: t.speed * diff.speedScale,
      accel: (t.speed * diff.speedScale / TICKS_PER_S) * 0.25,
      tag: t.tag, mode: type === "rocket" ? "aim" : "run",
      decideIn: DECIDE_EVERY, wanderT: 20 + Math.floor(g.rng() * 50),
      feed: 0, t: 0, chargeT: 0, lock: 0, segs: [], path: [],
    };
    if (type === "snake") {
      for (let i = 0; i < 7; i++) e.segs.push({ x: e.x, y: e.y });
    }
    g.enemies.push(e);
    g.lastEntered = type;
    return e;
  }

  function spawnEdgeX(g) {
    const side = g.rng() < 0.5 ? 0 : 1;
    return side === 0 ? 24 : W - 24;
  }
  function spawnEdgeY(g) { return 24 + g.rng() * (H - 48); }

  function spawnStep(g) {
    g.spawnIn -= 1;
    if (g.spawnIn > 0) return;
    g.spawnIn = spawnInterval(g);
    let type;
    if (g.queue && g.queue.length) type = g.queue.shift();
    else type = pickWeighted(g.rng, typesForTier(g.tier));
    const e = spawnEnemy(g, type);
    if (e) debris(g, e.x, e.y, ENEMY_TYPES[type].color, 4, 1.4);
  }

  /* ---------- the enemy decision ladder (rules, labelled) ---------- */
  /* A live bullet that is closing on this enemy — the fire line. */
  function inboundBullet(g, e) {
    let best = null, bestD = THREAT_R;
    for (const b of g.bullets) {
      const dx = e.x - b.x, dy = e.y - b.y;
      const d = Math.hypot(dx, dy);
      if (d > THREAT_R || d < 0.001) continue;
      const sp = Math.hypot(b.vx, b.vy) || 1;
      if ((b.vx * dx + b.vy * dy) / (sp * d) < 0.55) continue;   /* not closing */
      if (d < bestD) { bestD = d; best = b; }
    }
    return best;
  }

  function enemyDecide(g, e) {
    e.decideIn = DECIDE_EVERY;
    const p = g.player;
    const dx = p.x - e.x, dy = p.y - e.y;
    const dist = Math.hypot(dx, dy) || 1;
    const threat = inboundBullet(g, e);
    let tag = "chase";

    switch (e.type) {
      case "grunt":
      case "cubelet":
      case "weavling":
        tag = threat ? "jink" : "chase";
        break;
      case "drifter":
        tag = dist < 260 ? "chase" : "wander";   /* nearly passive, like the clone */
        break;
      case "cube":
        tag = threat ? "flee" : "wander";
        break;
      case "rocket":
        tag = e.mode === "charge" ? "charge" : e.mode === "brake" ? "brake" : "aim";
        break;
      case "snake":
        tag = nearestTrail(g, e) ? "trail" : (dist < 420 ? "chase" : "wander");
        break;
      case "weaver":
        tag = dist < 180 && threat ? "flee" : "chase";
        break;
      case "well":
        tag = e.feed >= WELL_BURST ? "burst" : "pull";
        break;
      default:
        tag = "chase";
    }
    e.tag = tag;
    record(g, tag);
    g.lastDecision = { tag, who: "enemy", type: e.type, id: e.id, tick: g.ticks };
    return tag;
  }

  function nearestTrail(g, e) {
    const trail = g.trail;
    let best = null, bestD = TRAIL_SENSE;
    for (let i = 0; i < trail.length; i++) {
      const d = Math.hypot(trail[i].x - e.x, trail[i].y - e.y);
      if (d < bestD) { bestD = d; best = trail[i]; }
    }
    return best;
  }

  function steer(e, tx, ty, weight) {
    const dx = tx - e.x, dy = ty - e.y;
    const d = Math.hypot(dx, dy) || 1;
    e.vx += (dx / d) * e.accel * (weight === undefined ? 1 : weight);
    e.vy += (dy / d) * e.accel * (weight === undefined ? 1 : weight);
  }

  function clampSpeed(e, extra) {
    /* speed is tuned in px/s; the sim runs in px/tick */
    const max = (e.speed / TICKS_PER_S) * (extra === undefined ? 1 : extra);
    const s = Math.hypot(e.vx, e.vy);
    if (s > max) { e.vx = (e.vx / s) * max; e.vy = (e.vy / s) * max; }
  }

  function jinkFrom(g, e, b) {
    /* perpendicular to the bullet's travel, away from its line */
    const sp = Math.hypot(b.vx, b.vy) || 1;
    const bx = b.vx / sp, by = b.vy / sp;
    const cross = bx * (e.y - b.y) - by * (e.x - b.x);
    const sign = cross > 0 ? -1 : 1;
    e.vx += -by * sign * e.accel * 1.4;
    e.vy += bx * sign * e.accel * 1.4;
  }

  function enemyStep(g, e) {
    e.t += 1;
    const p = g.player;

    if (e.decideIn > 0) e.decideIn -= 1;
    if (e.decideIn === 0) enemyDecide(g, e);

    switch (e.type) {
      case "grunt":
      case "cubelet":
      case "weavling": {
        const threat = inboundBullet(g, e);
        if (e.tag === "jink" && threat) jinkFrom(g, e, threat);
        else steer(e, p.x, p.y, 1);
        clampSpeed(e);
        break;
      }
      case "drifter": {
        if (e.tag === "chase") steer(e, p.x, p.y, 0.16);
        else {
          e.wanderT -= 1;
          if (e.wanderT <= 0) { e.h = g.rng() * Math.PI * 2; e.wanderT = 40 + Math.floor(g.rng() * 60); }
          e.vx += Math.cos(e.h) * e.accel * 0.5;
          e.vy += Math.sin(e.h) * e.accel * 0.5;
        }
        clampSpeed(e, 0.7);
        break;
      }
      case "cube": {
        const threat = inboundBullet(g, e);
        if (e.tag === "flee" && threat) {
          steer(e, e.x - (threat.x - e.x), e.y - (threat.y - e.y), 1.3);
        } else {
          e.wanderT -= 1;
          if (e.wanderT <= 0) { e.h = g.rng() * Math.PI * 2; e.wanderT = 50 + Math.floor(g.rng() * 70); }
          e.vx += Math.cos(e.h) * e.accel * 0.6;
          e.vy += Math.sin(e.h) * e.accel * 0.6;
        }
        clampSpeed(e, 0.8);
        break;
      }
      case "rocket": {
        if (e.mode === "aim") {
          const tx = p.x + p.vx * 10, ty = p.y + p.vy * 10;
          const want = Math.atan2(ty - e.y, tx - e.x);
          let d = want - e.h;
          while (d > Math.PI) d -= Math.PI * 2;
          while (d < -Math.PI) d += Math.PI * 2;
          e.h += Math.max(-0.09, Math.min(0.09, d));
          e.vx *= 0.9; e.vy *= 0.9;
          if (Math.abs(d) < ROCKET_ALIGN) { e.mode = "charge"; e.lock = e.h; e.chargeT = 0; enemyDecide(g, e); }
        } else if (e.mode === "charge") {
          e.vx = Math.cos(e.lock) * (e.speed / TICKS_PER_S) * 2.2;
          e.vy = Math.sin(e.lock) * (e.speed / TICKS_PER_S) * 2.2;
          e.chargeT += 1;
          const near = Math.hypot(p.x - e.x, p.y - e.y) < 16;
          const wall = e.x < 14 || e.x > W - 14 || e.y < 14 || e.y > H - 14;
          if (near || wall || e.chargeT > ROCKET_CHARGE_MAX) {
            e.mode = "brake";
            enemyDecide(g, e);
          }
        } else { /* brake */
          e.vx *= 0.86; e.vy *= 0.86;
          if (Math.hypot(e.vx, e.vy) < 0.3) { e.mode = "aim"; enemyDecide(g, e); }
        }
        break;
      }
      case "snake": {
        const target = nearestTrail(g, e) ||
          (Math.hypot(p.x - e.x, p.y - e.y) < 420 ? p : null);
        if (target) steer(e, target.x, target.y, 1.1);
        clampSpeed(e);
        e.path.unshift({ x: e.x, y: e.y });
        if (e.path.length > 120) e.path.pop();
        for (let i = 0; i < e.segs.length; i++) {
          const at = e.path[Math.min(e.path.length - 1, (i + 1) * 6)];
          if (at) { e.segs[i].x = at.x; e.segs[i].y = at.y; }
        }
        break;
      }
      case "weaver": {
        steer(e, p.x, p.y, 1);
        const w = Math.sin(e.t * 0.13) * 0.9;
        const dx = p.x - e.x, dy = p.y - e.y;
        const d = Math.hypot(dx, dy) || 1;
        e.vx += (-dy / d) * w * e.accel;
        e.vy += (dx / d) * w * e.accel;
        clampSpeed(e);
        break;
      }
      case "well": {
        if (e.tag === "burst") { burstWell(g, e); break; }
        steer(e, p.x, p.y, 0.25);
        clampSpeed(e, 0.5);
        /* pull everything nearby */
        const R = 150;
        for (const o of g.enemies) {
          if (o === e) continue;
          const dx = e.x - o.x, dy = e.y - o.y;
          const d = Math.hypot(dx, dy) || 1;
          if (d > R) continue;
          o.vx += (dx / d) * 0.5; o.vy += (dy / d) * 0.5;
          if (d < e.size) { o.hp = 0; e.feed += 2; }
        }
        for (let i = g.bullets.length - 1; i >= 0; i--) {
          const b = g.bullets[i];
          const dx = e.x - b.x, dy = e.y - b.y;
          const d = Math.hypot(dx, dy) || 1;
          if (d > R) continue;
          b.vx += (dx / d) * 0.55; b.vy += (dy / d) * 0.55;
          if (d < e.size) { g.bullets.splice(i, 1); e.feed += 1; }
        }
        const dxp = e.x - p.x, dyp = e.y - p.y;
        const dp = Math.hypot(dxp, dyp) || 1;
        if (dp < R && dp > 1) { p.vx += (dxp / dp) * 0.35; p.vy += (dyp / dp) * 0.35; }
        break;
      }
      default:
        steer(e, p.x, p.y, 1);
        clampSpeed(e);
    }

    e.vx *= 0.965; e.vy *= 0.965;
    e.x += e.vx; e.y += e.vy;

    /* walls: bounce, and a rocket's charge ends on contact */
    if (e.x < e.size) { e.x = e.size; e.vx = Math.abs(e.vx); if (e.type === "rocket") e.mode = "brake"; }
    if (e.x > W - e.size) { e.x = W - e.size; e.vx = -Math.abs(e.vx); if (e.type === "rocket") e.mode = "brake"; }
    if (e.y < e.size) { e.y = e.size; e.vy = Math.abs(e.vy); if (e.type === "rocket") e.mode = "brake"; }
    if (e.y > H - e.size) { e.y = H - e.size; e.vy = -Math.abs(e.vy); if (e.type === "rocket") e.mode = "brake"; }

    /* touching the player is death, unless the shield is up */
    if (p.alive && p.invuln <= 0 && Math.hypot(p.x - e.x, p.y - e.y) < e.size + 7) {
      killPlayer(g);
    }
  }

  function burstWell(g, e) {
    award(g, ENEMY_TYPES.well.points);   /* e.points is not a thing — the type owns it */
    pushGrid(g, e.x, e.y, 240, 7);
    debris(g, e.x, e.y, ENEMY_TYPES.well.color, ENEMY_TYPES.well.debris, 5);
    for (let i = 0; i < ENEMY_TYPES.well.geoms; i++) {
      g.geoms.push({ x: e.x + (g.rng() - 0.5) * 60, y: e.y + (g.rng() - 0.5) * 60, vx: 0, vy: 0 });
    }
    const p = g.player;
    const d = Math.hypot(p.x - e.x, p.y - e.y);
    if (d < 90 && p.invuln <= 0) killPlayer(g);
    e.dead = true;
    g.shake = Math.min(1, g.shake + 0.8);
    g.stats.kills += 1;
  }

  /* ---------- weapons ---------- */
  function fireWeapon(g) {
    const p = g.player;
    const w = weaponOf(g, p.weapon);
    if (!w) return;
    p.cd = w.cooldown;
    g.stats.shots += 1;
    if (w.beam) {
      const x2 = p.x + Math.cos(p.aim) * w.range;
      const y2 = p.y + Math.sin(p.aim) * w.range;
      g.beams.push({ x1: p.x, y1: p.y, x2, y2, life: 9, color: w.color });
      const hits = g.enemies.filter((e) => segCircle(p.x, p.y, x2, y2, e.x, e.y, e.size + 4));
      hits.sort((a, b) =>
        Math.hypot(a.x - p.x, a.y - p.y) - Math.hypot(b.x - p.x, b.y - p.y));
      for (const e of hits) damageEnemy(g, e, w.dmg, e.x, e.y);
      pushGrid(g, p.x, p.y, 90, 1.6);
      return;
    }
    if (w.ring) {
      g.rings.push({ x: p.x, y: p.y, r: w.r0, r1: w.r1, speed: w.speed, dmg: w.dmg,
        color: w.color, hit: {} });
      pushGrid(g, p.x, p.y, 70, 1.4);
      return;
    }
    for (let i = 0; i < w.count; i++) {
      const off = w.count > 1 ? (i - (w.count - 1) / 2) * w.spread : 0;
      const a = p.aim + off;
      g.bullets.push({
        x: p.x + Math.cos(a) * 10, y: p.y + Math.sin(a) * 10,
        vx: Math.cos(a) * w.speed, vy: Math.sin(a) * w.speed,
        dmg: w.dmg, life: w.life, color: w.color, weapon: p.weapon,
      });
    }
    pushGrid(g, p.x, p.y, 40, 0.7);
  }

  function segCircle(x1, y1, x2, y2, cx, cy, r) {
    const dx = x2 - x1, dy = y2 - y1;
    const len2 = dx * dx + dy * dy || 1;
    let t = ((cx - x1) * dx + (cy - y1) * dy) / len2;
    t = Math.max(0, Math.min(1, t));
    const px = x1 + dx * t, py = y1 + dy * t;
    return Math.hypot(cx - px, cy - py) <= r;
  }

  function updateBullets(g) {
    for (let i = g.bullets.length - 1; i >= 0; i--) {
      const b = g.bullets[i];
      const fx = b.x, fy = b.y;   /* swept: the whole step is the window, so fast shots cannot tunnel */
      b.x += b.vx; b.y += b.vy;
      b.life -= 1;
      if (b.life <= 0 || b.x < -20 || b.x > W + 20 || b.y < -20 || b.y > H + 20) {
        g.bullets.splice(i, 1);
        continue;
      }
      for (let j = g.enemies.length - 1; j >= 0; j--) {
        const e = g.enemies[j];
        if (e.dead) continue;
        if (!segCircle(fx, fy, b.x, b.y, e.x, e.y, e.size + 3)) continue;
        damageEnemy(g, e, b.dmg, b.x, b.y);
        g.bullets.splice(i, 1);
        break;
      }
    }
    /* rings: damage each enemy once as the front passes over it */
    for (let i = g.rings.length - 1; i >= 0; i--) {
      const r = g.rings[i];
      r.r += r.speed;
      if (r.r >= r.r1) { g.rings.splice(i, 1); continue; }
      for (const e of g.enemies) {
        if (e.dead || r.hit[e.id]) continue;
        const d = Math.hypot(e.x - r.x, e.y - r.y);
        if (Math.abs(d - r.r) < 12) {
          r.hit[e.id] = true;
          damageEnemy(g, e, r.dmg, e.x, e.y);
        }
      }
    }
    for (let i = g.beams.length - 1; i >= 0; i--) {
      g.beams[i].life -= 1;
      if (g.beams[i].life <= 0) g.beams.splice(i, 1);
    }
  }

  /* ---------- damage, death, splitting ---------- */
  function damageEnemy(g, e, dmg, hx, hy) {
    if (e.dead) return;
    /* a snake only hurts at the head: body hits clank and score nothing */
    if (e.type === "snake" && Math.hypot(e.x - hx, e.y - hy) > e.size + 4) {
      debris(g, hx, hy, "#64748b", 2, 1.2);
      return;
    }
    /* black holes eat fire instead of taking it — feeding one is how it bursts */
    if (e.type === "well") {
      e.feed += 1;
      debris(g, hx, hy, ENEMY_TYPES.well.color, 3, 1.5);
      if (e.feed >= WELL_BURST) {
        e.tag = "burst";
        record(g, "burst");
        burstWell(g, e);
      }
      return;
    }
    e.hp -= dmg;
    if (e.hp > 0) {
      debris(g, hx, hy, ENEMY_TYPES[e.type].color, 3, 1.6);
      pushGrid(g, e.x, e.y, 40, 1.1);
      /* a wounded splitter may split early: the player is inside its space */
      const split = SPLIT_CHILDREN[e.type];
      if (split && Math.hypot(g.player.x - e.x, g.player.y - e.y) < 90) {
        splitEnemy(g, e, split, 2);
      }
      return;
    }
    killEnemy(g, e, false);
  }

  function splitEnemy(g, e, spec, n) {
    const count = n || spec.n;
    const kids = [];
    for (let i = 0; i < count; i++) {
      if (g.enemies.length >= g.cfg.maxEnemies) break;
      const a = (Math.PI * 2 * i) / count + g.rng() * 0.4;
      const kid = spawnEnemy(g, spec.type, e.x + Math.cos(a) * (e.size + 12),
        e.y + Math.sin(a) * (e.size + 12));
      if (kid) { kid.vx = Math.cos(a) * 4.5; kid.vy = Math.sin(a) * 4.5; kids.push(kid); }
    }
    record(g, "split");
    debris(g, e.x, e.y, ENEMY_TYPES[e.type].color, 8, 2.4);
    pushGrid(g, e.x, e.y, 70, 2);
    return kids;
  }

  function killEnemy(g, e, byBomb) {
    if (e.dead) return;
    const t = ENEMY_TYPES[e.type];
    e.dead = true;
    g.stats.kills += 1;
    if (!byBomb) {
      award(g, t.points);
      for (let i = 0; i < t.geoms; i++) {
        g.geoms.push({ x: e.x + (g.rng() - 0.5) * 18, y: e.y + (g.rng() - 0.5) * 18,
          vx: (g.rng() - 0.5) * 1.2, vy: (g.rng() - 0.5) * 1.2 });
      }
    }
    debris(g, e.x, e.y, t.color, t.debris, byBomb ? 2.4 : 3.4);
    pushGrid(g, e.x, e.y, byBomb ? 90 : 64, byBomb ? 2.4 : 2.6);
    g.shake = Math.min(1, g.shake + (t.size / 18) * 0.35);
    const split = SPLIT_CHILDREN[e.type];
    if (split && !byBomb) splitEnemy(g, e, split);
  }

  function killPlayer(g) {
    const p = g.player;
    if (!p.alive || p.invuln > 0) return;
    g.lives -= 1;
    g.stats.deaths += 1;
    g.mult = 1;
    g.taken = 0;          /* the multiplier ladder restarts with you */
    g.shake = 1;
    g.flash = 0.75;
    debris(g, p.x, p.y, "#ffffff", 26, 5);
    pushGrid(g, p.x, p.y, 300, 8);
    /* the death bomb: a mercy clear, worth nothing */
    for (const e of g.enemies) {
      if (Math.hypot(e.x - p.x, e.y - p.y) < 300) killEnemy(g, e, true);
    }
    if (g.lives < 0) { g.over = true; p.alive = false; return; }
    p.x = W / 2; p.y = H / 2; p.vx = 0; p.vy = 0;
    p.invuln = Math.round(g.cfg.invuln * TICKS_PER_S);
  }

  function useBomb(g) {
    const p = g.player;
    if (g.over || !p.alive) return false;
    if (g.bombs <= 0) return false;
    g.bombs -= 1;
    g.stats.bombsUsed += 1;
    g.flash = 0.9;
    g.shake = 1;
    pushGrid(g, p.x, p.y, 900, 11);
    for (const e of g.enemies.slice()) {
      if (!e.dead) killEnemy(g, e, true);
    }
    debris(g, p.x, p.y, "#bae6fd", 60, 6);
    return true;
  }

  /* ---------- geoms, multiplier, lives, bombs ---------- */
  function updateGeoms(g) {
    const p = g.player;
    for (let i = g.geoms.length - 1; i >= 0; i--) {
      const q = g.geoms[i];
      q.x += q.vx; q.y += q.vy;
      q.vx *= 0.95; q.vy *= 0.95;
      const dx = p.x - q.x, dy = p.y - q.y;
      const d = Math.hypot(dx, dy) || 1;
      if (d < MULT_MAGNET) { q.vx += (dx / d) * 1.5; q.vy += (dy / d) * 1.5; }
      if (d < MULT_PICKUP) {
        g.geoms.splice(i, 1);
        g.taken += 1;
        g.stats.geomsTaken += 1;
        if (g.taken % GEOM_STEP === 0) {
          g.mult = Math.min(MULT_CAP, g.mult + 1);
          g.stats.multPeak = Math.max(g.stats.multPeak, g.mult);
        }
      }
    }
  }

  function checkAwards(g) {
    if (g.score >= g.nextLifeAt) {
      g.nextLifeAt += LIFE_SCORE_STEP;
      g.lives += 1;
    }
    if (g.score >= g.nextBombAt) {
      g.nextBombAt += BOMB_SCORE_STEP;
      g.bombs += 1;
    }
  }

  /* ---------- the player ---------- */
  function makeInput() {
    return { up: false, down: false, left: false, right: false, fire: false, aim: -Math.PI / 2 };
  }

  function updatePlayer(g) {
    const p = g.player;
    const c = g.cfg;
    if (!p.alive) return;
    p.invuln = Math.max(0, p.invuln - 1);
    const ax = (p.in.right ? 1 : 0) - (p.in.left ? 1 : 0);
    const ay = (p.in.down ? 1 : 0) - (p.in.up ? 1 : 0);
    const maxSpd = c.shipSpeed / TICKS_PER_S;
    const drag = 0.90 + 0.08 * c.inertia;
    const accel = maxSpd * (1 - drag) / drag;
    if (ax || ay) {
      const n = Math.hypot(ax, ay) || 1;
      p.vx += (ax / n) * accel * 1.6;
      p.vy += (ay / n) * accel * 1.6;
      thrustTrail(g, p);
    }
    p.vx *= drag; p.vy *= drag;
    const s = Math.hypot(p.vx, p.vy);
    if (s > maxSpd) { p.vx = (p.vx / s) * maxSpd; p.vy = (p.vy / s) * maxSpd; }
    p.x = Math.max(10, Math.min(W - 10, p.x + p.vx));
    p.y = Math.max(10, Math.min(H - 10, p.y + p.vy));
    p.aim = p.in.aim;
    if (p.cd > 0) p.cd -= 1;
    if (p.in.fire && p.cd <= 0) fireWeapon(g);

    /* the trail snakes eat — a fixed-length recent path */
    if (g.ticks % 3 === 0) {
      g.trail.unshift({ x: p.x, y: p.y });
      if (g.trail.length > 90) g.trail.pop();
    }
  }

  /* ---------- the tick ---------- */
  function tickGame(g, engineAsk) {
    if (g.over) return g;
    g.ticks += 1;
    g.waveTick += 1;

    const tier = tierFor(g.score, g.ticks / TICKS_PER_S, g.cfg.difficulty);
    if (tier > g.tier) {
      g.tier = tier;
      /* the newly unlocked geometry enters first — the engine names it */
      waveLead(g, engineAsk, "tier " + tier + " unlock", unlocksForTier(tier));
    }

    if (g.waveTick >= WAVE_LEN) nextWave(g, engineAsk);
    spawnStep(g);

    updatePlayer(g);
    for (let i = g.enemies.length - 1; i >= 0; i--) {
      const e = g.enemies[i];
      if (!e.dead) enemyStep(g, e);
      if (e.dead) g.enemies.splice(i, 1);
    }
    updateBullets(g);
    updateGeoms(g);
    updateParticles(g);
    updateGrid(g);
    checkAwards(g);

    g.shake = Math.max(0, g.shake - 0.035);
    g.flash = Math.max(0, g.flash - 0.05);
    return g;
  }

  /* ---------- readouts ---------- */
  function runSummary(g) {
    return {
      score: g.score, wave: g.wave, tier: g.tier, lastEntered: g.lastEntered,
      mult: g.mult, multPeak: g.stats.multPeak, lives: g.lives, bombs: g.bombs,
      kills: g.stats.kills, deaths: g.stats.deaths, shots: g.stats.shots,
      geoms: g.stats.geomsTaken, particles: g.particles.length,
      enemies: g.enemies.length, ticks: g.ticks, timeS: g.ticks / TICKS_PER_S,
      decisions: { total: g.decisions.total, tags: Object.assign({}, g.decisions.tags) },
      rungs: Object.assign({}, g.rungs), engineCalls: g.engineCalls,
      over: g.over,
    };
  }

  function configStamp(c) {
    return [
      c.difficulty, c.seed, "spawn" + c.spawnRate, "max" + c.maxEnemies,
      "spd" + c.shipSpeed, "inertia" + c.inertia, "lives" + c.lives,
      "inv" + c.invuln, "warp" + c.warp, "grid" + c.density, "bombs" + c.bombs,
    ].join("|");
  }

  /* ---------- game state ---------- */
  function createGame(cfg) {
    const c = normalizeConfig(cfg);
    const rng = makeRng(c.seed + ":gridwars");
    const diff = DIFFICULTIES[c.difficulty];
    const g = {
      cfg: c, rng, nextId: 1,
      ticks: 0, wave: 1, waveTick: 0, over: false,
      score: 0, mult: 1, taken: 0, lives: c.lives, bombs: c.bombs,
      nextLifeAt: LIFE_SCORE_STEP, nextBombAt: BOMB_SCORE_STEP,
      tier: 1, lastEntered: null, lastDecision: null, lastEngine: null,
      player: {
        x: W / 2, y: H / 2, vx: 0, vy: 0, aim: -Math.PI / 2,
        weapon: "pulse", cd: 0, alive: true,
        invuln: Math.round(c.invuln * TICKS_PER_S), in: null,
      },
      enemies: [], bullets: [], rings: [], beams: [], geoms: [], particles: [],
      trail: [], queue: waveComposition(1, 1, c.seed),
      spawnIn: Math.round(SPAWN_BASE / (c.spawnRate * diff.spawnScale)),
      grid: makeGrid(c.density),
      decisions: { total: 0, tags: {} },
      rungs: { rule: 0, engine: 0 }, engineCalls: 0,
      shake: 0, flash: 0,
      stats: { kills: 0, deaths: 0, bombsUsed: 0, shots: 0, multPeak: 1, geomsTaken: 0 },
    };
    g.input = makeInput();
    g.player.in = g.input;
    return g;
  }

  /* ---------- exports ---------- */
  globalThis.GridWarsCore = {
    /* constants */
    TICKS_PER_S, W, H, DECIDE_EVERY, WAVE_LEN, GEOM_STEP, MULT_CAP,
    BOMB_SCORE_STEP, LIFE_SCORE_STEP, PARTICLE_CAP, THREAT_R, TRAIL_SENSE,
    WELL_BURST, ROCKET_ALIGN, ROCKET_CHARGE_MAX, SURVIVAL_PER_S,
    TAGS, TIERS, DIFFICULTIES, ENEMY_TYPES, SPLIT_CHILDREN,
    WEAPONS, WEAPON_ORDER, DEFAULT_CONFIG,
    /* rng passthrough (same source the SDK exports) */
    hashSeed, mulberry32, makeRng,
    /* setup + config */
    normalizeConfig, normalizeWeapons, weaponOf, WEAPON_LIMITS,
    createGame, makeInput, makeGrid,
    /* tiers + waves */
    tierFor, effectiveScore, typesForTier, unlocksForTier,
    waveComposition, nextWave, waveLead, spawnInterval, spawnEnemy,
    /* decisions + sim */
    enemyDecide, inboundBullet, nearestTrail, steer, jinkFrom, enemyStep,
    fireWeapon, updateBullets, damageEnemy, splitEnemy, killEnemy, killPlayer,
    segCircle,
    useBomb, burstWell, updatePlayer, tickGame,
    /* grid + particles */
    pushGrid, updateGrid, debris,
    /* readouts */
    runSummary, configStamp,
  };
})();
