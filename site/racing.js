/* Apex Line shell — the 2.5D racer on the Decisions SDK. racing-core.js
   owns the deterministic sim; this file is chrome: the pseudo-perspective
   projection (segment scale + curve accumulation, the classic formulas),
   procedural cars, the suggested-line chip (which rung answered it), the
   session-only board, the stats overlay, and the engine bridge. Racers
   score four candidate lines with their personality weights; genuine ties
   go to the engine; traffic is a rule, never a choice. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const RC = globalThis.RacingCore;
  const root = document.getElementById("racing-root");
  if (!root || !DK || !RC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;
  const DT = 1 / RC.TICKS_PER_S;

  const CAM_DEPTH = 0.84;
  const CAM_H = 1.2;          /* camera height, segment units */
  const DRAW_SEGS = 70;
  const HORIZON = 0.52;       /* horizon at 52% of canvas height */

  /* track palette — canvas stays dark in both site themes, like pac */
  const PAL = {
    sky0: "#0b1f2a", sky1: "#143731", grassA: "#0d2a22", grassB: "#0f3128",
    roadA: "#22282e", roadB: "#262d34", rumbleA: "#e8eef2", rumbleB: "#c2413c",
    lane: "#d8e6e0", ink: "#d8e6e0",
  };

  /* ---------- state ---------- */
  const S = {
    cfg: { seed: "oc-line" },
    game: null,
    running: false,   /* boots paused — the play button starts the bot */
    last: 0, acc: 0,
    keys: { left: false, right: false, brake: false },
    boardOpen: false, statsOpen: false,
    board: DK.sessionBoard(),
    meters: DK.meters(),
    fps: 0,
  };

  /* no importBase: dynamic import() inside the SDK resolves against the SDK
     file's own URL (sdk/), so its default "../wasm/" is exactly right here */
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
  function fmtTime(s) {
    if (s === null || s === undefined) return "—";
    return s >= 60 ? Math.floor(s / 60) + ":" + (s % 60).toFixed(1).padStart(4, "0") : s.toFixed(1) + "s";
  }

  function cfgFromUI() {
    return {
      seed: S.seedKit ? S.seedKit.get() : "oc-line",
      lengthSegs: +byId("racing-length").value,
      curviness: +byId("racing-curve").value,
      hilliness: +byId("racing-hills").value,
      laps: +byId("racing-laps").value,
      opponents: +byId("racing-opponents").value,
      skill: +byId("racing-skill").value,
      traffic: +byId("racing-traffic").value,
      grip: +byId("racing-grip").value,
      topSpeedPct: +byId("racing-topspeed").value,
      botPlayer: S.cfg.botPlayer !== false,   /* default: the engine drives */
    };
  }

  function newGame() {
    S.cfg = cfgFromUI();
    S.game = RC.createGame(S.cfg);
    S.keys = { left: false, right: false, brake: false };
    render();
  }

  /* retire mid-race and the session board keeps the honest result */
  function recordDnf() {
    const g = S.game;
    if (!g || g.over || g.ticks < 60) return;
    S.board.record({
      ts: Date.now(), outcome: "dnf",
      total: g.t, best: g.player.bestLap,
      pos: livePosition(g), laps: g.player.lapTimes.length,
      engineCalls: g.engineCalls, seed: g.cfg.seed,
      stamp: RC.racingStamp(S.cfg),
    });
  }

  function setChip() {
    const chip = byId("racing-engine-chip");
    if (!chip) return;
    const st = engine.status;
    chip.textContent = "engine: " + st;
    chip.classList.toggle("on", st === "ready");
    chip.classList.toggle("off", st === "unavailable" || st === "off");
  }

  function livePosition(g) {
    const prog = (c) => (c.lap - 1) * g.track.totalLen + c.z;
    let ahead = 0;
    for (const c of g.cars) {
      if (c === g.player || c.kind === "traffic") continue;
      if (c.finished !== null) ahead += 1;
      else if (prog(c) > prog(g.player)) ahead += 1;
    }
    return ahead + 1;
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
      h("button", { class: "mz-chip mz-play", id: "racing-play", type: "button" }, "▶ play"),
      h("button", { class: "mz-chip", id: "racing-restart", type: "button" }, "↺ restart"),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "racing-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "racing-seed", class: "mz-input", value: S.cfg.seed, size: "10" }),
        h("button", { class: "mz-chip", id: "racing-dice", type: "button", title: "new random seed" }, "🎲")),
      h("button", { class: "mz-chip", id: "racing-bot", type: "button" }, "driver: bot"),
      h("button", { class: "mz-chip", id: "racing-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "racing-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "racing-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "racing-stats-btn", type: "button" }, "ⓘ stats"),
      h("button", { class: "mz-chip", id: "racing-reset-params", type: "button" }, "reset params"),
      h("button", { class: "mz-chip", id: "racing-fs", type: "button" }, "⛶ full screen"),
    );

    const canvas = h("canvas", { id: "racing-canvas", width: "960", height: "540", role: "img",
      "aria-label": "Apex Line — a 2.5D racer where the AI cars choose their racing lines out loud" });
    const under = h("div", { class: "mz-under game-under", id: "racing-under" });
    const trace = h("div", { class: "game-hintline", id: "racing-trace",
      "aria-live": "polite" }, "no decisions yet");

    const opts = DK.shell.accordion([
      {
        id: "track", label: "Track", open: true,
        kids: [
          slider("racing-length", "length (segments)", 200, 1600, 100, 500),
          slider("racing-curve", "curviness", 0, 1, 0.1, 0.5),
          slider("racing-hills", "hills", 0, 1, 0.1, 0.5),
        ],
      },
      {
        id: "race", label: "Race",
        kids: [
          slider("racing-laps", "laps", 1, 9, 1, 2),
          slider("racing-opponents", "racers", 0, 6, 1, 3),
          slider("racing-skill", "racer skill", 1, 5, 1, 3),
          slider("racing-traffic", "traffic", 0, 1, 0.1, 0.3),
        ],
      },
      {
        id: "car", label: "Car",
        kids: [
          slider("racing-grip", "grip", 1, 5, 1, 3),
          slider("racing-topspeed", "top speed", 0.5, 1.5, 0.05, 1, "×"),
        ],
      },
      {
        id: "about", label: "What decides what?",
        kids: [h("p", { class: "game-note" },
          "The track decides the shape; you decide the car (throttle is automatic, ",
          "steer with ← → or A D, brake with ↓ or S). Each AI racer scores four candidate ",
          "lines — apex, inside, outside, slipstream — against its personality weights ",
          "every stretch and takes the winner. A genuine tie between the top two goes to ",
          "the engine. Your suggested-line chip runs the same scoring with neutral weights ",
          "and names the rung that answered it. Traffic is a rule, not a choice.")],
      },
    ]);

    const main = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, canvas, under, trace),
      h("div", { class: "game-side" }, opts));
    root.appendChild(h("div", { class: "game-wrap" }, bar, main));

    buildOverlays();
    wire();
    S.seedKit = DK.shell.numericSeed(byId("racing-seed"), byId("racing-dice"), newGame);
    DK.shell.resetParams(byId("racing-reset-params"), root);
    DK.shell.fullscreen(root.querySelector(".game-wrap"), byId("racing-fs"));
    engine.setEnabled(true);   /* default on: ties reach the runtime */
    newGame();
    if (!window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      requestAnimationFrame(loop);
    } else {
      render(); /* reduced motion: static frame, tick on demand */
    }
  }

  /* ---------- overlays (shared chrome, racing data) ---------- */
  function overlay(id, title, bodyId, closeId) {
    return h("dialog", { class: "mz-overlay", id },
      h("div", { class: "mz-board-card" },
        h("div", { class: "mz-overlay-head" },
          h("span", { class: "mz-overlay-title" }, title),
          h("button", { class: "mz-chip", id: closeId, type: "button" }, "✕ close")),
        h("div", { class: "mz-overlay-scroll", id: bodyId })));
  }

  function buildOverlays() {
    root.appendChild(overlay("racing-board-overlay", "Session scoreboard", "racing-board-body", "racing-board-close"));
    root.appendChild(overlay("racing-stats-overlay", "Measured stats", "racing-stats-body", "racing-stats-close"));
  }

  function renderBoard() {
    const body = byId("racing-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" },
        "No finished runs yet — take the checkered flag (or retire with ↺ restart) and the ",
        "race lands here. Session only: nothing you do leaves this tab."));
      return;
    }
    body.appendChild(h("table", { class: "game-table" },
      h("thead", null, h("tr", null,
        ...["#", "outcome", "total", "best lap", "pos", "laps", "engine calls", "seed"].map((x) => h("th", null, x)))),
      h("tbody", null, rows.map((r, i) => h("tr", null,
        h("td", null, String(rows.length - i)),
        h("td", null, r.outcome),
        h("td", null, fmtTime(r.total)),
        h("td", null, fmtTime(r.best)),
        h("td", null, r.pos ? "P" + r.pos : "—"),
        h("td", null, String(r.laps)),
        h("td", null, String(r.engineCalls)),
        h("td", null, r.seed))))));
    body.appendChild(h("button", {
      class: "mz-chip", type: "button", onclick: () => { S.board.clear(); renderBoard(); },
    }, "clear board"));
  }

  function renderStats() {
    const body = byId("racing-stats-body");
    if (!body) return;
    body.textContent = "";
    const snap = S.meters.snapshot();
    const g = S.game;
    const rows = [
      ["frames/s (EMA)", S.fps.toFixed(0)],
      ["sim ticks", String(g ? g.ticks : 0)],
      ["line picks — weights", String(g ? g.rungs.line : 0)],
      ["line picks — engine", String(g ? g.rungs.engine : 0)],
      ["stretches — traffic rule", String(g ? g.rungs.traffic : 0)],
      ["engine calls (incl. your chip)", String(g ? g.engineCalls : 0)],
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
    body.appendChild(h("p", { class: "game-note" },
      "Measured in this tab only. The engine row counts genuine line ties the WASM runtime ",
      "answered; traffic stretches are rules the code names as rules."));
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

  function stepOnce() {
    const g = S.game;
    if (!g || g.over) {
      if (g && g.over && !g.recorded) {
        g.recorded = true;
        S.board.record({
          ts: Date.now(), outcome: g.result,
          total: g.finalTime, best: g.player.bestLap,
          pos: g.player.position, laps: g.player.lapTimes.length,
          engineCalls: g.engineCalls, seed: g.cfg.seed,
          stamp: RC.racingStamp(S.cfg),
        });
        if (S.boardOpen) renderBoard();
      }
      return;
    }
    const steer = (S.keys.right ? 1 : 0) - (S.keys.left ? 1 : 0);
    RC.tickGame(g, { steer, accel: !S.keys.brake, brake: S.keys.brake },
      engine.status === "ready" ? engineChoose : null);
  }

  /* ---------- render ---------- */

  /* pseudo-3D projection: one point, normalized units. camY is the camera's
     interpolated elevation — projecting against the bare constant put the
     near road above the screen top on every hill (measured on seed oc-line) */
  function project(xn, yn, zn, camY) {
    const scale = CAM_DEPTH / zn;
    return {
      scale,
      x: 960 / 2 + scale * xn * (960 / 2),
      y: 540 * HORIZON + scale * (camY - yn) * (540 / 2) * 0.9,
      w: scale * (960 / 2),
    };
  }

  function drawCar(ctx, x, y, wpx, color, player) {
    const hpx = wpx * 0.62;
    ctx.fillStyle = "#04100d";
    ctx.fillRect(x - wpx * 0.42, y - hpx * 0.12, wpx * 0.16, hpx * 0.3);   /* wheels */
    ctx.fillRect(x + wpx * 0.26, y - hpx * 0.12, wpx * 0.16, hpx * 0.3);
    ctx.fillStyle = color;
    ctx.beginPath();
    ctx.moveTo(x - wpx / 2, y);
    ctx.lineTo(x - wpx * 0.4, y - hpx * 0.75);
    ctx.lineTo(x + wpx * 0.4, y - hpx * 0.75);
    ctx.lineTo(x + wpx / 2, y);
    ctx.closePath();
    ctx.fill();
    ctx.fillStyle = "rgba(4,16,13,0.55)";
    ctx.fillRect(x - wpx * 0.22, y - hpx * 0.68, wpx * 0.44, hpx * 0.34);  /* cockpit */
    if (!player) {
      ctx.fillStyle = "#ff5a4d";
      ctx.fillRect(x - wpx * 0.4, y - hpx * 0.2, wpx * 0.12, hpx * 0.12);  /* tail lights */
      ctx.fillRect(x + wpx * 0.28, y - hpx * 0.2, wpx * 0.12, hpx * 0.12);
    } else {
      ctx.fillStyle = "#f8fafc";
      ctx.fillRect(x - wpx * 0.3, y + hpx * 0.02, wpx * 0.6, hpx * 0.12);  /* rear wing */
    }
  }

  function render() {
    const canvas = byId("racing-canvas");
    if (!canvas || !S.game) return;
    const g = S.game;
    const W = canvas.width, H = canvas.height;
    const ctx = canvas.getContext("2d");
    const p = g.player;
    const L = g.track.segs.length;
    const total = g.track.totalLen;

    /* sky + horizon glow */
    const sky = ctx.createLinearGradient(0, 0, 0, H * HORIZON);
    sky.addColorStop(0, PAL.sky0);
    sky.addColorStop(1, PAL.sky1);
    ctx.fillStyle = sky;
    ctx.fillRect(0, 0, W, H * HORIZON + 1);
    ctx.fillStyle = PAL.grassA;
    ctx.fillRect(0, H * HORIZON, W, H - H * HORIZON);

    /* camera anchor */
    const baseIdx = Math.floor(p.z / RC.SEG_LEN) % L;
    const pct = (p.z % RC.SEG_LEN) / RC.SEG_LEN;
    const baseSeg = g.track.segs[baseIdx];
    const camY = CAM_H + (baseSeg.y / RC.SEG_LEN) * (1 - pct) +
      (g.track.segs[(baseIdx + 1) % L].y / RC.SEG_LEN) * pct;
    const camX = Math.max(-1, Math.min(1, p.x));

    /* segments: near to far, curve accumulation, painter's clip on hills */
    let x = 0, dx = -(baseSeg.curve * pct);
    let maxY = H;
    const lane = [];
    for (let n = 0; n < DRAW_SEGS; n++) {
      const idx = (baseIdx + n) % L;
      const seg = g.track.segs[idx];
      const zn1 = n + 1 - pct;
      const zn2 = n + 2 - pct;
      const y1 = seg.y / RC.SEG_LEN;
      const y2 = g.track.segs[(idx + 1) % L].y / RC.SEG_LEN;
      const p1 = project(x - camX, y1, zn1, camY);
      const p2 = project(x + dx - camX, y2, zn2, camY);
      x += dx;
      dx += seg.curve;
      if (zn1 <= CAM_DEPTH || p2.y >= maxY) continue;
      const alt = Math.floor(idx / 3) % 2 === 0;
      /* grass band */
      ctx.fillStyle = alt ? PAL.grassA : PAL.grassB;
      ctx.fillRect(0, p2.y, W, p1.y - p2.y);
      /* rumble strips */
      ctx.fillStyle = alt ? PAL.rumbleA : PAL.rumbleB;
      ctx.beginPath();
      ctx.moveTo(p1.x - p1.w * 1.12, p1.y);
      ctx.lineTo(p1.x + p1.w * 1.12, p1.y);
      ctx.lineTo(p2.x + p2.w * 1.12, p2.y);
      ctx.lineTo(p2.x - p2.w * 1.12, p2.y);
      ctx.closePath();
      ctx.fill();
      /* road */
      ctx.fillStyle = alt ? PAL.roadA : PAL.roadB;
      ctx.beginPath();
      ctx.moveTo(p1.x - p1.w, p1.y);
      ctx.lineTo(p1.x + p1.w, p1.y);
      ctx.lineTo(p2.x + p2.w, p2.y);
      ctx.lineTo(p2.x - p2.w, p2.y);
      ctx.closePath();
      ctx.fill();
      /* center dashes on the alt rhythm */
      if (alt) {
        ctx.fillStyle = PAL.lane;
        ctx.globalAlpha = 0.5;
        ctx.beginPath();
        ctx.moveTo(p1.x - p1.w * 0.012, p1.y);
        ctx.lineTo(p1.x + p1.w * 0.012, p1.y);
        ctx.lineTo(p2.x + p2.w * 0.012, p2.y);
        ctx.lineTo(p2.x - p2.w * 0.012, p2.y);
        ctx.closePath();
        ctx.fill();
        ctx.globalAlpha = 1;
      }
      maxY = p2.y;
    }

    /* cars: far to near, projected at their depth with the road's curve */
    const cams = g.cars.map((car) => {
      let dz = car.z - p.z;
      if (dz < -total / 2) dz += total;
      if (dz >= total / 2) dz -= total;
      return { car, dz };
    }).filter((c) => c.dz > RC.SEG_LEN * 0.2 && c.dz < DRAW_SEGS * RC.SEG_LEN)
      .sort((a, b) => b.dz - a.dz);
    for (const { car, dz } of cams) {
      const dzn = dz / RC.SEG_LEN;
      const n0 = Math.floor(pct + dzn);
      const frac = (pct + dzn) - n0;
      const i0 = (baseIdx + n0) % L;
      const xnA = segXn(g, baseIdx, baseSeg, pct, n0);
      const xnB = segXn(g, baseIdx, baseSeg, pct, n0 + 1);
      const xn = xnA + (xnB - xnA) * frac;
      const ynA = g.track.segs[i0].y / RC.SEG_LEN;
      const ynB = g.track.segs[(i0 + 1) % L].y / RC.SEG_LEN;
      const yn = ynA + (ynB - ynA) * frac;
      const pr = project(xn + car.x - camX, yn, dzn, camY);
      if (pr.w <= 0.5) continue;
      drawCar(ctx, pr.x, pr.y, Math.max(6, car.kind === "traffic" ? pr.w * 0.2 : pr.w * 0.24),
        car.color, false);
    }

    /* the player's own car — camera-locked, leans with the steering */
    const lean = ((S.keys.right ? 1 : 0) - (S.keys.left ? 1 : 0)) * 8;
    drawCar(ctx, W / 2 + lean, H * 0.9, 150, p.color, true);

    /* under-row + trace line */
    const rungNames = { line: "weights", engine: "engine", traffic: "traffic rule" };
    byId("racing-under").innerHTML = "";
    const sug = g.suggest && g.suggest.line
      ? g.suggest.line + " · " + (rungNames[g.suggest.rung] || "local") : "—";
    byId("racing-under").appendChild(DK.shell.kvRow([
      ["state", g.over ? (g.result === "finished" ? "finished P" + g.player.position : "dnf")
        : (S.running ? "running" : "paused")],
      ["lap", g.player.lap + "/" + g.cfg.laps],
      ["time", fmtTime(g.t)],
      ["best lap", fmtTime(g.player.bestLap)],
      ["pos", "P" + livePosition(g)],
      ["speed", Math.round(g.player.speed / 24) + " km/h"],
      ["your line", sug],
      ["engine ties", String(g.rungs.engine)],
    ]));
    const ld = g.lastDecision;
    if (ld) {
      byId("racing-trace").textContent = "last decision: " + ld.who +
        " @ z " + ld.at.z + " → " + ld.dir + " · " + (rungNames[ld.rung] || ld.rung);
    }
  }

  /* road-center x accumulation up to n segments ahead of the base — the
     same integration the segment loop does, replayed for car depths */
  function segXn(g, baseIdx, baseSeg, pct, n) {
    let x = 0, dx = -(baseSeg.curve * pct);
    const L = g.track.segs.length;
    for (let i = 0; i < n; i++) {
      x += dx;
      dx += g.track.segs[(baseIdx + i) % L].curve;
    }
    return x;
  }

  /* ---------- wiring ---------- */
  function wire() {
    byId("racing-play").addEventListener("click", (e) => {
      S.running = !S.running;
      e.currentTarget.textContent = S.running ? "⏸ pause" : "▶ play";
    });
    byId("racing-restart").addEventListener("click", () => {
      recordDnf();
      newGame();
    });
    byId("racing-seed").addEventListener("change", () => newGame());
    byId("racing-bot").addEventListener("click", (e) => {
      S.cfg.botPlayer = !(S.cfg.botPlayer === true);
      e.currentTarget.textContent = S.cfg.botPlayer ? "driver: bot" : "driver: you";
      if (S.game) S.game.cfg.botPlayer = S.cfg.botPlayer;
    });
    byId("racing-engine-toggle").addEventListener("click", (e) => {
      const on = engine.status === "off" || engine.status === "unavailable";
      engine.setEnabled(on);
      e.currentTarget.textContent = on ? "engine: on" : "engine: off";
    });
    byId("racing-board-btn").addEventListener("click", () => {
      S.boardOpen = true; S.statsOpen = false;
      byId("racing-stats-overlay").classList.remove("open");
      byId("racing-board-overlay").classList.add("open");
      renderBoard();
    });
    byId("racing-board-close").addEventListener("click", () => {
      S.boardOpen = false;
      byId("racing-board-overlay").classList.remove("open");
    });
    byId("racing-stats-btn").addEventListener("click", () => {
      S.statsOpen = true; S.boardOpen = false;
      byId("racing-board-overlay").classList.remove("open");
      byId("racing-stats-overlay").classList.add("open");
      renderStats();
    });
    byId("racing-stats-close").addEventListener("click", () => {
      S.statsOpen = false;
      byId("racing-stats-overlay").classList.remove("open");
    });

    /* keyboard: ← → or A D steer, ↓ / S brakes, space pauses, R restarts */
    const KEYMAP = {
      ArrowLeft: "left", ArrowRight: "right", ArrowDown: "brake",
      a: "left", d: "right", s: "brake",
      A: "left", D: "right", S: "brake",
    };
    window.addEventListener("keydown", (e) => {
      if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
      const k = KEYMAP[e.key];
      if (k) { S.keys[k] = true; e.preventDefault(); }
      else if (e.key === " ") {
        S.running = !S.running;
        byId("racing-play").textContent = S.running ? "⏸ pause" : "▶ play";
        e.preventDefault();
      } else if (e.key === "r" || e.key === "R") {
        recordDnf();
        newGame();
      } else if (e.key === "Escape") {
        S.boardOpen = false; S.statsOpen = false;
        byId("racing-board-overlay").classList.remove("open");
        byId("racing-stats-overlay").classList.remove("open");
      }
    });
    window.addEventListener("keyup", (e) => {
      const k = KEYMAP[e.key];
      if (k) S.keys[k] = false;
    });

    setChip();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", build);
  } else {
    build();
  }
})();
