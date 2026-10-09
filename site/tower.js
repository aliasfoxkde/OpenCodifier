/* Tower Defense shell — the continuous-decision demo on the Decisions SDK.
   tower-core.js owns the deterministic sim; this file is chrome: the grid,
   routes, towers and waves rendered on canvas, the tower-policy override
   (click a tower to take the decision away from the ladder), the session
   board, the stats overlay, and the engine bridge. Both sides decide:
   enemies run the rule ladder (advance / divert / retreat / regroup /
   target tower), towers score six targeting policies per shot and hand
   genuine near-ties to the engine. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const TC = globalThis.TowerCore;
  const root = document.getElementById("tower-root");
  if (!root || !DK || !TC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;
  const W = TC.W, H = TC.H;
  const DT = 1 / TC.TICKS_PER_S;
  const RUNG_NAMES = { rule: "rule", engine: "engine" };
  const POLICY_TAG = { auto: "auto", nearest: "nearest", weakest: "weakest",
    strongest: "strongest", fastest: "fastest" };
  const POLICY_CYCLE = ["auto", "nearest", "weakest", "strongest", "fastest"];

  /* canvas stays dark in both site themes, like the other games */
  const PAL = {
    bg0: "#0d1420", bg1: "#141d2e", grid: "#1d2940", base: "#4de3ff",
    main: "#39445e", mainDot: "#5a6a8f", alt: "#2dd4a7", altOff: "#24303f",
    ink: "#e8eef8", dim: "#8b9ab8",
    grunt: "#94a3c4", runner: "#fde047", brute: "#ff5a4d", saboteur: "#c084fc",
    tower: "#7dd3fc", towerShot: "#bae6fd", danger: "#ff5a4d", ok: "#3ddc84",
    range: "rgba(125, 211, 252, 0.10)",
  };
  const ENEMY_COLOR = { grunt: PAL.grunt, runner: PAL.runner, brute: PAL.brute, saboteur: PAL.saboteur };

  /* ---------- state ---------- */
  const S = {
    cfg: { seed: "oc-tower" },
    game: null,
    running: false,   /* boots paused — the play button starts the bot */
    last: 0, acc: 0, speed: 1,
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
      seed: S.seedKit ? S.seedKit.get() : "oc-tower",
      waves: +byId("tower-waves").value,
      loadout: byId("tower-loadout").value,
      diversion: byId("tower-diversion").value === "on",
    };
  }

  function newGame() {
    S.cfg = cfgFromUI();
    S.game = TC.createGame(S.cfg);
    S.running = false;   /* restarts land paused, same as boot */
    byId("tower-play").textContent = "▶ play";
    render();
  }

  /* retire mid-run and the board keeps the honest result */
  function recordDnf() {
    const g = S.game;
    if (!g || g.over || g.ticks < 60) return;
    pushBoard("dnf");
  }

  function pushBoard(outcome) {
    const g = S.game;
    const s = TC.runSummary(g);
    S.board.record({
      ts: Date.now(), outcome,
      waves: s.wave + "/" + s.waves, lives: s.lives,
      kills: s.kills, leaked: s.leaked, diverted: s.diverted,
      perSecond: s.perSecond, engineCalls: s.engineCalls,
      seed: g.cfg.seed, stamp: TC.towerStamp(g.cfg),
    });
  }

  function setChip() {
    const chip = byId("tower-engine-chip");
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
    return h("label", { class: "game-slider" },
      h("span", { class: "game-slider-label" }, label), sel);
  }

  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) S.cfg.seed = deepSeed;

    const bar = h("div", { class: "game-bar" },
      h("button", { class: "mz-chip mz-play", id: "tower-play", type: "button" }, "▶ play"),
      h("button", { class: "mz-chip", id: "tower-restart", type: "button" }, "↺ restart"),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "tower-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "tower-seed", class: "mz-input", value: S.cfg.seed, size: "10" }),
        h("button", { class: "mz-chip", id: "tower-dice", type: "button", title: "new random seed" }, "🎲")),
      h("button", { class: "mz-chip", id: "tower-speed", type: "button" }, "speed: 1×"),
      h("button", { class: "mz-chip", id: "tower-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "tower-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "tower-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "tower-stats-btn", type: "button" }, "ⓘ stats"),
      h("button", { class: "mz-chip", id: "tower-reset-params", type: "button" }, "reset params"),
      h("button", { class: "mz-chip", id: "tower-fs", type: "button" }, "⛶ full screen"),
    );

    const canvas = h("canvas", { id: "tower-canvas", width: String(W), height: String(H),
      role: "img",
      "aria-label": "Tower Defense — enemies and towers both decide out loud; near-tie targeting choices go to the engine" });
    const under = h("div", { class: "mz-under game-under", id: "tower-under" });
    const trace = h("div", { class: "game-hintline", id: "tower-trace",
      "aria-live": "polite" }, "no decisions yet");

    const opts = DK.shell.accordion([
      {
        id: "defense", label: "The defense", open: true,
        kids: [
          select("tower-loadout", "tower loadout",
            [["standard", "standard — 5 towers, one special"],
             ["artillery", "artillery — 3 heavy, one special"],
             ["screen", "screen — 6 light, no special"],
             ["solo", "solo — one tower, hard"]], "standard"),
          slider("tower-waves", "waves", 1, 20, 1, 5),
          select("tower-diversion", "alternate route",
            [["on", "on — unlocks under pressure"], ["off", "off — main path only"]], "on"),
          h("p", { class: "game-note" },
            "Changing anything restarts the run from wave 1 with the same seed — ",
            "runs are deterministic, so a loadout change is a controlled experiment."),
        ],
      },
      {
        id: "override", label: "Take over a tower",
        kids: [
          h("p", { class: "game-note" },
            "Click a tower to cycle its policy: auto → nearest → weakest → ",
            "strongest → fastest. A forced tower stops asking the ladder and ",
            "does what you say — the stats overlay counts how many decisions ",
            "you took over."),
          h("button", { class: "mz-chip", id: "tower-release", type: "button" },
            "release all to auto"),
        ],
      },
      {
        id: "about", label: "What decides what?",
        kids: [h("p", { class: "game-note" },
          "Both sides decide, on different rungs. ENEMIES run the rule ladder every ",
          "half-second: advance, divert to the alternate route once pressure unlocks ",
          "it, retreat when hurt under fire, regroup near allies, and saboteurs ",
          "attack towers. TOWERS score six targeting policies per shot — nearest, ",
          "weakest, strongest, fastest, save-special, use-special — and when the top ",
          "two land within 0.12 value the ENGINE arbitrates with the whole snapshot ",
          "in the question. Every decision lands on a rung and the counter keeps ",
          "score: this game runs dozens of decisions per sim-second.")],
      },
    ]);

    const main = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, canvas, under, trace),
      h("div", { class: "game-side" }, opts));
    root.appendChild(h("div", { class: "game-wrap" }, bar, main));

    buildOverlays();
    wire();
    S.seedKit = DK.shell.numericSeed(byId("tower-seed"), byId("tower-dice"), newGame);
    DK.shell.resetParams(byId("tower-reset-params"), root, newGame);
    DK.shell.fullscreen(root.querySelector(".game-wrap"), byId("tower-fs"));
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
    root.appendChild(overlay("tower-board-overlay", "Session scoreboard", "tower-board-body", "tower-board-close"));
    root.appendChild(overlay("tower-stats-overlay", "Measured stats", "tower-stats-body", "tower-stats-close"));
  }

  function renderBoard() {
    const body = byId("tower-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" },
        "No finished runs yet — defend all the waves or lose every life and ",
        "the run lands here (↺ restart mid-run records a dnf). Session only."));
      return;
    }
    body.appendChild(h("table", { class: "game-table" },
      h("thead", null, h("tr", null,
        ...["#", "outcome", "waves", "lives", "kills", "leaked", "diverted", "dec/s", "engine", "seed"].map((x) => h("th", null, x)))),
      h("tbody", null, rows.map((r, i) => h("tr", null,
        h("td", null, String(rows.length - i)),
        h("td", null, r.outcome),
        h("td", null, r.waves),
        h("td", null, String(r.lives)),
        h("td", null, String(r.kills)),
        h("td", null, String(r.leaked)),
        h("td", null, String(r.diverted)),
        h("td", null, String(r.perSecond)),
        h("td", null, String(r.engineCalls)),
        h("td", null, r.seed))))));
    body.appendChild(h("button", {
      class: "mz-chip", type: "button", onclick: () => { S.board.clear(); renderBoard(); },
    }, "clear board"));
  }

  function renderStats() {
    const body = byId("tower-stats-body");
    if (!body) return;
    body.textContent = "";
    const snap = S.meters.snapshot();
    const g = S.game;
    const s = g ? TC.runSummary(g) : null;
    const rows = [
      ["frames/s (EMA)", S.fps.toFixed(0)],
      ["sim ticks", String(g ? g.ticks : 0)],
      ["decisions — total", s ? String(s.decisions) : "0"],
      ["decisions/s (sim time)", s ? String(s.perSecond) : "0"],
      ["decisions — rule ladder", s ? String(s.rungs.rule) : "0"],
      ["decisions — engine", s ? String(s.rungs.engine) : "0"],
      ["engine calls (near-ties)", s ? String(s.engineCalls) : "0"],
      ["towers taken over", g ? String(g.towers.reduce((n, t) => n + t.forced, 0)) : "0"],
      ["diversions taken", s ? String(s.diverted) : "0"],
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
      if (S.game.over) break;
    }
    if (dirty || S.game.over) render();
    if (S.statsOpen && (!S.meters.get("stats.draw") || t - S.meters.get("stats.draw").last > 250)) {
      S.meters.sample("stats.draw", 0);
      renderStats();
    }
    requestAnimationFrame(loop);
  }

  function stepOnce() {
    const g = S.game;
    if (!g) return;
    if (!g.over) {
      TC.tickGame(g, engine.status === "ready" ? engineChoose : null);
      return;
    }
    if (!g.recorded) {
      g.recorded = true;
      pushBoard(g.victory ? "victory" : "defeat");
      if (S.boardOpen) renderBoard();
    }
  }

  /* ---------- render ---------- */
  function drawRoutes(ctx, g) {
    const line = (route, color, width, dash) => {
      ctx.strokeStyle = color;
      ctx.lineWidth = width;
      ctx.setLineDash(dash || []);
      ctx.beginPath();
      route.wps.forEach((p, i) => (i ? ctx.lineTo(p.x, p.y) : ctx.moveTo(p.x, p.y)));
      ctx.stroke();
      ctx.setLineDash([]);
    };
    /* the main road: wide bed + center dashes */
    line(TC.MAIN, PAL.main, 16);
    line(TC.MAIN, PAL.mainDot, 2, [7, 9]);
    /* the alternate route: locked = faint dashed, unlocked = solid green */
    if (g.diversionUnlocked) {
      line(TC.ALT, PAL.alt, 11);
    } else {
      ctx.globalAlpha = 0.45;
      line(TC.ALT, PAL.altOff, 7, [4, 8]);
      ctx.globalAlpha = 1;
    }
    /* spawn + base */
    const spawn = TC.posOn(TC.MAIN, 0);
    ctx.fillStyle = PAL.danger;
    ctx.beginPath(); ctx.arc(spawn.x, spawn.y, 9, 0, 7); ctx.fill();
    ctx.fillStyle = PAL.dim;
    ctx.font = "600 10px ui-monospace, monospace"; ctx.textAlign = "left";
    ctx.fillText("spawn", spawn.x + 12, spawn.y + 4);
    const base = TC.posOn(TC.MAIN, TC.MAIN.total);
    ctx.fillStyle = PAL.base;
    ctx.fillRect(base.x - 24, base.y - 16, 18, 32);
    ctx.fillStyle = PAL.dim;
    ctx.fillText("base", base.x - 22, base.y - 22);
  }

  function drawTowers(ctx, g) {
    for (const t of g.towers) {
      const down = g.ticks < t.disabledUntil;
      /* range halo for the tower you would take over */
      ctx.fillStyle = PAL.range;
      ctx.beginPath(); ctx.arc(t.x, t.y, t.range, 0, 7); ctx.fill();
      ctx.strokeStyle = down ? PAL.danger : PAL.tower;
      ctx.lineWidth = 2;
      ctx.beginPath(); ctx.arc(t.x, t.y, 11, 0, 7); ctx.stroke();
      if (t.special) {
        ctx.beginPath(); ctx.arc(t.x, t.y, 15, 0, 7); ctx.stroke();
      }
      /* barrel points at the last thing it shot (cheap, honest chrome) */
      ctx.save();
      ctx.translate(t.x, t.y);
      ctx.rotate(Math.sin((g.ticks + t.x) * 0.01) * 0.6 - Math.PI / 2);
      ctx.fillStyle = down ? PAL.danger : PAL.tower;
      ctx.fillRect(-2, -14, 4, 12);
      ctx.restore();
      if (down) {
        const left = Math.ceil((t.disabledUntil - g.ticks) / TC.TICKS_PER_S);
        ctx.fillStyle = PAL.danger;
        ctx.font = "700 10px ui-monospace, monospace"; ctx.textAlign = "center";
        ctx.fillText("down " + left + "s", t.x, t.y - 19);
      } else {
        /* policy tag: AUTO or your forced letter */
        ctx.fillStyle = t.policy === "auto" ? PAL.dim : PAL.runner;
        ctx.font = "700 9px ui-monospace, monospace"; ctx.textAlign = "center";
        ctx.fillText(t.policy === "auto" ? "AUTO" : POLICY_TAG[t.policy].toUpperCase(), t.x, t.y - 19);
      }
      /* special charge arc */
      if (t.special) {
        ctx.strokeStyle = PAL.ok;
        ctx.lineWidth = 3;
        ctx.beginPath();
        ctx.arc(t.x, t.y, 19, -Math.PI / 2, -Math.PI / 2 + (t.charge / TC.SPECIAL_CHARGE) * Math.PI * 2);
        ctx.stroke();
      }
    }
  }

  function drawEnemies(ctx, g) {
    for (const e of g.enemies) {
      if (e.hp <= 0) continue;
      const p = TC.posOn(e.route === "alt" ? TC.ALT : TC.MAIN, e.t);
      const r = e.type === "brute" ? 9 : e.type === "runner" ? 5 : 6;
      ctx.fillStyle = ENEMY_COLOR[e.type] || PAL.grunt;
      if (e.type === "saboteur") {
        ctx.save();
        ctx.translate(p.x, p.y);
        ctx.rotate(Math.PI / 4);
        ctx.fillRect(-r, -r, r * 2, r * 2);
        ctx.restore();
      } else {
        ctx.beginPath(); ctx.arc(p.x, p.y, r, 0, 7); ctx.fill();
      }
      /* hp bar */
      const w = r * 2.4;
      ctx.fillStyle = "#00000090";
      ctx.fillRect(p.x - w / 2, p.y - r - 7, w, 3);
      ctx.fillStyle = e.hp / e.maxHp > 0.35 ? PAL.ok : PAL.danger;
      ctx.fillRect(p.x - w / 2, p.y - r - 7, w * Math.max(0, e.hp / e.maxHp), 3);
      /* state glyph — the ladder, visible */
      if (e.state === "retreat") {
        ctx.fillStyle = PAL.base;
        ctx.font = "700 9px ui-monospace, monospace"; ctx.textAlign = "center";
        ctx.fillText("←", p.x, p.y - r - 10);
      } else if (e.state === "regroup") {
        ctx.strokeStyle = PAL.base;
        ctx.lineWidth = 1;
        ctx.beginPath(); ctx.arc(p.x, p.y, r + 4, 0, 7); ctx.stroke();
      } else if (e.state === "target-tower") {
        ctx.fillStyle = PAL.danger;
        ctx.font = "700 9px ui-monospace, monospace"; ctx.textAlign = "center";
        ctx.fillText("⚡", p.x, p.y - r - 10);
      } else if (e.diverted && e.route === "alt") {
        ctx.fillStyle = PAL.alt;
        ctx.font = "700 9px ui-monospace, monospace"; ctx.textAlign = "center";
        ctx.fillText("↷", p.x, p.y - r - 10);
      }
    }
  }

  function drawFx(ctx, g) {
    for (const s of g.shots) {
      ctx.globalAlpha = s.life / 6;
      ctx.strokeStyle = PAL.towerShot;
      ctx.lineWidth = 1.5;
      ctx.beginPath(); ctx.moveTo(s.x1, s.y1); ctx.lineTo(s.x2, s.y2); ctx.stroke();
    }
    ctx.globalAlpha = 1;
    for (const b of g.blasts) {
      const k = 1 - b.life / 12;
      ctx.globalAlpha = b.life / 12;
      ctx.strokeStyle = PAL.ok;
      ctx.lineWidth = 3;
      ctx.beginPath(); ctx.arc(b.x, b.y, b.r * (0.5 + 0.5 * k), 0, 7); ctx.stroke();
    }
    ctx.globalAlpha = 1;
  }

  function render() {
    const canvas = byId("tower-canvas");
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
    for (let x = TC.CELL; x < W; x += TC.CELL) { ctx.moveTo(x, 0); ctx.lineTo(x, H); }
    for (let y = TC.CELL; y < H; y += TC.CELL) { ctx.moveTo(0, y); ctx.lineTo(W, y); }
    ctx.stroke();

    drawRoutes(ctx, g);
    drawTowers(ctx, g);
    drawEnemies(ctx, g);
    drawFx(ctx, g);

    /* HUD */
    const s = TC.runSummary(g);
    ctx.textAlign = "left";
    ctx.fillStyle = PAL.ink;
    ctx.font = "700 13px ui-monospace, monospace";
    ctx.fillText("wave " + Math.max(1, s.wave) + "/" + s.waves +
      (g.wave < g.cfg.waves || g.enemies.length ? "" : " — cleared"), 14, 24);
    ctx.fillStyle = s.lives > 6 ? PAL.ink : PAL.danger;
    ctx.fillText("lives " + s.lives, 14, 42);
    ctx.fillStyle = PAL.ink;
    ctx.fillText("gold " + s.gold, 14, 60);
    ctx.fillStyle = PAL.base;
    ctx.fillText("decisions/s " + s.perSecond.toFixed(1), 14, 78);
    ctx.fillStyle = PAL.alt;
    ctx.fillText("alt route " + (g.cfg.diversion ? (g.diversionUnlocked ? "OPEN" : "locked") : "absent"), 14, 96);

    if (g.over) {
      ctx.textAlign = "center";
      ctx.fillStyle = "rgba(10, 12, 20, 0.72)";
      ctx.fillRect(0, 0, W, H);
      ctx.fillStyle = g.victory ? PAL.ok : PAL.danger;
      ctx.font = "800 44px ui-monospace, monospace";
      ctx.fillText(g.victory ? "DEFENSE HELD" : "BASE OVERRUN", W / 2, H / 2 - 30);
      ctx.fillStyle = PAL.ink;
      ctx.font = "600 15px ui-monospace, monospace";
      ctx.fillText("wave " + s.wave + "/" + s.waves + " · " + s.kills + " kills · " + s.leaked +
        " leaked · " + s.diverted + " diverted · " + s.decisions + " decisions · ↺ (R) to redo", W / 2, H / 2 + 4);
    }

    /* under-row + trace */
    const ld = g.lastDecision;
    byId("tower-under").innerHTML = "";
    byId("tower-under").appendChild(DK.shell.kvRow([
      ["state", g.over ? (g.victory ? "defense held" : "base overrun")
        : (S.running ? "defending" : "paused")],
      ["wave", Math.max(1, s.wave) + "/" + s.waves],
      ["lives", String(s.lives)],
      ["enemies on field", String(g.enemies.length)],
      ["decisions/s", s.perSecond.toFixed(1)],
      ["diversions", String(s.diverted)],
      ["last decision", ld ? ld.who + " → " + ld.choice : "—"],
      ["answered by", ld && ld.rung ? RUNG_NAMES[ld.rung] : (ld ? "rule" : "—")],
      ["engine near-ties", String(s.rungs.engine)],
    ]));
    if (ld) {
      const at = ld.at || {};
      const detail = ld.who === "tower"
        ? "range " + at.inRange + ", charge " + at.charge + ", gap " +
          (ld.evGap === Infinity ? "∅" : ld.evGap)
        : "hp " + at.hp + ", pressure " + at.pressure;
      byId("tower-trace").textContent = "last decision: " + ld.who + " " + ld.id +
        " (" + detail + ") — " + ld.choice + " · " +
        (ld.rung ? RUNG_NAMES[ld.rung] : "rule");
    }
  }

  /* ---------- wiring ---------- */
  function cyclePolicy(g, x, y) {
    let best = null, bd = 24;
    for (const t of g.towers) {
      const d = Math.hypot(t.x - x, t.y - y);
      if (d < bd) { bd = d; best = t; }
    }
    if (!best) return false;
    const i = POLICY_CYCLE.indexOf(best.policy);
    best.policy = POLICY_CYCLE[(i + 1) % POLICY_CYCLE.length];
    return true;
  }

  function wire() {
    byId("tower-play").addEventListener("click", (e) => {
      S.running = !S.running;
      e.currentTarget.textContent = S.running ? "⏸ pause" : "▶ play";
    });
    byId("tower-restart").addEventListener("click", () => {
      recordDnf();
      newGame();
    });
    byId("tower-seed").addEventListener("change", () => newGame());
    byId("tower-speed").addEventListener("click", (e) => {
      const steps = [1, 2, 4, 0.5];
      S.speed = steps[(steps.indexOf(S.speed) + 1) % steps.length];
      e.currentTarget.textContent = "speed: " + S.speed + "×";
    });
    byId("tower-engine-toggle").addEventListener("click", (e) => {
      const on = engine.status === "off" || engine.status === "unavailable";
      engine.setEnabled(on);
      e.currentTarget.textContent = on ? "engine: on" : "engine: off";
    });
    byId("tower-release").addEventListener("click", () => {
      const g = S.game;
      if (g) g.towers.forEach((t) => { t.policy = "auto"; });
    });
    byId("tower-board-btn").addEventListener("click", () => {
      S.boardOpen = true; S.statsOpen = false;
      byId("tower-stats-overlay").classList.remove("open");
      byId("tower-board-overlay").classList.add("open");
      renderBoard();
    });
    byId("tower-board-close").addEventListener("click", () => {
      S.boardOpen = false;
      byId("tower-board-overlay").classList.remove("open");
    });
    byId("tower-stats-btn").addEventListener("click", () => {
      S.statsOpen = true; S.boardOpen = false;
      byId("tower-board-overlay").classList.remove("open");
      byId("tower-stats-overlay").classList.add("open");
      renderStats();
    });
    byId("tower-stats-close").addEventListener("click", () => {
      S.statsOpen = false;
      byId("tower-stats-overlay").classList.remove("open");
    });

    /* click a tower to cycle its policy */
    byId("tower-canvas").addEventListener("click", (e) => {
      const g = S.game;
      if (!g || g.over) return;
      const r = e.currentTarget.getBoundingClientRect();
      const x = (e.clientX - r.left) * (W / r.width);
      const y = (e.clientY - r.top) * (H / r.height);
      cyclePolicy(g, x, y);
      render();
    });

    /* keyboard: space pauses, R restarts, 1-4 speed, Esc closes overlays */
    window.addEventListener("keydown", (e) => {
      if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
      if (e.key === " ") {
        S.running = !S.running;
        byId("tower-play").textContent = S.running ? "⏸ pause" : "▶ play";
        e.preventDefault();
      } else if (e.key === "r" || e.key === "R") {
        recordDnf();
        newGame();
      } else if (e.key === "1" || e.key === "2" || e.key === "4") {
        S.speed = +e.key;
        byId("tower-speed").textContent = "speed: " + S.speed + "×";
      } else if (e.key === "Escape") {
        S.boardOpen = false; S.statsOpen = false;
        byId("tower-board-overlay").classList.remove("open");
        byId("tower-stats-overlay").classList.remove("open");
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
