/* RPSLS++ — shell.
   rpsls-core.js owns the graph, the predictors, and the EV brain; this
   file is chrome: the arena (your glyph vs the bot's, the verdict),
   the move-graph diagram (vertices on a circle, arrows winner ->
   loser, live-redrawn when a move JOINS mid-match), the move buttons,
   the history strip, the predictor-score bars, the extend-the-graph
   form (validated live, with the refusal reason shown), bot-vs-bot
   watch mode, the session scoreboard, the stats overlay, and the
   engine bridge. Rendering never touches any rng stream. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const RC = globalThis.RpslsCore;
  const root = document.getElementById("rpsls-root");
  if (!root || !DK || !RC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;
  const W = 960, H = 600;
  const ROUND_MS = 330;           /* bot-duel rounds per second at 1x */
  const STRIP_N = 30;             /* history strip width */
  const RUNG_NAMES = { rule: "rule", engine: "engine" };
  const PAL = {
    bg0: "#070b12", bg1: "#0d1320", grid: "#1a2233",
    ink: "#e8eef8", dim: "#8b9ab8",
    win: "#3ddc84", lose: "#ff5a4d", tie: "#8b9ab8",
    you: "#4de3ff", bot: "#c084fc", engine: "#ffb347", edge: "#3a4a66",
  };

  /* ---------- state ---------- */
  const S = {
    cfg: { seed: "rpsls-hunter" },
    game: null,
    running: true,
    last: 0, acc: 0, speed: 1,
    boardOpen: false, statsOpen: false,
    pushed: { n: 0, outcome: false }, logSeen: -1,
    askMs: 0,
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
      seed: S.seedKit ? S.seedKit.get() : "rpsls-hunter",
      band: +byId("rl-band").value,
      askBudget: +byId("rl-budget").value,
      botRounds: +byId("rl-rounds").value,
      mode: S.cfg.mode || "bots",   /* default: a self-playing duel */
    };
  }

  function newGame() {
    S.cfg = cfgFromUI();
    S.game = RC.createGame(S.cfg);
    S.running = false;   /* boots paused — the play button starts the duel */
    S.pushed = { n: 0, outcome: false };
    S.logSeen = -1;
    byId("rl-play").textContent = "▶ play";
    rebuildMoveButtons();
    rebuildBeatBoxes();
    render();
  }

  function pushBoard(g, outcome) {
    const s = RC.runSummary(g);
    S.board.record({
      ts: Date.now(), outcome: outcome || "checkpoint",
      at: s.round, mode: s.mode,
      you: s.scores.left, bot: s.scores.right, ties: s.scores.ties,
      rungs: s.rungs, engineCalls: s.engineCalls,
      moves: s.moves.length,
      seed: g.cfg.seed, stamp: RC.rpslsStamp(g.cfg),
    });
  }

  function setChip() {
    const chip = byId("rl-engine-chip");
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

  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) S.cfg.seed = deepSeed;

    const bar = h("div", { class: "game-bar" },
      h("button", { class: "mz-chip mz-play", id: "rl-play", type: "button" }, "▶ play"),
      h("button", { class: "mz-chip", id: "rl-restart", type: "button" }, "↺ restart"),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "rl-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "rl-seed", class: "mz-input", value: S.cfg.seed, size: "10" }),
        h("button", { class: "mz-chip", id: "rl-dice", type: "button", title: "new random seed" }, "🎲")),
      h("button", { class: "mz-chip", id: "rl-mode", type: "button" }, "mode: bot vs bot"),
      h("button", { class: "mz-chip", id: "rl-speed", type: "button" }, "speed: 1×"),
      h("button", { class: "mz-chip", id: "rl-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "rl-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "rl-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "rl-stats-btn", type: "button" }, "ⓘ stats"),
      h("button", { class: "mz-chip", id: "rl-reset-params", type: "button" }, "reset params"),
      h("button", { class: "mz-chip", id: "rl-fs", type: "button" }, "⛶ full screen"),
    );

    const canvas = h("canvas", { id: "rl-canvas", width: String(W), height: String(H),
      role: "img",
      "aria-label": "RPSLS++ — the extendable move graph on the left, the arena on the right: your move against a pattern-hunting bot whose frequency, markov, mirror and random predictors feed an expected-value brain that escalates close calls to the engine" });
    const moves = h("div", { class: "game-bar", id: "rl-move-buttons",
      role: "group", "aria-label": "Your move" });
    const panel = h("div", { class: "game-hintline", id: "rl-panel" },
      "pick a move — the bot has been reading your history");
    const under = h("div", { class: "mz-under game-under", id: "rl-under" });
    const trace = h("div", { class: "game-hintline", id: "rl-trace",
      "aria-live": "polite" }, "no rounds yet");
    const log = h("div", { class: "game-hintline", id: "rl-log",
      "aria-live": "polite" }, "the match log is empty");

    const opts = DK.shell.accordion([
      {
        id: "brain", label: "The bot's brain", open: true,
        kids: [
          slider("rl-band", "escalation band", 0.05, 1, 0.05, 0.15),
          slider("rl-budget", "engine budget", 0, 30, 1, 6, " asks"),
          slider("rl-rounds", "bot-duel rounds", 10, 500, 10, 60),
          h("p", { class: "game-note" },
            "The bot blends a frequency table with a markov-1 table over ",
            "your history, computes each move's expected value, and plays ",
            "the argmax — a seeded 5% explore keeps it from being ",
            "counter-looped. When the top two EVs land inside the band, ",
            "the close call goes to the engine with your history table on ",
            "the wire. Refusals fall to the same argmax, on the rung. ",
            "Changing anything restarts the match."),
        ],
      },
      {
        id: "extend", label: "Extend the graph",
        kids: [
          h("div", { class: "game-bar" },
            h("input", { type: "text", id: "rl-new-id", class: "mz-input",
              placeholder: "name (one word)", size: "12",
              "aria-label": "New move name" }),
            h("input", { type: "text", id: "rl-new-glyph", class: "mz-input",
              placeholder: "glyph", size: "3", maxlength: "3",
              "aria-label": "New move glyph" })),
          h("div", { class: "game-bar", id: "rl-beat-boxes",
            role: "group", "aria-label": "Moves the newcomer beats" }),
          h("button", { class: "mz-chip", id: "rl-add", type: "button" }, "➕ add the move"),
          h("p", { class: "game-note", id: "rl-add-note" },
            "A fair add from a 5-move roster beats exactly 2 of the ",
            "existing moves and loses to the rest — the graph reorients ",
            "itself and stays resolvable. Even rosters take no add: a ",
            "regular tournament cannot exist on an even count."),
        ],
      },
      {
        id: "about", label: "What decides what?",
        kids: [h("p", { class: "game-note" },
          "Your move is yours. The bot predicts your next move from your ",
          "history (frequency + markov-1, blended half-half once the ",
          "markov table has data), scores every move by expected value ",
          "(+1 win / 0 tie / −1 loss), and plays the argmax with a ",
          "roster-order tie-break. Four predictors are scored against ",
          "what actually happened — the bars under the arena show who is ",
          "earning trust. A top-two EV gap inside the band escalates to ",
          "the engine over the full wire contract (your history table ",
          "rides in state); an abstain, a malformed answer, or a spent ",
          "budget falls back to the same argmax with the rung saying ",
          "'rule'. Bot-vs-bot mode is a pure watch duel: both brains ",
          "are rules, nobody asks.")]
      },
    ]);

    const main = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, canvas, moves, panel, under, trace, log),
      h("div", { class: "game-side" }, opts));
    const wrap = h("div", { class: "game-wrap" }, bar, main);
    root.appendChild(wrap);

    buildOverlays();
    wire();
    S.seedKit = DK.shell.numericSeed(byId("rl-seed"), byId("rl-dice"), newGame);
    DK.shell.resetParams(byId("rl-reset-params"), root, newGame);
    DK.shell.fullscreen(wrap, byId("rl-fs"));
    engine.setEnabled(true);
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
    root.appendChild(overlay("rl-board-overlay", "Session scoreboard", "rl-board-body", "rl-board-close"));
    root.appendChild(overlay("rl-stats-overlay", "Measured stats", "rl-stats-body", "rl-stats-close"));
  }

  function renderBoard() {
    const body = byId("rl-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" },
        "No rows yet — the scoreboard records a checkpoint every 10 ",
        "rounds and the duel result when a bot-vs-bot match ends. Play ",
        "a few rounds. Session only."));
      return;
    }
    body.appendChild(h("table", { class: "game-table" },
      h("thead", null, h("tr", null,
        ...["#", "round", "row", "mode", "you", "bot", "ties", "rule/engine", "asks", "moves", "seed"].map((x) => h("th", null, x)))),
      h("tbody", null, rows.map((r, i) => h("tr", null,
        h("td", null, String(i + 1)),
        h("td", null, String(r.at)),
        h("td", null, r.outcome),
        h("td", null, r.mode),
        h("td", null, String(r.you)),
        h("td", null, String(r.bot)),
        h("td", null, String(r.ties)),
        h("td", null, r.rungs.rule + "/" + r.rungs.engine),
        h("td", null, String(r.engineCalls)),
        h("td", null, String(r.moves)),
        h("td", null, r.seed))))));
    body.appendChild(h("button", {
      class: "mz-chip", type: "button", onclick: () => { S.board.clear(); renderBoard(); },
    }, "clear scoreboard"));
  }

  function renderStats() {
    const body = byId("rl-stats-body");
    if (!body) return;
    body.textContent = "";
    const snap = S.meters.snapshot();
    const g = S.game;
    const s = g ? RC.runSummary(g) : null;
    const meta = g ? (g.meta.right || {}) : {};
    const rows = [
      ["frames/s (EMA)", S.fps.toFixed(0)],
      ["mode", s ? (s.mode === "you" ? "you vs bot" : "bot vs bot (no engine lane)") : "—"],
      ["round", s ? String(s.round) : "0"],
      ["you / bot / ties", s ? s.scores.left + " / " + s.scores.right + " / " + s.scores.ties : "—"],
      ["streak", s && s.streak.side
        ? s.streak.count + " for " + s.streak.side + " (best "
          + s.streak.best[s.streak.side] + ")" : "none"],
      ["moves on the graph", s ? String(s.moves.length) : "5"],
      ["graph verdict", g ? RC.validateGraph(g.moves).reason : "—"],
      ["decisions — rule rung", s ? String(s.rungs.rule) : "0"],
      ["decisions — engine rung", s ? String(s.rungs.engine) : "0"],
      ["engine calls", s ? String(s.engineCalls) : "0"],
      ["ask budget", g ? g.cfg.askBudget + "/match (tokens left " + g.tokens + ")" : "—"],
      ["escalation band", g ? String(g.cfg.band) : "—"],
      ["bot predictor scores", meta && Object.keys(meta).length
        ? RC.PREDICTORS.map((p) => RC.PREDICTOR_NAME[p].toLowerCase()
          + " " + (meta[p] || 0).toFixed(2)).join(" · ") : "warming up"],
      ["usage", s && Object.keys(s.usage).length
        ? Object.keys(s.usage).map((m) => m + " " + s.usage[m]).join(" · ") : "—"],
      ["log entries", s ? String(s.logEntries) : "0"],
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

  /* ---------- move buttons + extend form ---------- */
  function rebuildMoveButtons() {
    const wrap = byId("rl-move-buttons");
    if (!wrap || !S.game) return;
    wrap.textContent = "";
    const youMode = S.game.cfg.mode === "you";
    S.game.moves.forEach((m, i) => {
      const btn = h("button", {
        class: "mz-chip rl-move", type: "button", "data-move": m.id,
        "aria-label": "Play " + m.label + " (key " + ((i + 1) % 10) + ")",
      }, m.glyph + " " + m.label);
      if (!youMode) btn.disabled = true;
      wrap.appendChild(btn);
    });
  }

  function rebuildBeatBoxes() {
    const wrap = byId("rl-beat-boxes");
    if (!wrap || !S.game) return;
    wrap.textContent = "";
    for (const m of S.game.moves) {
      const box = h("input", { type: "checkbox", id: "rl-beat-" + m.id,
        value: m.id, "aria-label": "The newcomer beats " + m.label });
      box.addEventListener("change", updateAddNote);
      wrap.appendChild(h("label", { class: "game-slider rl-beat" }, box,
        h("span", { class: "game-slider-label" }, m.glyph + " " + m.label)));
    }
    updateAddNote();
  }

  function currentSpec() {
    const id = byId("rl-new-id").value.trim().toLowerCase();
    const glyph = byId("rl-new-glyph").value.trim() || "➕";
    const beats = S.game.moves.filter((m) => byId("rl-beat-" + m.id)
      && byId("rl-beat-" + m.id).checked).map((m) => m.id);
    return { id, label: id.charAt(0).toUpperCase() + id.slice(1), glyph, beats };
  }

  function updateAddNote() {
    const note = byId("rl-add-note");
    const btn = byId("rl-add");
    if (!note || !btn || !S.game) return;
    const spec = currentSpec();
    if (!spec.id) {
      note.textContent = "Name the move first, then tick who it beats.";
      btn.disabled = true;
      return;
    }
    const check = RC.checkAdd(S.game.moves, spec);
    note.textContent = check.ok
      ? "✓ " + check.reason + " — adding will reorient those edges live."
      : "✗ " + check.reason;
    btn.disabled = !check.ok;
  }

  /* ---------- loop ---------- */
  function loop(t) {
    if (!S.last) S.last = t;
    const dt = Math.min(0.1, (t - S.last) / 1000);
    S.last = t;
    S.fps = S.fps ? S.fps * 0.9 + (1 / Math.max(dt, 1e-4)) * 0.1 : 1 / Math.max(dt, 1e-4);
    let dirty = false;
    if (S.game && S.game.cfg.mode === "bots" && S.running && !S.game.over) {
      S.acc += dt * S.speed;
      const DT = ROUND_MS / 1000;
      const budget = 8 * Math.max(1, S.speed);
      for (let n = 0; S.acc >= DT && n < budget && !S.game.over; n++) {
        const t0 = nowMs();
        stepOnce();
        S.meters.sample("tick.ms", nowMs() - t0);
        S.acc -= DT;
        dirty = true;
      }
    }
    if (S.game && S.game.over && !S.pushed.outcome) {
      S.pushed.outcome = true;
      pushBoard(S.game, "duel-end");
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
    RC.playRound(g, null, engine.status === "ready" ? engineChoose : null);
    while (g.round >= (S.pushed.n + 1) * 10 && !g.over) {
      S.pushed.n += 1;
      pushBoard(g, null);
      if (S.boardOpen) renderBoard();
    }
  }

  /* ---------- render ---------- */
  function drawGraph(ctx, g) {
    const cx = 230, cy = 210, R = 150;
    const n = g.moves.length;
    const pos = g.moves.map((m, i) => {
      const a = -Math.PI / 2 + (2 * Math.PI * i) / n;
      return { m, x: cx + R * Math.cos(a), y: cy + R * Math.sin(a) };
    });
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 11px ui-monospace, monospace";
    ctx.textAlign = "left";
    ctx.fillText("the move graph — an arrow means 'beats'", 24, 26);
    /* edges first: winner -> loser, curved outward */
    for (let i = 0; i < n; i++) {
      for (const b of g.moves[i].beats) {
        const j = g.moves.findIndex((m) => m.id === b);
        if (j < 0) continue;
        const a = pos[i], bp = pos[j];
        const mx = (a.x + bp.x) / 2, my = (a.y + bp.y) / 2;
        const dx = mx - cx, dy = my - cy;
        const len = Math.max(1, Math.hypot(dx, dy));
        const qx = mx + (dx / len) * 26, qy = my + (dy / len) * 26;
        ctx.strokeStyle = PAL.edge;
        ctx.lineWidth = 1.4;
        ctx.beginPath();
        ctx.moveTo(a.x, a.y);
        ctx.quadraticCurveTo(qx, qy, bp.x, bp.y);
        ctx.stroke();
        /* arrowhead at the loser */
        const ang = Math.atan2(bp.y - qy, bp.x - qx);
        ctx.fillStyle = PAL.edge;
        ctx.beginPath();
        ctx.moveTo(bp.x, bp.y);
        ctx.lineTo(bp.x - 8 * Math.cos(ang - 0.4), bp.y - 8 * Math.sin(ang - 0.4));
        ctx.lineTo(bp.x - 8 * Math.cos(ang + 0.4), bp.y - 8 * Math.sin(ang + 0.4));
        ctx.closePath(); ctx.fill();
      }
    }
    /* vertices */
    for (const p of pos) {
      ctx.fillStyle = PAL.bg1;
      ctx.strokeStyle = p.m.id === (g.lastDecision && g.lastDecision.action)
        ? PAL.engine : PAL.dim;
      ctx.lineWidth = 2;
      ctx.beginPath(); ctx.arc(p.x, p.y, 22, 0, 7); ctx.fill(); ctx.stroke();
      ctx.font = "700 18px ui-monospace, monospace";
      ctx.fillStyle = PAL.ink;
      ctx.textAlign = "center";
      ctx.fillText(p.m.glyph, p.x, p.y + 6);
      ctx.font = "600 9px ui-monospace, monospace";
      ctx.fillStyle = PAL.dim;
      ctx.fillText(p.m.label.toLowerCase(), p.x, p.y + 38);
    }
    /* per-move usage under the graph */
    const total = Object.values(g.usage).reduce((s, x) => s + x, 0);
    ctx.textAlign = "left";
    ctx.font = "600 10px ui-monospace, monospace";
    let x = 24;
    for (const p of pos) {
      const u = g.usage[p.m.id] || 0;
      const pct = total ? Math.round((100 * u) / total) : 0;
      ctx.fillStyle = PAL.dim;
      ctx.fillText(p.m.glyph + " " + pct + "%", x, H - 96);
      x += 62;
    }
  }

  function drawArena(ctx, g) {
    const cx = 700, cy = 170;
    const youMode = g.cfg.mode === "you";
    const L = youMode ? "you" : "left";
    const R = youMode ? "the bot" : "right";
    const n = g.histories.left.length;
    ctx.textAlign = "center";
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 11px ui-monospace, monospace";
    ctx.fillText(g.cfg.mode === "you" ? "you  vs  the bot" : "bot  vs  bot", cx, 26);
    ctx.fillText("round " + g.round, cx, 44);
    if (n) {
      const lm = g.moves.find((m) => m.id === g.histories.left[n - 1]);
      const rm = g.moves.find((m) => m.id === g.histories.right[n - 1]);
      const r = RC.result(g.rules, g.histories.left[n - 1], g.histories.right[n - 1]);
      ctx.font = "700 64px ui-monospace, monospace";
      ctx.fillStyle = PAL.you;
      ctx.fillText(lm ? lm.glyph : "?", cx - 90, cy + 24);
      ctx.fillStyle = PAL.dim;
      ctx.font = "700 18px ui-monospace, monospace";
      ctx.fillText("vs", cx, cy + 8);
      ctx.font = "700 64px ui-monospace, monospace";
      ctx.fillStyle = PAL.bot;
      ctx.fillText(rm ? rm.glyph : "?", cx + 90, cy + 24);
      ctx.font = "700 15px ui-monospace, monospace";
      ctx.fillStyle = r === "win" ? PAL.win : r === "lose" ? PAL.lose : PAL.tie;
      ctx.fillText(r === "win" ? L.toUpperCase() + (youMode ? " WIN" : " WINS")
        + " — " + lm.label.toLowerCase()
        + " beats " + rm.label.toLowerCase()
        : r === "lose" ? R.toUpperCase() + " WINS — " + rm.label.toLowerCase()
          + " beats " + lm.label.toLowerCase()
        : "a tie — " + lm.label.toLowerCase() + " mirrors "
          + rm.label.toLowerCase(), cx, cy + 66);
    }
    /* scoreline + streak */
    ctx.fillStyle = PAL.you;
    ctx.font = "700 22px ui-monospace, monospace";
    ctx.fillText(String(g.scores.left), cx - 120, cy + 108);
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 11px ui-monospace, monospace";
    ctx.fillText(L, cx - 120, cy + 124);
    ctx.fillStyle = PAL.ink;
    ctx.font = "700 22px ui-monospace, monospace";
    ctx.fillText(g.scores.ties + " : " + g.scores.right, cx + 60, cy + 108);
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 11px ui-monospace, monospace";
    ctx.fillText("ties : " + R, cx + 60, cy + 124);
    if (g.streak.side && g.streak.count >= 2) {
      ctx.fillStyle = PAL.engine;
      ctx.fillText("streak: " + g.streak.count + " for "
        + (g.streak.side === "left" ? L : R), cx, cy + 148);
    }
    /* predictor trust bars (the bot's meta scores) */
    const meta = g.meta.right || {};
    let y = cy + 186;
    ctx.textAlign = "left";
    ctx.font = "600 10px ui-monospace, monospace";
    ctx.fillStyle = PAL.dim;
    ctx.fillText((youMode ? "the bot's predictors" : "right's predictors")
      + " — trust earned against the opponent", cx - 220, y - 10);
    for (const p of RC.PREDICTORS) {
      const v = Math.max(-1, Math.min(1, meta[p] || 0));
      ctx.fillStyle = PAL.dim;
      ctx.fillText(RC.PREDICTOR_NAME[p].toLowerCase(), cx - 220, y + 8);
      const bw = 150, bx = cx - 100;
      ctx.strokeStyle = PAL.grid;
      ctx.strokeRect(bx, y - 6, bw, 10);
      ctx.fillStyle = v >= 0 ? PAL.win : PAL.lose;
      ctx.fillRect(bx + bw / 2, y - 6, (bw / 2) * v, 10);
      y += 20;
    }
  }

  function drawStrip(ctx, g) {
    const y = H - 56, x0 = 470, pw = W - x0 - 24;
    const tail = g.histories.left.slice(-STRIP_N);
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 10px ui-monospace, monospace";
    ctx.textAlign = "left";
    const youMode = g.cfg.mode === "you";
    ctx.fillText("last " + STRIP_N + " rounds (green " + (youMode ? "you" : "left")
      + " · red " + (youMode ? "bot" : "right") + " · gray tie)", x0, y - 8);
    const cw = Math.max(3, pw / STRIP_N - 2);
    for (let i = 0; i < tail.length; i++) {
      const r = RC.result(g.rules, tail[i], g.histories.right[g.histories.right.length - tail.length + i]);
      ctx.fillStyle = r === "win" ? PAL.win : r === "lose" ? PAL.lose : PAL.tie;
      ctx.fillRect(x0 + i * (pw / STRIP_N), y, cw, 14);
    }
  }

  function render() {
    const canvas = byId("rl-canvas");
    if (!canvas || !S.game) return;
    const g = S.game;
    const ctx = canvas.getContext("2d");

    const sky = ctx.createLinearGradient(0, 0, 0, H);
    sky.addColorStop(0, PAL.bg0);
    sky.addColorStop(1, PAL.bg1);
    ctx.fillStyle = sky;
    ctx.fillRect(0, 0, W, H);

    drawGraph(ctx, g);
    drawArena(ctx, g);
    drawStrip(ctx, g);

    if (g.cfg.mode === "you") {
      ctx.fillStyle = PAL.dim;
      ctx.textAlign = "center";
      ctx.font = "600 10px ui-monospace, monospace";
      ctx.fillText("your move buttons are below the canvas — keys 1-"
        + Math.min(9, g.moves.length) + " play too", 700, H - 16);
    }
    if (g.over) {
      const w = g.scores.left === g.scores.right ? "a draw"
        : (g.scores.left > g.scores.right ? "left" : "right") + " wins";
      ctx.fillStyle = PAL.ink;
      ctx.textAlign = "center";
      ctx.font = "700 16px ui-monospace, monospace";
      ctx.fillText("duel over — " + w, 700, H - 70);
    }

    /* under-row + trace + log */
    const s = RC.runSummary(g);
    const under = byId("rl-under");
    under.innerHTML = "";
    under.appendChild(DK.shell.kvRow([
      ["mode", s.mode === "you" ? "you vs bot" : "bot duel"],
      ["round", String(s.round)],
      ["you", String(s.scores.left)],
      ["bot", String(s.scores.right)],
      ["ties", String(s.scores.ties)],
      ["moves", String(s.moves.length)],
      ["asks", String(s.engineCalls)],
      ["tokens left", String(s.tokens)],
      ["streak", s.streak.side ? s.streak.count + " " + s.streak.side : "—"],
    ]));
    const ld = g.lastDecision;
    if (ld) {
      const mv = g.moves.find((m) => m.id === ld.action);
      const at = [ld.why];
      if (ld.rung === "engine" && S.askMs) at.push(DK.fmtMs(S.askMs));
      byId("rl-trace").textContent = "last decision: "
        + (mv ? mv.glyph + " " + mv.label.toUpperCase() : "—")
        + " · " + ld.side + " bot · " + (RUNG_NAMES[ld.rung] || ld.rung)
        + " (" + at.filter(Boolean).join(", ") + ")";
    }
    if (g.log.length !== S.logSeen) {
      S.logSeen = g.log.length;
      const recent = g.log.slice(-5).map((l) => "r" + l.round + " " + l.note);
      byId("rl-log").textContent = recent.length ? "match log: " + recent.join(" · ") : "the match log is empty";
    }
  }

  /* ---------- wiring ---------- */
  function wire() {
    byId("rl-play").addEventListener("click", (e) => {
      S.running = !S.running;
      e.currentTarget.textContent = S.running ? "⏸ pause" : "▶ play";
    });
    byId("rl-restart").addEventListener("click", () => newGame());
    byId("rl-seed").addEventListener("change", () => newGame());
    byId("rl-mode").addEventListener("click", (e) => {
      S.cfg.mode = S.cfg.mode === "you" ? "bots" : "you";
      e.currentTarget.textContent = S.cfg.mode === "you"
        ? "mode: you vs bot" : "mode: bot vs bot";
      newGame();
    });
    byId("rl-speed").addEventListener("click", (e) => {
      const steps = [1, 2, 4, 0.5];
      S.speed = steps[(steps.indexOf(S.speed) + 1) % steps.length];
      e.currentTarget.textContent = "speed: " + S.speed + "×";
    });
    byId("rl-engine-toggle").addEventListener("click", (e) => {
      const on = engine.status === "off" || engine.status === "unavailable";
      engine.setEnabled(on);
      e.currentTarget.textContent = on ? "engine: on" : "engine: off";
    });
    /* your move: buttons + digit keys */
    byId("rl-move-buttons").addEventListener("click", (e) => {
      const btn = e.target.closest("button[data-move]");
      const g = S.game;
      if (!btn || !g || g.cfg.mode !== "you" || g.over) return;
      const t0 = nowMs();
      RC.playRound(g, btn.getAttribute("data-move"),
        engine.status === "ready" ? engineChoose : null);
      S.meters.sample("tick.ms", nowMs() - t0);
      while (g.round >= (S.pushed.n + 1) * 10 && !g.over) {
        S.pushed.n += 1;
        pushBoard(g, null);
        if (S.boardOpen) renderBoard();
      }
      render();
    });
    /* the extend form */
    byId("rl-new-id").addEventListener("input", updateAddNote);
    byId("rl-add").addEventListener("click", () => {
      const g = S.game;
      if (!g || g.over) return;
      const check = RC.addMove(g, currentSpec());
      if (check.ok) {
        byId("rl-new-id").value = "";
        byId("rl-new-glyph").value = "";
      }
      rebuildMoveButtons();
      rebuildBeatBoxes();
      render();
    });
    byId("rl-board-btn").addEventListener("click", () => {
      S.boardOpen = true; S.statsOpen = false;
      byId("rl-stats-overlay").classList.remove("open");
      byId("rl-board-overlay").classList.add("open");
      renderBoard();
    });
    byId("rl-board-close").addEventListener("click", () => {
      S.boardOpen = false;
      byId("rl-board-overlay").classList.remove("open");
    });
    byId("rl-stats-btn").addEventListener("click", () => {
      S.statsOpen = true; S.boardOpen = false;
      byId("rl-board-overlay").classList.remove("open");
      byId("rl-stats-overlay").classList.add("open");
      renderStats();
    });
    byId("rl-stats-close").addEventListener("click", () => {
      S.statsOpen = false;
      byId("rl-stats-overlay").classList.remove("open");
    });

    /* keyboard: digits play moves, space pauses duels, R restarts,
       1/2/4 speed via the button only when not a move digit — digits
       are moves here, so speed keys are the button */
    window.addEventListener("keydown", (e) => {
      if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
      const g = S.game;
      if (/^[1-9]$/.test(e.key) && g && g.cfg.mode === "you" && !g.over) {
        const i = +e.key - 1;
        if (i < g.moves.length) {
          const btn = document.querySelector("#rl-move-buttons button[data-move=\""
            + g.moves[i].id + "\"]");
          if (btn) btn.click();
        }
        return;
      }
      if (e.key === " ") {
        S.running = !S.running;
        byId("rl-play").textContent = S.running ? "⏸ pause" : "▶ play";
        e.preventDefault();
      } else if (e.key === "r" || e.key === "R") {
        newGame();
      } else if (e.key === "Escape") {
        S.boardOpen = false; S.statsOpen = false;
        byId("rl-board-overlay").classList.remove("open");
        byId("rl-stats-overlay").classList.remove("open");
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
