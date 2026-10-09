/* The Prisoner's Dilemma — shell.
   dilemma-core.js owns the deterministic match; this file is chrome:
   the duel view (two agent panels, last-round glyphs, the move ribbon),
   the cooperation-over-time chart, the editable payoff matrix, the
   headless tournament overlay (round-robin table + series chart), the
   session scoreboard, the stats overlay, and the engine bridge. The
   engine AGENT asks the runtime under its hard per-match token budget;
   a refusal (or a spent budget) falls to the mood rule ON THE RECORD —
   the rung trace says which. Rendering never touches any rng stream. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const DC = globalThis.DilemmaCore;
  const root = document.getElementById("dilemma-root");
  if (!root || !DK || !DC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;
  const W = 960, H = 600;
  const ROUND_MS = 100;             /* 10 rounds/s at 1× */
  const CHECKPOINT_EVERY = 50;      /* rounds per board row */
  const RUNG_NAMES = { rule: "rule", engine: "engine" };
  const MOVE_GLYPH = { cooperate: "C", defect: "D" };
  const PAL = {
    bg0: "#070b12", bg1: "#0d1320", grid: "#1a2233",
    ink: "#e8eef8", dim: "#8b9ab8", ok: "#3ddc84",
    coop: "#3ddc84", defect: "#ff5a4d",
    left: "#4de3ff", right: "#c084fc",
    warn: "#fbbf24", danger: "#ff5a4d", engine: "#ffb347",
  };

  /* ---------- state ---------- */
  const S = {
    cfg: { seed: "pd-axelrod" },
    game: null,
    running: false,   /* boots paused — the play button starts the bot */
    last: 0, acc: 0, speed: 1,
    boardOpen: false, statsOpen: false, tourOpen: false,
    pushed: { n: 0, outcome: false }, logSeen: -1,
    askMs: 0,
    tour: null,
    board: DK.sessionBoard(),
    meters: DK.meters(),
    fps: 0,
  };

  const engine = DK.engineBridge({
    onStatus: () => setChip(),
  });

  function engineChoose(request) {
    const t0 = nowMs();
    const res = engine.choose(request);
    S.askMs = nowMs() - t0;
    S.meters.sample("engine.ms", S.askMs);
    return res;
  }

  /* ---------- helpers ---------- */
  function cfgFromUI() {
    return {
      seed: S.seedKit ? S.seedKit.get() : "pd-axelrod",
      rounds: +byId("pd-rounds").value,
      noise: +byId("pd-noise").value,
      askBudget: +byId("pd-budget").value,
      left: byId("pd-left").value,
      right: byId("pd-right").value,
      matrix: {
        cc: +byId("pd-mx-cc").value, cd: +byId("pd-mx-cd").value,
        dc: +byId("pd-mx-dc").value, dd: +byId("pd-mx-dd").value,
      },
    };
  }

  function newGame() {
    S.cfg = cfgFromUI();
    S.game = DC.createGame(S.cfg);
    S.running = false;   /* restarts land paused, same as boot */
    S.pushed = { n: 0, outcome: false };
    S.logSeen = -1;
    byId("pd-play").textContent = "▶ play";
    render();
  }

  function pushBoard(g, outcome) {
    const s = DC.runSummary(g);
    S.board.record({
      ts: Date.now(), outcome: outcome || "checkpoint",
      at: s.round, left: s.left, right: s.right,
      sl: s.scores.left, sr: s.scores.right,
      coopL: s.coopRate.left, coopR: s.coopRate.right,
      decisions: s.decisions, rungs: s.rungs,
      engineCalls: s.engineCalls, flips: s.flips,
      seed: g.cfg.seed, stamp: DC.dilemmaStamp(g.cfg),
    });
  }

  function setChip() {
    const chip = byId("pd-engine-chip");
    if (!chip) return;
    const st = engine.status;
    chip.textContent = "engine: " + st;
    chip.classList.toggle("on", st === "ready");
    chip.classList.toggle("off", st === "unavailable" || st === "off");
  }

  /* ---------- build ---------- */
  function slider(id, label, min, max, step, val, unit) {
    const out = h("span", { class: "game-sliderval", id: id + "-val" }, String(val) + (unit || ""));
    const input = h("input", {
      type: "range", id, min: String(min), max: String(max), step: String(step), value: String(val),
      "aria-label": label,
    });
    input.addEventListener("input", () => { out.textContent = input.value + (unit || ""); });
    input.addEventListener("change", () => newGame());
    return h("label", { class: "game-slider" },
      h("span", { class: "game-slider-label" }, label), input, out);
  }

  function stratSelect(id, label, val) {
    const sel = h("select", { id, class: "mz-input", "aria-label": label });
    for (const s of DC.ROSTER) {
      const o = h("option", { value: s }, DC.STRATEGY_NAME[s]);
      if (s === val) o.selected = true;
      sel.appendChild(o);
    }
    sel.addEventListener("change", () => newGame());
    return h("label", { class: "game-slider" },
      h("span", { class: "game-slider-label" }, label), sel);
  }

  function matrixInput(cell) {
    const input = h("input", {
      type: "number", id: "pd-mx-" + cell, class: "mz-input",
      min: "-9", max: "9", step: "1", value: String(DC.DEFAULT_MATRIX[cell]),
      "aria-label": DC.MATRIX_LABEL[cell], style: "width:4.5em",
    });
    input.addEventListener("change", () => newGame());
    return h("label", { class: "game-slider" },
      h("span", { class: "game-slider-label" }, DC.MATRIX_LABEL[cell]), input);
  }

  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) S.cfg.seed = deepSeed;

    const bar = h("div", { class: "game-bar" },
      h("button", { class: "mz-chip mz-play", id: "pd-play", type: "button" }, "▶ play"),
      h("button", { class: "mz-chip", id: "pd-restart", type: "button" }, "↺ restart"),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "pd-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "pd-seed", class: "mz-input", value: S.cfg.seed, size: "10" }),
        h("button", { class: "mz-chip", id: "pd-dice", type: "button", title: "new random seed" }, "🎲")),
      h("button", { class: "mz-chip", id: "pd-speed", type: "button" }, "speed: 1×"),
      h("button", { class: "mz-chip", id: "pd-tour-btn", type: "button" }, "▶ tournament"),
      h("button", { class: "mz-chip", id: "pd-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "pd-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "pd-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "pd-stats-btn", type: "button" }, "ⓘ stats"),
      h("button", { class: "mz-chip", id: "pd-reset-params", type: "button" }, "reset params"),
      h("button", { class: "mz-chip", id: "pd-fs", type: "button" }, "⛶ full screen"),
    );

    const canvas = h("canvas", { id: "pd-canvas", width: String(W), height: String(H),
      role: "img",
      "aria-label": "The Prisoner's Dilemma — two agents play cooperate or defect each round; the engine agent asks the runtime under a hard token budget and its refusals fall to the mood rule on the record" });
    const panel = h("div", { class: "game-hintline", id: "pd-panel" },
      "each round both agents move at once — the trace below says who decided");
    const under = h("div", { class: "mz-under game-under", id: "pd-under" });
    const trace = h("div", { class: "game-hintline", id: "pd-trace",
      "aria-live": "polite" }, "no rounds yet");
    const log = h("div", { class: "game-hintline", id: "pd-log",
      "aria-live": "polite" }, "the match log is empty");

    const opts = DK.shell.accordion([
      {
        id: "match", label: "The match", open: true,
        kids: [
          stratSelect("pd-left", "left agent", "engine"),
          stratSelect("pd-right", "right agent", "titfortat"),
          slider("pd-rounds", "rounds", 10, 500, 10, 200),
          slider("pd-noise", "transmission noise", 0, 0.5, 0.05, 0),
          slider("pd-budget", "engine budget", 0, 10, 1, 4, " asks"),
          h("p", { class: "game-note" }, "The payoff matrix (payoffs are mine, per round):"),
          h("div", { class: "game-bar" },
            matrixInput("cc"), matrixInput("cd"), matrixInput("dc"), matrixInput("dd")),
          h("p", { class: "game-note" },
            "Changing anything restarts the match from round 0 with the ",
            "same seed — runs are deterministic. Noise flips a move IN ",
            "TRANSMISSION: the opponent sees the flip, the scorer pays the ",
            "true move, and every flip is logged."),
        ],
      },
      {
        id: "tournament", label: "Run the tournament",
        kids: [
          h("p", { class: "game-note" },
            "The ▶ tournament button plays every roster pair head-to-head ",
            "under the current matrix, rounds and noise — seeded per pair, ",
            "so the table replays identically. The engine joins with a ",
            "fresh budget per pairing. Watch the cooperation series: some ",
            "worlds reward defection, and the table will say so."),
        ],
      },
      {
        id: "about", label: "What decides what?",
        kids: [h("p", { class: "game-note" },
          "Seven roster strategies decide from what they have SEEN — which ",
          "noise may have garbled. The ENGINE AGENT is a player, not a ",
          "referee: each round it sends the interaction history over the ",
          "wire and plays the honored answer; when the answer names ",
          "nothing live, or the engine abstains, or the per-match token ",
          "budget is spent, the MOOD RULE holds (cooperate if the ",
          "opponent's last 8 seen moves are at least half cooperation) ",
          "and the rung says 'rule'. There is no captain in this game — ",
          "the engine agent IS the player, and refusal is a normal, ",
          "recorded outcome, never an error. Rungs count BOTH sides of ",
          "every round.")]
      },
    ]);

    const main = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, canvas, panel, under, trace, log),
      h("div", { class: "game-side" }, opts));
    const wrap = h("div", { class: "game-wrap" }, bar, main);
    root.appendChild(wrap);

    buildOverlays();
    wire();
    S.seedKit = DK.shell.numericSeed(byId("pd-seed"), byId("pd-dice"), newGame);
    DK.shell.resetParams(byId("pd-reset-params"), root, newGame);
    DK.shell.fullscreen(wrap, byId("pd-fs"));
    engine.setEnabled(true);   /* default on: the engine agent plays */
    newGame();
    if (!window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      requestAnimationFrame(loop);
    } else {
      render(); /* reduced motion: static frame */
    }
  }

  /* ---------- overlays ---------- */
  function overlay(id, title, bodyId, closeId) {
    return h("dialog", { class: "mz-overlay", id },
      h("div", { class: "mz-board-card" },
        h("div", { class: "mz-overlay-head" },
          h("span", { class: "mz-overlay-title" }, title),
          h("button", { class: "mz-chip", id: closeId, type: "button" }, "✕ close")),
        h("div", { class: "mz-overlay-scroll", id: bodyId })));
  }

  function buildOverlays() {
    root.appendChild(overlay("pd-board-overlay", "Session scoreboard", "pd-board-body", "pd-board-close"));
    root.appendChild(overlay("pd-stats-overlay", "Measured stats", "pd-stats-body", "pd-stats-close"));
    root.appendChild(overlay("pd-tour-overlay", "Round-robin tournament", "pd-tour-body", "pd-tour-close"));
  }

  function renderBoard() {
    const body = byId("pd-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" },
        "No rows yet — the scoreboard records a checkpoint every 50 ",
        "rounds and the match result when a match ends. Play one out. ",
        "Session only."));
      return;
    }
    body.appendChild(h("table", { class: "game-table" },
      h("thead", null, h("tr", null,
        ...["#", "round", "row", "left", "right", "L/R score", "coop L/R", "rule/engine", "asks", "seed"].map((x) => h("th", null, x)))),
      h("tbody", null, rows.map((r, i) => h("tr", null,
        h("td", null, String(i + 1)),
        h("td", null, String(r.at)),
        h("td", null, r.outcome),
        h("td", null, r.left),
        h("td", null, r.right),
        h("td", null, r.sl + "/" + r.sr),
        h("td", null, r.coopL + "/" + r.coopR),
        h("td", null, r.rungs.rule + "/" + r.rungs.engine),
        h("td", null, String(r.engineCalls)),
        h("td", null, r.seed))))));
    body.appendChild(h("button", {
      class: "mz-chip", type: "button", onclick: () => { S.board.clear(); renderBoard(); },
    }, "clear scoreboard"));
  }

  function renderStats() {
    const body = byId("pd-stats-body");
    if (!body) return;
    body.textContent = "";
    const snap = S.meters.snapshot();
    const g = S.game;
    const s = g ? DC.runSummary(g) : null;
    const rows = [
      ["frames/s (EMA)", S.fps.toFixed(0)],
      ["round", s ? s.round + " of " + s.rounds : "0"],
      ["matchup", s ? DC.STRATEGY_NAME[s.left] + " vs " + DC.STRATEGY_NAME[s.right] : "—"],
      ["scores L / R", s ? s.scores.left + " / " + s.scores.right : "—"],
      ["coop rate L / R", s ? s.coopRate.left + " / " + s.coopRate.right : "—"],
      ["noise flips", s ? String(s.flips) : "0"],
      ["decisions — total", s ? String(s.decisions) : "0"],
      ["decisions — rule rung (both sides)", s ? String(s.rungs.rule) : "0"],
      ["decisions — engine rung", s ? String(s.rungs.engine) : "0"],
      ["engine calls", s ? String(s.engineCalls) : "0"],
      ["ask budget", g ? g.cfg.askBudget + "/match (tokens left " + g.tokens + ")" : "—"],
      ["matrix R/S/T/P", g ? [g.cfg.matrix.cc, g.cfg.matrix.cd, g.cfg.matrix.dc,
        g.cfg.matrix.dd].join(" / ") : "—"],
      ["history samples", s ? String(s.history) : "0"],
      ["log entries", s ? String(s.logEntries) : "0"],
      ["last tournament", S.tour
        ? S.tour.roster.length + " strategies · " + S.tour.table.length + " pairings · "
          + S.tour.engineCalls + " engine asks" : "not run yet"],
      ["engine status", engine.status],
    ];
    if (snap["engine.ms"]) {
      rows.push(["engine latency — avg", DK.fmtMs(snap["engine.ms"].avg)]);
      rows.push(["engine latency — worst", DK.fmtMs(snap["engine.ms"].worst)]);
      rows.push(["engine latency — last", DK.fmtMs(snap["engine.ms"].last)]);
    }
    if (snap["tick.ms"]) {
      rows.push(["round cost — avg", DK.fmtMs(snap["tick.ms"].avg)]);
      rows.push(["round cost — worst", DK.fmtMs(snap["tick.ms"].worst)]);
    }
    body.appendChild(h("div", { class: "mz-stats-grid" }, rows.map(([k, v]) =>
      h("div", { class: "mz-stats-row" },
        h("span", { class: "mz-stats-label" }, k),
        h("span", { class: "mz-stats-value" }, v)))));
  }

  function renderTour() {
    const body = byId("pd-tour-body");
    if (!body || !S.tour) return;
    body.textContent = "";
    const t = S.tour;
    body.appendChild(h("p", { class: "game-note" },
      t.roster.length + " strategies, every pair under the current matrix — "
      + t.interactions + " rounds of interaction, " + t.decisions
      + " decisions (" + t.engineCalls + " engine, the rest rules), "
      + t.flips + " noise flips. Deterministic: the same seed replays "
      + "this table exactly."));
    const ranked = new Map();
    for (const s of t.roster) ranked.set(s, 0);
    for (const r of t.table) {
      ranked.set(r.a, ranked.get(r.a) + r.sa);
      ranked.set(r.b, ranked.get(r.b) + r.sb);
    }
    const standings = t.roster.slice().sort((x, y) => ranked.get(y) - ranked.get(x));
    body.appendChild(h("div", { class: "game-bar", style: "flex-wrap:wrap" },
      standings.map((s, i) => h("span", { class: "mz-chip game-rung" },
        (i + 1) + ". " + DC.STRATEGY_NAME[s] + " " + ranked.get(s)))));
    body.appendChild(h("table", { class: "game-table" },
      h("thead", null, h("tr", null,
        ...["left", "right", "left score", "right score", "coop L", "coop R"].map((x) => h("th", null, x)))),
      h("tbody", null, t.table.map((r) => h("tr", null,
        h("td", null, DC.STRATEGY_NAME[r.a]),
        h("td", null, DC.STRATEGY_NAME[r.b]),
        h("td", null, String(r.sa)),
        h("td", null, String(r.sb)),
        h("td", null, String(r.coopA)),
        h("td", null, String(r.coopB)))))));
    const cv = h("canvas", { id: "pd-tour-canvas", width: "640", height: "180",
      role: "img",
      "aria-label": "Cooperation rate over rounds, aggregated across every pairing" });
    body.appendChild(cv);
    drawSeries(cv.getContext("2d"), t.series, t.roster.length ? t.roster : null, 640, 180);
  }

  function drawSeries(ctx, series, _roster, w, hh) {
    ctx.fillStyle = PAL.bg0;
    ctx.fillRect(0, 0, w, hh);
    const x0 = 40, y0 = 16, pw = w - x0 - 16, ph = hh - y0 - 28;
    ctx.strokeStyle = PAL.grid;
    ctx.lineWidth = 1;
    for (const f of [0, 0.25, 0.5, 0.75, 1]) {
      const y = y0 + ph * (1 - f);
      ctx.beginPath(); ctx.moveTo(x0, y); ctx.lineTo(x0 + pw, y); ctx.stroke();
      ctx.fillStyle = PAL.dim;
      ctx.font = "600 10px ui-monospace, monospace"; ctx.textAlign = "right";
      ctx.fillText(f.toFixed(2), x0 - 6, y + 3);
    }
    if (!series.length) return;
    const maxR = series[series.length - 1].r;
    const px = (r) => x0 + pw * (r / maxR);
    const py = (c) => y0 + ph * (1 - c);
    ctx.strokeStyle = PAL.ok;
    ctx.lineWidth = 2;
    ctx.beginPath();
    for (let i = 0; i < series.length; i++) {
      const x = px(series[i].r), y = py(series[i].coop);
      if (i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
    }
    ctx.stroke();
    ctx.fillStyle = PAL.dim;
    ctx.textAlign = "center";
    ctx.fillText("cooperation rate over all pairings — round " + maxR, x0 + pw / 2, hh - 8);
  }

  /* ---------- loop ---------- */
  function loop(t) {
    if (!S.running) { requestAnimationFrame(loop); return; }   /* paused: no work */
    if (!S.last) S.last = t;
    const dt = Math.min(0.1, (t - S.last) / 1000);
    S.last = t;
    S.fps = S.fps ? S.fps * 0.9 + (1 / Math.max(dt, 1e-4)) * 0.1 : 1 / Math.max(dt, 1e-4);
    S.acc += dt * S.speed;
    let dirty = false;
    const DT = ROUND_MS / 1000;
    const budget = 8 * Math.max(1, S.speed);      /* rounds per frame cap */
    for (let n = 0; S.acc >= DT && n < budget && S.game && !S.game.over; n++) {
      const t0 = nowMs();
      stepOnce();
      S.meters.sample("tick.ms", nowMs() - t0);
      S.acc -= DT;
      dirty = true;
    }
    if (S.game && S.game.over && !S.pushed.outcome) {
      S.pushed.outcome = true;
      pushBoard(S.game, "match-end");
      if (S.boardOpen) renderBoard();
    }
    if (dirty) render();
    if (S.statsOpen && (!S.meters.get("stats.draw") || t - S.meters.get("stats.draw").last > 250)) {
      S.meters.sample("stats.draw", 0);
      renderStats();
    }
    requestAnimationFrame(loop);
  }

  function stepOnce() {
    const g = S.game;
    if (!g) return;
    DC.tickGame(g, engine.status === "ready" ? engineChoose : null);
    while (g.round >= (S.pushed.n + 1) * CHECKPOINT_EVERY && !g.over) {
      S.pushed.n += 1;
      pushBoard(g, null);
      if (S.boardOpen) renderBoard();
    }
  }

  /* ---------- render ---------- */
  function drawAgent(ctx, g, side, x, w) {
    const a = g.agents[side];
    const color = side === "left" ? PAL.left : PAL.right;
    const isEngine = a.id === "engine";
    ctx.textAlign = side === "left" ? "left" : "right";
    const tx = side === "left" ? x : x + w;
    ctx.fillStyle = color;
    ctx.font = "700 15px ui-monospace, monospace";
    ctx.fillText(DC.STRATEGY_NAME[a.id], tx, 34);
    if (isEngine) {
      ctx.fillStyle = PAL.engine;
      ctx.font = "600 10px ui-monospace, monospace";
      ctx.fillText("ENGINE AGENT · tokens " + g.tokens, tx, 50);
    } else {
      ctx.fillStyle = PAL.dim;
      ctx.font = "600 10px ui-monospace, monospace";
      ctx.fillText("roster", tx, 50);
    }
    ctx.fillStyle = PAL.ink;
    ctx.font = "700 30px ui-monospace, monospace";
    ctx.fillText(String(g.scores[side]), tx, 84);
    const coop = (() => {
      const t = g.tally[side];
      const tot = t.cc + t.cd + t.dc + t.dd;
      return tot ? (t.cc + t.cd) / tot : 0;
    })();
    /* cooperation meter under the score */
    const my = 94, mw = 130;
    const mx = side === "left" ? tx : tx - mw;
    ctx.fillStyle = PAL.bg1;
    ctx.fillRect(mx, my, mw, 8);
    ctx.fillStyle = PAL.coop;
    ctx.fillRect(mx, my, mw * coop, 8);
    ctx.strokeStyle = PAL.grid;
    ctx.strokeRect(mx, my, mw, 8);
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 10px ui-monospace, monospace";
    ctx.fillText("coop " + Math.round(coop * 100) + "%", mx + (side === "left" ? mw + 8 : -38), my + 8);
  }

  function drawLastRound(ctx, g) {
    const m = g.moves[g.moves.length - 1];
    const cx = W / 2;
    ctx.textAlign = "center";
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 11px ui-monospace, monospace";
    ctx.fillText(g.over ? "match over" : "round " + g.round + " of " + g.cfg.rounds, cx, 34);
    if (!m) return;
    ctx.font = "700 44px ui-monospace, monospace";
    ctx.fillStyle = m.a === "cooperate" ? PAL.coop : PAL.defect;
    ctx.fillText(MOVE_GLYPH[m.a], cx - 70, 84);
    ctx.fillStyle = PAL.dim;
    ctx.font = "700 16px ui-monospace, monospace";
    ctx.fillText("vs", cx, 80);
    ctx.font = "700 44px ui-monospace, monospace";
    ctx.fillStyle = m.b === "cooperate" ? PAL.coop : PAL.defect;
    ctx.fillText(MOVE_GLYPH[m.b], cx + 70, 84);
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 11px ui-monospace, monospace";
    ctx.fillText("payoff +" + m.pa + " / +" + m.pb
      + (m.fa || m.fb ? " · ⚡ noise" : ""), cx, 102);
  }

  function drawRibbon(ctx, g) {
    const y = 132, rh = 16, gap = 3;
    const x0 = 40, pw = W - 80;
    const N = 60;
    const tail = g.moves.slice(-N);
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 10px ui-monospace, monospace";
    ctx.textAlign = "left";
    ctx.fillText("last " + N + " rounds", x0, y - 6);
    ctx.textAlign = "right";
    ctx.fillText("left / right", x0 + pw, y - 6);
    const cw = Math.max(3, pw / N - gap);
    for (let i = 0; i < tail.length; i++) {
      const m = tail[i];
      const x = x0 + i * (pw / N);
      ctx.fillStyle = m.a === "cooperate" ? PAL.coop : PAL.defect;
      ctx.fillRect(x, y, cw, rh);
      ctx.fillStyle = m.b === "cooperate" ? PAL.coop : PAL.defect;
      ctx.fillRect(x, y + rh + gap, cw, rh);
      if (m.fa) {               /* left's move garbled in transmission */
        ctx.fillStyle = PAL.warn;
        ctx.fillRect(x, y - 3, cw, 2);
      }
      if (m.fb) {
        ctx.fillStyle = PAL.warn;
        ctx.fillRect(x, y + 2 * rh + gap + 1, cw, 2);
      }
    }
  }

  function drawChart(ctx, g) {
    const x0 = 40, y0 = 210, pw = W - 80, ph = H - y0 - 46;
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 10px ui-monospace, monospace";
    ctx.textAlign = "left";
    ctx.fillText("cooperation over time", x0, y0 - 8);
    ctx.strokeStyle = PAL.grid;
    ctx.lineWidth = 1;
    for (const f of [0, 0.5, 1]) {
      const y = y0 + ph * (1 - f);
      ctx.beginPath(); ctx.moveTo(x0, y); ctx.lineTo(x0 + pw, y); ctx.stroke();
      ctx.fillStyle = PAL.dim;
      ctx.textAlign = "right";
      ctx.fillText(f.toFixed(1), x0 - 6, y + 3);
    }
    const h = g.history;
    if (h.length < 2) return;
    const px = (r) => x0 + pw * (r / g.cfg.rounds);
    const py = (c) => y0 + ph * (1 - Math.max(0, Math.min(1, c)));
    const line = (key, color) => {
      ctx.strokeStyle = color;
      ctx.lineWidth = 2;
      ctx.beginPath();
      for (let i = 0; i < h.length; i++) {
        const x = px(h[i].r), y = py(h[i][key]);
        if (i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
      }
      ctx.stroke();
    };
    line("coopL", PAL.left);
    line("coopR", PAL.right);
    ctx.textAlign = "left";
    ctx.fillStyle = PAL.left;
    ctx.fillText("■ left", x0, y0 + ph + 18);
    ctx.fillStyle = PAL.right;
    ctx.fillText("■ right", x0 + 60, y0 + ph + 18);
    ctx.fillStyle = PAL.warn;
    ctx.fillText("■ = garbled move in the ribbon", x0 + 130, y0 + ph + 18);
  }

  function render() {
    const canvas = byId("pd-canvas");
    if (!canvas || !S.game) return;
    const g = S.game;
    const ctx = canvas.getContext("2d");

    const sky = ctx.createLinearGradient(0, 0, 0, H);
    sky.addColorStop(0, PAL.bg0);
    sky.addColorStop(1, PAL.bg1);
    ctx.fillStyle = sky;
    ctx.fillRect(0, 0, W, H);

    drawAgent(ctx, g, "left", 40, 260);
    drawAgent(ctx, g, "right", W - 300, 260);
    drawLastRound(ctx, g);
    drawRibbon(ctx, g);
    drawChart(ctx, g);

    if (g.over) {
      const w = g.scores.left === g.scores.right ? "a draw"
        : (g.scores.left > g.scores.right ? DC.STRATEGY_NAME[g.cfg.left]
          : DC.STRATEGY_NAME[g.cfg.right]) + " wins";
      ctx.fillStyle = PAL.ink;
      ctx.textAlign = "center";
      ctx.font = "700 16px ui-monospace, monospace";
      ctx.fillText("match over — " + w + " (" + g.scores.left + " to "
        + g.scores.right + ")", W / 2, H - 8);
    }

    /* under-row + trace + log */
    const s = DC.runSummary(g);
    const under = byId("pd-under");
    under.innerHTML = "";
    under.appendChild(DK.shell.kvRow([
      ["round", s.round + "/" + s.rounds],
      ["left", s.left],
      ["right", s.right],
      ["score L", String(s.scores.left)],
      ["score R", String(s.scores.right)],
      ["coop L", String(s.coopRate.left)],
      ["coop R", String(s.coopRate.right)],
      ["noise flips", String(s.flips)],
      ["asks", String(s.engineCalls)],
      ["tokens left", String(s.tokens)],
    ]));
    const ld = g.lastDecision;
    if (ld) {
      const at = [ld.why];
      if (ld.rung === "engine" && S.askMs) at.push(DK.fmtMs(S.askMs));
      byId("pd-trace").textContent = "last decision: "
        + (ld.action ? MOVE_GLYPH[ld.action] + " " + ld.action.toUpperCase() : "—")
        + " · " + ld.side + " agent · " + (RUNG_NAMES[ld.rung] || ld.rung)
        + " (" + at.filter(Boolean).join(", ") + ")";
    }
    if (g.log.length !== S.logSeen) {
      S.logSeen = g.log.length;
      const recent = g.log.slice(-5).map((l) => "r" + l.round + " " + l.note);
      byId("pd-log").textContent = recent.length ? "match log: " + recent.join(" · ") : "the match log is empty";
    }
  }

  /* ---------- wiring ---------- */
  function wire() {
    byId("pd-play").addEventListener("click", (e) => {
      S.running = !S.running;
      e.currentTarget.textContent = S.running ? "⏸ pause" : "▶ play";
    });
    byId("pd-restart").addEventListener("click", () => newGame());
    byId("pd-seed").addEventListener("change", () => newGame());
    byId("pd-speed").addEventListener("click", (e) => {
      const steps = [1, 2, 4, 0.5];
      S.speed = steps[(steps.indexOf(S.speed) + 1) % steps.length];
      e.currentTarget.textContent = "speed: " + S.speed + "×";
    });
    byId("pd-engine-toggle").addEventListener("click", (e) => {
      const on = engine.status === "off" || engine.status === "unavailable";
      engine.setEnabled(on);
      e.currentTarget.textContent = on ? "engine: on" : "engine: off";
    });
    byId("pd-tour-btn").addEventListener("click", () => {
      S.tour = DC.tournament({
        seed: S.cfg.seed, rounds: Math.min(100, S.cfg.rounds),
        noise: S.cfg.noise, askBudget: S.cfg.askBudget,
        left: S.cfg.left, right: S.cfg.right, matrix: S.cfg.matrix,
      }, engine.status === "ready" ? engineChoose : null);
      S.tourOpen = true; S.boardOpen = false; S.statsOpen = false;
      byId("pd-board-overlay").classList.remove("open");
      byId("pd-stats-overlay").classList.remove("open");
      byId("pd-tour-overlay").classList.add("open");
      renderTour();
    });
    byId("pd-board-btn").addEventListener("click", () => {
      S.boardOpen = true; S.statsOpen = false; S.tourOpen = false;
      byId("pd-stats-overlay").classList.remove("open");
      byId("pd-tour-overlay").classList.remove("open");
      byId("pd-board-overlay").classList.add("open");
      renderBoard();
    });
    byId("pd-board-close").addEventListener("click", () => {
      S.boardOpen = false;
      byId("pd-board-overlay").classList.remove("open");
    });
    byId("pd-stats-btn").addEventListener("click", () => {
      S.statsOpen = true; S.boardOpen = false; S.tourOpen = false;
      byId("pd-board-overlay").classList.remove("open");
      byId("pd-tour-overlay").classList.remove("open");
      byId("pd-stats-overlay").classList.add("open");
      renderStats();
    });
    byId("pd-stats-close").addEventListener("click", () => {
      S.statsOpen = false;
      byId("pd-stats-overlay").classList.remove("open");
    });
    byId("pd-tour-close").addEventListener("click", () => {
      S.tourOpen = false;
      byId("pd-tour-overlay").classList.remove("open");
    });

    /* keyboard: space pauses, R restarts, 1/2/4 speed, T tournament,
       Esc closes overlays */
    window.addEventListener("keydown", (e) => {
      if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
      if (e.key === " ") {
        S.running = !S.running;
        byId("pd-play").textContent = S.running ? "⏸ pause" : "▶ play";
        e.preventDefault();
      } else if (e.key === "r" || e.key === "R") {
        newGame();
      } else if (e.key === "1" || e.key === "2" || e.key === "4") {
        S.speed = +e.key;
        byId("pd-speed").textContent = "speed: " + S.speed + "×";
      } else if (e.key === "t" || e.key === "T") {
        byId("pd-tour-btn").click();
      } else if (e.key === "Escape") {
        S.boardOpen = false; S.statsOpen = false; S.tourOpen = false;
        byId("pd-board-overlay").classList.remove("open");
        byId("pd-stats-overlay").classList.remove("open");
        byId("pd-tour-overlay").classList.remove("open");
      }
    });

    setChip();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", build);
  } else {
    build();
  }
})();
