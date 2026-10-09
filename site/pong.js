/* Pong shell — the Playground Pong game on the Decisions SDK.
   pong-core.js owns the deterministic sim; this file is chrome: canvas,
   controls, the reward editors (the actual point of the game), the
   session-only scoreboard, the stats overlay, and the engine bridge.
   Two bots under different reward weights play differently, visibly —
   that is the demo. Rung chip shows whether the last tie went to the
   engine or to the local formula. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const PC = globalThis.PongCore;
  const root = document.getElementById("pong-root");
  if (!root || !DK || !PC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;

  /* court palette — canvas stays dark in both site themes, like the maze */
  const PAL = {
    bg: "#04100d", line: "rgba(94,234,212,0.18)", ink: "#d8e6e0",
    dim: "#88aca3", teal: "#5eead4", green: "#34d399",
    left: "#5eead4", right: "#fbbf24", ball: "#e7f5f0",
  };

  /* ---------- state ---------- */
  const S = {
    cfg: { seed: "oc-pong" },
    modes: { left: "bot", right: "bot" },
    rewards: { left: PC.normalizeRewards(), right: PC.normalizeRewards() },
    match: null,
    running: false,   /* boots paused — the play button starts the bot */
    last: 0, acc: 0,
    humanY: { left: null, right: null },
    keys: { left: 0, right: 0 },          /* -1/0/1 from W/S and arrows */
    rung: { formula: 0, engine: 0, last: "formula" },
    boardOpen: false, statsOpen: false,
    board: DK.sessionBoard(),
    meters: DK.meters(),
    fps: 0,
  };

  /* no importBase: dynamic import() inside the SDK resolves against the SDK
     file's own URL (sdk/), so its default "../wasm/" is exactly right here */
  const engine = DK.engineBridge({
    onStatus: (st) => setChip(),
  });

  function engineChoose(request) {
    const t0 = nowMs();
    const res = engine.choose(request);
    S.meters.sample("engine.ms", nowMs() - t0);
    if (res) {
      S.rung.engine += 1;
      S.rung.last = "engine";
    } else {
      S.rung.formula += 1;
      S.rung.last = "formula";
    }
    return res;
  }

  /* ---------- helpers ---------- */
  function cfgFromUI() {
    return {
      seed: S.seedKit ? S.seedKit.get() : "oc-pong",
      ballSpeed: +byId("pong-speed").value,
      paddleH: +byId("pong-ph").value,
      paddleSpeed: +byId("pong-ps").value,
      winScore: +byId("pong-win").value,
      reactEvery: +byId("pong-react").value,
      noise: +byId("pong-noise").value / 100,
    };
  }

  function newMatch() {
    S.cfg = cfgFromUI();
    S.match = PC.createMatch(S.cfg, S.rewards.left, S.rewards.right);
    PC.serve(S.match);
    S.rung = { formula: 0, engine: 0, last: "formula" };
    S.humanY = { left: null, right: null };
    render();
  }

  function setChip() {
    const chip = byId("pong-engine-chip");
    if (!chip) return;
    const st = engine.status;
    chip.textContent = "engine: " + st;
    chip.classList.toggle("on", st === "ready");
    chip.classList.toggle("off", st === "unavailable" || st === "off");
  }

  /* ---------- build ---------- */
  function slider(id, label, min, max, step, val, unit) {
    const out = h("span", { class: "pong-sliderval", id: id + "-val" }, String(val) + (unit || ""));
    const input = h("input", {
      type: "range", id, min: String(min), max: String(max), step: String(step), value: String(val),
      "aria-label": label,
    });
    input.addEventListener("input", () => {
      out.textContent = input.value + (unit || "");
    });
    input.addEventListener("change", () => newMatch());
    return h("label", { class: "pong-slider" }, h("span", { class: "pong-slider-label" }, label), input, out);
  }

  function rewardSliders(side) {
    const wrap = h("div", { class: "pong-rewards", "data-side": side });
    const defs = [
      ["pursue", "chase the intercept"], ["bounce", "edge hits for spin"],
      ["center", "rest at center after a hit"], ["taunt", "mirror the opponent"],
      ["risk", "avoid wall-hugging targets"],
    ];
    for (const [tag, blurb] of defs) {
      const val = S.rewards[side].weights[tag];
      const input = h("input", {
        type: "range", min: "0", max: "5", step: "0.5", value: String(val),
        id: "pong-" + side + "-" + tag, "aria-label": side + " " + tag,
      });
      const out = h("span", { class: "pong-sliderval" }, val.toFixed(1));
      input.addEventListener("input", () => {
        out.textContent = (+input.value).toFixed(1);
      });
      input.addEventListener("change", () => {
        S.rewards[side].weights[tag] = +input.value;
        newMatch();
      });
      wrap.appendChild(h("label", { class: "pong-slider" },
        h("span", { class: "pong-slider-label" }, tag + " — ", h("em", null, blurb)), input, out));
    }
    return wrap;
  }

  function modeBtn(side) {
    const btn = h("button", {
      class: "mz-chip", type: "button", id: "pong-mode-" + side,
      "aria-pressed": "false",
    }, side + ": bot");
    btn.addEventListener("click", () => {
      S.modes[side] = S.modes[side] === "bot" ? "human" : "bot";
      btn.textContent = side + ": " + S.modes[side];
      btn.setAttribute("aria-pressed", String(S.modes[side] === "human"));
      S.humanY[side] = null;
    });
    return btn;
  }

  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) S.cfg.seed = deepSeed;

    /* top bar */
    const bar = h("div", { class: "pong-bar" },
      h("button", { class: "mz-chip mz-play", id: "pong-play", type: "button" }, "▶ play"),
      h("button", { class: "mz-chip", id: "pong-reset", type: "button" }, "↺ reset"),
      h("span", { class: "pong-seed-wrap" },
        h("label", { for: "pong-seed", class: "pong-slider-label" }, "seed "),
        h("input", { type: "number", id: "pong-seed", class: "mz-input", value: S.cfg.seed, size: "10" }),
        h("button", { class: "mz-chip", id: "pong-dice", type: "button", title: "new random seed" }, "🎲")),
      modeBtn("left"), modeBtn("right"),
      h("button", { class: "mz-chip", id: "pong-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip pong-rung", id: "pong-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "pong-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "pong-stats-btn", type: "button" }, "ⓘ stats"),
      h("button", { class: "mz-chip", id: "pong-reset-params", type: "button" }, "reset params"),
      h("button", { class: "mz-chip", id: "pong-fs", type: "button" }, "⛶ full screen"),
    );

    /* canvas + under-readout */
    const canvas = h("canvas", { id: "pong-canvas", width: "900", height: "540", role: "img",
      "aria-label": "Pong court — two policies playing under user-set reward weights" });
    const under = h("div", { class: "mz-under pong-under", id: "pong-under" });

    /* options accordion */
    const opts = DK.shell.accordion([
      {
        id: "match", label: "Match", open: true,
        kids: [
          slider("pong-speed", "ball speed", 20, 80, 5, 40),
          slider("pong-ph", "paddle height", 5, 18, 1, 10),
          slider("pong-ps", "paddle speed", 18, 60, 2, 34),
          slider("pong-win", "win score", 1, 11, 1, 3),
        ],
      },
      {
        id: "skill", label: "Bot skill",
        kids: [
          slider("pong-react", "reaction cadence (ticks)", 1, 20, 1, 6),
          slider("pong-noise", "aim noise", 0, 50, 5, 12, "%"),
        ],
      },
      { id: "rewards-left", label: "Left rewards", kids: [rewardSliders("left")] },
      { id: "rewards-right", label: "Right rewards", kids: [rewardSliders("right")] },
      {
        id: "about", label: "What am I watching?",
        kids: [h("p", { class: "pong-note" },
          "Each paddle scores four candidate targets (intercept, edge-spin, rest, mirror) with the ",
          "reward weights you set, and chases the winner at capped speed — so a fast ball beats a ",
          "perfect tracker and misses are physics. When two candidates tie exactly, the decision ",
          "goes to the OpenCodifier engine if it's loaded, otherwise to the same weighted formula; ",
          "the chip counts which rung answered.")],
      },
    ]);

    const main = h("div", { class: "pong-layout" },
      h("div", { class: "pong-center" }, canvas, under),
      h("div", { class: "pong-side" }, opts));
    root.appendChild(h("div", { class: "pong-wrap" }, bar, main));

    buildOverlays();
    wire();
    S.seedKit = DK.shell.numericSeed(byId("pong-seed"), byId("pong-dice"), newMatch);
    DK.shell.resetParams(byId("pong-reset-params"), root, newMatch);
    DK.shell.fullscreen(root.querySelector(".pong-wrap"), byId("pong-fs"));
    engine.setEnabled(true);   /* default on, like the maze: ties reach the runtime */
    newMatch();
    if (!window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      requestAnimationFrame(loop);
    } else {
      render(); /* reduced motion: static court, tick on demand */
    }
  }

  /* ---------- overlays (shared chrome, pong data) ---------- */
  function overlay(id, title, bodyId, closeId) {
    return h("dialog", { class: "mz-overlay", id },
      h("div", { class: "mz-board-card" },
        h("div", { class: "mz-overlay-head" },
          h("span", { class: "mz-overlay-title" }, title),
          h("button", { class: "mz-chip", id: closeId, type: "button" }, "✕ close")),
        h("div", { class: "mz-overlay-scroll", id: bodyId })));
  }

  function buildOverlays() {
    const board = overlay("pong-board-overlay", "Session scoreboard", "pong-board-body", "pong-board-close");
    const stats = overlay("pong-stats-overlay", "Measured stats", "pong-stats-body", "pong-stats-close");
    root.appendChild(board);
    root.appendChild(stats);
  }

  function renderBoard() {
    const body = byId("pong-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "pong-note" },
        "No finished matches yet — play one to the win score and it lands here. ",
        "Session only: nothing you do leaves this tab."));
      return;
    }
    const frag = h("div", null, h("table", { class: "pong-table" },
      h("thead", null, h("tr", null,
        ...["#", "winner", "score", "longest rally", "seed", "engine ties"].map((x) => h("th", null, x)))),
      h("tbody", null, rows.map((r, i) => h("tr", null,
        h("td", null, String(rows.length - i)),
        h("td", null, r.winner),
        h("td", null, r.score),
        h("td", null, String(r.longestRally)),
        h("td", null, r.seed),
        h("td", null, String(r.engineCalls)))))));
    body.appendChild(frag);
    body.appendChild(h("button", {
      class: "mz-chip", type: "button", onclick: () => { S.board.clear(); renderBoard(); },
    }, "clear board"));
  }

  function renderStats() {
    const body = byId("pong-stats-body");
    if (!body) return;
    body.textContent = "";
    const snap = S.meters.snapshot();
    const rows = [
      ["frames/s (EMA)", S.fps.toFixed(0)],
      ["sim ticks", String(S.match ? S.match.ticks : 0)],
      ["ties → engine", String(S.rung.engine)],
      ["ties → formula", String(S.rung.formula)],
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
    body.appendChild(h("p", { class: "pong-note" },
      "Measured in this tab only. The engine row counts tie-breaks the WASM runtime answered; ",
      "with the engine off the same weighted formula answers every tie."));
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
    while (S.acc >= PC.DT) {
      const t0 = nowMs();
      stepOnce();
      S.meters.sample("tick.ms", nowMs() - t0);
      S.acc -= PC.DT;
      dirty = true;
      if (S.match.over) break;
    }
    if (dirty) render();
    if (S.statsOpen && (!S.meters.get("stats.draw") || t - S.meters.get("stats.draw").last > 250)) {
      S.meters.sample("stats.draw", 0);
      renderStats();
    }
    requestAnimationFrame(loop);
  }

  function stepOnce() {
    const m = S.match;
    if (!m || m.over) {
      if (m && m.over && !m.recorded) {
        m.recorded = true;
        const sum = PC.matchSummary(m);
        S.board.record({
          ts: Date.now(), winner: sum.winner,
          score: sum.left + "–" + sum.right,
          longestRally: sum.longestRally, seed: sum.seed,
          engineCalls: sum.engineCalls, stamp: PC.matchStamp(S.cfg, S.rewards.left, S.rewards.right),
        });
        if (S.boardOpen) renderBoard();
      }
      return;
    }
    const input = { engineChoose: engine.status === "ready" ? engineChoose : null };
    for (const side of ["left", "right"]) {
      input[side] = S.modes[side] === "human" ? humanTarget(side) : null;
    }
    PC.tickMatch(m, input);
  }

  function humanTarget(side) {
    if (S.keys[side]) {
      const p = S.match.paddles[side];
      const cur = S.humanY[side] === null ? p.y : S.humanY[side];
      return Math.max(0, Math.min(PC.FIELD_H, cur + S.keys[side] * 4));
    }
    return S.humanY[side];
  }

  /* ---------- render ---------- */
  function render() {
    const canvas = byId("pong-canvas");
    if (!canvas || !S.match) return;
    const ctx = canvas.getContext("2d");
    const W = canvas.width, Hh = canvas.height;
    const sx = W / PC.FIELD_W, sy = Hh / PC.FIELD_H;
    ctx.fillStyle = PAL.bg;
    ctx.fillRect(0, 0, W, Hh);
    /* center line + border */
    ctx.strokeStyle = PAL.line;
    ctx.setLineDash([8, 10]);
    ctx.beginPath(); ctx.moveTo(W / 2, 0); ctx.lineTo(W / 2, Hh); ctx.stroke();
    ctx.setLineDash([]);
    ctx.strokeRect(1, 1, W - 2, Hh - 2);
    /* scores */
    ctx.fillStyle = PAL.dim;
    ctx.font = "700 64px ui-monospace, Menlo, monospace";
    ctx.textAlign = "center";
    ctx.fillText(String(S.match.paddles.left.score), W * 0.38, 78);
    ctx.fillText(String(S.match.paddles.right.score), W * 0.62, 78);
    /* paddles */
    for (const side of ["left", "right"]) {
      const p = S.match.paddles[side];
      const px = side === "left" ? (PC.PADDLE_X - 1) * sx : (PC.FIELD_W - PC.PADDLE_X - 1) * sx;
      ctx.fillStyle = PAL[side];
      ctx.fillRect(px, (p.y - p.h / 2) * sy, 2 * sx, p.h * sy);
    }
    /* ball + trail */
    const b = S.match.ball;
    if (b) {
      ctx.fillStyle = PAL.ball;
      ctx.beginPath();
      ctx.arc(b.x * sx, b.y * sy, 6, 0, Math.PI * 2);
      ctx.fill();
    }
    /* rung chip + state line */
    const m = S.match;
    byId("pong-under").innerHTML = "";
    byId("pong-under").appendChild(DK.shell.kvRow([
      ["state", m.over ? "match over — " + m.winner + " wins" : (S.running ? "rally " + (m.rallies.length + 1) : "paused")],
      ["hits", String(m.paddles.left.hits + m.paddles.right.hits)],
      ["longest rally", String(m.rallies.length ? Math.max.apply(null, m.rallies) : m.rallyHits)],
      ["last tie", S.rung.last],
      ["engine ties", String(S.rung.engine)],
    ]));
  }

  /* ---------- wiring ---------- */
  function wire() {
    byId("pong-play").addEventListener("click", (e) => {
      S.running = !S.running;
      e.currentTarget.textContent = S.running ? "⏸ pause" : "▶ play";
    });
    byId("pong-reset").addEventListener("click", () => newMatch());
    byId("pong-seed").addEventListener("change", () => newMatch());
    byId("pong-engine-toggle").addEventListener("click", (e) => {
      const on = engine.status === "off" || engine.status === "unavailable";
      engine.setEnabled(on);
      e.currentTarget.textContent = on ? "engine: on" : "engine: off";
    });
    byId("pong-board-btn").addEventListener("click", () => {
      S.boardOpen = true; S.statsOpen = false;
      byId("pong-stats-overlay").classList.remove("open");
      byId("pong-board-overlay").classList.add("open");
      renderBoard();
    });
    byId("pong-board-close").addEventListener("click", () => {
      S.boardOpen = false;
      byId("pong-board-overlay").classList.remove("open");
    });
    byId("pong-stats-btn").addEventListener("click", () => {
      S.statsOpen = true; S.boardOpen = false;
      byId("pong-board-overlay").classList.remove("open");
      byId("pong-stats-overlay").classList.add("open");
      renderStats();
    });
    byId("pong-stats-close").addEventListener("click", () => {
      S.statsOpen = false;
      byId("pong-stats-overlay").classList.remove("open");
    });

    /* keyboard: W/S drives left, arrows drive right; space pauses */
    window.addEventListener("keydown", (e) => {
      if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
      if (e.key === "w" || e.key === "W") S.keys.left = -1;
      else if (e.key === "s" || e.key === "S") S.keys.left = 1;
      else if (e.key === "ArrowUp") S.keys.right = -1;
      else if (e.key === "ArrowDown") S.keys.right = 1;
      else if (e.key === " ") {
        S.running = !S.running;
        byId("pong-play").textContent = S.running ? "⏸ pause" : "▶ play";
        e.preventDefault();
      } else if (e.key === "Escape") {
        S.boardOpen = false; S.statsOpen = false;
        byId("pong-board-overlay").classList.remove("open");
        byId("pong-stats-overlay").classList.remove("open");
      }
    });
    window.addEventListener("keyup", (e) => {
      if (e.key === "w" || e.key === "W" || e.key === "s" || e.key === "S") S.keys.left = 0;
      if (e.key === "ArrowUp" || e.key === "ArrowDown") S.keys.right = 0;
    });

    /* mouse aims the paddle of whichever side (or sides) is human */
    byId("pong-canvas").addEventListener("mousemove", (e) => {
      const rect = e.currentTarget.getBoundingClientRect();
      const y = ((e.clientY - rect.top) / rect.height) * PC.FIELD_H;
      for (const side of ["left", "right"]) {
        if (S.modes[side] === "human") S.humanY[side] = y;
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
