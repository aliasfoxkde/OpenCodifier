/* Traffic Simulator shell — the congestion demo on the Decisions SDK.
   traffic-core.js owns the deterministic sim; this file is chrome: the
   road grid, the cars (colored by state), the phase lights, the
   click-to-follow trace, the emergency dispatch button, the session
   board (auto-recorded at every 50th completed trip), the stats overlay
   and the engine bridge. CARS are rules, never questions. LIGHTS are
   the decision agents: each re-scores every sim-second from queue
   counts; a genuine near-tie (|gap| ≤ 120) goes to the engine over
   {keep, switch} — under the city-wide batching budget. Emergency
   preemption is a rule checked every tick and is never asked. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const TC = globalThis.TrafficCore;
  const root = document.getElementById("traffic-root");
  if (!root || !DK || !TC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;
  const W = TC.GW * TC.CELL, H = TC.GH * TC.CELL;
  const DT = 1 / TC.TICKS_PER_S;
  const RUNG_NAMES = { rule: "rule", engine: "engine" };
  const MILESTONE_EVERY = 50;          /* completed trips per board row */
  const CAR_ANGLE = [-Math.PI / 2, 0, Math.PI / 2, Math.PI];   /* N E S W */
  const CAR_COLOR = { cruise: "#e2e8f0", stop: "#fbbf24", wait: "#ff5a4d",
    cross: "#2dd4a7" };

  /* canvas stays dark in both site themes, like the other games */
  const PAL = {
    bg0: "#0b0f16", bg1: "#0e1520", block: "#141c2a", road: "#232c3d",
    lane: "#39445c", box: "#1c2536",
    ink: "#e8eef8", dim: "#8b9ab8",
    green: "#3ddc84", red: "#ff5a4d", emergency: "#ff2d2d",
    follow: "#fde047",
  };

  /* ---------- state ---------- */
  const S = {
    cfg: { seed: "oc-traffic" },
    game: null,
    running: false,   /* boots paused — the play button starts the bot */
    last: 0, acc: 0, speed: 1,
    boardOpen: false, statsOpen: false,
    follow: null,                    /* car id clicked in the canvas */
    pushedTrips: 0,
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
      seed: S.seedKit ? S.seedKit.get() : "oc-traffic",
      spawnRate: +byId("traffic-spawn").value,
      congestion: +byId("traffic-congestion").value,
      askBudget: +byId("traffic-budget").value,
    };
  }

  function newGame() {
    S.cfg = cfgFromUI();
    S.game = TC.createGame(S.cfg);
    S.running = false;   /* restarts land paused, same as boot */
    S.follow = null;
    S.pushedTrips = 0;
    byId("traffic-play").textContent = "▶ play";
    byId("traffic-emergency").disabled = false;
    render();
  }

  /* board rows land automatically at every 50th completed trip */
  function pushBoard(g) {
    const s = TC.runSummary(g);
    S.board.record({
      ts: Date.now(), outcome: "milestone",
      trips: s.completed, cars: s.cars,
      decisions: s.decisions, perSecond: s.perSecond,
      engineCalls: s.engineCalls, throughput: s.throughput,
      waitP50: s.waitP50, seed: g.cfg.seed, stamp: TC.trafficStamp(g.cfg),
    });
  }

  function setChip() {
    const chip = byId("traffic-engine-chip");
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
      h("button", { class: "mz-chip mz-play", id: "traffic-play", type: "button" }, "▶ play"),
      h("button", { class: "mz-chip", id: "traffic-restart", type: "button" }, "↺ restart"),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "traffic-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "traffic-seed", class: "mz-input", value: S.cfg.seed, size: "10" }),
        h("button", { class: "mz-chip", id: "traffic-dice", type: "button", title: "new random seed" }, "🎲")),
      h("button", { class: "mz-chip", id: "traffic-speed", type: "button" }, "speed: 1×"),
      h("button", { class: "mz-chip", id: "traffic-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "traffic-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "traffic-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "traffic-stats-btn", type: "button" }, "ⓘ stats"),
      h("button", { class: "mz-chip", id: "traffic-reset-params", type: "button" }, "reset params"),
      h("button", { class: "mz-chip", id: "traffic-fs", type: "button" }, "⛶ full screen"),
    );

    const canvas = h("canvas", { id: "traffic-canvas", width: String(W), height: String(H),
      role: "img",
      "aria-label": "Traffic Simulator — cars decide stop, go, wait and turn by rule; the signal lights are decision agents whose near-tie phase calls go to the engine under a batching budget, and emergency preemption is a rule, never a question" });
    const under = h("div", { class: "mz-under game-under", id: "traffic-under" });
    const trace = h("div", { class: "game-hintline", id: "traffic-trace",
      "aria-live": "polite" }, "no decisions yet — click a car to follow it");

    const opts = DK.shell.accordion([
      {
        id: "city", label: "The city", open: true,
        kids: [
          slider("traffic-spawn", "spawn rate", 0, 90, 2, 36, " cars/min"),
          slider("traffic-congestion", "congestion", 0, 1, 0.05, 0.3),
          slider("traffic-budget", "engine budget", 0, 10, 1, 2, "/s"),
          h("p", { class: "game-note" },
            "Changing anything restarts the city from tick 0 with the same seed — ",
            "runs are deterministic, so a congestion change is a controlled experiment."),
        ],
      },
      {
        id: "emergency", label: "Dispatch an emergency",
        kids: [
          h("p", { class: "game-note" },
            "An emergency vehicle runs every red on its approach, and every red ",
            "light ahead of it flips within a tick of the vehicle entering its ",
            "12-cell horizon. Preemption is a RULE — it is never a question, and ",
            "no request that names an emergency ever reaches the engine. Watch ",
            "the preempt entries land in the decision trace."),
          h("button", { class: "mz-chip", id: "traffic-emergency", type: "button" },
            "🚨 dispatch emergency"),
        ],
      },
      {
        id: "follow", label: "Follow one car",
        kids: [
          h("p", { class: "game-note" },
            "Click a car to pin its state to the trace line below the canvas — ",
            "cruise (white), stopped (amber), waiting 3 s or more (red), crossing ",
            "a box (teal). Click empty ground to release it. The board auto-records ",
            "a row at every 50th completed trip."),
          h("button", { class: "mz-chip", id: "traffic-release", type: "button" },
            "release followed car"),
        ],
      },
      {
        id: "about", label: "What decides what?",
        kids: [h("p", { class: "game-note" },
          "CARS are rules, never questions: STOP (car ahead), STOP (red light), ",
          "GO, WAIT (stopped ≥ 3 s), TURN (argmax over the exit-arm queues at ",
          "every intersection entry) and REROUTE (the argmax flips to a side ",
          "street when the straight queue is long). LIGHTS are the decision ",
          "agents: each re-scores every sim-second — keep vs switch from the ",
          "queue counts. Rules first: under the 6 s minimum phase → keep, past ",
          "24 s → switch, emergency on the horizon → preempted every tick. A ",
          "genuine near-tie (gap ≤ 120) goes to the ENGINE over {keep, switch} ",
          "— if the city-wide batching budget has a token; no token → the argmax ",
          "rule decides and the rung says so. The throughput and wait percentiles ",
          "in the stats make the congestion slider visible.")],
      },
    ]);

    const main = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, canvas, under, trace),
      h("div", { class: "game-side" }, opts));
    root.appendChild(h("div", { class: "game-wrap" }, bar, main));

    buildOverlays();
    wire();
    S.seedKit = DK.shell.numericSeed(byId("traffic-seed"), byId("traffic-dice"), newGame);
    DK.shell.resetParams(byId("traffic-reset-params"), root);
    DK.shell.fullscreen(root.querySelector(".game-wrap"), byId("traffic-fs"));
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
    root.appendChild(overlay("traffic-board-overlay", "Session scoreboard", "traffic-board-body", "traffic-board-close"));
    root.appendChild(overlay("traffic-stats-overlay", "Measured stats", "traffic-stats-body", "traffic-stats-close"));
  }

  function renderBoard() {
    const body = byId("traffic-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" },
        "No milestones yet — the board records a row automatically at every ",
        "50th completed trip. Keep the city flowing and watch it fill. Session only."));
      return;
    }
    body.appendChild(h("table", { class: "game-table" },
      h("thead", null, h("tr", null,
        ...["#", "trips", "cars", "dec/s", "engine", "cars/min", "wait p50", "seed"].map((x) => h("th", null, x)))),
      h("tbody", null, rows.map((r, i) => h("tr", null,
        h("td", null, String(rows.length - i)),
        h("td", null, String(r.trips)),
        h("td", null, String(r.cars)),
        h("td", null, String(r.perSecond)),
        h("td", null, String(r.engineCalls)),
        h("td", null, String(r.throughput)),
        h("td", null, String(r.waitP50)),
        h("td", null, r.seed))))));
    body.appendChild(h("button", {
      class: "mz-chip", type: "button", onclick: () => { S.board.clear(); renderBoard(); },
    }, "clear board"));
  }

  function renderStats() {
    const body = byId("traffic-stats-body");
    if (!body) return;
    body.textContent = "";
    const snap = S.meters.snapshot();
    const g = S.game;
    const s = g ? TC.runSummary(g) : null;
    const preempts = g ? g.log.filter((l) => l.kind === "light" && l.note.includes("preempt")).length : 0;
    const rows = [
      ["frames/s (EMA)", S.fps.toFixed(0)],
      ["sim ticks", String(g ? g.ticks : 0)],
      ["cars on the map", s ? String(s.cars) : "0"],
      ["cars moving", s ? String(s.moving) : "0"],
      ["cars waiting", s ? String(s.waiting) : "0"],
      ["decisions — total", s ? String(s.decisions) : "0"],
      ["decisions/s (sim time)", s ? String(s.perSecond) : "0"],
      ["decisions — rule ladder", s ? String(s.rungs.rule) : "0"],
      ["decisions — engine", s ? String(s.rungs.engine) : "0"],
      ["engine calls (near-ties)", s ? String(s.engineCalls) : "0"],
      ["ask budget", g ? g.cfg.askBudget + "/s (tokens " + g.tokens.toFixed(2) + ")" : "—"],
      ["trips completed", s ? String(s.completed) : "0"],
      ["throughput (cars/min)", s ? String(s.throughput) : "0"],
      ["avg trip length", s ? s.avgTripS + "s" : "—"],
      ["wait — p50", s ? s.waitP50 + "s" : "—"],
      ["wait — p90", s ? s.waitP90 + "s" : "—"],
      ["emergency vehicle", s && s.emergency ? s.emergency : "—"],
      ["emergency preemptions", String(preempts)],
      ["signal lights", s ? String(s.lights) : "0"],
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
    TC.tickGame(g, engine.status === "ready" ? engineChoose : null);
    /* board rows land as trip milestones happen, never mid-decision */
    while (Math.floor(g.completed / MILESTONE_EVERY) > S.pushedTrips) {
      S.pushedTrips += 1;
      pushBoard(g);
      if (S.boardOpen) renderBoard();
    }
    /* the emergency button tracks the vehicle: one at a time */
    const btn = byId("traffic-emergency");
    if (btn) {
      const busy = !!g.emergency;
      btn.disabled = busy;
      btn.textContent = busy ? "🚨 " + g.emergency + " en route" : "🚨 dispatch emergency";
    }
  }

  /* ---------- render ---------- */
  function drawCity(ctx) {
    /* blocks (non-road cells) as a tinted city fabric */
    ctx.fillStyle = PAL.block;
    for (let cy = 0; cy < TC.GH; cy++) {
      for (let cx = 0; cx < TC.GW; cx++) {
        if (!TC.isRoad(cx, cy)) ctx.fillRect(cx * TC.CELL, cy * TC.CELL, TC.CELL, TC.CELL);
      }
    }
    /* the roads */
    ctx.fillStyle = PAL.road;
    for (const vx of TC.VX) ctx.fillRect(vx * TC.CELL, 0, TC.CELL * 2, H);
    for (const hy of TC.HY) ctx.fillRect(0, hy * TC.CELL, W, TC.CELL * 2);
    /* center dashes */
    ctx.strokeStyle = PAL.lane;
    ctx.lineWidth = 1;
    ctx.setLineDash([6, 8]);
    ctx.beginPath();
    for (const vx of TC.VX) {
      ctx.moveTo(vx * TC.CELL + TC.CELL, 0);
      ctx.lineTo(vx * TC.CELL + TC.CELL, H);
    }
    for (const hy of TC.HY) {
      ctx.moveTo(0, hy * TC.CELL + TC.CELL);
      ctx.lineTo(W, hy * TC.CELL + TC.CELL);
    }
    ctx.stroke();
    ctx.setLineDash([]);
    /* intersection boxes */
    ctx.fillStyle = PAL.box;
    for (const vx of TC.VX) for (const hy of TC.HY) {
      ctx.fillRect(vx * TC.CELL, hy * TC.CELL, TC.CELL * 2, TC.CELL * 2);
    }
  }

  function drawLights(ctx, g) {
    /* one two-dot indicator per light: left dot = the NS axis, right = EW */
    for (const light of g.lights) {
      const cx = light.vx * TC.CELL + TC.CELL;
      const cy = light.hy * TC.CELL + TC.CELL;
      ctx.strokeStyle = PAL.lane;
      ctx.lineWidth = 1;
      ctx.beginPath(); ctx.arc(cx, cy, 10, 0, 7); ctx.stroke();
      ctx.beginPath();
      ctx.arc(cx - 5, cy, 4.5, 0, 7);
      ctx.fillStyle = light.phase === "NS" ? PAL.green : PAL.red;
      ctx.fill();
      ctx.beginPath();
      ctx.arc(cx + 5, cy, 4.5, 0, 7);
      ctx.fillStyle = light.phase === "EW" ? PAL.green : PAL.red;
      ctx.fill();
    }
  }

  function drawCars(ctx, g) {
    for (const car of g.cars) {
      const x = car.x * TC.CELL + TC.CELL / 2;
      const y = car.y * TC.CELL + TC.CELL / 2;
      ctx.save();
      ctx.translate(x, y);
      ctx.rotate(CAR_ANGLE[car.dir]);
      if (car.emergency) {
        /* the emergency: red body, white bar, ticking beacon */
        ctx.fillStyle = PAL.emergency;
        ctx.fillRect(-7, -4.5, 14, 9);
        ctx.fillStyle = "#ffffff";
        ctx.fillRect(-2, -4.5, 4, 9);
        ctx.fillStyle = Math.floor(g.ticks / 8) % 2 === 0 ? "#ffffff" : PAL.emergency;
        ctx.beginPath(); ctx.arc(0, -8, 2.5, 0, 7); ctx.fill();
      } else {
        ctx.fillStyle = CAR_COLOR[car.state] || PAL.ink;
        ctx.fillRect(-6.5, -4, 13, 8);
        ctx.fillStyle = "rgba(12, 16, 22, 0.55)";
        ctx.fillRect(1.5, -3, 3.5, 6);          /* windshield hint */
      }
      ctx.restore();
    }
    /* the followed car gets a ring */
    if (S.follow) {
      const f = g.cars.find((c) => c.id === S.follow);
      if (f) {
        ctx.strokeStyle = PAL.follow;
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.arc(f.x * TC.CELL + TC.CELL / 2, f.y * TC.CELL + TC.CELL / 2, 11, 0, 7);
        ctx.stroke();
      }
    }
  }

  function render() {
    const canvas = byId("traffic-canvas");
    if (!canvas || !S.game) return;
    const g = S.game;
    const ctx = canvas.getContext("2d");
    const s = TC.runSummary(g);

    const sky = ctx.createLinearGradient(0, 0, 0, H);
    sky.addColorStop(0, PAL.bg0);
    sky.addColorStop(1, PAL.bg1);
    ctx.fillStyle = sky;
    ctx.fillRect(0, 0, W, H);

    drawCity(ctx);
    drawLights(ctx, g);
    drawCars(ctx, g);

    /* HUD */
    ctx.textAlign = "left";
    ctx.fillStyle = PAL.ink;
    ctx.font = "700 13px ui-monospace, monospace";
    ctx.fillText("cars " + g.cars.length, 14, 24);
    ctx.fillStyle = PAL.green;
    ctx.fillText("completed " + s.completed + " (" + s.throughput.toFixed(1) + "/min)", 14, 42);
    ctx.fillStyle = CAR_COLOR.wait;
    ctx.fillText("wait p50 " + s.waitP50.toFixed(1) + "s · p90 " + s.waitP90.toFixed(1) + "s", 14, 60);
    ctx.fillStyle = PAL.ink;
    ctx.fillText("decisions/s " + s.perSecond.toFixed(1), 14, 78);

    /* under-row + trace */
    const ld = g.lastDecision;
    byId("traffic-under").innerHTML = "";
    byId("traffic-under").appendChild(DK.shell.kvRow([
      ["state", S.running ? "flowing" : "paused"],
      ["cars", String(g.cars.length)],
      ["moving/waiting", s.moving + "/" + s.waiting],
      ["completed", String(s.completed)],
      ["decisions/s", s.perSecond.toFixed(1)],
      ["engine asks", String(s.engineCalls)],
      ["engine-honored", String(s.rungs.engine)],
      ["wait p50/p90", s.waitP50 + "/" + s.waitP90 + "s"],
      ["emergency", s.emergency || "—"],
      ["following", S.follow || "—"],
    ]));
    if (ld) {
      const at = ld.at || {};
      const prefix = S.follow
        ? (ld.id === S.follow ? "followed car → " : "following " + S.follow + " (last decision elsewhere) · ")
        : "";
      let detail;
      if (at.pick) detail = "exit " + at.pick;
      else if (at.phase) detail = at.phase + " age " + at.ageS + "s"
        + (ld.choice === "keep" || ld.choice === "switch" ? "" : "");
      else if (at.waitedS !== undefined) detail = "waited " + at.waitedS + "s";
      else detail = "rule transition";
      if (typeof at.gap === "number") detail += ", gap " + at.gap;
      byId("traffic-trace").textContent = prefix + "last decision: " + ld.id +
        " (" + detail + ") — " + ld.choice + " · " + (ld.rung ? RUNG_NAMES[ld.rung] : "rule");
    }
  }

  /* ---------- wiring ---------- */
  function pickCar(g, x, y) {
    /* cell-snapped: traffic is sparse and cars are 13 px, so a pixel
       hit-test misses constantly — pick the nearest car-cell instead */
    const cx = Math.floor(x / TC.CELL), cy = Math.floor(y / TC.CELL);
    let best = null, bd = 2.5;
    for (const c of g.cars) {
      const d = Math.hypot(c.x - cx, c.y - cy);
      if (d < bd) { bd = d; best = c; }
    }
    return best;
  }

  function wire() {
    byId("traffic-play").addEventListener("click", (e) => {
      S.running = !S.running;
      e.currentTarget.textContent = S.running ? "⏸ pause" : "▶ play";
    });
    byId("traffic-restart").addEventListener("click", () => newGame());
    byId("traffic-seed").addEventListener("change", () => newGame());
    byId("traffic-speed").addEventListener("click", (e) => {
      const steps = [1, 2, 4, 0.5];
      S.speed = steps[(steps.indexOf(S.speed) + 1) % steps.length];
      e.currentTarget.textContent = "speed: " + S.speed + "×";
    });
    byId("traffic-engine-toggle").addEventListener("click", (e) => {
      const on = engine.status === "off" || engine.status === "unavailable";
      engine.setEnabled(on);
      e.currentTarget.textContent = on ? "engine: on" : "engine: off";
    });
    byId("traffic-emergency").addEventListener("click", () => {
      const g = S.game;
      if (!g) return;
      const id = TC.injectEmergency(g);
      if (id) {
        g.log.push({ tick: g.ticks, kind: "dispatch", note: "dispatched " + id });
        render();
      }
    });
    byId("traffic-release").addEventListener("click", () => { S.follow = null; });
    byId("traffic-board-btn").addEventListener("click", () => {
      S.boardOpen = true; S.statsOpen = false;
      byId("traffic-stats-overlay").classList.remove("open");
      byId("traffic-board-overlay").classList.add("open");
      renderBoard();
    });
    byId("traffic-board-close").addEventListener("click", () => {
      S.boardOpen = false;
      byId("traffic-board-overlay").classList.remove("open");
    });
    byId("traffic-stats-btn").addEventListener("click", () => {
      S.statsOpen = true; S.boardOpen = false;
      byId("traffic-board-overlay").classList.remove("open");
      byId("traffic-stats-overlay").classList.add("open");
      renderStats();
    });
    byId("traffic-stats-close").addEventListener("click", () => {
      S.statsOpen = false;
      byId("traffic-stats-overlay").classList.remove("open");
    });

    /* click a car to follow its decisions; empty ground releases */
    byId("traffic-canvas").addEventListener("click", (e) => {
      const g = S.game;
      if (!g) return;
      const r = e.currentTarget.getBoundingClientRect();
      const x = (e.clientX - r.left) * (W / r.width);
      const y = (e.clientY - r.top) * (H / r.height);
      const car = pickCar(g, x, y);
      S.follow = car ? car.id : null;
      render();
    });

    /* keyboard: space pauses, R restarts, 1-4 speed, E dispatches, Esc closes */
    window.addEventListener("keydown", (e) => {
      if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
      if (e.key === " ") {
        S.running = !S.running;
        byId("traffic-play").textContent = S.running ? "⏸ pause" : "▶ play";
        e.preventDefault();
      } else if (e.key === "r" || e.key === "R") {
        newGame();
      } else if (e.key === "1" || e.key === "2" || e.key === "4") {
        S.speed = +e.key;
        byId("traffic-speed").textContent = "speed: " + S.speed + "×";
      } else if (e.key === "e" || e.key === "E") {
        const g = S.game;
        if (g && !g.emergency) {
          const id = TC.injectEmergency(g);
          if (id) g.log.push({ tick: g.ticks, kind: "dispatch", note: "dispatched " + id });
        }
      } else if (e.key === "Escape") {
        S.boardOpen = false; S.statsOpen = false;
        byId("traffic-board-overlay").classList.remove("open");
        byId("traffic-stats-overlay").classList.remove("open");
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
