/* Pong core — pure simulation + paddle policy for the Playground Pong game.
   No DOM, no canvas, no timers: deterministic given (seed, config), and
   unit-testable under `node --test` like maze-core.js (import for side
   effect, read the global; sdk/decisions-sdk.js must load first).

   Design contract (docs/planning/playground/DECISIONS-SDK-PLAN.md §5.1):
   - fixed-timestep physics (1/60 s), wall/paddle reflection with plane-cross
     interpolation — no tunneling, no floating-point surprises run to run;
   - each paddle policy scores CANDIDATE targets (intercept / edge-spin /
     rest-center / mirror-the-opponent) with the user's reward weights;
     movement is speed-capped, so a fast ball beats a perfect tracker —
     misses are physics, not scripting;
   - a genuine tie between the top two candidates is the engine's job
     (score request); with the engine off the same weighted formula decides,
     and the caller records which rung answered;
   - the point of the game is watchable incentive design: two bots under
     different reward weights play differently, visibly. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;

  const FIELD_W = 100;
  const FIELD_H = 60;
  const PADDLE_X = 5;                 /* paddle center plane, left and mirrored */
  const BALL_R = 1;
  const DT = 1 / 60;
  const MAX_SPEED = 140;
  const WALL_MARGIN = 2;              /* risk term: comfort distance from walls */

  /* reward tags — the user-editable incentive surface */
  const TAGS = ["pursue", "bounce", "center", "taunt", "risk"];
  const TAG_FOR = { pursue: "intercept", bounce: "edge", center: "center", taunt: "mirror" };
  const DEFAULT_REWARDS = {
    version: 1,
    weights: { pursue: 3, bounce: 1, center: 1, taunt: 0, risk: 1 },
  };

  function normalizeRewards(raw) {
    const src = raw && typeof raw === "object" ? raw : {};
    const w = src.weights && typeof src.weights === "object" ? src.weights : src;
    const out = {};
    /* clampf, not clamp: the sliders step in halves (2.5 stays 2.5) — and
       snap to a tenth so hostile floats can't smuggle precision into stamps */
    for (const t of TAGS) {
      out[t] = Math.round(DK.clampf(w[t], 0, 5, DEFAULT_REWARDS.weights[t]) * 10) / 10;
    }
    return { version: 1, weights: out };
  }

  function normalizeConfig(raw) {
    const c = raw || {};
    return {
      seed: String(c.seed || "oc-pong"),
      ballSpeed: DK.clamp(c.ballSpeed, 20, 80, 40),
      accel: DK.clampf(c.accel, 1.0, 1.2, 1.04),
      paddleH: DK.clamp(c.paddleH, 5, 18, 10),
      paddleSpeed: DK.clamp(c.paddleSpeed, 18, 60, 34),
      winScore: DK.clamp(c.winScore, 1, 11, 3),
      reactEvery: DK.clamp(c.reactEvery, 1, 20, 6),
      noise: DK.clampf(c.noise, 0, 0.5, 0.12),
      maxTicks: DK.clamp(c.maxTicks, 600, 60000, 10800), /* 180 s sim cap */
    };
  }

  /* stamp — verdicts only compare like-for-like: geometry, speeds, and BOTH
     weight vectors are in; cosmetics are not */
  function matchStamp(cfg, leftW, rightW) {
    const c = normalizeConfig(cfg);
    const l = normalizeRewards(leftW).weights;
    const r = normalizeRewards(rightW).weights;
    return [c.seed, c.ballSpeed, c.accel, c.paddleH, c.paddleSpeed, c.winScore,
      c.reactEvery, c.noise,
      TAGS.map((t) => l[t]).join(","), TAGS.map((t) => r[t]).join(","),
    ].join("|");
  }

  /* ---------- ball physics ---------- */

  function reflectWalls(b) {
    if (b.y < BALL_R) { b.y = 2 * BALL_R - b.y; b.vy = Math.abs(b.vy); }
    else if (b.y > FIELD_H - BALL_R) { b.y = 2 * (FIELD_H - BALL_R) - b.y; b.vy = -Math.abs(b.vy); }
  }

  /* advance the ball one tick; returns the paddle hit this tick or null.
     Plane-cross interpolation: y AT the crossing x decides the hit, so a
     fast ball cannot tunnel through a paddle. */
  function stepBall(b, paddles) {
    const px0 = b.x;
    const py0 = b.y;
    b.x += b.vx * DT;
    b.y += b.vy * DT;
    reflectWalls(b);
    for (const side of ["left", "right"]) {
      const p = paddles[side];
      const plane = side === "left" ? PADDLE_X + 1 : FIELD_W - PADDLE_X - 1;
      const crossed = side === "left"
        ? (px0 > plane && b.x <= plane)
        : (px0 < plane && b.x >= plane);
      if (!crossed) continue;
      /* y when the ball reached the plane (lerp between this tick's endpoints) */
      const t = Math.min(1, Math.max(0,
        Math.abs(plane - px0) / Math.max(1e-9, Math.abs(b.x - px0))));
      const yAt = py0 + (b.y - py0) * t;
      const top = p.y - p.h / 2 - BALL_R;
      const bot = p.y + p.h / 2 + BALL_R;
      if (yAt >= top && yAt <= bot) {
        const off = (yAt - p.y) / (p.h / 2);        /* -1..1 across the face */
        const dir = side === "left" ? 1 : -1;
        const sp = Math.min(MAX_SPEED, Math.abs(b.vx) * p.accel);
        b.vx = dir * sp;
        /* spin: edge hits steepen the return — the "bounce" reward's lever */
        b.vy = b.vy * 0.35 + off * sp * 0.55;
        b.x = plane + dir * 0.01;
        return side;
      }
    }
    return null;
  }

  /* project where the ball crosses this paddle's plane (same reflection
     rules, cheap forward simulation — the bot's whole world model) */
  function predictIntercept(b, side) {
    const sim = { x: b.x, y: b.y, vx: b.vx, vy: b.vy };
    const plane = side === "left" ? PADDLE_X + 1 : FIELD_W - PADDLE_X - 1;
    for (let i = 0; i < 400; i++) {
      sim.x += sim.vx * DT;
      sim.y += sim.vy * DT;
      reflectWalls(sim);
      if ((side === "left" && sim.x <= plane) || (side === "right" && sim.x >= plane)) {
        return sim.y;
      }
    }
    return sim.y;
  }

  /* ---------- paddle policy: weighted candidates, argmax, genuine ties ---------- */

  /* candidates for one decision tick. base = salience before weights;
     risk term penalizes wall-hugging targets; movement cost penalizes
     far targets so the paddle doesn't dither. */
  function policyCandidates(state, side) {
    const p = state.paddles[side];
    const opp = state.paddles[side === "left" ? "right" : "left"];
    const intercept = predictIntercept(state.ball, side);
    /* edge target: clip the ball with the face's far edge to steepen the
       return toward the opponent's back corner */
    const toward = opp.y < intercept ? 1 : -1;
    const edge = Math.max(BALL_R, Math.min(FIELD_H - BALL_R,
      intercept + toward * (p.h / 2) * 0.8));
    const sinceOwn = state.ticks - p.lastOwnHit;
    const restBias = Math.max(0, 1 - sinceOwn / 180);   /* fades over 3 s */
    const c = [
      { name: "intercept", y: intercept, base: 1.0, tag: "pursue" },
      { name: "edge", y: edge, base: 0.85, tag: "bounce" },
      { name: "center", y: FIELD_H / 2, base: 0.5 * restBias, tag: "center" },
      { name: "mirror", y: opp.y, base: 0.4, tag: "taunt" },
    ];
    return c.map((cand) => {
      const wallProx = Math.max(0, 1 - Math.min(cand.y, FIELD_H - cand.y) / (WALL_MARGIN + 6));
      const move = Math.abs(p.y - cand.y) / FIELD_H;
      return {
        name: cand.name, y: cand.y, tag: cand.tag,
        score: cand.base - 0.6 * wallProx - 0.15 * move,
      };
    });
  }

  /* the local formula: score = w[tag] * base − w.risk * wallProx − move.
     Returns the argmax candidate, or { tie: [a, b] } when the top two are
     indistinguishable — that tie is the engine's to break. */
  function pickTarget(cands, weights) {
    const w = normalizeRewards(weights).weights;
    const scored = cands.map((c) => ({
      name: c.name, y: c.y,
      score: (w[c.tag] || 0) * c.score - w.risk * 0.4 *
        Math.max(0, 1 - Math.min(c.y, FIELD_H - c.y) / (WALL_MARGIN + 6)),
    }));
    scored.sort((a, b) => b.score - a.score || a.name.localeCompare(b.name));
    if (scored.length >= 2 && Math.abs(scored[0].score - scored[1].score) < 1e-9) {
      return { tie: [scored[0], scored[1]], best: scored[0] };
    }
    return { best: scored[0], tie: null };
  }

  /* ---------- match simulation ---------- */

  function makePaddle(side, cfg) {
    return {
      side, y: FIELD_H / 2, h: cfg.paddleH,
      speed: cfg.paddleSpeed, accel: cfg.accel,
      target: FIELD_H / 2, lastOwnHit: -9999, rallyBias: 0,
      score: 0, hits: 0, misses: 0,
    };
  }

  function createMatch(cfg, leftRewards, rightRewards) {
    const c = normalizeConfig(cfg);
    const rng = DK.makeRng(c.seed + ":pong");
    return {
      cfg: c, rng,
      rewards: { left: normalizeRewards(leftRewards), right: normalizeRewards(rightRewards) },
      paddles: { left: makePaddle("left", c), right: makePaddle("right", c) },
      ball: null, ticks: 0, rallies: [], rallyHits: 0,
      serveDir: rng() < 0.5 ? 1 : -1,
      over: false, winner: null, engineCalls: 0,
    };
  }

  function serve(m) {
    const ang = (m.rng() * 2 - 1) * 0.5;               /* ±0.5 rad */
    const sp = m.cfg.ballSpeed;
    m.ball = {
      x: FIELD_W / 2, y: FIELD_H / 2,
      vx: Math.cos(ang) * sp * m.serveDir,
      vy: Math.sin(ang) * sp,
    };
    m.rallyHits = 0;
    /* each rally both paddles carry one systematic aim bias — the human
       failure mode: a consistent misread of this particular serve, not
       random noise. Noise 0 = perfect play, and matches get long. */
    for (const side of ["left", "right"]) {
      m.paddles[side].rallyBias = (m.rng() * 2 - 1) * m.cfg.noise * FIELD_H * 1.2;
    }
  }

  /* move paddle toward its target at capped speed — the miss mechanism */
  function movePaddle(p) {
    const dy = p.target - p.y;
    const step = p.speed * DT;
    if (Math.abs(dy) <= step) p.y = p.target;
    else p.y += Math.sign(dy) * step;
    p.y = Math.max(p.h / 2, Math.min(FIELD_H - p.h / 2, p.y));
  }

  function scorePoint(m, scorerSide) {
    m.paddles[scorerSide].score += 1;
    const loser = scorerSide === "left" ? "right" : "left";
    m.paddles[loser].misses += 1;
    m.rallies.push(m.rallyHits);
    if (m.paddles[scorerSide].score >= m.cfg.winScore) {
      m.over = true;
      m.winner = scorerSide;
    } else {
      m.serveDir = scorerSide === "left" ? 1 : -1;
      serve(m);
    }
  }

  /* one bot decision: refresh the target from the weighted policy. React
     cadence + seeded noise keep perfect prediction from being perfect
     play; the engine (when wired by the shell) breaks genuine ties. */
  function botThink(m, side, engineChoose) {
    const p = m.paddles[side];
    if (m.ticks % m.cfg.reactEvery !== 0) return;
    const cands = policyCandidates(m, side);
    const w = m.rewards[side].weights;
    const jitter = (m.rng() * 2 - 1) * m.cfg.noise * FIELD_H;
    let pick = pickTarget(cands, w);
    if (pick.tie && engineChoose) {
      /* genuine tie → a real choice question over the tied candidates, the
         same wire shape the maze tiebreak speaks (engine "score" questions
         are ordered-severity scales, not candidate rankings). The engine is
         allowed to abstain; then the local formula keeps the tie. */
      const res = engineChoose({
        state: {
          text: "Pong tie: two paddle targets score exactly equal under the " +
            "user's reward weights. Pick one.",
          facts: {},
        },
        questions: [{
          type: "choice", id: "paddle-target",
          text: "Which target should the " + side + " paddle take?",
          candidates: pick.tie.map((c) => ({
            id: c.name, description: "target y " + Math.round(c.y) +
              " — tied at score " + c.score.toFixed(3),
          })),
        }],
        policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" },
      });
      if (res) {
        m.engineCalls += 1;
        const ans = res && res.answers && res.answers[0];
        const topId = ans && ans.choice;
        const chosen = pick.tie.find((c) => c.name === topId);
        if (chosen) pick = { best: chosen, tie: null };
      }
    }
    p.target = Math.max(p.h / 2,
      Math.min(FIELD_H - p.h / 2, pick.best.y + p.rallyBias + jitter));
  }

  function humanThink(m, side, wantY) {
    if (wantY === null || wantY === undefined) return;
    m.paddles[side].target = Math.max(0, Math.min(FIELD_H, wantY));
  }

  function tickMatch(m, input) {
    if (m.over) return m;
    /* decisions first: both sides pick targets from the current picture */
    for (const side of ["left", "right"]) {
      const humanY = input && input[side];
      if (humanY === null || humanY === undefined) botThink(m, side, input && input.engineChoose);
      else humanThink(m, side, humanY);
    }
    for (const side of ["left", "right"]) movePaddle(m.paddles[side]);
    const hit = stepBall(m.ball, m.paddles);
    m.ticks += 1;
    if (hit) {
      m.rallyHits += 1;
      const p = m.paddles[hit];
      p.hits += 1;
      p.lastOwnHit = m.ticks;
    } else if (m.ball.x < 0) scorePoint(m, "right");
    else if (m.ball.x > FIELD_W) scorePoint(m, "left");
    else if (m.ticks >= m.cfg.maxTicks) {
      /* safety net, never expected: longest-rally leader takes it */
      m.over = true;
      m.winner = m.paddles.left.hits >= m.paddles.right.hits ? "left" : "right";
    }
    return m;
  }

  /* full bot-vs-bot match in one call — the headless path the tests and the
     page's "simulate match" use; identical to driving tickMatch by hand */
  function simulateMatch(cfg, leftRewards, rightRewards, engineChoose) {
    const m = createMatch(cfg, leftRewards, rightRewards);
    serve(m);
    while (!m.over) tickMatch(m, { engineChoose });
    return m;
  }

  function matchSummary(m) {
    return {
      seed: m.cfg.seed,
      winner: m.winner,
      left: m.paddles.left.score,
      right: m.paddles.right.score,
      rallies: m.rallies.length,
      longestRally: m.rallies.length ? Math.max.apply(null, m.rallies) : 0,
      avgRally: m.rallies.length
        ? m.rallies.reduce((a, b) => a + b, 0) / m.rallies.length : 0,
      ticks: m.ticks,
      engineCalls: m.engineCalls,
      leftHits: m.paddles.left.hits,
      rightHits: m.paddles.right.hits,
    };
  }

  /* ---------- exports ---------- */
  globalThis.PongCore = {
    FIELD_W, FIELD_H, PADDLE_X, DT, MAX_SPEED,
    TAGS, DEFAULT_REWARDS, normalizeRewards, normalizeConfig, matchStamp,
    reflectWalls, stepBall, predictIntercept,
    policyCandidates, pickTarget,
    createMatch, serve, tickMatch, simulateMatch, matchSummary,
  };
})();
