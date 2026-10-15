/* AI Civilization in 60 Seconds — shell.
   civ60-core.js owns the deterministic sim; this file is chrome only:
     - the town SCENE: drawn vector sprites (people, homes, farms, granary,
       market, palisade, river, trees, raid flames) under a time-of-day tint
     - the LEDGER strip: the resource history chart fed by g.history
     - the DOM resource bar (population / food / wood / homes / day), the
       goal banner, the chronicle panel and the end-of-run verdict
     - event injection, the session board (auto-recorded at every 500th
       decision), the stats overlay, and the engine bridge.
   Citizens decide at ~1.33 Hz: SLEEP, urgent EAT and HAVE-CHILD are rules
   that are never asked; work / farm / build / trade / explore / fight /
   rationing-eat / flee are scored on the snapshot, and genuine near-ties
   go to the engine — under the batching budget, because sixty citizens
   would otherwise flood the runtime with eighty questions a minute. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const CC = globalThis.CivCore;
  const root = document.getElementById("civ60-root");
  if (!root || !DK || !CC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;
  const W = 960, H = 520;          /* the town scene */
  const CW = 960, CH = 132;        /* the ledger strip */
  const DT = 1 / CC.TICKS_PER_S;
  const RUNG_NAMES = { rule: "rule", engine: "engine" };
  const MILESTONE_EVERY = 500;      /* decisions per board row */
  const CHART = { x0: 50, x1: 742, y0: 26, y1: 112 };
  const DAY_S = 90;                 /* sim-seconds per in-world day */
  const LOG_MAX = 60;               /* chronicle entries kept in the DOM */
  const BOOT_HINT = "the city decides for itself — citizens farm, work, build and trade on their own scores. Inject a shortage and watch the cascade.";
  const GOAL = "Keep the city alive and housed. You supply the weather; the citizens decide. There is no win state — collapse (the last citizen gone) is the only loss, and everything short of it is a verdict you read off the ledger.";

  /* canvas stays dark in both site themes, like the other games */
  const PAL = {
    skyDay0: "#7fb2d9", skyDay1: "#cfe3ee",
    skyDusk0: "#3b3a6b", skyDusk1: "#d98a5f",
    skyNight0: "#0b1226", skyNight1: "#1d2b45",
    grass: "#2c4a2e", grassDark: "#24402a", grassLite: "#35573a",
    hill: "#3d5a54", hill2: "#33504b", tuft: "#24402a",
    skin: "#f2d5b6", path: "#8a7350",
    water: "#1d4f70", waterLite: "#2b6a92",
    field: "#4a3a24", crop: "#7fae4a", cropLite: "#95c25c",
    soil: "#5a4530",
    trunk: "#5b4632", leaf: "#2f6b3d", leaf2: "#3c8048",
    wall: "#c9b18a", wallEdge: "#8a7350", roof: "#8f4b3a", roof2: "#7a4234",
    door: "#5b4632", windowDay: "#3d5a6b", windowLit: "#ffd479",
    stone: "#9aa3ad", stoneEdge: "#6b7480",
    awning0: "#b5563f", awning1: "#e8dcc0",
    post: "#6b5638",
    flame: "#ff8c3a", flameCore: "#ffe08a",
    raider: "#b3372c",
    grid: "#1a2233", ink: "#f2f6fc", dim: "#9fb0cc",
    ok: "#3ddc84", food: "#8ee65f", wood: "#d9a05b", pop: "#4de3ff",
    homes: "#c084fc", danger: "#ff5a4d", festival: "#fde047",
    follow: "#fde047", carried: "#fde047",
    scrim: "rgba(7, 11, 17, 0.72)",
  };
  const BEHAVIOR_COLOR = {
    work: PAL.wood, farm: PAL.food, build: "#f59e42", trade: PAL.festival,
    explore: "#7dd3fc", fight: PAL.danger, eat: "#ffb38a", flee: "#ff8fc7",
    sleep: "#8b9ab8", "have-child": "#c084fc", rest: "#7c8aa5",
  };
  const LEGEND_ORDER = ["farm", "work", "build", "trade", "explore",
    "fight", "eat", "sleep", "have-child", "flee", "rest"];

  /* ---------- state ---------- */
  const S = {
    cfg: { seed: "oc-civ" },
    game: null,
    running: false,   /* boots paused — the play button starts the bot */
    last: 0, acc: 0, speed: 1,
    boardOpen: false, statsOpen: false, verdictOpen: false,
    follow: null,                  /* citizen id clicked in the canvas */
    pushedMilestones: 0, logSeen: 0,
    board: DK.sessionBoard(),
    meters: DK.meters(),
    fps: 0,
    decor: null,                   /* per-seed scenery: trees, ripples */
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
      seed: S.seedKit ? S.seedKit.get() : "oc-civ",
      citizens: +byId("civ60-citizens").value,
      askBudget: +byId("civ60-budget").value,
      events: byId("civ60-events").value === "on",
    };
  }

  function newGame() {
    S.cfg = cfgFromUI();
    S.game = CC.createGame(S.cfg);
    S.running = false;   /* restarts land paused, same as boot */
    S.follow = null;
    S.pushedMilestones = 0;
    S.logSeen = 0;
    S.decor = makeDecor(S.cfg.seed);
    const log = byId("civ60-log");
    if (log) log.textContent = "";
    const play = byId("civ60-play");
    if (play) play.textContent = "▶ play";
    setEventsEnabled();
    syncVerdict();
    syncVeil();
    render();
  }

  /* board rows land as milestones happen: every 500th decision */
  function pushBoard(g) {
    const s = CC.runSummary(g);
    S.board.record({
      ts: Date.now(), outcome: "milestone",
      milestone: s.decisions, pop: s.pop, food: s.food,
      decisions: s.decisions, perSecond: s.perSecond,
      engineCalls: s.engineCalls, coherent: s.coherent,
      seed: g.cfg.seed, stamp: CC.civStamp(g.cfg),
    });
  }

  function setChip() {
    const chip = byId("civ60-engine-chip");
    if (!chip) return;
    const st = engine.status;
    chip.textContent = "engine: " + st;
    chip.classList.toggle("on", st === "ready");
    chip.classList.toggle("off", st === "unavailable" || st === "off");
  }

  function simSeconds(g) {
    return g.ticks / CC.TICKS_PER_S;
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

  function eventButton(kind, label) {
    return h("button", {
      class: "mz-chip", type: "button", id: "civ60-ev-" + kind,
    }, label);
  }

  /* the live resource bar: DOM chips above the canvas, updated per frame */
  function hudChip(id, swatch, label) {
    return h("span", { class: "civ-chip", id: "civ60-hud-" + id },
      swatch ? h("i", { class: "civ-sw", style: "background:" + swatch }) : null,
      h("small", null, label + " "),
      h("b", null, "—"));
  }

  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) S.cfg.seed = deepSeed;

    const bar = h("div", { class: "game-bar" },
      h("button", { class: "mz-chip mz-play", id: "civ60-play", type: "button" }, "▶ play"),
      h("button", { class: "mz-chip", id: "civ60-restart", type: "button" }, "↺ restart"),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "civ60-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "civ60-seed", class: "mz-input", value: S.cfg.seed, size: "10" }),
        h("button", { class: "mz-chip", id: "civ60-dice", type: "button", title: "new random seed" }, "🎲")),
      h("button", { class: "mz-chip", id: "civ60-speed", type: "button" }, "speed: 1×"),
      h("button", { class: "mz-chip", id: "civ60-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "civ60-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "civ60-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "civ60-stats-btn", type: "button" }, "ⓘ stats"),
      h("button", { class: "mz-chip", id: "civ60-verdict-btn", type: "button" }, "⚑ verdict"),
      h("button", { class: "mz-chip", id: "civ60-reset-params", type: "button" }, "reset params"),
      h("button", { class: "mz-chip", id: "civ60-fs", type: "button" }, "⛶ full screen"),
    );

    const hud = h("div", { class: "civ-hud", id: "civ60-hud", role: "status", "aria-label": "city resources" },
      hudChip("pop", PAL.pop, "citizens"),
      hudChip("food", PAL.food, "food"),
      hudChip("wood", PAL.wood, "wood"),
      hudChip("homes", PAL.homes, "homes"),
      hudChip("hungry", PAL.danger, "hungry"),
      hudChip("roofless", "#b98fd6", "roofless"),
      hudChip("day", null, "day"),
      hudChip("dec", null, "dec/s"),
      hudChip("asks", null, "engine asks"),
      hudChip("ledger", null, "ledger"),
      h("span", { class: "civ-chip civ-alert", id: "civ60-alert", hidden: "" }));

    const goal = h("p", { class: "civ-goal", id: "civ60-goal" },
      h("b", null, "The goal. "), GOAL);

    const canvas = h("canvas", { id: "civ60-canvas", width: String(W), height: String(H),
      role: "img",
      "aria-label": "AI Civilization in 60 Seconds — a drawn town of homes, farms, granary and palisade; citizens appear where they work and a verdict line narrates the run" });
    const chart = h("canvas", { id: "civ60-chart", width: String(CW), height: String(CH),
      role: "img",
      "aria-label": "The ledger — food, wood, population and homes over time, one sample every five sim-seconds" });

    const veil = h("div", { class: "game-veil", id: "civ60-veil", role: "group",
      "aria-label": "Simulation paused" },
      h("button", { class: "game-veil-chip", type: "button", id: "civ60-veil-chip" }, "▶ press play"),
      h("span", { class: "game-veil-hint", id: "civ60-veil-hint" }, BOOT_HINT));

    const status = h("div", { class: "game-hintline civ-status", id: "civ60-status",
      "aria-live": "polite" }, "tick 0 · t+0.0s · decisions 0 · 0.0 dec/s");
    const trace = h("div", { class: "game-hintline", id: "civ60-trace",
      "aria-live": "polite" }, "no decisions yet — click a citizen to follow it");

    const legend = h("div", { class: "civ-legend", id: "civ60-legend" },
      LEGEND_ORDER.map((n) => h("span", { class: "civ-legend-item" },
        h("i", { class: "civ-sw", style: "background:" + BEHAVIOR_COLOR[n] }), n)),
      h("span", { class: "civ-legend-item civ-legend-note" },
        "ring: red = starving · grey = exhausted · yellow = followed"));

    const verdict = h("div", { class: "civ-verdict", id: "civ60-verdict", hidden: "" },
      h("div", { class: "civ-verdict-head" }, "⚑ verdict ",
        h("span", { class: "civ-verdict-title", id: "civ60-verdict-title" }, "")),
      h("div", { class: "civ-verdict-body", id: "civ60-verdict-body" }));

    const center = h("div", { class: "game-center" },
      goal,
      h("div", { class: "game-view" }, canvas, veil),
      status, chart, legend, trace, verdict);

    const chron = h("div", { class: "civ-logpanel" },
      h("div", { class: "civ-logpanel-head" }, "chronicle",
        h("span", { class: "civ-logpanel-count", id: "civ60-log-count" }, "0 events")),
      h("div", { class: "civ-logpanel-body", id: "civ60-log", "aria-live": "polite" },
        h("div", { class: "civ-log-line civ-log-empty" }, "nothing has happened yet — press play.")));

    const opts = DK.shell.accordion([
      {
        id: "city", label: "The city", open: true,
        kids: [
          slider("civ60-citizens", "citizens", 4, 60, 1, 12),
          slider("civ60-budget", "engine budget", 0, 10, 1, 2, "/s"),
          select("civ60-events", "event buttons",
            [["on", "on — inject events freely"], ["off", "off — no events this run"]], "on"),
          h("p", { class: "game-note" },
            "Changing anything restarts the city from tick 0 with the same seed — ",
            "runs are deterministic, so a budget change is a controlled experiment."),
        ],
      },
      {
        id: "events", label: "Inject an event",
        kids: [
          h("div", { class: "game-bar", id: "civ60-event-buttons" },
            eventButton("shortage", "☠ shortage"),
            eventButton("harvest", "🌾 harvest"),
            eventButton("raid", "⚔ raid"),
            eventButton("festival", "✦ festival"),
            eventButton("migrants", "➕ migrants")),
          h("p", { class: "game-note" },
            "Events are how you read the cascade. A shortage empties the ",
            "granary → hunger climbs → FARM and EAT take over → wood production ",
            "slows → construction stalls → homelessness grows. A raid rallies ",
            "the guard line — enough FIGHT decisions repel it, fear makes ",
            "citizens FLEE. The buttons need a live bridge? No — events are ",
            "yours, the engine only arbitrates near-ties."),
          h("p", { class: "game-note", id: "civ60-events-note" }, ""),
        ],
      },
      {
        id: "follow", label: "Follow one citizen",
        kids: [
          h("p", { class: "game-note" },
            "Click a citizen in the scene to pin its decision stream to the ",
            "trace line below the ledger. Citizens stand where they work: ",
            "farmers in the fields, woodcutters in the timber stand, guards on ",
            "the palisade, sleepers at their own front doors. Click empty ",
            "ground to release. The board auto-records a row at every 500th ",
            "decision."),
          h("button", { class: "mz-chip", id: "civ60-release", type: "button" },
            "release followed citizen"),
        ],
      },
      {
        id: "about", label: "What decides what?",
        kids: [h("p", { class: "game-note" },
          "Every citizen decides ~1.3×/second — but not every decision is a ",
          "question. SLEEP (energy drained), urgent EAT (hunger ≥ 80, food on ",
          "hand) and HAVE-CHILD (surplus + a roof to raise them under) are ",
          "RULES, never asked. The rest is scored on the snapshot: farm vs ",
          "work vs build vs trade vs explore — plus fight and flee when a raid ",
          "is on, and rationing-eat when hunger is climbing on a thin granary. ",
          "When the top two land within 1.2 value points the ENGINE arbitrates… ",
          "if the city has a token left. The batching budget refills at N asks ",
          "per sim-second; no token → the argmax rule decides and the rung ",
          "says so. Population stays coherent by construction: pop = start + ",
          "births + migrants − deaths − fled — the verdict panel checks it ",
          "every frame.")]
      },
    ]);

    const main = h("div", { class: "game-layout" },
      center,
      h("div", { class: "game-side" }, chron, opts));
    root.appendChild(h("div", { class: "game-wrap" }, bar, hud, main));

    buildOverlays();
    wire();
    S.seedKit = DK.shell.numericSeed(byId("civ60-seed"), byId("civ60-dice"), newGame);
    DK.shell.resetParams(byId("civ60-reset-params"), root);
    DK.shell.fullscreen(root.querySelector(".game-wrap"), byId("civ60-fs"));
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
    root.appendChild(overlay("civ60-board-overlay", "Session scoreboard", "civ60-board-body", "civ60-board-close"));
    root.appendChild(overlay("civ60-stats-overlay", "Measured stats", "civ60-stats-body", "civ60-stats-close"));
  }

  function renderBoard() {
    const body = byId("civ60-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" },
        "No milestones yet — the board records a row automatically at every ",
        "500th decision. Steer the city through a shortage and watch it fill. ",
        "Session only."));
      return;
    }
    body.appendChild(h("table", { class: "game-table" },
      h("thead", null, h("tr", null,
        ...["#", "milestone", "pop", "food", "dec/s", "engine", "coherent", "seed"].map((x) => h("th", null, x)))),
      h("tbody", null, rows.map((r, i) => h("tr", null,
        h("td", null, String(rows.length - i)),
        h("td", null, String(r.milestone)),
        h("td", null, String(r.pop)),
        h("td", null, String(r.food)),
        h("td", null, String(r.perSecond)),
        h("td", null, String(r.engineCalls)),
        h("td", null, r.coherent ? "yes" : "NO"),
        h("td", null, r.seed))))));
    body.appendChild(h("button", {
      class: "mz-chip", type: "button", onclick: () => { S.board.clear(); renderBoard(); },
    }, "clear board"));
  }

  function renderStats() {
    const body = byId("civ60-stats-body");
    if (!body) return;
    body.textContent = "";
    const snap = S.meters.snapshot();
    const g = S.game;
    const s = g ? CC.runSummary(g) : null;
    const rows = [
      ["frames/s (EMA)", S.fps.toFixed(0)],
      ["sim ticks", String(g ? g.ticks : 0)],
      ["sim seconds", s ? String(s.simSeconds) : "0"],
      ["citizens", s ? String(s.pop) : "0"],
      ["decisions — total", s ? String(s.decisions) : "0"],
      ["decisions/s (sim time)", s ? String(s.perSecond) : "0"],
      ["decisions — rule ladder", s ? String(s.rungs.rule) : "0"],
      ["decisions — engine", s ? String(s.rungs.engine) : "0"],
      ["engine calls (near-ties)", s ? String(s.engineCalls) : "0"],
      ["ask budget", g ? g.cfg.askBudget + "/s (tokens " + g.tokens.toFixed(2) + ")" : "—"],
      ["food / wood", s ? s.food + " / " + s.wood : "—"],
      ["homes", s ? String(s.homes) : "0"],
      ["births / deaths / fled", s ? s.births + " / " + s.deaths + " / " + s.fled : "—"],
      ["migrants / repelled raids", s ? s.migrants + " / " + s.repelled : "—"],
      ["scout discoveries (food)", s ? String(s.discovered) : "0"],
      ["coherence", s ? (s.coherent ? "holds: pop = " + (g ? CC.coherentPop(g) : 0)
        : "BROKEN") : "—"],
      ["log entries", s ? String(s.events) : "0"],
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
    if (S.game && S.game.over) setRunning(false);
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
    CC.tickGame(g, engine.status === "ready" ? engineChoose : null);
    /* board rows land as milestones happen, never mid-decision */
    while (g.decisions >= (S.pushedMilestones + 1) * MILESTONE_EVERY) {
      S.pushedMilestones += 1;
      pushBoard(g);
      if (S.boardOpen) renderBoard();
    }
  }

  /* ---------- scenery (drawn per seed, no image files) ---------- */
  function makeDecor(seed) {
    const rng = DK.makeRng("scene:" + seed);
    const inBox = (b, pad) => ({
      x: b.x + pad + rng() * Math.max(1, b.w - pad * 2),
      y: b.y + pad + rng() * Math.max(1, b.h - pad * 2),
    });
    const trees = [];
    for (let i = 0; i < 9; i++) {
      const p = inBox({ x: 52, y: 186, w: 240, h: 112 }, 10);
      trees.push({ x: p.x, y: p.y, s: 0.85 + rng() * 0.5, alt: i % 2 === 0 });
    }
    for (let i = 0; i < 7; i++) {
      trees.push({ x: 66 + rng() * 270, y: 476 + rng() * 16, s: 0.7 + rng() * 0.35, alt: i % 2 === 1 });
    }
    const ripples = [];
    for (let i = 0; i < 6; i++) ripples.push({ y: 176 + i * 56 + rng() * 18, w: 18 + rng() * 26 });
    const hills = [];
    for (let i = 0; i < 9; i++) {
      hills.push({ x: -30 + i * 118 + rng() * 40, r: 26 + rng() * 26, far: i % 2 === 0 });
    }
    const tufts = [];
    for (let i = 0; i < 46; i++) {
      tufts.push({ x: 24 + rng() * (W - 48), y: 168 + rng() * (H - 190), n: 2 + Math.floor(rng() * 2) });
    }
    const clouds = [];
    for (let i = 0; i < 4; i++) {
      clouds.push({
        x: 60 + rng() * (W - 200), y: 24 + rng() * 54,
        s: 0.7 + rng() * 0.7, puff: 3 + Math.floor(rng() * 2),
      });
    }
    return { trees, ripples, hills, tufts, clouds };
  }

  function homeSlot(i) {
    /* the first twelve homes sit on the town grid; past that, smaller
       infill houses pack the same blocks — the scene must hold every home */
    if (i < 12) {
      const col = i % 4, row = Math.floor(i / 4);
      return { x: 348 + col * 80, y: 186 + row * 74, w: 62, h: 52 };
    }
    const k = i - 12;
    const col = k % 4, row = Math.floor(k / 4);
    return { x: 358 + col * 80, y: 206 + row * 74, w: 44, h: 38 };
  }

  function drawTree(ctx, x, y, s, alt) {
    ctx.strokeStyle = PAL.trunk;
    ctx.lineWidth = Math.max(1.4, 2.1 * s);
    ctx.beginPath(); ctx.moveTo(x, y); ctx.lineTo(x, y - 11 * s); ctx.stroke();
    ctx.fillStyle = alt ? PAL.leaf2 : PAL.leaf;
    for (let i = 0; i < 3; i++) {
      const w = (11 - i * 2.8) * s;
      const yy = y - 9 * s - i * 7.5 * s;
      ctx.beginPath();
      ctx.moveTo(x, yy - 12 * s);
      ctx.lineTo(x + w, yy);
      ctx.lineTo(x - w, yy);
      ctx.closePath(); ctx.fill();
    }
  }

  function drawHome(ctx, x, y, w, hh, lit) {
    const wallY = y + hh * 0.4, wallH = hh * 0.6;
    ctx.fillStyle = PAL.wall;
    ctx.strokeStyle = PAL.wallEdge;
    ctx.lineWidth = 1.1;
    ctx.fillRect(x, wallY, w, wallH);
    ctx.strokeRect(x, wallY, w, wallH);
    ctx.fillStyle = lit ? PAL.windowLit : PAL.windowDay;
    ctx.fillRect(x + w * 0.16, wallY + wallH * 0.22, Math.max(4, w * 0.14), Math.max(4, wallH * 0.3));
    ctx.fillStyle = PAL.door;
    ctx.fillRect(x + w * 0.52, y + hh * 0.62, Math.max(6, w * 0.2), hh * 0.38);
    ctx.fillStyle = PAL.roof;
    ctx.beginPath();
    ctx.moveTo(x - 5, wallY);
    ctx.lineTo(x + w / 2, y);
    ctx.lineTo(x + w + 5, wallY);
    ctx.closePath(); ctx.fill();
    ctx.strokeStyle = PAL.roof2;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(x - 5, wallY); ctx.lineTo(x + w + 5, wallY);
    ctx.stroke();
  }

  function drawGranary(ctx, g, s, lit) {
    const { x, y, w, h: hh } = { x: 690, y: 186, w: 78, h: 66 };
    ctx.fillStyle = PAL.stone;
    ctx.strokeStyle = PAL.stoneEdge;
    ctx.lineWidth = 1.2;
    ctx.fillRect(x, y + 22, w, hh - 22);
    ctx.strokeRect(x, y + 22, w, hh - 22);
    /* banding: a silo, not a shed */
    ctx.strokeStyle = "rgba(0,0,0,0.22)";
    ctx.lineWidth = 1;
    for (let i = 1; i < 4; i++) {
      ctx.beginPath();
      ctx.moveTo(x, y + 22 + i * ((hh - 22) / 4));
      ctx.lineTo(x + w, y + 22 + i * ((hh - 22) / 4));
      ctx.stroke();
    }
    ctx.fillStyle = PAL.roof2;
    ctx.beginPath();
    ctx.moveTo(x - 5, y + 22);
    ctx.lineTo(x + w / 2, y + 2);
    ctx.lineTo(x + w + 5, y + 22);
    ctx.closePath(); ctx.fill();
    ctx.fillStyle = lit ? PAL.windowLit : PAL.door;
    ctx.fillRect(x + w / 2 - 6, y + hh - 18, 12, 18);
    /* the fill gauge: food against a full granary (4 days × citizens) */
    const cap = Math.max(4, CC.pop(g) * 4);
    const frac = Math.max(0, Math.min(1, g.food / cap));
    const gx = x + w + 6, gy = y + 26, gw = 8, gh = hh - 46;
    ctx.strokeStyle = PAL.stoneEdge;
    ctx.strokeRect(gx, gy, gw, gh);
    ctx.fillStyle = PAL.food;
    ctx.fillRect(gx + 1, gy + 1 + (gh - 2) * (1 - frac), gw - 2, (gh - 2) * frac);
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 9px ui-monospace, monospace";
    ctx.textAlign = "left";
    ctx.fillText("granary " + Math.round(g.food), x - 4, y + hh + 14);
  }

  function drawMarket(ctx, lit) {
    const x = 694, y = 330, w = 88, hh = 52;
    ctx.strokeStyle = PAL.post;
    ctx.lineWidth = 3;
    ctx.beginPath();
    ctx.moveTo(x + 5, y + hh); ctx.lineTo(x + 5, y + 10);
    ctx.moveTo(x + w - 5, y + hh); ctx.lineTo(x + w - 5, y + 10);
    ctx.stroke();
    ctx.fillStyle = PAL.wall;
    ctx.fillRect(x + 2, y + hh - 14, w - 4, 8);
    ctx.strokeStyle = PAL.wallEdge;
    ctx.lineWidth = 1;
    ctx.strokeRect(x + 2, y + hh - 14, w - 4, 8);
    /* the goods: food left, timber right */
    ctx.fillStyle = PAL.food;
    for (let i = 0; i < 3; i++) {
      ctx.beginPath(); ctx.arc(x + 14 + i * 10, y + hh - 18, 3.2, 0, 7); ctx.fill();
    }
    ctx.fillStyle = PAL.wood;
    for (let i = 0; i < 2; i++) ctx.fillRect(x + w - 34 + i * 9, y + hh - 21, 7, 5);
    /* striped awning with a scalloped edge */
    const stripes = 5, sw = (w + 10) / stripes;
    for (let i = 0; i < stripes; i++) {
      ctx.fillStyle = i % 2 ? PAL.awning1 : PAL.awning0;
      ctx.fillRect(x - 5 + i * sw, y, sw, 11);
      ctx.beginPath();
      ctx.arc(x - 5 + i * sw + sw / 2, y + 11, sw / 2, 0, Math.PI);
      ctx.fill();
    }
    ctx.strokeStyle = PAL.wallEdge;
    ctx.strokeRect(x - 5, y, w + 10, 11);
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 9px ui-monospace, monospace";
    ctx.textAlign = "left";
    ctx.fillText("market" + (lit ? " · closed" : ""), x - 4, y + hh + 16);
  }

  function drawPalisade(ctx, g, raid) {
    /* a timber stockade: vertical planks with pointed tips, a walkway rail,
       and a gate gap where the road leaves town */
    const x = 822, y0 = 186, y1 = H;
    const w = 16, half = w / 2;
    const seg = (a, b) => {
      const wood = ctx.createLinearGradient(x - half, 0, x + half, 0);
      wood.addColorStop(0, PAL.post);
      wood.addColorStop(1, "#5c4a30");
      ctx.fillStyle = wood;
      ctx.fillRect(x - half, a + 8, w, b - a - 8);
      ctx.fillStyle = "rgba(0, 0, 0, 0.26)";
      for (let px = x - half + 4; px < x + half; px += 4) ctx.fillRect(px, a + 8, 1, b - a - 8);
      /* pointed tips along the top of this segment */
      ctx.fillStyle = PAL.post;
      for (let py = a; py < b - 12; py += 12) {
        ctx.beginPath();
        ctx.moveTo(x - half, py + 10);
        ctx.lineTo(x, py);
        ctx.lineTo(x + half, py + 10);
        ctx.closePath();
        ctx.fill();
      }
      ctx.strokeStyle = "rgba(0, 0, 0, 0.4)";
      ctx.lineWidth = 1;
      ctx.beginPath(); ctx.moveTo(x - half, a + 10); ctx.lineTo(x + half, a + 10); ctx.stroke();
    };
    seg(y0, 318);
    seg(364, y1);
    /* the gate: two heavier posts and a lintel over the road */
    ctx.fillStyle = "#6b5636";
    ctx.fillRect(x - half - 3, 318, 5, 46);
    ctx.fillRect(x + half - 2, 318, 5, 46);
    ctx.fillRect(x - half - 3, 314, w + 6, 5);
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 9px ui-monospace, monospace";
    ctx.textAlign = "right";
    ctx.fillText("palisade", x - half - 6, y0 + 4);
    if (!raid) return;
    /* raiders across the water + fire on the timbers */
    for (let i = 0; i < g.raid.raiders; i++) {
      drawPerson(ctx, 906 + (i % 2) * 16, 220 + i * 78, PAL.raider, "spear");
    }
    for (let i = 0; i < 3; i++) drawFlame(ctx, x + 2, 250 + i * 84, 1 + (i % 2) * 0.35);
  }

  function drawFlame(ctx, x, y, s) {
    for (const [col, k, off] of [[PAL.flame, 1, 0], [PAL.flameCore, 0.55, 1.5]]) {
      ctx.fillStyle = col;
      ctx.beginPath();
      ctx.moveTo(x + off, y - 17 * s * k);
      ctx.quadraticCurveTo(x + 6 * s * k + off, y - 6 * s * k, x + off, y);
      ctx.quadraticCurveTo(x - 6 * s * k + off, y - 6 * s * k, x + off, y - 17 * s * k);
      ctx.fill();
    }
  }

  function drawScene(ctx, g) {
    const s = CC.runSummary(g);
    const sec = simSeconds(g);
    const phase = dayPhase(sec);
    const sky = skyColors(phase);
    const lit = phaseName(phase) === "night" || phaseName(phase) === "dusk";

    /* sky */
    const grad = ctx.createLinearGradient(0, 0, 0, 150);
    grad.addColorStop(0, sky[0]);
    grad.addColorStop(1, sky[1]);
    ctx.fillStyle = grad;
    ctx.fillRect(0, 0, W, 150);

    /* sun or moon on its arc */
    const arc = 0.15 + 0.7 * (phaseName(phase) === "night"
      ? (phase - 0.62) / 0.26 : phase / 0.5);
    const bx = 90 + arc * (W - 180);
    const by = 132 - Math.sin(Math.PI * Math.min(1, Math.max(0, arc))) * 92;
    if (phaseName(phase) === "night") {
      ctx.fillStyle = "#dbe4f5";
      ctx.beginPath(); ctx.arc(bx, by, 11, 0, 7); ctx.fill();
      ctx.fillStyle = sky[0];
      ctx.beginPath(); ctx.arc(bx + 5, by - 3, 9, 0, 7); ctx.fill();
    } else {
      ctx.fillStyle = phaseName(phase) === "day" ? "#ffe9a8" : "#ffc078";
      ctx.beginPath(); ctx.arc(bx, by, 13, 0, 7); ctx.fill();
      ctx.strokeStyle = phaseName(phase) === "day" ? "rgba(255, 233, 168, 0.55)" : "rgba(255, 192, 120, 0.5)";
      ctx.lineWidth = 2;
      ctx.beginPath(); ctx.arc(bx, by, 19, 0, 7); ctx.stroke();
      ctx.fillStyle = "rgba(255, 233, 168, 0.14)";
      ctx.beginPath(); ctx.arc(bx, by, 28, 0, 7); ctx.fill();
    }

    /* seeded clouds, dimmer after dark */
    ctx.fillStyle = lit ? "rgba(190, 205, 235, 0.16)" : "rgba(255, 255, 255, 0.5)";
    for (const c of S.decor.clouds) {
      for (let k = 0; k < c.puff; k++) {
        ctx.beginPath();
        ctx.arc(c.x + k * 15 * c.s, c.y + (k % 2) * 4, 11 * c.s * (k % 2 ? 0.8 : 1), 0, 7);
        ctx.fill();
      }
      ctx.fillRect(c.x - 12 * c.s, c.y + 2, c.puff * 15 * c.s + 8, 7 * c.s);
    }

    /* ground first, then a low rolling horizon behind it */
    const ground = ctx.createLinearGradient(0, 150, 0, H);
    ground.addColorStop(0, PAL.grassLite);
    ground.addColorStop(1, PAL.grassDark);
    ctx.fillStyle = ground;
    ctx.fillRect(0, 150, W, H - 150);
    ctx.fillStyle = PAL.hill;
    for (const hl of S.decor.hills) {
      if (!hl.far) continue;
      ctx.beginPath();
      ctx.arc(hl.x, 154, hl.r, Math.PI, 0);
      ctx.fill();
    }
    ctx.fillStyle = PAL.hill2;
    for (const hl of S.decor.hills) {
      if (hl.far) continue;
      ctx.beginPath();
      ctx.arc(hl.x, 158, hl.r * 0.8, Math.PI, 0);
      ctx.fill();
    }

    /* a worn path from the gate to the square, and grass tufts for texture */
    ctx.strokeStyle = PAL.path;
    ctx.globalAlpha = 0.2;
    ctx.lineWidth = 8;
    ctx.lineCap = "round";
    ctx.beginPath();
    ctx.moveTo(818, 344);
    ctx.quadraticCurveTo(680, 380, 540, 448);
    ctx.stroke();
    ctx.globalAlpha = 1;
    ctx.lineCap = "butt";
    ctx.strokeStyle = PAL.tuft;
    ctx.lineWidth = 1.2;
    for (const t of S.decor.tufts) {
      ctx.beginPath();
      for (let k = 0; k < t.n; k++) {
        ctx.moveTo(t.x + k * 3, t.y);
        ctx.lineTo(t.x + k * 3 + 1.5, t.y - 4);
      }
      ctx.stroke();
    }

    /* river, right edge: far bank, water, then ripples drifting down it */
    ctx.fillStyle = PAL.water;
    ctx.fillRect(896, 150, W - 896, H - 150);
    const waterGrad = ctx.createLinearGradient(896, 0, W, 0);
    waterGrad.addColorStop(0, "rgba(255, 255, 255, 0)");
    waterGrad.addColorStop(1, "rgba(0, 0, 0, 0.22)");
    ctx.fillStyle = waterGrad;
    ctx.fillRect(896, 150, W - 896, H - 150);
    ctx.strokeStyle = PAL.waterLite;
    ctx.lineWidth = 2;
    ctx.beginPath(); ctx.moveTo(896, 150); ctx.lineTo(896, H); ctx.stroke();
    ctx.lineWidth = 1.4;
    for (const r of S.decor.ripples) {
      ctx.beginPath();
      ctx.moveTo(908, r.y);
      ctx.quadraticCurveTo(908 + r.w / 2, r.y - 3, 908 + r.w, r.y);
      ctx.stroke();
      ctx.beginPath();
      ctx.moveTo(918, r.y + 22);
      ctx.quadraticCurveTo(918 + r.w / 2, r.y + 19, 918 + r.w * 0.8, r.y + 22);
      ctx.stroke();
    }

    /* farm field: four plots, each a raised bed with crop rows */
    for (let p = 0; p < 4; p++) {
      const px = 48 + (p % 2) * 131, py = 332 + Math.floor(p / 2) * 65;
      const pw = 123, ph = 57;
      ctx.fillStyle = PAL.soil;
      ctx.fillRect(px, py, pw, ph);
      ctx.strokeStyle = "rgba(0, 0, 0, 0.35)";
      ctx.lineWidth = 1.5;
      ctx.strokeRect(px, py, pw, ph);
      ctx.lineCap = "round";
      for (let r = 0; r < 4; r++) {
        const ry = py + 12 + r * 12;
        ctx.strokeStyle = r % 2 ? PAL.crop : PAL.cropLite;
        ctx.lineWidth = 3;
        ctx.beginPath();
        ctx.moveTo(px + 8, ry);
        ctx.lineTo(px + pw - 8, ry);
        ctx.stroke();
      }
      ctx.lineCap = "butt";
    }
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 9px ui-monospace, monospace";
    ctx.textAlign = "left";
    ctx.fillText("farms", 46, 326);

    /* timber stand */
    for (const t of S.decor.trees) drawTree(ctx, t.x, t.y, t.s, t.alt);
    ctx.fillStyle = PAL.dim;
    ctx.fillText("timber", 46, 180);

    /* homes: one sprite per home the city actually has */
    const drawn = Math.min(g.homes, 24);
    for (let i = 0; i < drawn; i++) {
      const slot = homeSlot(i);
      drawHome(ctx, slot.x, slot.y, slot.w, slot.h, lit);
    }
    if (g.homes > 24) {
      ctx.fillStyle = PAL.dim;
      ctx.font = "600 10px ui-monospace, monospace";
      ctx.fillText("+" + (g.homes - 24) + " more homes", 348, 420);
    }
    /* the next plot is marked: BUILD has somewhere to aim */
    if (g.homes < 24 && CC.homeless(g) >= 0) {
      const next = homeSlot(g.homes);
      ctx.strokeStyle = "rgba(245, 158, 66, 0.75)";
      ctx.lineWidth = 1.4;
      ctx.setLineDash([4, 3]);
      ctx.strokeRect(next.x, next.y + next.h * 0.4, next.w, next.h * 0.6);
      ctx.setLineDash([]);
    }

    drawGranary(ctx, g, s, lit);
    drawMarket(ctx, lit);
    drawPalisade(ctx, g, !!g.raid);

    /* festival bunting across the square */
    if (g.festival > 0) {
      ctx.strokeStyle = PAL.post;
      ctx.lineWidth = 1.4;
      ctx.beginPath(); ctx.moveTo(344, 168); ctx.quadraticCurveTo(500, 196, 660, 168); ctx.stroke();
      for (let i = 0; i < 8; i++) {
        const t = (i + 0.5) / 8;
        const bx = 344 + (660 - 344) * t;
        const by = 168 + (196 - 168) * Math.sin(Math.PI * t) * 0.9;
        ctx.fillStyle = i % 2 ? PAL.festival : PAL.awning0;
        ctx.beginPath();
        ctx.moveTo(bx - 5, by); ctx.lineTo(bx + 5, by); ctx.lineTo(bx, by + 9);
        ctx.closePath(); ctx.fill();
      }
    }

    /* time-of-day tint over everything drawn so far */
    const tint = {
      night: "rgba(9, 14, 38, 0.34)",
      dusk: "rgba(96, 44, 60, 0.20)",
      dawn: "rgba(140, 82, 44, 0.13)",
      day: null,
    }[phaseName(phase)];
    if (tint) {
      ctx.fillStyle = tint;
      ctx.fillRect(0, 0, W, H);
    }
    return lit;
  }

  function dayPhase(sec) {
    return (sec % DAY_S) / DAY_S;
  }
  function phaseName(phase) {
    if (phase < 0.5) return "day";
    if (phase < 0.62) return "dusk";
    if (phase < 0.88) return "night";
    return "dawn";
  }
  function skyColors(phase) {
    return {
      day: [PAL.skyDay0, PAL.skyDay1],
      dusk: [PAL.skyDusk0, PAL.skyDusk1],
      night: [PAL.skyNight0, PAL.skyNight1],
      dawn: [PAL.skyDusk0, PAL.skyDay1],
    }[phaseName(phase)];
  }

  /* ---------- people ---------- */
  function drawPerson(ctx, x, y, color, mode) {
    if (mode === "lie") {
      ctx.strokeStyle = color;
      ctx.lineWidth = 3.2;
      ctx.lineCap = "round";
      ctx.beginPath(); ctx.moveTo(x - 5, y); ctx.lineTo(x + 3, y); ctx.stroke();
      ctx.fillStyle = color;
      ctx.beginPath(); ctx.arc(x + 6.5, y, 2.8, 0, 7); ctx.fill();
      return;
    }
    ctx.strokeStyle = color;
    ctx.fillStyle = color;
    ctx.lineCap = "round";
    /* legs */
    ctx.lineWidth = 1.8;
    ctx.beginPath();
    ctx.moveTo(x, y); ctx.lineTo(x - 3, y + 5);
    ctx.moveTo(x, y); ctx.lineTo(x + 3, y + 5);
    ctx.stroke();
    /* the torso carries the behaviour colour */
    ctx.lineWidth = 4;
    ctx.beginPath(); ctx.moveTo(x, y - 7.5); ctx.lineTo(x, y + 0.5); ctx.stroke();
    /* a face, so the sprite reads as a person and not a pin */
    ctx.fillStyle = PAL.skin;
    ctx.beginPath(); ctx.arc(x, y - 11, 3.2, 0, 7); ctx.fill();
    ctx.strokeStyle = "rgba(0, 0, 0, 0.25)";
    ctx.lineWidth = 0.8;
    ctx.stroke();
    if (mode === "spear") {
      ctx.lineWidth = 1.3;
      ctx.beginPath(); ctx.moveTo(x + 5, y + 4); ctx.lineTo(x + 5, y - 14); ctx.stroke();
    } else if (mode === "carry") {
      ctx.fillStyle = PAL.carried;
      ctx.fillRect(x + 4, y - 9, 4, 4);
    }
  }

  /* citizens stand where they work — the scene is the decision distribution
     made spatial: farmers in the fields, guards on the palisade, sleepers
     at their own front doors */
  const AREA = {
    farm: [[74, 356], [116, 356], [158, 356], [200, 356], [74, 408], [116, 408],
      [158, 408], [200, 408], [242, 356], [242, 408]],
    work: [[84, 208], [136, 228], [196, 204], [248, 232], [104, 268], [164, 274],
      [224, 268], [268, 296], [124, 300], [200, 240]],
    trade: [[706, 404], [736, 410], [766, 404], [752, 418]],
    explore: [[866, 196], [874, 256], [866, 316], [876, 376], [866, 436], [872, 476]],
    fight: [[810, 196], [810, 252], [810, 308], [810, 364], [810, 420], [810, 476]],
    flee: [[88, 470], [136, 478], [184, 470], [232, 478], [272, 470], [306, 478]],
    eat: [[702, 266], [730, 272], [758, 266], [744, 282]],
    "have-child": [[566, 430], [598, 430], [630, 430], [582, 448], [614, 448]],
    rest: [[486, 432], [526, 444], [566, 454], [506, 458], [546, 430], [466, 448]],
  };

  function citizenSpot(g, index, c) {
    const b = c.behavior;
    if (b === "sleep" || b === "build") {
      /* sleepers at their own front door; builders on the next plot */
      const i = b === "sleep" ? index % Math.max(1, g.homes) : g.homes;
      const slot = homeSlot(Math.min(i, 23));
      return { x: slot.x + slot.w / 2 + (b === "sleep" ? 0 : 8), y: slot.y + slot.h + 8 };
    }
    const list = AREA[b] || AREA.rest;
    const p = list[index % list.length];
    const j = ((index * 37) % 9) - 4;
    return { x: p[0] + j * 0.8, y: p[1] + (j > 0 ? 2 : -2) };
  }

  function drawCitizens(ctx, g) {
    const perBehavior = {};
    g.citizens.forEach((c, i) => {
      (perBehavior[c.behavior] = perBehavior[c.behavior] || []).push([i, c]);
    });
    for (const [b, list] of Object.entries(perBehavior)) {
      list.forEach(([index, c], k) => {
        const spot = citizenSpot(g, k, c);
        const color = BEHAVIOR_COLOR[b] || "#e2e8f0";
        const mode = b === "sleep" ? "lie"
          : b === "fight" ? "spear"
          : b === "flee" ? "stand"
          : (b === "eat" || b === "trade" || b === "have-child") ? "carry"
          : "stand";
        drawPerson(ctx, spot.x, spot.y, color, mode);
        if (c.hunger >= 80) {
          ctx.strokeStyle = PAL.danger;
          ctx.lineWidth = 1.5;
          ctx.beginPath(); ctx.arc(spot.x, spot.y - 5, 11, 0, 7); ctx.stroke();
        } else if (c.energy <= 25) {
          ctx.strokeStyle = "rgba(139, 154, 184, 0.8)";
          ctx.lineWidth = 1;
          ctx.beginPath(); ctx.arc(spot.x, spot.y - 5, 10, 0, 7); ctx.stroke();
        }
        if (S.follow && c.id === S.follow) {
          ctx.strokeStyle = PAL.follow;
          ctx.lineWidth = 1.6;
          ctx.beginPath(); ctx.arc(spot.x, spot.y - 5, 14, 0, 7); ctx.stroke();
          ctx.fillStyle = PAL.ink;
          ctx.font = "700 10px ui-monospace, monospace";
          ctx.textAlign = "center";
          ctx.fillText(c.id + " · " + b, spot.x, spot.y - 24);
        }
      });
    }
  }

  /* ---------- the ledger ---------- */
  function drawChart(ctx, g) {
    ctx.fillStyle = "#0b1016";
    ctx.fillRect(0, 0, CW, CH);
    const { x0, x1, y0, y1 } = CHART;
    ctx.strokeStyle = "#1a2233";
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (let i = 0; i <= 2; i++) {
      const y = y0 + ((y1 - y0) * i) / 2;
      ctx.moveTo(x0, y); ctx.lineTo(x1, y);
    }
    ctx.stroke();

    const hist = g.history;
    ctx.font = "600 10px ui-monospace, monospace";
    if (hist.length < 2) {
      ctx.fillStyle = PAL.dim;
      ctx.textAlign = "left";
      ctx.fillText(hist.length === 1
        ? "the ledger — first sample in, the lines start at the second sample (10 sim-s)"
        : "the ledger — food, wood, population and homes; first sample at 5 sim-seconds", x0, y0 - 8);
      return;
    }
    const series = [
      ["food", PAL.food, (p) => p.food],
      ["wood", PAL.wood, (p) => p.wood],
      ["pop", PAL.pop, (p) => p.pop],
      ["homes", PAL.homes, (p) => p.homes],
    ];
    /* one shared scale — four lines each normalized to their own max cannot
       be compared, and an incomparable chart reads as nonsense */
    const max = Math.max(1, ...hist.map((p) => Math.max(p.food, p.wood, p.pop, p.homes)));
    for (const [name, color, get] of series) {
      ctx.beginPath();
      hist.forEach((p, i) => {
        const x = x0 + ((x1 - x0) * i) / (hist.length - 1);
        const y = y1 - ((y1 - y0 - 4) * get(p)) / max;
        if (i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
      });
      ctx.strokeStyle = color;
      ctx.lineWidth = 1.6;
      ctx.stroke();
      ctx.lineTo(x1, y1);
      ctx.lineTo(x0, y1);
      ctx.closePath();
      ctx.fillStyle = color;
      ctx.globalAlpha = 0.09;
      ctx.fill();
      ctx.globalAlpha = 1;
    }
    /* legend + latest values, on the right */
    series.forEach(([name, color, get], i) => {
      const ly = y0 + 2 + i * 21;
      ctx.fillStyle = color;
      ctx.fillRect(758, ly - 7, 8, 8);
      ctx.fillStyle = PAL.dim;
      ctx.textAlign = "left";
      ctx.fillText(name, 772, ly);
      ctx.fillStyle = PAL.ink;
      ctx.font = "700 11px ui-monospace, monospace";
      ctx.textAlign = "right";
      ctx.fillText(String(get(hist[hist.length - 1])), 862, ly);
      ctx.font = "600 10px ui-monospace, monospace";
    });
    ctx.fillStyle = PAL.dim;
    ctx.textAlign = "left";
    ctx.fillText("the ledger · 0–" + max + " · 1 sample / 5 sim-s · t="
      + Math.round(simSeconds(g)) + "s", x0, y0 - 8);
  }

  /* ---------- narration ---------- */
  /* the plain-language read: what is happening, and what your move is.
     Derived only from the deterministic snapshot — never a question. */
  function cityVerdict(g, s) {
    const hungry = g.citizens.filter((c) => c.hunger >= 80).length;
    const roofless = CC.homeless(g);
    if (g.over) return { tone: PAL.danger,
      text: "the city is empty — every life was accounted for: pop = start + births + migrants − deaths − fled" };
    if (g.raid) return { tone: PAL.danger,
      text: "⚔ raid — each guard is scoring FIGHT vs FLEE right now (" + CC.defenders(g) + " defending)" };
    if (hungry > 0) return { tone: PAL.danger,
      text: hungry + " starving — urgent EAT and FARM take the queue while wood and building wait" };
    if (roofless > 0) return { tone: PAL.wood,
      text: roofless + " without a roof — BUILD is score-favored until homes catch up (" + s.homes + " built)" };
    if (s.food < s.pop * 2) return { tone: PAL.festival,
      text: "granary is thin (" + s.food + " food for " + s.pop + ") — FARM is climbing the scores" };
    return { tone: PAL.ok,
      text: "steady — farms, workshops and roofs in balance · " + s.decisions + " decisions so far" };
  }

  function nextMove(g, s) {
    const hungry = g.citizens.filter((c) => c.hunger >= 80).length;
    if (g.over) return "your move: ↺ restart, or change citizens / budget — the same seed replays the city exactly";
    if (!S.running && s.simSeconds === 0)
      return "your move: press ▶ play — the city runs itself; the event buttons are your experiment levers";
    if (g.raid) return "your move: watch the palisade — enough FIGHT repels it, fear sends citizens FLEE";
    if (hungry > 0) return "your move: watch the cascade — hunger → EAT/FARM → wood stalls → BUILD waits";
    if (CC.homeless(g) > 0) return "your move: click a citizen to pin one builder's decision stream to the trace";
    return "your move: inject ☠ shortage — one event bends every behavior at once";
  }

  function drawNarration(ctx, g, s) {
    const v = cityVerdict(g, s);
    ctx.fillStyle = PAL.scrim;
    ctx.fillRect(0, 0, W, 42);
    ctx.textAlign = "left";
    ctx.fillStyle = v.tone;
    ctx.font = "700 13px ui-monospace, monospace";
    ctx.fillText(v.text, 14, 18);
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 11px ui-monospace, monospace";
    ctx.fillText(nextMove(g, s), 14, 34);
  }

  /* ---------- DOM readouts ---------- */
  function setHud(id, value, warn) {
    const el = byId("civ60-hud-" + id);
    if (!el) return;
    const b = el.querySelector("b");
    if (b) b.textContent = value;
    el.classList.toggle("warn", !!warn);
    el.classList.toggle("mute", warn === null);
  }

  function syncHud(g, s) {
    const hungry = g.citizens.filter((c) => c.hunger >= 80).length;
    const roofless = CC.homeless(g);
    setHud("pop", String(s.pop));
    setHud("food", String(s.food), s.food < s.pop * 2);
    setHud("wood", String(s.wood));
    setHud("homes", String(s.homes), roofless > 0);
    setHud("hungry", String(hungry), hungry > 0);
    setHud("roofless", String(roofless), roofless > 0);
    setHud("day", "day " + (Math.floor(simSeconds(g) / DAY_S) + 1) + " · " + phaseName(dayPhase(simSeconds(g))));
    setHud("dec", s.perSecond.toFixed(1));
    setHud("asks", String(s.engineCalls));
    setHud("ledger", s.coherent ? "balanced" : "BROKEN", s.coherent ? false : true);

    const alert = byId("civ60-alert");
    if (alert) {
      if (g.raid) {
        alert.hidden = false;
        alert.textContent = "⚔ RAID · " + CC.defenders(g) + " defenders vs "
          + g.raid.raiders + " raiders · "
          + Math.max(0, Math.ceil(g.raid.left / CC.TICKS_PER_S)) + "s";
      } else if (g.festival > 0) {
        alert.hidden = false;
        alert.textContent = "✦ festival ×1.25 · " + Math.ceil(g.festival / CC.TICKS_PER_S) + "s";
      } else {
        alert.hidden = true;
      }
    }
  }

  function syncStatus(g, s) {
    const el = byId("civ60-status");
    if (!el) return;
    el.textContent = "tick " + g.ticks + " · t+" + simSeconds(g).toFixed(1) + "s · decisions "
      + s.decisions + " · " + s.perSecond.toFixed(1) + " dec/s · citizens " + s.pop
      + " · food " + s.food + " · wood " + s.wood + " · homes " + s.homes;
  }

  /* the chronicle: newest on top, bounded in the DOM */
  function syncLog(g) {
    const body = byId("civ60-log");
    if (!body) return;
    if (g.log.length === S.logSeen) return;
    const fresh = g.log.slice(S.logSeen);
    S.logSeen = g.log.length;
    const empty = body.querySelector(".civ-log-empty");
    if (empty) empty.remove();
    for (const l of fresh) {
      body.insertBefore(h("div", { class: "civ-log-line" },
        h("span", { class: "civ-log-t" }, "t+" + Math.round(l.tick / CC.TICKS_PER_S) + "s "),
        l.note), body.firstChild);
    }
    while (body.children.length > LOG_MAX) body.removeChild(body.lastChild);
    const count = byId("civ60-log-count");
    if (count) count.textContent = g.log.length + (g.log.length === 1 ? " event" : " events");
  }

  /* the verdict: every number off the deterministic snapshot */
  function syncVerdict() {
    const panel = byId("civ60-verdict");
    const g = S.game;
    if (!panel || !g) return;
    const s = CC.runSummary(g);
    const over = g.over;
    panel.hidden = !(over || S.verdictOpen);
    if (!over && !S.verdictOpen) return;
    const peakFood = g.history.length ? Math.max(...g.history.map((p) => p.food)) : s.food;
    byId("civ60-verdict-title").textContent = over
      ? "the run is over at t+" + s.simSeconds + "s"
      : "so far, at t+" + s.simSeconds + "s";
    const body = byId("civ60-verdict-body");
    body.textContent = "";
    const head = over
      ? (s.pop === 0 ? "Collapse. The last citizen is gone." : "The run stopped.")
      : "Nothing has collapsed yet — this is the city as it stands.";
    body.appendChild(h("p", { class: "civ-verdict-lede" }, head));
    const rows = [
      ["population", s.pop + " = " + g.cfg.citizens + " start + " + s.births
        + (s.births === 1 ? " birth + " : " births + ") + s.migrants + " migrants − " + s.deaths
        + (s.deaths === 1 ? " death − " : " deaths − ") + s.fled + " fled"],
      ["coherence", s.coherent ? "holds — the ledger balances exactly"
        : "BROKEN — the identity does not close, which would be a defect"],
      ["homes", s.homes + " built · " + CC.homeless(g) + " citizens without one"],
      ["granary", s.food + " food · peak " + peakFood + " · wood " + s.wood],
      ["decisions", s.decisions + " total · " + s.perSecond + "/s · rule ladder "
        + s.rungs.rule + " · engine-honored " + s.rungs.engine + " (" + s.engineCalls + " asks)"],
      ["the world", s.repelled + (s.repelled === 1 ? " raid" : " raids") + " repelled · "
        + s.discovered + " food scouted · " + s.events
        + (s.events === 1 ? " event" : " events") + " in the chronicle"],
      ["your hand", (g.cfg.events ? "events on" : "events off") + " · budget "
        + g.cfg.askBudget + " asks/s · seed " + g.cfg.seed],
    ];
    body.appendChild(h("div", { class: "mz-stats-grid" }, rows.map(([k, v]) =>
      h("div", { class: "mz-stats-row" },
        h("span", { class: "mz-stats-label" }, k),
        h("span", { class: "mz-stats-value" }, v)))));
  }

  /* ---------- render ---------- */
  function render() {
    const canvas = byId("civ60-canvas");
    if (!canvas || !S.game) return;
    const g = S.game;
    const ctx = canvas.getContext("2d");
    const s = CC.runSummary(g);

    drawScene(ctx, g);
    drawCitizens(ctx, g);
    drawNarration(ctx, g, s);
    drawChart(byId("civ60-chart").getContext("2d"), g);

    syncHud(g, s);
    syncStatus(g, s);
    syncLog(g);
    syncVerdict();

    const ld = g.lastDecision;
    if (ld) {
      const prefix = S.follow
        ? (ld.id === S.follow ? "followed citizen → " : "following " + S.follow + " (last ask elsewhere) · ")
        : "";
      const at = ld.at || {};
      byId("civ60-trace").textContent = prefix + "last decision: " + ld.id +
        " (hunger " + at.hunger + ", energy " + at.energy + ", fear " + at.fear +
        (ld.evGap === null || ld.evGap === undefined ? "" : ", gap " +
          (ld.evGap === Infinity ? "∅" : ld.evGap)) + ") — " +
        ld.choice + " · " + (ld.rung ? RUNG_NAMES[ld.rung] : "rule");
    }
  }

  /* ---------- running state ---------- */
  function setRunning(on) {
    S.running = on;
    if (!on) S.last = 0;
    const play = byId("civ60-play");
    if (play) play.textContent = on ? "⏸ pause" : "▶ play";
    syncVeil();
  }

  function syncVeil() {
    const veil = byId("civ60-veil");
    const chip = byId("civ60-veil-chip");
    const hint = byId("civ60-veil-hint");
    const g = S.game;
    if (!veil || !g) return;
    if (S.running) { veil.hidden = true; return; }
    veil.hidden = false;
    if (g.over) {
      chip.textContent = "↺ restart";
      hint.textContent = "the city is empty at t+" + CC.runSummary(g).simSeconds
        + "s — the verdict panel is the whole run, read off the snapshot.";
    } else if (simSeconds(g) > 0) {
      chip.textContent = "▶ resume";
      hint.textContent = "paused at t+" + simSeconds(g).toFixed(0) + "s — the city holds its breath; the ledger keeps every sample so far.";
    } else {
      chip.textContent = "▶ press play";
      hint.textContent = BOOT_HINT;
    }
  }

  /* ---------- wiring ---------- */
  function pickCitizen(g, x, y) {
    let best = null, bd = 16;
    for (let i = 0; i < g.citizens.length; i++) {
      const c = g.citizens[i];
      const k = (perBehaviorIndex(g, c) || 0);
      const p = citizenSpot(g, k, c);
      const d = Math.hypot(p.x - x, p.y - (y + 5));
      if (d < bd) { bd = d; best = c; }
    }
    return best;
  }

  /* index of citizen c within its behavior group — matches draw order */
  function perBehaviorIndex(g, c) {
    let k = 0;
    for (const other of g.citizens) {
      if (other === c) return k;
      if (other.behavior === c.behavior) k += 1;
    }
    return null;
  }

  function setEventsEnabled() {
    const on = S.game && S.game.cfg.events;
    for (const kind of CC.EVENTS) {
      const btn = byId("civ60-ev-" + kind);
      if (btn) btn.disabled = !on;
    }
    const note = byId("civ60-events-note");
    if (note) note.textContent = on ? "" : "event buttons are off for this run — restart with them on to inject.";
  }

  function wire() {
    byId("civ60-play").addEventListener("click", () => setRunning(!S.running));
    byId("civ60-veil").addEventListener("click", () => {
      if (S.game && S.game.over) { newGame(); return; }
      setRunning(true);
    });
    byId("civ60-restart").addEventListener("click", () => newGame());
    byId("civ60-seed").addEventListener("change", () => newGame());
    byId("civ60-speed").addEventListener("click", (e) => {
      const steps = [1, 2, 4, 0.5];
      S.speed = steps[(steps.indexOf(S.speed) + 1) % steps.length];
      e.currentTarget.textContent = "speed: " + S.speed + "×";
    });
    byId("civ60-engine-toggle").addEventListener("click", (e) => {
      const on = engine.status === "off" || engine.status === "unavailable";
      engine.setEnabled(on);
      e.currentTarget.textContent = on ? "engine: on" : "engine: off";
    });
    byId("civ60-release").addEventListener("click", () => { S.follow = null; render(); });
    byId("civ60-verdict-btn").addEventListener("click", () => {
      S.verdictOpen = !S.verdictOpen;
      syncVerdict();
    });
    for (const kind of CC.EVENTS) {
      byId("civ60-ev-" + kind).addEventListener("click", () => {
        const g = S.game;
        if (!g || g.over) return;
        CC.injectEvent(g, kind);
        render();
      });
    }
    byId("civ60-board-btn").addEventListener("click", () => {
      S.boardOpen = true; S.statsOpen = false;
      byId("civ60-stats-overlay").classList.remove("open");
      byId("civ60-board-overlay").classList.add("open");
      renderBoard();
    });
    byId("civ60-board-close").addEventListener("click", () => {
      S.boardOpen = false;
      byId("civ60-board-overlay").classList.remove("open");
    });
    byId("civ60-stats-btn").addEventListener("click", () => {
      S.statsOpen = true; S.boardOpen = false;
      byId("civ60-board-overlay").classList.remove("open");
      byId("civ60-stats-overlay").classList.add("open");
      renderStats();
    });
    byId("civ60-stats-close").addEventListener("click", () => {
      S.statsOpen = false;
      byId("civ60-stats-overlay").classList.remove("open");
    });

    /* click a citizen to follow its decisions; empty ground releases */
    byId("civ60-canvas").addEventListener("click", (e) => {
      const g = S.game;
      if (!g) return;
      const r = e.currentTarget.getBoundingClientRect();
      const x = (e.clientX - r.left) * (W / r.width);
      const y = (e.clientY - r.top) * (H / r.height);
      const c = pickCitizen(g, x, y);
      S.follow = c ? c.id : null;
      render();
    });

    /* keyboard: space pauses, R restarts, 1/2/4 speed, Esc closes overlays */
    window.addEventListener("keydown", (e) => {
      if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
      if (e.key === " ") {
        setRunning(!S.running);
        e.preventDefault();
      } else if (e.key === "r" || e.key === "R") {
        newGame();
      } else if (e.key === "1" || e.key === "2" || e.key === "4") {
        S.speed = +e.key;
        byId("civ60-speed").textContent = "speed: " + S.speed + "×";
      } else if (e.key === "Escape") {
        S.boardOpen = false; S.statsOpen = false;
        byId("civ60-board-overlay").classList.remove("open");
        byId("civ60-stats-overlay").classList.remove("open");
      }
    });

    setEventsEnabled();
    setChip();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", build);
  } else {
    build();
  }
})();
