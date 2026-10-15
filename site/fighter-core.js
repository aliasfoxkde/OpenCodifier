/* Sparring Partner core — pure simulation for the side-view fighter. No
   DOM, no canvas, no timers: deterministic given (seed, config, input
   script), unit-testable under `node --test` like racing-core.js (import
   for side effect, read the global; sdk/decisions-sdk.js must load first —
   the seeded RNG and clamps are the SDK's, not reimplemented here).

   Design contract (docs/planning/playground/DECISIONS-SDK-PLAN.md §5.5):
   - 1v1 on a 1D ring: walk, jab, hook, block, duck, dash, special (meter);
     attacks resolve on their first active frame — range check, block chip,
     duck slips under high swings;
   - the opponent keeps a DECAYED histogram of the player's committed moves
     per situation (range band × player stance) — that table is the learning;
   - each decision tick the table predicts the player's next move (Laplace,
     integer per-mille) and every candidate response gets an EV over a value
     matrix; a NEAR-TIE between the top two (within 0.3 points — the spec
     says "close EVs"; calibrated on bot matches, ~1 in 12 decisions is
     genuinely contested and goes to the engine) escalates to the engine as a real choice with the
     habit table in `state` — the engine reasons over recorded habits;
   - ring control beyond decision range is a rule, labelled as one, the way
     racing labels traffic; a bot player (seeded, readable) drives whole
     matches in tests, and resetLearning() erases the table (auditability). */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;

  const TICKS_PER_S = 60;
  const RING_W = 100;             /* world units, you.x < zed.x always */
  const WALL = 4;                 /* ring edge inset */
  const MIN_GAP = 7;              /* no pass-through, no overlap */
  const ROUND_TICKS = 60 * 60;    /* one minute per round */
  const INTERLUDE_TICKS = 90;     /* frozen beat between rounds */
  const DECIDE_MAX = 44;          /* beyond this: ring rule, not a choice */
  const TIE_EPS = 300;            /* milli-points (0.3 pt): near-tie goes to engine.
                                   Calibrated on 520 bot decisions: 8.8% land
                                   here — a handful of real escalations a bout */
  const METER_MAX = 100;
  const CHIP = 0.2;               /* blocked damage fraction */
  const WALK_SPEED = 24;

  const MOVES = {
    jab:     { startup: 4,  active: 3,  recovery: 6,  range: 18, dmg: 6,  high: false },
    hook:    { startup: 9,  active: 4,  recovery: 12, range: 15, dmg: 14, high: true },
    special: { startup: 16, active: 5,  recovery: 16, range: 22, dmg: 24, high: true },
  };
  const DASH = { startup: 2, active: 8, recovery: 4, speed: 110 };
  const BLOCK_HOLD = 18;
  const DUCK_HOLD = 12;

  /* every committed action the habit table records — combat answers too */
  const ANSWERS = ["jab", "hook", "block", "duck", "dash", "special"];
  const MOVE_NAMES = {
    jab: "jab — fast mid poke",
    hook: "hook — heavy high swing",
    block: "block — soak most of the hit",
    duck: "duck — slip under high swings",
    dash: "dash — close or open the gap",
    special: "special — full meter, huge",
  };

  /* VALUE[response][predicted] in points: what answering X is worth when
     the player is about to do Y. The habit table is a distribution over Y,
     so it directly steers these EVs — spam jab and "jab" stops being free.
     The anti-special columns are mild on purpose: special needs a FULL
     meter and the Laplace floor keeps a pseudo-probability on it forever,
     so large negatives here would leak into every EV. */
  const VALUE = {
    jab:     { jab: 0,   hook: -8, block: 2,  duck: 6,  dash: -1, special: -8 },
    hook:    { jab: -8,  hook: 0,  block: 2,  duck: -6, dash: -2, special: -6 },
    block:   { jab: -1,  hook: -3, block: 0,  duck: 0,  dash: 1,  special: -3 },
    duck:    { jab: -6,  hook: 10, block: -1, duck: 0,  dash: -1, special: 10 },
    dash:    { jab: -4,  hook: -5, block: 1,  duck: 1,  dash: 0,  special: -3 },
    special: { jab: 18,  hook: 10, block: 3,  duck: -6, dash: -3, special: 0 },
  };

  const DEFAULT_CONFIG = {
    seed: "oc-spar",
    adaptivity: 0.75,   /* 0.5..0.95 — histogram decay per observation */
    aggression: 1,      /* 0..4 — flat bonus on attack candidates */
    difficulty: 2,      /* 1..5 — maps to the opponent's reaction delay */
    rounds: 3,          /* 1, 3, 5 — first to ceil(rounds/2) */
    botPlayer: false,
  };
  const REACTION = [30, 24, 18, 12, 8];   /* ticks between decisions */

  function normalizeConfig(raw) {
    const c = Object.assign({}, DEFAULT_CONFIG, raw && typeof raw === "object" ? raw : {});
    c.seed = String(c.seed == null ? DEFAULT_CONFIG.seed : c.seed);
    c.adaptivity = DK.clampf(+c.adaptivity, 0.5, 0.95, DEFAULT_CONFIG.adaptivity);
    c.aggression = Math.round(DK.clampf(+c.aggression, 0, 4, DEFAULT_CONFIG.aggression));
    c.difficulty = Math.round(DK.clampf(+c.difficulty, 1, 5, DEFAULT_CONFIG.difficulty));
    c.rounds = Math.round(DK.clampf(+c.rounds, 1, 5, DEFAULT_CONFIG.rounds));
    if (c.rounds === 2) c.rounds = 3;
    if (c.rounds === 4) c.rounds = 5;
    c.botPlayer = c.botPlayer === true;
    return c;
  }

  /* the stamp separates seed, learning speed, style, delay, and length */
  function fighterStamp(cfg) {
    const c = normalizeConfig(cfg);
    return ["fighter", c.seed, c.adaptivity, c.aggression, c.difficulty,
      c.rounds].join("|");
  }

  /* ---------- fighters ---------- */

  function makeFighter(kind, name, color, x, facing) {
    return {
      kind, name, color, x, facing,
      health: 100, meter: 0,
      state: "idle",        /* idle|startup|active|recovery|block|duck|dash */
      move: null,           /* committed move id while it runs */
      frame: 0,
      dashDir: 1,
      holdUntil: 0,         /* zed's block/duck release tick */
      walkDir: 0, walkUntil: 0,   /* zed's ring-rule walk window */
      plan: null,           /* bot player's current intent */
      wins: 0, hits: 0, dealt: 0, taken: 0,
    };
  }

  function createGame(raw) {
    const cfg = normalizeConfig(raw);
    const stamp = fighterStamp(cfg);
    return {
      cfg, stamp,
      you: makeFighter("player", "you", "#fde047", 32, 1),
      zed: makeFighter("opp", "zed", "#c2413c", 68, -1),
      reaction: REACTION[cfg.difficulty - 1],
      decideAt: 0,                       /* next tick zed may decide */
      t: 0, ticks: 0,
      round: 1, roundTicks: 0,
      interlude: 0, roundResult: null,   /* {winner, how} between rounds */
      habits: { sit: {}, observations: 0 },
      rungs: { habit: 0, engine: 0, rule: 0 },
      engineCalls: 0,
      lastDecision: null,
      flash: null,                       /* {side, t} hit feedback for the shell */
      over: false, result: null,         /* "won" | "lost" */
      prng: DK.makeRng("player:" + stamp),  /* the bot driver's stream, seeded */
    };
  }

  const dist = (g) => g.zed.x - g.you.x;   /* >= MIN_GAP by construction */
  const band = (d) => (d < 22 ? "close" : d <= DECIDE_MAX ? "mid" : "far");
  const stanceOf = (f) => {
    if (f.state === "startup" || f.state === "active") return "attack";
    if (f.state === "recovery") return "recover";
    if (f.state === "dash") return "dash";
    return f.state;   /* idle | block | duck */
  };
  const actionable = (f) => f.state === "idle";
  const situationKey = (g) => band(dist(g)) + "×" + stanceOf(g.you);

  /* ---------- the learning ---------- */

  /* decayed histogram update: every key relaxes toward 0, the observed
     move gets +1 — adaptivity is how fast old habits are forgotten. The
     situation is what the opponent SAW, so callers pass the pre-commit
     stance (recording after the commit would log every move as "attack") */
  function observe(g, moveId, sit) {
    const s = sit || situationKey(g);
    const row = g.habits.sit[s] || (g.habits.sit[s] = {});
    for (const k of ANSWERS) row[k] = (row[k] || 0) * g.cfg.adaptivity + (k === moveId ? 1 : 0);
    g.habits.observations += 1;
  }

  /* commit + record in one step, in the right order */
  function tryCommit(g, f, moveId, dashDir) {
    const sit = situationKey(g);
    if (!commit(f, moveId, dashDir)) return false;
    observe(g, moveId, sit);
    return true;
  }

  /* predicted player move, integer per-mille, Laplace pseudo-count 1 */
  function habitProbs(g, sit) {
    const row = g.habits.sit[sit] || {};
    let total = 0;
    for (const k of ANSWERS) total += row[k] || 0;
    const probs = {};
    for (const k of ANSWERS) {
      probs[k] = Math.round(1000 * ((row[k] || 0) + 1) / (total + ANSWERS.length));
    }
    return probs;
  }

  function habitTable(g) {
    const rows = [];
    for (const sit of Object.keys(g.habits.sit).sort()) {
      const row = g.habits.sit[sit];
      for (const k of ANSWERS) if ((row[k] || 0) > 0.01) rows.push({ sit, move: k, count: row[k] });
    }
    return rows;
  }

  function resetLearning(g) {
    g.habits = { sit: {}, observations: 0 };
  }

  const pct = (permille) => Math.round(permille / 10) + "%";

  /* ---------- decisions ---------- */

  /* pick zed's response: EVs over the value matrix weighted by the habit
     table; a near-tie between the top two escalates to the engine */
  function decideResponse(g, engineChoose) {
    const sit = situationKey(g);
    const probs = habitProbs(g, sit);
    const d = dist(g);
    const aggr = g.cfg.aggression * 200;   /* flat attack bonus, milli-points */
    const cands = [];
    for (const id of ANSWERS) {
      if (id === "special" && g.zed.meter < METER_MAX) continue;   /* can't commit it */
      let ev = 0;
      for (const p of ANSWERS) ev += probs[p] * VALUE[id][p];
      if (id === "jab" || id === "hook" || id === "special") {
        ev += aggr;
        if (d > MOVES[id].range) ev -= 500;   /* out of range: whiff risk */
      }
      cands.push({ id, ev, description: MOVE_NAMES[id] });
    }
    cands.sort((a, b) => b.ev - a.ev || ANSWERS.indexOf(a.id) - ANSWERS.indexOf(b.id));
    const nearTie = cands.length > 1 && (cands[0].ev - cands[1].ev) <= TIE_EPS;
    let pick = cands[0].id;
    let rung = "habit";
    if (nearTie && engineChoose) {
      /* the full wire contract — state + question text + policy. A bare
         {questions} request makes the WASM decide() throw (measured on the
         racer: the bridge burned its fail limit and went unavailable) */
      const predTxt = ANSWERS.map((k) => k + " " + pct(probs[k])).join(", ");
      const answer = engineChoose({
        state: {
          text: "Habit read: in situation " + sit + " the player's decayed record is " +
            predTxt + ". Zed's answers " + cands[0].id + " (" + (cands[0].ev / 1000).toFixed(2) +
            ") and " + cands[1].id + " (" + (cands[1].ev / 1000).toFixed(2) +
            ") score within " + (TIE_EPS / 1000).toFixed(2) + " points. Health Zed " + Math.round(g.zed.health) +
            " / player " + Math.round(g.you.health) + ", meter " + Math.round(g.zed.meter) +
            "/" + METER_MAX + ", round " + g.round + ". Pick a response.",
          facts: {},
        },
        questions: [{
          type: "choice", id: "fighter-response",
          text: "How should Zed answer in situation " + sit + "?",
          candidates: cands.map((c) => ({ id: c.id, description: c.description })),
        }],
        policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" },
      });
      g.engineCalls += 1;
      const ans = answer && answer.outcome === "accept" && Array.isArray(answer.answers)
        ? answer.answers.find((a) => a && typeof a.choice === "string") : null;
      const choice = ans ? ans.choice : null;
      if (choice && cands.some((c) => c.id === choice)) {
        pick = choice;
        rung = "engine";
      }
    }
    commit(g.zed, pick, -g.zed.facing);   /* zed's dash opens the gap */
    if (pick === "block") g.zed.holdUntil = g.ticks + BLOCK_HOLD;
    if (pick === "duck") g.zed.holdUntil = g.ticks + DUCK_HOLD;
    g.rungs[rung] += 1;
    const bestOf = (pr) => ANSWERS.reduce((b, k) => (pr[k] > pr[b] ? k : b), ANSWERS[0]);
    g.lastDecision = {
      who: "zed", sit,
      predicted: bestOf(probs) + " " + pct(probs[bestOf(probs)]),
      pick, rung, nearTie,
      evGap: Math.round(cands[0].ev - (cands[1] ? cands[1].ev : cands[0].ev)),
      at: { d: Math.round(dist(g)) },
    };
    return { pick, rung, nearTie };
  }

  /* commit a move: only from idle — mid-swing inputs are dropped. dashDir
     is +1 toward the opponent, -1 away (zed always opens the gap) */
  function commit(f, moveId, dashDir) {
    if (!actionable(f)) return false;
    if (moveId === "special" && f.meter < METER_MAX) return false;
    if (MOVES[moveId]) {
      f.state = "startup";
      f.move = moveId;
      f.frame = 0;
      if (moveId === "special") f.meter = 0;
      return true;
    }
    if (moveId === "dash") {
      f.state = "startup";
      f.move = "dash";
      f.dashDir = dashDir || f.facing;
      f.frame = 0;
      return true;
    }
    if (moveId === "block") { f.state = "block"; f.move = "block"; f.frame = 0; return true; }
    if (moveId === "duck") { f.state = "duck"; f.move = "duck"; f.frame = 0; return true; }
    return false;
  }

  /* ---------- resolution ---------- */

  function resolveHit(g, att, def, moveId) {
    const m = MOVES[moveId];
    if (Math.abs(def.x - att.x) > m.range) return;         /* whiff */
    if (def.state === "duck" && m.high) return;            /* slipped under */
    let dmg = m.dmg;
    const blocked = def.state === "block";
    if (blocked) dmg = Math.max(1, Math.round(m.dmg * CHIP));
    def.health = Math.max(0, def.health - dmg);
    def.taken += dmg;
    att.dealt += dmg;
    att.hits += 1;
    att.meter = Math.min(METER_MAX, att.meter + dmg * (blocked ? 0.2 : 0.55));
    def.meter = Math.min(METER_MAX, def.meter + dmg * (blocked ? 0.1 : 0.45));
    g.flash = { side: def === g.you ? "you" : "zed", t: 8 };
  }

  /* advance one fighter: state machine first (attacks resolve on their
     first active frame), then holds and walking for the idle ones */
  function tickFighter(g, f, input) {
    const you = f === g.you;
    const inp = you ? (input || {}) : null;
    if (f.state === "startup" || f.state === "active" || f.state === "recovery") {
      f.frame += 1;
      const m = MOVES[f.move];
      if (f.state === "startup") {
        if (f.frame >= (m ? m.startup : DASH.startup)) {
          f.state = "active";
          f.frame = 0;
          if (m) resolveHit(g, f, you ? g.zed : g.you, f.move);
        }
      } else if (f.state === "active") {
        if (!m) f.x += f.dashDir * DASH.speed / TICKS_PER_S;
        if (f.frame >= (m ? m.active : DASH.active)) { f.state = "recovery"; f.frame = 0; }
      } else if (f.frame >= (m ? m.recovery : DASH.recovery)) {
        f.state = "idle";
        f.move = null;
        f.frame = 0;
      }
      return;
    }
    if (f.state === "block" || f.state === "duck") {
      const held = you ? (f.state === "block" ? inp.block : inp.duck)
        : g.ticks < f.holdUntil;
      if (!held) { f.state = "idle"; f.move = null; f.frame = 0; }
      return;
    }
    /* idle: walk, then try to commit by priority (special > hook > jab >
       dash > block > duck). The human's commits are what zed learns. */
    f.x += (you ? DK.clampf(inp.walk || 0, -1, 1)
      : (g.ticks < f.walkUntil ? f.walkDir : 0)) * WALK_SPEED / TICKS_PER_S;
    if (!you) return;
    if (inp.special && tryCommit(g, f, "special")) ;
    else if (inp.hook && tryCommit(g, f, "hook")) ;
    else if (inp.jab && tryCommit(g, f, "jab")) ;
    else if (inp.dashIn && tryCommit(g, f, "dash", 1)) ;
    else if (inp.dashOut && tryCommit(g, f, "dash", -1)) ;
    else if (inp.block && tryCommit(g, f, "block")) ;
    else if (inp.duck && tryCommit(g, f, "duck")) ;
  }

  /* zed's brain: ring rule far away, habit/EV decision in range on the
     reaction cadence. Zed prefers to read the player BETWEEN moves — a
     cadence tick that lands mid-swing defers (at most one reaction window,
     or a perpetual-stance player would freeze the read forever) */
  function oppControl(g, engineChoose) {
    const zed = g.zed;
    if (!actionable(zed)) return;
    const d = dist(g);
    if (d > DECIDE_MAX) {
      zed.walkDir = -1;
      zed.walkUntil = g.ticks + 24;
      g.rungs.rule += 1;
      return;
    }
    if (g.ticks < g.decideAt) return;
    if (stanceOf(g.you) !== "idle" && g.ticks < g.decideAt + g.reaction) return;
    g.decideAt = g.ticks + g.reaction;
    decideResponse(g, engineChoose);
  }

  /* the bot player — a seeded, readable baseline, honest about being one */
  function botInput(g) {
    const you = g.you;
    if (!actionable(you)) return {};
    const p = you.plan;
    if (p && g.ticks < p.until) {
      if (p.kind === "block") return { block: true };
      if (p.kind === "duck") return { duck: true };
      if (p.kind === "walk") return { walk: p.dir };
      return {};
    }
    const rng = g.prng;
    const d = dist(g);
    const r = rng();
    if (d > 40) {
      if (r < 0.2) { tryCommit(g, you, "dash", 1); return {}; }
      you.plan = { kind: "walk", dir: 1, until: g.ticks + 14 + Math.floor(rng() * 22) };
    } else if (d > 20) {
      if (r < 0.22) { tryCommit(g, you, "dash", 1); return {}; }
      if (r < 0.45) { tryCommit(g, you, "jab"); return {}; }
      you.plan = { kind: "walk", dir: 1, until: g.ticks + 10 + Math.floor(rng() * 18) };
    } else {
      if (r < 0.38) { tryCommit(g, you, "jab"); return {}; }
      if (r < 0.6) { tryCommit(g, you, "hook"); return {}; }
      if (r < 0.78) { you.plan = { kind: "block", until: g.ticks + 14 + Math.floor(rng() * 14) }; }
      else if (r < 0.9) { you.plan = { kind: "duck", until: g.ticks + 8 + Math.floor(rng() * 12) }; }
      else { you.plan = { kind: "walk", dir: -1, until: g.ticks + 8 + Math.floor(rng() * 12) }; }
    }
    return botInput(g);   /* stored holds/walks resolve on the next pass */
  }

  /* ---------- rounds ---------- */

  function resetRound(g) {
    for (const f of [g.you, g.zed]) {
      f.health = 100;
      f.meter = 0;
      f.state = "idle";
      f.move = null;
      f.frame = 0;
      f.holdUntil = 0;
      f.walkUntil = 0;
      f.plan = null;
    }
    g.you.x = 32;
    g.zed.x = 68;
    g.roundTicks = 0;
  }

  function endRound(g, winner, how) {
    g.roundResult = { winner, how };
    if (winner === "you") g.you.wins += 1;
    else if (winner === "zed") g.zed.wins += 1;
    const need = Math.ceil(g.cfg.rounds / 2);
    if (g.you.wins >= need || g.zed.wins >= need) {
      g.over = true;
      g.result = g.you.wins >= need ? "won" : "lost";
      return;
    }
    g.interlude = INTERLUDE_TICKS;
  }

  function tickGame(g, input, engineChoose) {
    if (g.over) return g;
    g.engineChoose = engineChoose || null;
    if (g.interlude > 0) {
      g.interlude -= 1;
      if (g.interlude === 0) {
        g.round += 1;
        g.roundResult = null;
        resetRound(g);
      }
      return g;
    }
    g.t += 1 / TICKS_PER_S;
    g.ticks += 1;
    g.roundTicks += 1;
    if (g.flash) { g.flash.t -= 1; if (g.flash.t <= 0) g.flash = null; }

    const humanInput = input || {};
    const youInput = g.cfg.botPlayer ? botInput(g) : {
      walk: DK.clampf(humanInput.walk || 0, -1, 1),
      jab: !!humanInput.jab, hook: !!humanInput.hook,
      block: !!humanInput.block, duck: !!humanInput.duck,
      special: !!humanInput.special,
      dashIn: !!humanInput.dashIn, dashOut: !!humanInput.dashOut,
    };

    oppControl(g, engineChoose);
    tickFighter(g, g.you, youInput);
    tickFighter(g, g.zed, null);

    /* no pass-through, no overlap — sides never swap */
    const gap = g.zed.x - g.you.x;
    if (gap < MIN_GAP) {
      const push = (MIN_GAP - gap) / 2;
      g.you.x -= push;
      g.zed.x += push;
    }
    g.you.x = DK.clampf(g.you.x, WALL, RING_W - WALL - MIN_GAP);
    g.zed.x = DK.clampf(g.zed.x, WALL + MIN_GAP, RING_W - WALL);

    /* round end: KO or the clock */
    const youDown = g.you.health <= 0;
    const zedDown = g.zed.health <= 0;
    if (youDown || zedDown) {
      endRound(g, youDown && zedDown ? "draw" : youDown ? "zed" : "you", "ko");
    } else if (g.roundTicks >= ROUND_TICKS) {
      endRound(g, g.you.health > g.zed.health ? "you"
        : g.zed.health > g.you.health ? "zed" : "draw", "time");
    }
    return g;
  }

  function runSummary(g) {
    return {
      seed: g.cfg.seed,
      stamp: fighterStamp(g.cfg),
      result: g.result,
      over: g.over,
      ticks: g.ticks,
      round: g.round,
      wins: { you: g.you.wins, zed: g.zed.wins },
      health: { you: g.you.health, zed: g.zed.health },
      hits: { you: g.you.hits, zed: g.zed.hits },
      dealt: { you: g.you.dealt, zed: g.zed.dealt },
      observations: g.habits.observations,
      rungs: Object.assign({}, g.rungs),
      engineCalls: g.engineCalls,
      lastDecision: g.lastDecision ? Object.assign({}, g.lastDecision) : null,
    };
  }

  /* whole-match bot simulation — the deterministic drive the tests use */
  function simulateMatch(cfg, engineChoose) {
    const g = createGame(cfg);
    g.cfg.botPlayer = true;
    let guard = TICKS_PER_S * 60 * 30;
    while (!g.over && guard-- > 0) tickGame(g, null, engineChoose);
    return g;
  }

  globalThis.FighterCore = {
    TICKS_PER_S, RING_W, MIN_GAP, ROUND_TICKS, INTERLUDE_TICKS, TIE_EPS,
    METER_MAX, MOVES, DASH, ANSWERS, VALUE, MOVE_NAMES, DEFAULT_CONFIG,
    REACTION, normalizeConfig, fighterStamp, createGame, tickGame,
    runSummary, simulateMatch, commit, resolveHit, observe, habitProbs,
    habitTable, resetLearning, decideResponse, situationKey, dist,
    actionable, botInput,
  };
})();
