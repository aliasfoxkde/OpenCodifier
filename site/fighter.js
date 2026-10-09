/* Sparring Partner shell — the side-view fighter on the Decisions SDK.
   fighter-core.js owns the deterministic sim; this file is chrome: the
   procedural boxers, health/meter/round chrome, the habit readout (which
   rung answered zed), the learning reset (auditability), the session-only
   board, the stats overlay, and the engine bridge. Zed keeps a decayed
   histogram of YOUR moves per situation; near-tie EVs go to the engine,
   which reasons over your recorded habits. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const FC = globalThis.FighterCore;
  const root = document.getElementById("fighter-root");
  if (!root || !DK || !FC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;
  const DT = 1 / FC.TICKS_PER_S;

  const W = 960, H = 420;
  const FLOOR = 330;          /* canvas y of the ring floor */
  const RUNG_NAMES = { habit: "habit table", engine: "engine", rule: "ring rule" };

  /* canvas stays dark in both site themes, like the other games */
  const PAL = {
    bg0: "#100d18", bg1: "#1c1226", floor: "#2a2138", rope: "#8b7fa8",
    ink: "#e8e4f2", you: "#fde047", zed: "#c2413c", meter: "#4de3ff",
    hp: "#3ddc84", hpLow: "#ff5a4d",
  };

  /* ---------- state ---------- */
  const S = {
    cfg: { seed: "oc-spar" },
    game: null,
    running: false,   /* boots paused — the play button starts the bot */
    last: 0, acc: 0,
    keys: { left: false, right: false, block: false, duck: false },
    queue: {},                /* edge-triggered attacks, consumed per tick */
    boardOpen: false, statsOpen: false,
    board: DK.sessionBoard(),
    meters: DK.meters(),
    fps: 0,
  };

  /* no importBase: the SDK's default "../wasm/" is right for a root page */
  const engine = DK.engineBridge({
    onStatus: () => setChip(),
  });

  function engineChoose(request) {
    const t0 = nowMs();
    const res = engine.choose(request);
    S.meters.sample("engine.ms", nowMs() - t0);
    return res;
  }

  /* ---------- helpers ---------- */
  function cfgFromUI() {
    return {
      seed: S.seedKit ? S.seedKit.get() : "oc-spar",
      adaptivity: +byId("fighter-adapt").value,
      aggression: +byId("fighter-aggr").value,
      difficulty: +byId("fighter-diff").value,
      rounds: +byId("fighter-rounds").value,
      botPlayer: S.cfg.botPlayer !== false,   /* default: the engine fights */
    };
  }

  function newGame() {
    S.cfg = cfgFromUI();
    S.game = FC.createGame(S.cfg);
    S.keys = { left: false, right: false, block: false, duck: false };
    S.queue = {};
    render();
  }

  /* retire mid-match and the board keeps the honest result */
  function recordDnf() {
    const g = S.game;
    if (!g || g.over || g.ticks < 60) return;
    S.board.record({
      ts: Date.now(), outcome: "dnf",
      rounds: g.you.wins + "-" + g.zed.wins, round: g.round,
      damage: g.you.dealt, engineCalls: g.engineCalls, seed: g.cfg.seed,
      stamp: FC.fighterStamp(S.cfg),
    });
  }

  function setChip() {
    const chip = byId("fighter-engine-chip");
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
    input.addEventListener("input", () => {
      out.textContent = input.value + (unit || "");
    });
    input.addEventListener("change", () => newGame());
    return h("label", { class: "game-slider" },
      h("span", { class: "game-slider-label" }, label), input, out);
  }

  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) S.cfg.seed = deepSeed;

    const bar = h("div", { class: "game-bar" },
      h("button", { class: "mz-chip mz-play", id: "fighter-play", type: "button" }, "▶ play"),
      h("button", { class: "mz-chip", id: "fighter-restart", type: "button" }, "↺ restart"),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "fighter-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "fighter-seed", class: "mz-input", value: S.cfg.seed, size: "10" }),
        h("button", { class: "mz-chip", id: "fighter-dice", type: "button", title: "new random seed" }, "🎲")),
      h("button", { class: "mz-chip", id: "fighter-bot", type: "button" }, "driver: bot"),
      h("button", { class: "mz-chip", id: "fighter-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "fighter-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "fighter-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "fighter-stats-btn", type: "button" }, "ⓘ stats"),
      h("button", { class: "mz-chip", id: "fighter-reset-params", type: "button" }, "reset params"),
      h("button", { class: "mz-chip", id: "fighter-fs", type: "button" }, "⛶ full screen"),
    );

    const canvas = h("canvas", { id: "fighter-canvas", width: String(W), height: String(H),
      role: "img", "aria-label": "Sparring Partner — a fighter whose opponent learns your habits out loud" });
    const under = h("div", { class: "mz-under game-under", id: "fighter-under" });
    const trace = h("div", { class: "game-hintline", id: "fighter-trace",
      "aria-live": "polite" }, "no decisions yet");

    const opts = DK.shell.accordion([
      {
        id: "sparring", label: "The sparring", open: true,
        kids: [
          slider("fighter-adapt", "adaptivity (forget speed)", 0.5, 0.95, 0.05, 0.75),
          slider("fighter-aggr", "zed's aggression", 0, 4, 1, 1),
          slider("fighter-diff", "difficulty (reaction)", 1, 5, 1, 2),
          slider("fighter-rounds", "rounds", 1, 5, 2, 3),
        ],
      },
      {
        id: "learning", label: "Learning",
        kids: [
          h("button", { class: "mz-chip", id: "fighter-reset-learning", type: "button" },
            "learning: reset"),
          h("p", { class: "game-note", id: "fighter-learning-note" },
            "Zed keeps a decayed histogram of your committed moves per situation ",
            "(range × your stance). Reset it and zed is a stranger again. The stats ",
            "overlay shows the whole table — nothing leaves this tab.")],
      },
      {
        id: "about", label: "What decides what?",
        kids: [h("p", { class: "game-note" },
          "You decide the fighter (← → or A D walk, J jab, K hook, L block, S duck, ",
          "Q/E dash, O special when the meter is full). Zed reads your decayed habit ",
          "table for the current situation, predicts your next move, and scores every ",
          "answer against it. When two answers score within 0.3 points, the ENGINE ",
          "arbitrates with the habit table in the question — it reasons over your ",
          "recorded habits, not ours. Ring control at long range is a rule, not a ",
          "choice, and says so.")],
      },
    ]);

    const main = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, canvas, under, trace),
      h("div", { class: "game-side" }, opts));
    root.appendChild(h("div", { class: "game-wrap" }, bar, main));

    buildOverlays();
    wire();
    S.seedKit = DK.shell.numericSeed(byId("fighter-seed"), byId("fighter-dice"), newGame);
    DK.shell.resetParams(byId("fighter-reset-params"), root);
    DK.shell.fullscreen(root.querySelector(".game-wrap"), byId("fighter-fs"));
    engine.setEnabled(true);   /* default on: near-ties reach the runtime */
    newGame();
    if (!window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      requestAnimationFrame(loop);
    } else {
      render(); /* reduced motion: static frame, tick on demand */
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
    root.appendChild(overlay("fighter-board-overlay", "Session scoreboard", "fighter-board-body", "fighter-board-close"));
    root.appendChild(overlay("fighter-stats-overlay", "Measured stats", "fighter-stats-body", "fighter-stats-close"));
  }

  function renderBoard() {
    const body = byId("fighter-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" },
        "No finished matches yet — win or lose a majority and the bout lands ",
        "here (retiring with ↺ restart records a dnf). Session only."));
      return;
    }
    body.appendChild(h("table", { class: "game-table" },
      h("thead", null, h("tr", null,
        ...["#", "outcome", "rounds", "round", "damage", "engine calls", "seed"].map((x) => h("th", null, x)))),
      h("tbody", null, rows.map((r, i) => h("tr", null,
        h("td", null, String(rows.length - i)),
        h("td", null, r.outcome),
        h("td", null, r.rounds),
        h("td", null, String(r.round)),
        h("td", null, String(r.damage)),
        h("td", null, String(r.engineCalls)),
        h("td", null, r.seed))))));
    body.appendChild(h("button", {
      class: "mz-chip", type: "button", onclick: () => { S.board.clear(); renderBoard(); },
    }, "clear board"));
  }

  function renderStats() {
    const body = byId("fighter-stats-body");
    if (!body) return;
    body.textContent = "";
    const snap = S.meters.snapshot();
    const g = S.game;
    const rows = [
      ["frames/s (EMA)", S.fps.toFixed(0)],
      ["sim ticks", String(g ? g.ticks : 0)],
      ["moves observed", String(g ? g.habits.observations : 0)],
      ["decisions — habit table", String(g ? g.rungs.habit : 0)],
      ["decisions — engine", String(g ? g.rungs.engine : 0)],
      ["approaches — ring rule", String(g ? g.rungs.rule : 0)],
      ["engine calls (incl. near-ties)", String(g ? g.engineCalls : 0)],
      ["engine status", engine.status],
    ];
    if (snap["engine.ms"]) {
      rows.push(["engine latency — avg", DK.fmtMs(snap["engine.ms"].avg)]);
      rows.push(["engine latency — worst", DK.fmtMs(snap["engine.ms"].worst)]);
      rows.push(["engine latency — last", DK.fmtMs(snap["engine.ms"].last)]);
    }
    body.appendChild(h("div", { class: "mz-stats-grid" }, rows.map(([k, v]) =>
      h("div", { class: "mz-stats-row" },
        h("span", { class: "mz-stats-label" }, k),
        h("span", { class: "mz-stats-value" }, v)))));
    /* the habit table itself — the learning, exposed */
    body.appendChild(h("p", { class: "game-note" }, "Zed's habit table (decayed counts):"));
    const table = g ? FC.habitTable(g) : [];
    if (!table.length) {
      body.appendChild(h("p", { class: "game-note" },
        "Empty — commit moves and zed starts filling this in."));
    } else {
      body.appendChild(h("table", { class: "game-table" },
        h("thead", null, h("tr", null,
          ...["situation", "your move", "decayed count"].map((x) => h("th", null, x)))),
        h("tbody", null, table.slice(0, 40).map((r) => h("tr", null,
          h("td", null, r.sit),
          h("td", null, r.move),
          h("td", null, r.count.toFixed(2)))))));
      if (table.length > 40) {
        body.appendChild(h("p", { class: "game-note" },
          "…and " + (table.length - 40) + " more rows."));
      }
    }
  }

  /* ---------- loop ---------- */
  function loop(t) {
    if (!S.running) { requestAnimationFrame(loop); return; }   /* paused: no work */
    if (!S.last) S.last = t;
    const dt = Math.min(0.1, (t - S.last) / 1000);
    S.last = t;
    S.fps = S.fps ? S.fps * 0.9 + (1 / Math.max(dt, 1e-4)) * 0.1 : 1 / Math.max(dt, 1e-4);
    S.acc += dt;
    let dirty = false;
    while (S.acc >= DT) {
      const t0 = nowMs();
      stepOnce();
      S.meters.sample("tick.ms", nowMs() - t0);
      S.acc -= DT;
      dirty = true;
      if (S.game.over) break;
    }
    if (dirty) render();
    if (S.statsOpen && (!S.meters.get("stats.draw") || t - S.meters.get("stats.draw").last > 250)) {
      S.meters.sample("stats.draw", 0);
      renderStats();
    }
    requestAnimationFrame(loop);
  }

  function humanInput() {
    const q = S.queue;
    S.queue = {};
    return {
      walk: (S.keys.right ? 1 : 0) - (S.keys.left ? 1 : 0),
      jab: !!q.jab, hook: !!q.hook, special: !!q.special,
      dashIn: !!q.dashIn, dashOut: !!q.dashOut,
      block: S.keys.block, duck: S.keys.duck,
    };
  }

  function stepOnce() {
    const g = S.game;
    if (!g || g.over) {
      if (g && g.over && !g.recorded) {
        g.recorded = true;
        S.board.record({
          ts: Date.now(), outcome: g.result,
          rounds: g.you.wins + "-" + g.zed.wins, round: g.round,
          damage: g.you.dealt, engineCalls: g.engineCalls, seed: g.cfg.seed,
          stamp: FC.fighterStamp(S.cfg),
        });
        if (S.boardOpen) renderBoard();
      }
      return;
    }
    const input = g.cfg.botPlayer ? {} : humanInput();
    FC.tickGame(g, input, engine.status === "ready" ? engineChoose : null);
  }

  /* ---------- render ---------- */

  /* a procedural boxer: legs, torso, head, two arms; the lead arm does the
     move — pulled back on startup, extended on active, retracting after */
  function drawBoxer(ctx, f, flash) {
    const cx = 60 + (f.x / FC.RING_W) * (W - 120);
    const dir = f.facing;
    const ducking = f.state === "duck";
    const bodyH = ducking ? 62 : 96;
    const gy = FLOOR;
    const hipY = gy - 34;
    const shoulderY = hipY - bodyH;
    const torsoW = 34;

    ctx.save();
    if (flash) {
      ctx.shadowColor = "rgba(255,255,255,0.9)";
      ctx.shadowBlur = 14;
    }

    /* legs */
    ctx.strokeStyle = PAL.ink;
    ctx.lineWidth = 7;
    ctx.lineCap = "round";
    ctx.beginPath();
    ctx.moveTo(cx - 8, hipY); ctx.lineTo(cx - 14, gy);
    ctx.moveTo(cx + 8, hipY); ctx.lineTo(cx + 16, gy);
    ctx.stroke();

    /* torso */
    ctx.fillStyle = f.color;
    ctx.beginPath();
    ctx.roundRect(cx - torsoW / 2, shoulderY, torsoW, bodyH, 10);
    ctx.fill();

    /* head + guard */
    ctx.fillStyle = PAL.ink;
    ctx.beginPath();
    ctx.arc(cx + dir * 5, shoulderY - 13, 12, 0, Math.PI * 2);
    ctx.fill();

    /* arms — the lead arm acts, the rear one guards */
    const m = f.move ? (FC.MOVES[f.move] || FC.DASH) : null;
    let ext = 0;    /* 0 guard, 1 full extension */
    if (f.state === "startup") ext = -0.25 * Math.min(1, f.frame / Math.max(1, m.startup));
    else if (f.state === "active") ext = 1;
    else if (f.state === "recovery") ext = Math.max(0, 1 - f.frame / m.recovery);
    const reach = ext >= 0 ? 44 * ext : -14 * ext;
    const armY = shoulderY + (ducking ? 18 : 8);
    ctx.strokeStyle = f.color;
    ctx.lineWidth = 8;
    ctx.beginPath();
    ctx.moveTo(cx + dir * 10, armY);
    ctx.lineTo(cx + dir * (26 + reach), armY - (ext > 0 ? 4 : 10));
    ctx.stroke();
    /* glove */
    ctx.fillStyle = "#f8fafc";
    ctx.beginPath();
    ctx.arc(cx + dir * (26 + reach), armY - (ext > 0 ? 4 : 10), 8, 0, Math.PI * 2);
    ctx.fill();
    ctx.beginPath();
    ctx.moveTo(cx - dir * 8, armY + 8);
    ctx.lineTo(cx - dir * 18, armY + 14);
    ctx.stroke();

    /* block: a raised guard */
    if (f.state === "block") {
      ctx.strokeStyle = "rgba(77,227,255,0.9)";
      ctx.lineWidth = 5;
      ctx.beginPath();
      ctx.arc(cx + dir * 22, shoulderY + 14, 26, -Math.PI * 0.6, Math.PI * 0.4);
      ctx.stroke();
    }
    /* special: a charge glow while the meter burns */
    if (f.move === "special" && (f.state === "startup" || f.state === "active")) {
      ctx.fillStyle = "rgba(255,184,77,0.25)";
      ctx.beginPath();
      ctx.arc(cx, shoulderY + 20, 46, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.restore();
  }

  function drawBars(ctx, g) {
    const hw = 340;
    /* health — you left, zed right, draining toward the middle */
    ctx.fillStyle = "rgba(232,228,242,0.18)";
    ctx.fillRect(20, 18, hw, 16);
    ctx.fillRect(W - 20 - hw, 18, hw, 16);
    const youFrac = g.you.health / 100;
    const zedFrac = g.zed.health / 100;
    ctx.fillStyle = youFrac > 0.3 ? PAL.hp : PAL.hpLow;
    ctx.fillRect(20, 18, hw * youFrac, 16);
    ctx.fillStyle = zedFrac > 0.3 ? PAL.hp : PAL.hpLow;
    ctx.fillRect(W - 20 - hw * zedFrac, 18, hw * zedFrac, 16);
    /* meter under health */
    ctx.fillStyle = "rgba(232,228,242,0.12)";
    ctx.fillRect(20, 38, hw, 7);
    ctx.fillRect(W - 20 - hw, 38, hw, 7);
    ctx.fillStyle = PAL.meter;
    ctx.fillRect(20, 38, hw * (g.you.meter / FC.METER_MAX), 7);
    ctx.fillRect(W - 20 - hw * (g.zed.meter / FC.METER_MAX), 38, hw * (g.zed.meter / FC.METER_MAX), 7);
    /* round pips */
    const need = Math.ceil(g.cfg.rounds / 2);
    for (let i = 0; i < need; i++) {
      ctx.beginPath();
      ctx.arc(24 + i * 18, 56, 5, 0, Math.PI * 2);
      ctx.fillStyle = i < g.you.wins ? PAL.you : "rgba(232,228,242,0.25)";
      ctx.fill();
      ctx.beginPath();
      ctx.arc(W - 24 - i * 18, 56, 5, 0, Math.PI * 2);
      ctx.fillStyle = i < g.zed.wins ? PAL.zed : "rgba(232,228,242,0.25)";
      ctx.fill();
    }
    /* timer */
    const secs = Math.max(0, Math.ceil((FC.ROUND_TICKS - g.roundTicks) / FC.TICKS_PER_S));
    ctx.fillStyle = PAL.ink;
    ctx.font = "700 26px ui-monospace, monospace";
    ctx.textAlign = "center";
    ctx.fillText(String(secs), W / 2, 42);
    ctx.font = "600 12px ui-monospace, monospace";
    ctx.fillText("ROUND " + g.round, W / 2, 60);
  }

  function render() {
    const canvas = byId("fighter-canvas");
    if (!canvas || !S.game) return;
    const g = S.game;
    const ctx = canvas.getContext("2d");

    /* gym backdrop + ring floor */
    const sky = ctx.createLinearGradient(0, 0, 0, H);
    sky.addColorStop(0, PAL.bg0);
    sky.addColorStop(1, PAL.bg1);
    ctx.fillStyle = sky;
    ctx.fillRect(0, 0, W, H);
    ctx.fillStyle = PAL.floor;
    ctx.fillRect(0, FLOOR, W, H - FLOOR);
    ctx.strokeStyle = PAL.rope;
    ctx.lineWidth = 3;
    ctx.beginPath();
    ctx.moveTo(20, FLOOR); ctx.lineTo(W - 20, FLOOR);
    ctx.stroke();
    ctx.lineWidth = 2;
    ctx.globalAlpha = 0.5;
    ctx.beginPath();
    ctx.moveTo(30, FLOOR - 90); ctx.lineTo(W - 30, FLOOR - 90);
    ctx.moveTo(30, FLOOR - 140); ctx.lineTo(W - 30, FLOOR - 140);
    ctx.stroke();
    ctx.globalAlpha = 1;

    drawBars(ctx, g);
    drawBoxer(ctx, g.you, g.flash && g.flash.side === "you");
    drawBoxer(ctx, g.zed, g.flash && g.flash.side === "zed");

    /* interlude / match text */
    ctx.textAlign = "center";
    if (g.over) {
      ctx.fillStyle = PAL.ink;
      ctx.font = "800 44px ui-monospace, monospace";
      ctx.fillText(g.result === "won" ? "YOU TAKE THE BOUT" : "ZED TAKES THE BOUT", W / 2, H / 2 - 40);
      ctx.font = "600 16px ui-monospace, monospace";
      ctx.fillText(g.you.wins + " — " + g.zed.wins + "  ·  restart (R) for another", W / 2, H / 2 - 8);
    } else if (g.roundResult) {
      ctx.fillStyle = PAL.ink;
      ctx.font = "800 40px ui-monospace, monospace";
      const who = g.roundResult.winner === "you" ? "ROUND — YOU"
        : g.roundResult.winner === "zed" ? "ROUND — ZED" : "ROUND — DRAW";
      ctx.fillText(who, W / 2, H / 2 - 40);
      ctx.font = "600 16px ui-monospace, monospace";
      ctx.fillText(g.roundResult.how === "ko" ? "knockdown" : "time", W / 2, H / 2 - 8);
    }

    /* under-row + trace */
    const rungName = (r) => RUNG_NAMES[r] || r;
    const ld = g.lastDecision;
    byId("fighter-under").innerHTML = "";
    byId("fighter-under").appendChild(DK.shell.kvRow([
      ["state", g.over ? (g.result === "won" ? "bout won" : "bout lost")
        : g.roundResult ? "between rounds" : (S.running ? "fighting" : "paused")],
      ["round", g.round + "  (" + g.you.wins + "-" + g.zed.wins + ")"],
      ["your hp", Math.round(g.you.health)],
      ["zed hp", Math.round(g.zed.health)],
      ["your meter", Math.round(g.you.meter)],
      ["zed's read", ld ? ld.predicted : "—"],
      ["zed's answer", ld ? ld.pick + " · " + rungName(ld.rung) : "—"],
      ["engine near-ties", String(g.rungs.engine)],
    ]));
    if (ld) {
      byId("fighter-trace").textContent = "last decision: " + ld.who + " @ " + ld.sit +
        " (d " + ld.at.d + ") — read " + ld.predicted + " → " + ld.pick + " · " + rungName(ld.rung);
    }
  }

  /* ---------- wiring ---------- */
  function wire() {
    byId("fighter-play").addEventListener("click", (e) => {
      S.running = !S.running;
      e.currentTarget.textContent = S.running ? "⏸ pause" : "▶ play";
    });
    byId("fighter-restart").addEventListener("click", () => {
      recordDnf();
      newGame();
    });
    byId("fighter-seed").addEventListener("change", () => newGame());
    byId("fighter-bot").addEventListener("click", (e) => {
      S.cfg.botPlayer = !(S.cfg.botPlayer === true);
      e.currentTarget.textContent = S.cfg.botPlayer ? "driver: bot" : "driver: you";
      if (S.game) S.game.cfg.botPlayer = S.cfg.botPlayer;
    });
    byId("fighter-engine-toggle").addEventListener("click", (e) => {
      const on = engine.status === "off" || engine.status === "unavailable";
      engine.setEnabled(on);
      e.currentTarget.textContent = on ? "engine: on" : "engine: off";
    });
    byId("fighter-reset-learning").addEventListener("click", () => {
      if (S.game) FC.resetLearning(S.game);
    });
    byId("fighter-board-btn").addEventListener("click", () => {
      S.boardOpen = true; S.statsOpen = false;
      byId("fighter-stats-overlay").classList.remove("open");
      byId("fighter-board-overlay").classList.add("open");
      renderBoard();
    });
    byId("fighter-board-close").addEventListener("click", () => {
      S.boardOpen = false;
      byId("fighter-board-overlay").classList.remove("open");
    });
    byId("fighter-stats-btn").addEventListener("click", () => {
      S.statsOpen = true; S.boardOpen = false;
      byId("fighter-board-overlay").classList.remove("open");
      byId("fighter-stats-overlay").classList.add("open");
      renderStats();
    });
    byId("fighter-stats-close").addEventListener("click", () => {
      S.statsOpen = false;
      byId("fighter-stats-overlay").classList.remove("open");
    });

    /* keyboard: ← → / A D walk, J jab, K hook, L block, S or ↓ duck,
       Q/E dash, O special, space pauses, R restarts */
    const HOLD = {
      ArrowLeft: "left", a: "left", A: "left",
      ArrowRight: "right", d: "right", D: "right",
      l: "block", L: "block",
      s: "duck", S: "duck", ArrowDown: "duck",
    };
    const TAP = { j: "jab", J: "jab", k: "hook", K: "hook", o: "special", O: "special",
      e: "dashIn", E: "dashIn", q: "dashOut", Q: "dashOut" };
    window.addEventListener("keydown", (e) => {
      if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
      const hold = HOLD[e.key];
      const tap = TAP[e.key];
      if (hold) { S.keys[hold] = true; e.preventDefault(); }
      else if (tap) { if (!e.repeat) S.queue[tap] = true; e.preventDefault(); }
      else if (e.key === " ") {
        S.running = !S.running;
        byId("fighter-play").textContent = S.running ? "⏸ pause" : "▶ play";
        e.preventDefault();
      } else if (e.key === "r" || e.key === "R") {
        recordDnf();
        newGame();
      } else if (e.key === "Escape") {
        S.boardOpen = false; S.statsOpen = false;
        byId("fighter-board-overlay").classList.remove("open");
        byId("fighter-stats-overlay").classList.remove("open");
      }
    });
    window.addEventListener("keyup", (e) => {
      const hold = HOLD[e.key];
      if (hold) S.keys[hold] = false;
    });

    setChip();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", build);
  } else {
    build();
  }
})();
