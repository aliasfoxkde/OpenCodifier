/* Spacecraft Emergency — shell.
   spacecraft-core.js owns the deterministic sim; this file is chrome:
   the cockpit (gauges, sensor strip, distance track), the escalation
   panel where the captain answers an abstention, fault/event injection,
   the flight log, the session board (auto-recorded every 60 sim-seconds
   and at the outcome), the stats overlay, and the engine bridge. The
   flight computer decides every 2 sim-seconds: rule gates fire first
   (hull critical, fire with a failing crew, thermal limit, out-of-range
   abort); otherwise actions are scored and honest confidence comes from
   the score gap times sensor agreement — below 0.30 the computer
   ABSTAINS and the sim freezes until the captain picks; 0.30–0.55 goes
   to the engine, and an engine abstain escalates too. The render path
   never touches g.rng — sensor readings shown are the snapshots the
   decision ladder already took. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const SC = globalThis.ShipCore;
  const root = document.getElementById("spacecraft-root");
  if (!root || !DK || !SC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;
  const W = 960, H = 600;
  const DT = 1 / SC.TICKS_PER_S;
  const CHECKPOINT_EVERY = 60;      /* sim-seconds per board row */
  const RUNG_NAMES = { rule: "rule", engine: "engine", player: "captain" };

  /* canvas stays dark in both site themes, like the other games */
  const PAL = {
    bg0: "#070b12", bg1: "#0d1320", grid: "#1a2233",
    ink: "#e8eef8", dim: "#8b9ab8", ok: "#3ddc84",
    fuel: "#8ee65f", hull: "#4de3ff", temp: "#f59e42", power: "#c084fc",
    crew: "#7dd3fc", danger: "#ff5a4d", warn: "#fbbf24", beacon: "#ff5a4d",
    ship: "#e2e8f0", flame: "#ffb347", star: "#5b6b8c",
  };

  /* ---------- state ---------- */
  const S = {
    cfg: { seed: "oc-voyager" },
    game: null,
    running: false,   /* boots paused — the play button starts the bot */
    last: 0, acc: 0, speed: 1,
    boardOpen: false, statsOpen: false,
    pushed: { n: 0, outcome: false }, logSeen: -1,
    stars: [],
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
    S.meters.sample("engine.ms", nowMs() - t0);
    return res;
  }

  /* ---------- helpers ---------- */
  function cfgFromUI() {
    return {
      seed: S.seedKit ? S.seedKit.get() : "oc-voyager",
      drift: +byId("sc-drift").value,
      failures: +byId("sc-failures").value,
      difficulty: +byId("sc-difficulty").value,
      askBudget: +byId("sc-budget").value,
    };
  }

  function newGame() {
    S.cfg = cfgFromUI();
    S.game = SC.createGame(S.cfg);
    S.running = false;   /* restarts land paused, same as boot */
    S.pushed = { n: 0, outcome: false };
    S.logSeen = -1;
    const srng = DK.makeRng("stars:" + S.cfg.seed);   /* separate stream */
    S.stars = Array.from({ length: 140 }, () => ({
      x: srng() * W, y: srng() * H, r: 0.5 + srng() * 1.4,
      tw: srng() * 6.28,
    }));
    byId("sc-play").textContent = "▶ play";
    render();
  }

  /* board rows: a checkpoint every 60 sim-seconds, plus the outcome */
  function pushBoard(g, outcome) {
    const s = SC.runSummary(g);
    S.board.record({
      ts: Date.now(), outcome: outcome || "checkpoint",
      at: s.simSeconds, phase: s.aborting ? "return" : "outbound",
      fuel: s.fuel, hull: s.hull, temp: s.temp,
      decisions: s.decisions, rungs: s.rungs, captain: s.playerCalls,
      engineCalls: s.engineCalls, events: s.eventsFired,
      seed: g.cfg.seed, stamp: SC.shipStamp(g.cfg),
    });
  }

  function setChip() {
    const chip = byId("sc-engine-chip");
    if (!chip) return;
    const st = engine.status;
    chip.textContent = "engine: " + st;
    chip.classList.toggle("on", st === "ready");
    chip.classList.toggle("off", st === "unavailable" || st === "off");
  }

  function simSeconds(g) {
    return g.ticks / SC.TICKS_PER_S;
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

  function injectButton(kind, label) {
    return h("button", { class: "mz-chip", type: "button", id: "sc-ev-" + kind }, label);
  }

  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) S.cfg.seed = deepSeed;

    const bar = h("div", { class: "game-bar" },
      h("button", { class: "mz-chip mz-play", id: "sc-play", type: "button" }, "▶ play"),
      h("button", { class: "mz-chip", id: "sc-restart", type: "button" }, "↺ restart"),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "sc-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "sc-seed", class: "mz-input", value: S.cfg.seed, size: "10" }),
        h("button", { class: "mz-chip", id: "sc-dice", type: "button", title: "new random seed" }, "🎲")),
      h("button", { class: "mz-chip", id: "sc-speed", type: "button" }, "speed: 1×"),
      h("button", { class: "mz-chip", id: "sc-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "sc-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "sc-board-btn", type: "button" }, "★ flight log"),
      h("button", { class: "mz-chip", id: "sc-stats-btn", type: "button" }, "ⓘ stats"),
      h("button", { class: "mz-chip", id: "sc-reset-params", type: "button" }, "reset params"),
      h("button", { class: "mz-chip", id: "sc-fs", type: "button" }, "⛶ full screen"),
    );

    const canvas = h("canvas", { id: "sc-canvas", width: String(W), height: String(H),
      role: "img",
      "aria-label": "Spacecraft Emergency — the flight computer scores six actions every two seconds, abstains when telemetry conflicts or scores dead-heat, and the captain answers from the console below" });
    const panel = h("div", { class: "game-hintline", id: "sc-panel",
      "aria-live": "assertive" }, "flight computer confident — no escalation pending");
    const under = h("div", { class: "mz-under game-under", id: "sc-under" });
    const trace = h("div", { class: "game-hintline", id: "sc-trace",
      "aria-live": "polite" }, "no decisions yet — the first call comes at 2 sim-seconds");
    const log = h("div", { class: "game-hintline", id: "sc-log",
      "aria-live": "polite" }, "the flight log is empty");

    const opts = DK.shell.accordion([
      {
        id: "flight", label: "The flight", open: true,
        kids: [
          slider("sc-drift", "sensor drift", 0, 1, 0.05, 0.15),
          slider("sc-failures", "failure odds", 0, 1, 0.05, 0.2),
          slider("sc-difficulty", "event difficulty", 0, 1, 0.1, 0.5),
          slider("sc-budget", "engine budget", 0, 10, 1, 2, "/s"),
          h("p", { class: "game-note" },
            "Changing anything restarts the crossing from tick 0 with the ",
            "same seed — runs are deterministic, so a drift change is a ",
            "controlled experiment. Difficulty 0 is a calm crossing: no ",
            "random events, rules and scores only."),
        ],
      },
      {
        id: "malfunction", label: "Inject a malfunction",
        kids: [
          h("div", { class: "game-bar", id: "sc-ev-buttons" },
            injectButton("fault", "⚡ sensor fault"),
            injectButton("debris", "☄ debris ahead"),
            injectButton("storm", "🌡 dust storm"),
            injectButton("fire", "🔥 fire aboard")),
          h("p", { class: "game-note" },
            "A sensor fault sticks one backup gauge high or low for ~15 ",
            "seconds; if the two readings of a channel disagree past its ",
            "tolerance, the computer's confidence collapses and it may ",
            "abstain into your hands. The buttons need a live bridge? No — ",
            "malfunctions are yours, the engine only arbitrates uncertain ",
            "calls."),
        ],
      },
      {
        id: "console", label: "The captain's console",
        kids: [
          h("p", { class: "game-note" },
            "When the computer abstains the sim FREEZES — the panel under ",
            "the canvas shows its reason, both readings of every conflicting ",
            "gauge, and the two candidate actions it scored. Click one; the ",
            "resolution lands on the 'captain' rung and the crossing ",
            "resumes. An engine refusal escalates the same way: refusal is ",
            "a first-class outcome here, never an error."),
        ],
      },
      {
        id: "about", label: "What decides what?",
        kids: [h("p", { class: "game-note" },
          "Every rule fires before any question: hull ≤ 15 or a fire with a ",
          "failing crew → EMERGENCY; temp ≥ 95 → SHUTDOWN; destination out ",
          "of range while Earth is in range → RETURN HOME. The rest is ",
          "scored on the snapshot (six actions, fuel/hull/temp/power ",
          "aware); confidence = score gap × sensor agreement, where each ",
          "conflicting gauge quarters the agreement. Below 0.30 the ",
          "computer abstains to the captain; 0.30–0.55 goes to the engine ",
          "under the ask budget; an engine abstain escalates too. A ",
          "malformed engine answer is NOT a refusal — the argmax holds and ",
          "the rung says so. Decision latency is stamped on each decision, ",
          "never in the summary, so replays stay byte-identical.")]
      },
    ]);

    const main = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, canvas, panel, under, trace, log),
      h("div", { class: "game-side" }, opts));
    const wrap = h("div", { class: "game-wrap" }, bar, main);
    root.appendChild(wrap);

    buildOverlays();
    wire();
    S.seedKit = DK.shell.numericSeed(byId("sc-seed"), byId("sc-dice"), newGame);
    DK.shell.resetParams(byId("sc-reset-params"), root, newGame);
    DK.shell.fullscreen(wrap, byId("sc-fs"));
    engine.setEnabled(true);   /* default on: uncertain calls reach the runtime */
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
    root.appendChild(overlay("sc-board-overlay", "Session flight log", "sc-board-body", "sc-board-close"));
    root.appendChild(overlay("sc-stats-overlay", "Measured stats", "sc-stats-body", "sc-stats-close"));
  }

  function renderBoard() {
    const body = byId("sc-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" },
        "No rows yet — the log records a checkpoint every 60 sim-seconds ",
        "and the outcome when the crossing ends. Fly a ship home through a ",
        "fire and it fills. Session only."));
      return;
    }
    body.appendChild(h("table", { class: "game-table" },
      h("thead", null, h("tr", null,
        ...["#", "t (s)", "row", "phase", "fuel", "hull", "rule/engine/captain", "events", "seed"].map((x) => h("th", null, x)))),
      h("tbody", null, rows.map((r, i) => h("tr", null,
        h("td", null, String(i + 1)),
        h("td", null, String(r.at)),
        h("td", null, r.outcome),
        h("td", null, r.phase),
        h("td", null, String(r.fuel)),
        h("td", null, String(r.hull)),
        h("td", null, r.rungs.rule + "/" + r.rungs.engine + "/" + r.rungs.player),
        h("td", null, String(r.events)),
        h("td", null, r.seed))))));
    body.appendChild(h("button", {
      class: "mz-chip", type: "button", onclick: () => { S.board.clear(); renderBoard(); },
    }, "clear log"));
  }

  function renderStats() {
    const body = byId("sc-stats-body");
    if (!body) return;
    body.textContent = "";
    const snap = S.meters.snapshot();
    const g = S.game;
    const s = g ? SC.runSummary(g) : null;
    const rows = [
      ["frames/s (EMA)", S.fps.toFixed(0)],
      ["sim ticks", String(g ? g.ticks : 0)],
      ["sim seconds", s ? String(s.simSeconds) : "0"],
      ["phase", s ? (s.aborting ? "returning to Earth" : "outbound") : "—"],
      ["outcome", s ? (s.outcome || "in flight") : "—"],
      ["fuel / hull", s ? s.fuel + " / " + s.hull : "—"],
      ["temp / power / crew", s ? s.temp + " / " + s.power + " / " + s.crew : "—"],
      ["live sensor faults", s ? String(s.faults) : "0"],
      ["events fired", s ? String(s.eventsFired) : "0"],
      ["decisions — total", s ? String(s.decisions) : "0"],
      ["decisions — rule ladder", s ? String(s.rungs.rule) : "0"],
      ["decisions — engine", s ? String(s.rungs.engine) : "0"],
      ["decisions — captain", s ? String(s.rungs.player) : "0"],
      ["engine calls (uncertain band)", s ? String(s.engineCalls) : "0"],
      ["ask budget", g ? g.cfg.askBudget + "/s (tokens " + g.tokens.toFixed(2) + ")" : "—"],
      ["captain resolutions", s ? String(s.playerCalls) : "0"],
      ["pending right now", s ? (s.pending || "none") : "—"],
      ["log entries", s ? String(s.logEntries) : "0"],
      ["engine status", engine.status],
    ];
    if (snap["engine.ms"]) {
      rows.push(["engine latency — avg", DK.fmtMs(snap["engine.ms"].avg)]);
      rows.push(["engine latency — worst", DK.fmtMs(snap["engine.ms"].worst)]);
      rows.push(["engine latency — last", DK.fmtMs(snap["engine.ms"].last)]);
    }
    if (snap["tick.ms"]) {
      rows.push(["tick cost — avg", DK.fmtMs(snap["tick.ms"].avg)]);
      rows.push(["tick cost — worst", DK.fmtMs(snap["tick.ms"].worst)]);
    }
    body.appendChild(h("div", { class: "mz-stats-grid" }, rows.map(([k, v]) =>
      h("div", { class: "mz-stats-row" },
        h("span", { class: "mz-stats-label" }, k),
        h("span", { class: "mz-stats-value" }, v)))));
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
    const budget = 8 * Math.max(1, S.speed);      /* ticks per frame cap */
    for (let n = 0; S.acc >= DT && n < budget && S.game && !S.game.over; n++) {
      const t0 = nowMs();
      stepOnce();
      S.meters.sample("tick.ms", nowMs() - t0);
      S.acc -= DT;
      dirty = true;
    }
    if (S.game && S.game.over && !S.pushed.outcome) {
      S.pushed.outcome = true;
      pushBoard(S.game, S.game.outcome);
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
    SC.tickGame(g, engine.status === "ready" ? engineChoose : null);
    while (simSeconds(g) >= (S.pushed.n + 1) * CHECKPOINT_EVERY && !g.over) {
      S.pushed.n += 1;
      pushBoard(g, null);
      if (S.boardOpen) renderBoard();
    }
  }

  /* ---------- render ---------- */
  function drawTrack(ctx, g) {
    const y = 46, x0 = 60, x1 = W - 60;
    ctx.strokeStyle = PAL.grid;
    ctx.lineWidth = 2;
    ctx.beginPath(); ctx.moveTo(x0, y); ctx.lineTo(x1, y); ctx.stroke();
    /* destination (right) and Earth (left) */
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 10px ui-monospace, monospace"; ctx.textAlign = "left";
    ctx.fillText("EARTH", x0 - 8, y + 16);
    ctx.textAlign = "right";
    ctx.fillText("DESTINATION", x1 + 8, y + 16);
    /* progress marker: outbound = distance remaining shrinking; on abort
       the marker walks back toward Earth */
    const p = g.aborting
      ? Math.max(0, 1 - g.homeDist / g.dist0)
      : Math.max(0, 1 - g.dist / g.dist0);
    const sx = x0 + (x1 - x0) * p;
    ctx.fillStyle = PAL.ink;
    ctx.beginPath(); ctx.arc(sx, y, 5, 0, 7); ctx.fill();
    ctx.fillStyle = PAL.dim;
    ctx.textAlign = "center";
    const toGo = g.aborting ? g.homeDist : g.dist;
    ctx.fillText((g.aborting ? "returning · " : "outbound · ")
      + Math.round(toGo) + " to go", sx, y - 12);
  }

  function drawShip(ctx, g) {
    const cx = W / 2, cy = 190;
    const dir = g.aborting ? -1 : 1;
    ctx.save();
    ctx.translate(cx, cy);
    ctx.scale(dir, 1);
    /* hull */
    ctx.fillStyle = PAL.ship;
    ctx.beginPath();
    ctx.moveTo(34, 0); ctx.lineTo(-18, -14); ctx.lineTo(-26, 0); ctx.lineTo(-18, 14);
    ctx.closePath(); ctx.fill();
    /* window */
    ctx.fillStyle = PAL.bg1;
    ctx.beginPath(); ctx.arc(8, 0, 5, 0, 7); ctx.fill();
    /* flame scales with power */
    if (g.power > 0 && g.fuel > 0) {
      const fl = 10 + (g.power / 100) * 22 + Math.sin(g.ticks * 0.4) * 3;
      ctx.fillStyle = PAL.flame;
      ctx.beginPath();
      ctx.moveTo(-26, -5); ctx.lineTo(-26 - fl, 0); ctx.lineTo(-26, 5);
      ctx.closePath(); ctx.fill();
    }
    ctx.restore();
    /* fire beacon + smoke */
    if (g.fire) {
      const on = Math.floor(g.ticks / 12) % 2 === 0;
      ctx.fillStyle = on ? PAL.beacon : PAL.warn;
      ctx.beginPath(); ctx.arc(cx + 20, cy - 22, on ? 6 : 4, 0, 7); ctx.fill();
      ctx.font = "700 11px ui-monospace, monospace"; ctx.textAlign = "center";
      ctx.fillText("FIRE", cx + 20, cy - 32);
    }
    if (g.ahead) {
      /* the hazard: scattered rocks dead ahead */
      ctx.fillStyle = PAL.dim;
      for (let i = 0; i < 7; i++) {
        const a = i * 1.7 + Math.floor(g.ticks / 30) * 0.3;
        ctx.beginPath();
        ctx.arc(cx + dir * 150 + Math.cos(a) * 34, cy + Math.sin(a) * 26, 3.2, 0, 7);
        ctx.fill();
      }
      ctx.fillStyle = PAL.warn;
      ctx.font = "700 11px ui-monospace, monospace"; ctx.textAlign = "center";
      ctx.fillText(g.ahead.toUpperCase() + " AHEAD", cx + dir * 150, cy - 42);
    }
  }

  function drawGauges(ctx, g) {
    const gx = 24, gw = 190, gh = 12;
    const rows = [
      ["FUEL", Math.max(0, Math.min(1, g.fuel / 120)), PAL.fuel, 25],
      ["HULL", Math.max(0, Math.min(1, g.hull / 100)), g.hull < 40 ? PAL.danger : PAL.hull, 15],
      ["TEMP", Math.max(0, Math.min(1, g.temp / 120)), g.temp >= 95 ? PAL.danger : PAL.temp, 95 / 120],
      ["PWR", Math.max(0, Math.min(1, g.power / 100)), PAL.power, 0],
      ["CREW", Math.max(0, Math.min(1, g.crew / 100)), g.crew < 30 ? PAL.danger : PAL.crew, 0],
    ];
    let y = 260;
    ctx.textAlign = "left";
    for (const [name, v, color, mark] of rows) {
      ctx.fillStyle = PAL.dim;
      ctx.font = "600 10px ui-monospace, monospace";
      ctx.fillText(name, gx, y + 9);
      ctx.fillStyle = PAL.bg1;
      ctx.fillRect(gx + 42, y, gw, gh);
      ctx.fillStyle = color;
      ctx.fillRect(gx + 42, y, gw * v, gh);
      ctx.strokeStyle = PAL.grid;
      ctx.lineWidth = 1;
      ctx.strokeRect(gx + 42, y, gw, gh);
      if (mark > 0) {           /* danger threshold tick */
        const mx = gx + 42 + gw * mark;
        ctx.strokeStyle = PAL.danger;
        ctx.beginPath(); ctx.moveTo(mx, y - 2); ctx.lineTo(mx, y + gh + 2); ctx.stroke();
      }
      ctx.fillStyle = PAL.ink;
      ctx.fillText(String(Math.round(name === "FUEL" ? g.fuel
        : name === "TEMP" ? g.temp : v * 100)), gx + 42 + gw + 8, y + 9);
      y += 24;
    }
  }

  function drawSensorStrip(ctx, g) {
    /* the readings the decision ladder last saw — render NEVER calls
       readSensors(): that would consume the RNG stream and desync the
       deterministic sim */
    const sn = g.lastDecision && g.lastDecision.sensors
      ? g.lastDecision.sensors : null;
    const conf = g.pending ? g.pending.conflicts : null;
    const y = H - 26;
    ctx.font = "600 10px ui-monospace, monospace";
    ctx.textAlign = "left";
    let x = 24;
    ctx.fillStyle = PAL.dim;
    ctx.fillText("SENSORS", x, y);
    x += 62;
    for (const ch of SC.CHANNELS) {
      const c = sn ? sn[ch] : null;
      const bad = conf ? conf.includes(ch) : false;
      ctx.fillStyle = bad ? PAL.danger : PAL.ok;
      const label = SC.CHANNEL_LABEL[ch] + " "
        + (c ? c.a.toFixed(0) + "/" + c.b.toFixed(0) : "a/b")
        + (bad ? " CONFLICT" : " ok");
      ctx.fillText(label, x, y);
      x += 30 + ctx.measureText(label).width;
    }
  }

  function render() {
    const canvas = byId("sc-canvas");
    if (!canvas || !S.game) return;
    const g = S.game;
    const ctx = canvas.getContext("2d");

    const sky = ctx.createLinearGradient(0, 0, 0, H);
    sky.addColorStop(0, PAL.bg0);
    sky.addColorStop(1, PAL.bg1);
    ctx.fillStyle = sky;
    ctx.fillRect(0, 0, W, H);

    /* starfield — slow parallax drift, drawn from the pre-generated
       per-seed array (deterministic, no per-frame RNG) */
    for (const st of S.stars) {
      const tw = 0.55 + 0.45 * Math.sin(g.ticks * 0.02 + st.tw);
      ctx.fillStyle = PAL.star;
      ctx.globalAlpha = 0.35 + 0.5 * tw;
      ctx.beginPath(); ctx.arc(st.x, st.y, st.r, 0, 7); ctx.fill();
    }
    ctx.globalAlpha = 1;

    drawTrack(ctx, g);
    drawShip(ctx, g);
    drawGauges(ctx, g);
    drawSensorStrip(ctx, g);

    if (g.aborting) {
      ctx.fillStyle = PAL.warn;
      ctx.textAlign = "right";
      ctx.font = "700 13px ui-monospace, monospace";
      ctx.fillText("⚠ ABORT — RETURNING TO EARTH", W - 14, 28);
    }
    if (g.engineFault) {
      ctx.fillStyle = PAL.danger;
      ctx.textAlign = "right";
      ctx.font = "700 12px ui-monospace, monospace";
      ctx.fillText("ENGINE FAULT", W - 14, 48);
    }
    if (g.over) {
      const lines = {
        docked: "docked — the crossing is complete",
        returned: "safely home — the abort was the right call",
        adrift: "fuel exhausted — the ship drifts silent",
        destroyed: "the ship is lost — hull failure",
        lost: "contact lost",
      };
      ctx.fillStyle = g.outcome === "docked" || g.outcome === "returned" ? PAL.ok : PAL.danger;
      ctx.textAlign = "center";
      ctx.font = "700 18px ui-monospace, monospace";
      ctx.fillText(lines[g.outcome] || g.outcome, W / 2, H / 2 + 90);
      ctx.fillStyle = PAL.dim;
      ctx.font = "600 11px ui-monospace, monospace";
      ctx.fillText("press restart for another crossing", W / 2, H / 2 + 110);
    }

    /* under-row + trace + flight log */
    const s = SC.runSummary(g);
    const under = byId("sc-under");
    under.innerHTML = "";
    under.appendChild(DK.shell.kvRow([
      ["state", g.over ? s.outcome : S.running ? (g.pending ? "AWAITING CAPTAIN" : "in flight") : "paused"],
      ["phase", s.aborting ? "return" : "outbound"],
      ["to go", String(Math.round(g.aborting ? g.homeDist : g.dist))],
      ["fuel", s.fuel.toFixed(1)],
      ["hull", String(s.hull)],
      ["temp", s.temp + "°C"],
      ["power", s.power + "%"],
      ["asks", String(s.engineCalls)],
      ["engine-honored", String(s.rungs.engine)],
      ["captain", String(s.playerCalls)],
    ]));
    const ld = g.lastDecision;
    if (ld) {
      const at = [];
      if (ld.why) at.push(ld.why);
      if (ld.gap !== null && ld.gap !== undefined) at.push("gap " + ld.gap);
      at.push(DK.fmtMs(ld.ms || 0));
      byId("sc-trace").textContent = "last decision: "
        + (ld.action ? SC.ACTION_NAME[ld.action] : "ABSTAINED")
        + " · " + (RUNG_NAMES[ld.rung] || ld.rung)
        + " (" + at.join(", ") + ")";
    }
    if (g.log.length !== S.logSeen) {
      S.logSeen = g.log.length;
      const recent = g.log.slice(-5).map((l) =>
        "t+" + Math.round(l.tick / SC.TICKS_PER_S) + "s " + l.note);
      byId("sc-log").textContent = recent.length ? "flight log: " + recent.join(" · ") : "the flight log is empty";
    }

    /* the captain's console: rendered ONLY from g.pending — the freeze
       state — so the buttons exist exactly while the sim is paused.
       Rebuilt only when the pending actually changes: the loop keeps
       rendering while frozen, and rebuilding under a click every frame
       would race the button out from under the cursor */
    const panel = byId("sc-panel");
    const pendKey = g.pending
      ? g.pending.reason + "|" + g.pending.question.text
      : "";
    if (panel.dataset.pend !== pendKey) {
      panel.dataset.pend = pendKey;
      panel.textContent = "";
      if (g.pending) {
        const p = g.pending;
        panel.appendChild(h("span", { class: "game-slider-label" },
          "⏸ CAPTAIN'S CALL — " + p.reason + ". " + p.question.text + " "));
        for (const c of p.question.candidates) {
          panel.appendChild(h("button", {
            class: "mz-chip", type: "button", "data-choice": c.id,
            title: c.description,
          }, SC.ACTION_NAME[c.id]));
        }
        if (p.conflicts && p.conflicts.length) {
          const readings = p.conflicts.map((ch) =>
            SC.CHANNEL_LABEL[ch] + " " + p.sensors[ch].a.toFixed(0) + " vs "
              + p.sensors[ch].b.toFixed(0)).join(" · ");
          panel.appendChild(h("span", { class: "game-slider-label" },
            "  [" + readings + "]"));
        }
      } else {
        panel.textContent = "flight computer confident — no escalation pending";
      }
    }
    setInjectEnabled();
  }

  /* ---------- wiring ---------- */
  function setInjectEnabled() {
    const on = S.game && !S.game.over && !S.game.pending;
    for (const kind of ["fault"].concat(SC.EVENT_KINDS)) {
      const btn = byId("sc-ev-" + kind);
      if (btn) btn.disabled = !on;
    }
  }

  function wire() {
    byId("sc-play").addEventListener("click", (e) => {
      S.running = !S.running;
      e.currentTarget.textContent = S.running ? "⏸ pause" : "▶ play";
    });
    byId("sc-restart").addEventListener("click", () => newGame());
    byId("sc-seed").addEventListener("change", () => newGame());
    byId("sc-speed").addEventListener("click", (e) => {
      const steps = [1, 2, 4, 0.5];
      S.speed = steps[(steps.indexOf(S.speed) + 1) % steps.length];
      e.currentTarget.textContent = "speed: " + S.speed + "×";
    });
    byId("sc-engine-toggle").addEventListener("click", (e) => {
      const on = engine.status === "off" || engine.status === "unavailable";
      engine.setEnabled(on);
      e.currentTarget.textContent = on ? "engine: on" : "engine: off";
    });
    byId("sc-ev-fault").addEventListener("click", () => {
      const g = S.game;
      if (!g || g.over || g.pending) return;
      SC.injectSensorFault(g, SC.CHANNELS[Math.floor(g.rng() * SC.CHANNELS.length)]);
      render();
    });
    for (const kind of SC.EVENT_KINDS) {
      byId("sc-ev-" + kind).addEventListener("click", () => {
        const g = S.game;
        if (!g || g.over || g.pending) return;
        SC.injectEvent(g, kind);
        render();
      });
    }
    byId("sc-board-btn").addEventListener("click", () => {
      S.boardOpen = true; S.statsOpen = false;
      byId("sc-stats-overlay").classList.remove("open");
      byId("sc-board-overlay").classList.add("open");
      renderBoard();
    });
    byId("sc-board-close").addEventListener("click", () => {
      S.boardOpen = false;
      byId("sc-board-overlay").classList.remove("open");
    });
    byId("sc-stats-btn").addEventListener("click", () => {
      S.statsOpen = true; S.boardOpen = false;
      byId("sc-board-overlay").classList.remove("open");
      byId("sc-stats-overlay").classList.add("open");
      renderStats();
    });
    byId("sc-stats-close").addEventListener("click", () => {
      S.statsOpen = false;
      byId("sc-stats-overlay").classList.remove("open");
    });

    /* the console: one delegated listener — the panel is rebuilt on
       every render while a pending is live */
    byId("sc-panel").addEventListener("click", (e) => {
      const btn = e.target.closest("button[data-choice]");
      const g = S.game;
      if (!btn || !g || !g.pending) return;
      SC.resolvePending(g, btn.getAttribute("data-choice"));
      render();
    });

    /* keyboard: space pauses, R restarts, 1/2/4 speed, Esc closes overlays */
    window.addEventListener("keydown", (e) => {
      if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
      if (e.key === " ") {
        S.running = !S.running;
        byId("sc-play").textContent = S.running ? "⏸ pause" : "▶ play";
        e.preventDefault();
      } else if (e.key === "r" || e.key === "R") {
        newGame();
      } else if (e.key === "1" || e.key === "2" || e.key === "4") {
        S.speed = +e.key;
        byId("sc-speed").textContent = "speed: " + S.speed + "×";
      } else if (e.key === "Escape") {
        S.boardOpen = false; S.statsOpen = false;
        byId("sc-board-overlay").classList.remove("open");
        byId("sc-stats-overlay").classList.remove("open");
      }
    });

    setInjectEnabled();
    setChip();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", build);
  } else {
    build();
  }
})();
