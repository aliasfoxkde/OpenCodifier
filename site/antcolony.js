/* Ant Colony shell — the emergence demo on the Decisions SDK.
   antcolony-core.js owns the deterministic sim; this file is chrome: the
   pheromone field rendered as a live heatmap, the nest/food/spider/ants
   drawn on canvas, click-to-follow an ant's decision stream, the session
   board (auto-recorded at every 100th delivery), the stats overlay, and
   the engine bridge. Each ant decides at ~1 Hz: RETURN and GATHER are
   rules that are never asked; follow-trail / search / explore / defend
   are scored, and genuine near-ties go to the engine — but only up to
   the batching budget, because a colony of 1000 ants cannot ask 1000
   times a second. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const AC = globalThis.AntCore;
  const root = document.getElementById("antcolony-root");
  if (!root || !DK || !AC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;
  const W = AC.W, H = AC.H;
  const DT = 1 / AC.TICKS_PER_S;
  const RUNG_NAMES = { rule: "rule", engine: "engine" };
  const PHER_CUTOFF = 0.02;         /* cells below this stay unpainted */
  const BOOT_HINT = "ants wander, lay pheromone trails, and converge on food — watch the trails sharpen.";
  const CLICK_HINT = "click an ant to read its decision stream; click empty ground to let it go.";

  /* canvas stays dark in both site themes, like the other games */
  const PAL = {
    bg0: "#0c1210", bg1: "#111a18", grid: "#18241f",
    pher: "42, 212, 167",                              /* rgb for the alpha math */
    nest: "#4de3ff", food: "#8ee65f", carried: "#fde047",
    ink: "#e8eef8", dim: "#8b9ab8",
    ant: "#e2e8f0", antReturn: "#7dd3fc", antFollow: "#2dd4a7",
    antSearch: "#e2e8f0", antExplore: "#8b9ab8", antDefend: "#ff5a4d",
    spider: "#c084fc", danger: "#ff5a4d", ok: "#3ddc84",
    block: "#2a3550",
  };
  const ANT_COLOR = {
    "return": PAL.antReturn, "follow-trail": PAL.antFollow,
    "search": PAL.antSearch, "explore": PAL.antExplore,
    "defend": PAL.antDefend, "gather": PAL.carried,
  };

  /* ---------- state ---------- */
  const S = {
    cfg: { seed: "oc-ants" },
    game: null,
    running: false,   /* boots paused — the play button starts the bot */
    last: 0, acc: 0, speed: 1,
    boardOpen: false, statsOpen: false,
    follow: null,                    /* ant id clicked in the canvas */
    pushedMilestones: 0,
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
      seed: S.seedKit ? S.seedKit.get() : "oc-ants",
      ants: +byId("antcolony-ants").value,
      foodPiles: +byId("antcolony-food").value,
      evaporation: +byId("antcolony-evap").value,
      deposit: +byId("antcolony-deposit").value,
      decideRate: +byId("antcolony-rate").value,
      askBudget: +byId("antcolony-budget").value,
      obstacles: byId("antcolony-obstacles").value,
    };
  }

  function newGame() {
    S.cfg = cfgFromUI();
    S.game = AC.createGame(S.cfg);
    S.running = false;   /* restarts land paused, same as boot */
    S.follow = null;
    S.pushedMilestones = 0;
    byId("antcolony-play").textContent = "▶ play";
    syncVeil();
    render();
  }

  /* milestones land on the board automatically: every 100th delivery */
  function pushBoard(outcome, g) {
    const s = AC.runSummary(g);
    S.board.record({
      ts: Date.now(), outcome,
      delivered: s.delivered, decisions: s.decisions,
      perSecond: s.perSecond, engineCalls: s.engineCalls,
      trailMax: s.trailMax, milestone: s.milestones,
      seed: g.cfg.seed, stamp: AC.colonyStamp(g.cfg),
    });
  }

  function setChip() {
    const chip = byId("antcolony-engine-chip");
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

  function select(id, label, options, val) {
    const sel = h("select", { id, "aria-label": label },
      options.map(([v, text]) => h("option", { value: v, selected: v === val ? "" : null }, text)));
    sel.addEventListener("change", () => newGame());
    return h("label", { class: "game-slider" },
      h("span", { class: "game-slider-label" }, label), sel);
  }

  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) S.cfg.seed = deepSeed;

    const bar = h("div", { class: "game-bar" },
      h("button", { class: "mz-chip mz-play", id: "antcolony-play", type: "button" }, "▶ play"),
      h("button", { class: "mz-chip", id: "antcolony-restart", type: "button" }, "↺ restart"),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "antcolony-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "antcolony-seed", class: "mz-input", value: S.cfg.seed, size: "10" }),
        h("button", { class: "mz-chip", id: "antcolony-dice", type: "button", title: "new random seed" }, "🎲")),
      h("button", { class: "mz-chip", id: "antcolony-speed", type: "button" }, "speed: 1×"),
      h("button", { class: "mz-chip", id: "antcolony-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "antcolony-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "antcolony-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "antcolony-stats-btn", type: "button" }, "ⓘ stats"),
      h("button", { class: "mz-chip", id: "antcolony-reset-params", type: "button" }, "reset params"),
      h("button", { class: "mz-chip", id: "antcolony-fs", type: "button" }, "⛶ full screen"),
    );

    const canvas = h("canvas", { id: "antcolony-canvas", width: String(W), height: String(H),
      role: "img",
      "aria-label": "Ant Colony — up to a thousand ants decide at one hertz; trails are laid, not computed, and near-tie behaviors go to the engine under a batching budget" });
    const veil = h("div", { class: "game-veil", id: "antcolony-veil", role: "group",
      "aria-label": "Simulation paused" },
      h("button", { class: "game-veil-chip", type: "button", id: "antcolony-veil-chip" }, "▶ press play"),
      h("span", { class: "game-veil-hint", id: "antcolony-veil-hint" }, BOOT_HINT));
    const hint = h("p", { class: "game-hintline ac-hint" }, BOOT_HINT + " " + CLICK_HINT);
    const status = h("div", { class: "game-hintline ac-status", id: "antcolony-status",
      "aria-live": "polite" }, "ants out: 0 · carrying: 0 · food collected: 0 · ticks: 0");
    const under = h("div", { class: "mz-under game-under", id: "antcolony-under" });
    const trace = h("div", { class: "game-hintline", id: "antcolony-trace",
      "aria-live": "polite" }, "no decisions yet — click an ant to follow it");

    const opts = DK.shell.accordion([
      {
        id: "colony", label: "The colony", open: true,
        kids: [
          slider("antcolony-ants", "ants", 10, 1000, 10, 120),
          slider("antcolony-food", "food piles", 1, 8, 1, 3),
          slider("antcolony-evap", "evaporation", 0, 0.9, 0.05, 0.4),
          slider("antcolony-deposit", "deposit", 0.5, 8, 0.5, 2),
          slider("antcolony-rate", "decide-rate", 0.5, 4, 0.5, 1, "/s"),
          slider("antcolony-budget", "engine budget", 0, 10, 1, 2, "/s"),
          select("antcolony-obstacles", "obstacles",
            [["none", "none — open field"], ["wall", "wall with one gap"]], "none"),
          h("p", { class: "game-note" },
            "Changing anything restarts the colony from tick 0 with the same seed — ",
            "runs are deterministic, so an evaporation change is a controlled experiment."),
        ],
      },
      {
        id: "follow", label: "Follow one ant",
        kids: [
          h("p", { class: "game-note" },
            "Click an ant (zoom in on a dense colony — they are 3 px) to pin its ",
            "decision stream to the trace line below the canvas. Click empty ",
            "ground to release it. The board auto-records a row at every 100th ",
            "delivery."),
          h("button", { class: "mz-chip", id: "antcolony-release", type: "button" },
            "release followed ant"),
        ],
      },
      {
        id: "about", label: "What decides what?",
        kids: [h("p", { class: "game-note" },
          "Every ant decides at the decide-rate — but not every decision is a ",
          "question. RETURN (carrying food) and GATHER (food underfoot) are RULES, ",
          "never asked. The rest is scored on the snapshot: follow-trail vs search ",
          "vs explore — plus defend when the spider is close — and when the top two ",
          "land within 0.12 value the ENGINE arbitrates… if the colony has a token ",
          "left. The batching budget refills at N asks per sim-second, because a ",
          "thousand ants asking every second would be a thousand questions a second. ",
          "No token → the argmax rule decides and the rung says so. The trail ",
          "network is emergent: there is no pathfinding code anywhere.")],
      },
    ]);

    const main = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, hint,
        h("div", { class: "game-view" }, canvas, veil),
        status, under, trace),
      h("div", { class: "game-side" }, opts));
    root.appendChild(h("div", { class: "game-wrap" }, bar, main));

    buildOverlays();
    wire();
    S.seedKit = DK.shell.numericSeed(byId("antcolony-seed"), byId("antcolony-dice"), newGame);
    DK.shell.resetParams(byId("antcolony-reset-params"), root);
    DK.shell.fullscreen(root.querySelector(".game-wrap"), byId("antcolony-fs"));
    engine.setEnabled(true);   /* default on: budgeted near-ties reach the runtime */
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
    root.appendChild(overlay("antcolony-board-overlay", "Session scoreboard", "antcolony-board-body", "antcolony-board-close"));
    root.appendChild(overlay("antcolony-stats-overlay", "Measured stats", "antcolony-stats-body", "antcolony-stats-close"));
  }

  function renderBoard() {
    const body = byId("antcolony-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" },
        "No milestones yet — the board records a row automatically at every ",
        "100th delivery. Keep the colony alive and watch it fill. Session only."));
      return;
    }
    body.appendChild(h("table", { class: "game-table" },
      h("thead", null, h("tr", null,
        ...["#", "milestone", "delivered", "dec/s", "engine", "trail max", "seed"].map((x) => h("th", null, x)))),
      h("tbody", null, rows.map((r, i) => h("tr", null,
        h("td", null, String(rows.length - i)),
        h("td", null, String(r.milestone)),
        h("td", null, String(r.delivered)),
        h("td", null, String(r.perSecond)),
        h("td", null, String(r.engineCalls)),
        h("td", null, String(r.trailMax)),
        h("td", null, r.seed))))));
    body.appendChild(h("button", {
      class: "mz-chip", type: "button", onclick: () => { S.board.clear(); renderBoard(); },
    }, "clear board"));
  }

  function renderStats() {
    const body = byId("antcolony-stats-body");
    if (!body) return;
    body.textContent = "";
    const snap = S.meters.snapshot();
    const g = S.game;
    const s = g ? AC.runSummary(g) : null;
    const rows = [
      ["frames/s (EMA)", S.fps.toFixed(0)],
      ["sim ticks", String(g ? g.ticks : 0)],
      ["ants", g ? String(g.ants.length) : "0"],
      ["decisions — total", s ? String(s.decisions) : "0"],
      ["decisions/s (sim time)", s ? String(s.perSecond) : "0"],
      ["decisions — rule ladder", s ? String(s.rungs.rule) : "0"],
      ["decisions — engine", s ? String(s.rungs.engine) : "0"],
      ["engine calls (near-ties)", s ? String(s.engineCalls) : "0"],
      ["ask budget", g ? g.cfg.askBudget + "/s (tokens " + g.tokens.toFixed(2) + ")" : "—"],
      ["food gathered", s ? String(s.gathers) : "0"],
      ["delivered to nest", s ? String(s.delivered) : "0"],
      ["trail strength — max", s ? String(s.trailMax) : "0"],
      ["spider flights", s ? String(s.spiderFled) : "0"],
      ["milestones recorded", s ? String(s.milestones) : "0"],
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
    for (let n = 0; S.acc >= DT && n < budget; n++) {
      const t0 = nowMs();
      stepOnce();
      S.meters.sample("tick.ms", nowMs() - t0);
      S.acc -= DT;
      dirty = true;
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
    AC.tickGame(g, engine.status === "ready" ? engineChoose : null);
    /* board rows land as milestones happen, never mid-decision */
    while (g.milestones.length > S.pushedMilestones) {
      S.pushedMilestones += 1;
      pushBoard("milestone", g);
      if (S.boardOpen) renderBoard();
    }
  }

  /* ---------- render ---------- */
  function drawField(ctx, g) {
    /* the pheromone heatmap: teal alpha ∝ strength. One pass over the
       grid, skipping dead cells — this is the trail network made visible */
    const pher = g.pher;
    const cell = AC.CELL;
    for (let cy = 0; cy < AC.GH; cy++) {
      for (let cx = 0; cx < AC.GW; cx++) {
        const v = pher[cy * AC.GW + cx];
        if (v < PHER_CUTOFF) continue;
        ctx.fillStyle = "rgba(" + PAL.pher + ", " + Math.min(0.55, v * 0.3).toFixed(3) + ")";
        ctx.fillRect(cx * cell, cy * cell, cell, cell);
      }
    }
  }

  function drawStatic(ctx, g) {
    /* obstacles */
    ctx.fillStyle = PAL.block;
    for (let cy = 0; cy < AC.GH; cy++) {
      for (let cx = 0; cx < AC.GW; cx++) {
        if (g.blocked[cy * AC.GW + cx]) ctx.fillRect(cx * AC.CELL, cy * AC.CELL, AC.CELL, AC.CELL);
      }
    }
    /* the nest */
    ctx.strokeStyle = PAL.nest;
    ctx.lineWidth = 2;
    ctx.beginPath(); ctx.arc(AC.NEST.x, AC.NEST.y, AC.NEST_RADIUS + 4, 0, 7); ctx.stroke();
    ctx.fillStyle = PAL.nest;
    ctx.beginPath(); ctx.arc(AC.NEST.x, AC.NEST.y, 5, 0, 7); ctx.fill();
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 10px ui-monospace, monospace"; ctx.textAlign = "center";
    ctx.fillText("nest · " + g.delivered + " delivered", AC.NEST.x, AC.NEST.y + AC.NEST_RADIUS + 18);
    /* food piles: a cluster of dots, shrinking as the pile is eaten */
    for (const p of g.piles) {
      if (p.amount <= 0) {
        ctx.strokeStyle = PAL.dim;
        ctx.lineWidth = 1;
        ctx.beginPath(); ctx.arc(p.x, p.y, 6, 0, 7); ctx.stroke();
        continue;
      }
      const full = 6, n = Math.max(1, Math.round((p.amount / 400) * full));
      ctx.fillStyle = PAL.food;
      for (let i = 0; i < n; i++) {
        const a = (i / full) * Math.PI * 2;
        ctx.beginPath();
        ctx.arc(p.x + Math.cos(a) * 5, p.y + Math.sin(a) * 5, 3, 0, 7);
        ctx.fill();
      }
      ctx.beginPath(); ctx.arc(p.x, p.y, 2, 0, 7); ctx.fill();
    }
    /* the spider */
    const sp = g.spider;
    ctx.fillStyle = g.ticks < sp.fleeUntil ? PAL.dim : PAL.spider;
    ctx.beginPath(); ctx.arc(sp.x, sp.y, 8, 0, 7); ctx.fill();
    ctx.strokeStyle = ctx.fillStyle;
    ctx.lineWidth = 1.5;
    for (let i = 0; i < 4; i++) {
      const a = sp.heading + (i - 1.5) * 0.55;
      ctx.beginPath();
      ctx.moveTo(sp.x, sp.y);
      ctx.lineTo(sp.x + Math.cos(a) * 13, sp.y + Math.sin(a) * 13);
      ctx.stroke();
    }
    ctx.fillStyle = ctx.fillStyle === PAL.dim ? PAL.dim : PAL.danger;
    ctx.font = "700 9px ui-monospace, monospace"; ctx.textAlign = "center";
    ctx.fillText(g.ticks < sp.fleeUntil ? "fleeing" : "spider", sp.x, sp.y - 14);
  }

  function drawAnts(ctx, g) {
    /* ants are 3 px oriented dashes; the color IS the behavior */
    /* dark under-stroke first, colour on top: 120 ants must read against
       near-black soil at 100% zoom, not just in a screenshot */
    ctx.lineCap = "round";
    for (const a of g.ants) {
      const x2 = a.x + Math.cos(a.heading) * 4.5;
      const y2 = a.y + Math.sin(a.heading) * 4.5;
      ctx.strokeStyle = "rgba(4, 8, 6, 0.72)";
      ctx.lineWidth = 4;
      ctx.beginPath(); ctx.moveTo(a.x, a.y); ctx.lineTo(x2, y2); ctx.stroke();
      ctx.strokeStyle = ANT_COLOR[a.behavior] || PAL.ant;
      ctx.lineWidth = 2;
      ctx.beginPath(); ctx.moveTo(a.x, a.y); ctx.lineTo(x2, y2); ctx.stroke();
      if (a.carrying) {
        ctx.fillStyle = PAL.carried;
        ctx.beginPath(); ctx.arc(x2, y2, 1.9, 0, 7); ctx.fill();
        ctx.strokeStyle = "rgba(4, 8, 6, 0.72)";
        ctx.lineWidth = 0.8;
        ctx.beginPath(); ctx.arc(x2, y2, 1.9, 0, 7); ctx.stroke();
      }
    }
    ctx.lineCap = "butt";
    /* the followed ant gets a ring */
    if (S.follow) {
      const f = g.ants.find((a) => a.id === S.follow);
      if (f) {
        ctx.strokeStyle = PAL.carried;
        ctx.lineWidth = 1;
        ctx.beginPath(); ctx.arc(f.x, f.y, 7, 0, 7); ctx.stroke();
      }
    }
  }

  /* the on-canvas legend: what the colours and the glow mean. Drawn every
     frame in a corner overlay — it is the only key the field has. */
  function roundRect(ctx, x, y, w, hh, r) {
    ctx.beginPath();
    ctx.moveTo(x + r, y);
    ctx.arcTo(x + w, y, x + w, y + hh, r);
    ctx.arcTo(x + w, y + hh, x, y + hh, r);
    ctx.arcTo(x, y + hh, x, y, r);
    ctx.arcTo(x, y, x + w, y, r);
    ctx.closePath();
  }

  function drawLegend(ctx) {
    const rows = [
      { kind: "pher", label: "pheromone trail — brighter = stronger" },
      { kind: "ring", color: PAL.nest, label: "nest — food is delivered here" },
      { kind: "dots", color: PAL.food, label: "food pile (finite)" },
      { kind: "dash", color: PAL.antSearch, label: "searching" },
      { kind: "dash", color: PAL.antFollow, label: "following the trail" },
      { kind: "dash", color: PAL.antReturn, label: "returning with food" },
      { kind: "dash", color: PAL.antExplore, label: "exploring" },
      { kind: "dash", color: PAL.antDefend, label: "defending" },
      { kind: "spider", color: PAL.spider, label: "spider — a quorum drives it off" },
    ];
    const w = 268, x = W - w - 14, y = 14, rowH = 16;
    const hh = rows.length * rowH + 12;
    ctx.fillStyle = "rgba(6, 12, 10, 0.7)";
    ctx.strokeStyle = "rgba(255, 255, 255, 0.09)";
    ctx.lineWidth = 1;
    roundRect(ctx, x, y, w, hh, 8);
    ctx.fill();
    ctx.stroke();
    ctx.font = "600 10px ui-monospace, monospace";
    ctx.textAlign = "left";
    rows.forEach((r, i) => {
      const cy = y + 10 + i * rowH + 5;
      if (r.kind === "pher") {
        [0.08, 0.2, 0.38, 0.55].forEach((a, k) => {
          ctx.fillStyle = "rgba(" + PAL.pher + ", " + a + ")";
          ctx.fillRect(x + 10 + k * 9, cy - 4, 7, 8);
        });
      } else if (r.kind === "ring") {
        ctx.strokeStyle = r.color;
        ctx.lineWidth = 2;
        ctx.beginPath(); ctx.arc(x + 15, cy, 5, 0, 7); ctx.stroke();
      } else if (r.kind === "dots") {
        ctx.fillStyle = r.color;
        for (let k = 0; k < 3; k++) {
          ctx.beginPath(); ctx.arc(x + 9 + k * 7, cy + (k % 2 ? -2 : 2), 2.2, 0, 7); ctx.fill();
        }
      } else if (r.kind === "dash") {
        ctx.strokeStyle = r.color;
        ctx.lineWidth = 2;
        ctx.beginPath(); ctx.moveTo(x + 9, cy + 2); ctx.lineTo(x + 20, cy - 3); ctx.stroke();
      } else {
        ctx.fillStyle = r.color;
        ctx.beginPath(); ctx.arc(x + 15, cy, 4.5, 0, 7); ctx.fill();
        ctx.strokeStyle = r.color;
        ctx.lineWidth = 1;
        for (let k = 0; k < 3; k++) {
          const a = -0.6 + k * 0.6;
          ctx.beginPath();
          ctx.moveTo(x + 15, cy);
          ctx.lineTo(x + 15 + Math.cos(a) * 8, cy + Math.sin(a) * 8);
          ctx.stroke();
        }
      }
      ctx.fillStyle = PAL.dim;
      ctx.fillText(r.label, x + 30, cy + 3.5);
    });
  }

  /* the textual heartbeat: proof the colony is alive without reading pixels */
  function syncStatus(g, s) {
    const el = byId("antcolony-status");
    if (!el) return;
    const carrying = g.ants.filter((a) => a.carrying).length;
    el.textContent = "ants out: " + (g.ants.length - carrying) + " · carrying: " + carrying
      + " · food collected: " + s.gathers + " · delivered: " + s.delivered
      + " · ticks: " + g.ticks;
  }

  function render() {
    const canvas = byId("antcolony-canvas");
    if (!canvas || !S.game) return;
    const g = S.game;
    const ctx = canvas.getContext("2d");

    const sky = ctx.createLinearGradient(0, 0, 0, H);
    sky.addColorStop(0, PAL.bg0);
    sky.addColorStop(1, PAL.bg1);
    ctx.fillStyle = sky;
    ctx.fillRect(0, 0, W, H);
    /* faint grid */
    ctx.strokeStyle = PAL.grid;
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (let x = AC.CELL; x < W; x += AC.CELL) { ctx.moveTo(x, 0); ctx.lineTo(x, H); }
    for (let y = AC.CELL; y < H; y += AC.CELL) { ctx.moveTo(0, y); ctx.lineTo(W, y); }
    ctx.stroke();

    drawField(ctx, g);
    drawStatic(ctx, g);
    drawAnts(ctx, g);
    drawLegend(ctx);

    /* HUD */
    const s = AC.runSummary(g);
    syncStatus(g, s);
    ctx.textAlign = "left";
    ctx.fillStyle = PAL.ink;
    ctx.font = "700 13px ui-monospace, monospace";
    ctx.fillText("ants " + g.ants.length, 14, 24);
    ctx.fillStyle = PAL.ok;
    ctx.fillText("delivered " + s.delivered, 14, 42);
    ctx.fillStyle = PAL.ink;
    ctx.fillText("decisions/s " + s.perSecond.toFixed(1), 14, 60);
    ctx.fillStyle = PAL.antFollow;
    ctx.fillText("trail max " + s.trailMax.toFixed(2), 14, 78);

    /* under-row + trace */
    const ld = g.lastDecision;
    byId("antcolony-under").innerHTML = "";
    byId("antcolony-under").appendChild(DK.shell.kvRow([
      ["state", S.running ? "foraging" : "paused"],
      ["ants", String(g.ants.length)],
      ["delivered", String(s.delivered)],
      ["decisions/s", s.perSecond.toFixed(1)],
      ["engine asks", String(s.engineCalls)],
      ["engine-honored", String(s.rungs.engine)],
      ["trail max", s.trailMax.toFixed(2)],
      ["milestones", String(s.milestones)],
      ["following", S.follow || "—"],
    ]));
    if (ld) {
      const at = ld.at || {};
      const prefix = S.follow
        ? (ld.id === S.follow ? "followed ant → " : "following " + S.follow + " (last ask elsewhere) · ")
        : "";
      const detail = ld.evGap === null || ld.evGap === undefined
        ? "nest " + at.nest + "px, carrying " + (at.carrying ? "yes" : "no")
        : "gap " + (ld.evGap === Infinity ? "∅" : ld.evGap) + ", nest " + at.nest + "px";
      byId("antcolony-trace").textContent = prefix + "last decision: " + ld.id +
        " (" + detail + ") — " + ld.choice + " · " + (ld.rung ? RUNG_NAMES[ld.rung] : "rule");
    }
  }

  /* ---------- wiring ---------- */
  function pickAnt(g, x, y) {
    let best = null, bd = 16;
    for (const a of g.ants) {
      const d = Math.hypot(a.x - x, a.y - y);
      if (d < bd) { bd = d; best = a; }
    }
    return best;
  }

  /* ---------- running state ---------- */
  function simSeconds(g) { return g.ticks / AC.TICKS_PER_S; }

  function setRunning(on) {
    S.running = on;
    if (!on) S.last = 0;
    const play = byId("antcolony-play");
    if (play) play.textContent = on ? "⏸ pause" : "▶ play";
    syncVeil();
  }

  function syncVeil() {
    const veil = byId("antcolony-veil");
    const chip = byId("antcolony-veil-chip");
    const hint = byId("antcolony-veil-hint");
    const g = S.game;
    if (!veil || !g) return;
    if (S.running) { veil.hidden = true; return; }
    veil.hidden = false;
    if (simSeconds(g) > 0) {
      chip.textContent = "▶ resume";
      hint.textContent = "paused at t+" + simSeconds(g).toFixed(0) + "s — the trails hold; "
        + AC.runSummary(g).gathers + " food collected so far.";
    } else {
      chip.textContent = "▶ press play";
      hint.textContent = BOOT_HINT;
    }
  }

  function wire() {
    byId("antcolony-play").addEventListener("click", () => setRunning(!S.running));
    byId("antcolony-veil").addEventListener("click", () => setRunning(true));
    byId("antcolony-restart").addEventListener("click", () => newGame());
    byId("antcolony-seed").addEventListener("change", () => newGame());
    byId("antcolony-speed").addEventListener("click", (e) => {
      const steps = [1, 2, 4, 0.5];
      S.speed = steps[(steps.indexOf(S.speed) + 1) % steps.length];
      e.currentTarget.textContent = "speed: " + S.speed + "×";
    });
    byId("antcolony-engine-toggle").addEventListener("click", (e) => {
      const on = engine.status === "off" || engine.status === "unavailable";
      engine.setEnabled(on);
      e.currentTarget.textContent = on ? "engine: on" : "engine: off";
    });
    byId("antcolony-release").addEventListener("click", () => { S.follow = null; });
    byId("antcolony-board-btn").addEventListener("click", () => {
      S.boardOpen = true; S.statsOpen = false;
      byId("antcolony-stats-overlay").classList.remove("open");
      byId("antcolony-board-overlay").classList.add("open");
      renderBoard();
    });
    byId("antcolony-board-close").addEventListener("click", () => {
      S.boardOpen = false;
      byId("antcolony-board-overlay").classList.remove("open");
    });
    byId("antcolony-stats-btn").addEventListener("click", () => {
      S.statsOpen = true; S.boardOpen = false;
      byId("antcolony-board-overlay").classList.remove("open");
      byId("antcolony-stats-overlay").classList.add("open");
      renderStats();
    });
    byId("antcolony-stats-close").addEventListener("click", () => {
      S.statsOpen = false;
      byId("antcolony-stats-overlay").classList.remove("open");
    });

    /* click an ant to follow its decisions; empty ground releases */
    byId("antcolony-canvas").addEventListener("click", (e) => {
      const g = S.game;
      if (!g) return;
      const r = e.currentTarget.getBoundingClientRect();
      const x = (e.clientX - r.left) * (W / r.width);
      const y = (e.clientY - r.top) * (H / r.height);
      const ant = pickAnt(g, x, y);
      S.follow = ant ? ant.id : null;
      render();
    });

    /* keyboard: space pauses, R restarts, 1-4 speed, Esc closes overlays */
    window.addEventListener("keydown", (e) => {
      if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
      if (e.key === " ") {
        setRunning(!S.running);
        e.preventDefault();
      } else if (e.key === "r" || e.key === "R") {
        newGame();
      } else if (e.key === "1" || e.key === "2" || e.key === "4") {
        S.speed = +e.key;
        byId("antcolony-speed").textContent = "speed: " + S.speed + "×";
      } else if (e.key === "Escape") {
        S.boardOpen = false; S.statsOpen = false;
        byId("antcolony-board-overlay").classList.remove("open");
        byId("antcolony-stats-overlay").classList.remove("open");
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
