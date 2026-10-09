/* Pac-maze shell — the Playground Pacman game on the Decisions SDK.
   pac-core.js owns the deterministic sim; this file is chrome: canvas,
   controls, the ghost trace line (which rung answered each move), the
   session-only board, the stats overlay, and the engine bridge. Ghosts
   score candidate directions with their personality weights; genuine ties
   go to the engine; frightened retreat is a labelled rule, never a choice. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const PC = globalThis.PacCore;
  const root = document.getElementById("pac-root");
  if (!root || !DK || !PC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;
  const DT = 1 / PC.TICKS_PER_S;

  /* maze palette — canvas stays dark in both site themes, like pong/life */
  const PAL = {
    bg: "#04100d", wall: "rgba(94,234,212,0.28)", ink: "#d8e6e0",
    dim: "#88aca3", teal: "#5eead4", player: "#fbd24d",
    pellet: "#e7f5f0", power: "#7dd3fc", fright: "#3b82f6",
    flash: "#f8fafc",
  };

  /* ---------- state ---------- */
  const S = {
    cfg: { seed: "oc-pac" },
    game: null,
    running: false,   /* boots paused — the play button starts the bot */
    last: 0, acc: 0,
    want: null,
    rung: { last: "—", engine: 0 },
    boardOpen: false, statsOpen: false,
    board: DK.sessionBoard(),
    meters: DK.meters(),
    fps: 0,
    gens: 0,
    hi: 0,
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
    return res;
  }

  /* ---------- helpers ---------- */
  function cfgFromUI() {
    const boardSel = byId("pac-board");
    return {
      seed: S.seedKit ? S.seedKit.get() : "oc-pac",
      board: boardSel ? boardSel.value : "classic",
      cells: +byId("pac-size").value,
      braidPct: +byId("pac-braid").value,
      ghosts: +byId("pac-gcount").value,
      ghostSpeed: +byId("pac-gspeed").value,
      frightS: +byId("pac-fright").value,
      playerSpeed: +byId("pac-pspeed").value,
      lives: +byId("pac-lives").value,
      pelletFill: +byId("pac-fill").value / 100,
      botPlayer: S.cfg.botPlayer !== false,   /* default: the engine plays */
    };
  }

  function newGame() {
    S.cfg = cfgFromUI();
    S.game = PC.createGame(S.cfg);
    S.want = null;
    S.rung = { last: "—", engine: 0 };
    /* classic is 28x31 plus HUD rows; generated stays square */
    const canvas = byId("pac-canvas");
    if (canvas) {
      const classic = S.game.board.classic === true;
      canvas.width = classic ? 672 : 720;
      canvas.height = classic ? 840 : 720;
      canvas.style.aspectRatio = classic ? "672 / 840" : "1 / 1";
    }
    render();
  }

  /* generated-only controls dim out on the arcade board */
  function syncGenerated() {
    const classic = !byId("pac-board") || byId("pac-board").value !== "generated";
    for (const id of ["pac-size", "pac-braid", "pac-fill"]) {
      const el = byId(id);
      if (!el) continue;
      el.disabled = classic;
      const row = el.closest("label");
      if (row) row.classList.toggle("is-off", classic);
    }
    const note = byId("pac-gen-note");
    if (note) note.hidden = !classic;
  }

  function setChip() {
    const chip = byId("pac-engine-chip");
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
      h("button", { class: "mz-chip mz-play", id: "pac-play", type: "button" }, "▶ play"),
      h("button", { class: "mz-chip", id: "pac-restart", type: "button" }, "↺ restart"),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "pac-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "pac-seed", class: "mz-input", value: S.cfg.seed, size: "10" }),
        h("button", { class: "mz-chip", id: "pac-dice", type: "button", title: "new random seed" }, "🎲")),
      h("button", { class: "mz-chip", id: "pac-bot", type: "button" }, "player: bot"),
      h("button", { class: "mz-chip", id: "pac-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "pac-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "pac-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "pac-stats-btn", type: "button" }, "ⓘ stats"),
      h("button", { class: "mz-chip", id: "pac-reset-params", type: "button" }, "reset params"),
      h("button", { class: "mz-chip", id: "pac-fs", type: "button" }, "⛶ full screen"),
    );

    const canvas = h("canvas", { id: "pac-canvas", width: "720", height: "720", role: "img",
      style: "aspect-ratio: 1 / 1",
      "aria-label": "Pac-maze board — eat pellets, dodge personality ghosts, power pellets turn the tables" });
    const under = h("div", { class: "mz-under game-under", id: "pac-under" });
    const trace = h("div", { class: "game-hintline", id: "pac-trace",
      "aria-live": "polite" }, "no decisions yet");

    const sizeOpts = [["10", "21×21"], ["15", "31×31"], ["20", "41×41"]].map(([v, l]) =>
      h("option", { value: v, selected: v === "15" ? "" : null }, l));

    const opts = DK.shell.accordion([
      {
        id: "board", label: "Board", open: true,
        kids: [
          h("label", { class: "game-slider" },
            h("span", { class: "game-slider-label" }, "board"),
            h("select", { id: "pac-board", class: "mz-input" },
              h("option", { value: "classic", selected: "" }, "arcade 28×31"),
              h("option", { value: "generated" }, "SDK generated maze"))),
          h("label", { class: "game-slider", id: "pac-size-row" },
            h("span", { class: "game-slider-label" }, "maze size"),
            h("select", { id: "pac-size", class: "mz-input" }, ...sizeOpts)),
          slider("pac-braid", "braiding (loops)", 0, 40, 2, 14, "%"),
          h("p", { class: "game-note", id: "pac-gen-note" },
            "maze size, braiding and pellet fill shape generated boards only"),
        ],
      },
      {
        id: "ghosts", label: "Ghosts",
        kids: [
          slider("pac-gcount", "ghost count", 1, 6, 1, 4),
          slider("pac-gspeed", "ghost speed", 1.5, 9, 0.3, 4.2),
          slider("pac-fright", "fright duration", 2, 15, 0.5, 7, "s"),
        ],
      },
      {
        id: "run", label: "Run",
        kids: [
          slider("pac-pspeed", "player speed", 2, 9, 0.2, 5.4),
          slider("pac-lives", "lives", 1, 5, 1, 3),
          slider("pac-fill", "pellet fill", 40, 100, 5, 85, "%"),
        ],
      },
      {
        id: "about", label: "What decides what?",
        kids: [h("p", { class: "game-note" },
          "On the arcade board the ghosts follow the classic targeting rules (blinky chases you, ",
          "pinky aims four ahead, inky triangulates, clyde wavers) through the scatter/chase ",
          "clock; on generated boards each ghost scores directions with its personality weights ",
          "instead. Either way the choice of direction at every junction is scored, and a genuine ",
          "tie goes to the engine; frightened retreat is a rule, not a choice. The trace line ",
          "names which rung answered every move.")],
      },
    ]);

    const main = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, canvas, under, trace),
      h("div", { class: "game-side" }, opts));
    root.appendChild(h("div", { class: "game-wrap" }, bar, main));

    buildOverlays();
    wire();
    S.seedKit = DK.shell.numericSeed(byId("pac-seed"), byId("pac-dice"), newGame);
    DK.shell.resetParams(byId("pac-reset-params"), root, newGame);
    DK.shell.fullscreen(root.querySelector(".game-wrap"), byId("pac-fs"));
    engine.setEnabled(true);   /* default on, like the maze: ties reach the runtime */
    syncGenerated();
    newGame();
    if (!window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      requestAnimationFrame(loop);
    } else {
      render(); /* reduced motion: static board, tick on demand */
    }
  }

  /* ---------- overlays (shared chrome, pac data) ---------- */
  function overlay(id, title, bodyId, closeId) {
    return h("dialog", { class: "mz-overlay", id },
      h("div", { class: "mz-board-card" },
        h("div", { class: "mz-overlay-head" },
          h("span", { class: "mz-overlay-title" }, title),
          h("button", { class: "mz-chip", id: closeId, type: "button" }, "✕ close")),
        h("div", { class: "mz-overlay-scroll", id: bodyId })));
  }

  function buildOverlays() {
    root.appendChild(overlay("pac-board-overlay", "Session scoreboard", "pac-board-body", "pac-board-close"));
    root.appendChild(overlay("pac-stats-overlay", "Measured stats", "pac-stats-body", "pac-stats-close"));
  }

  function renderBoard() {
    const body = byId("pac-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" },
        "No finished runs yet — lose your last life (or clear the maze) and the run lands here. ",
        "Session only: nothing you do leaves this tab."));
      return;
    }
    body.appendChild(h("table", { class: "game-table" },
      h("thead", null, h("tr", null,
        ...["#", "outcome", "score", "level", "pellets", "ghosts eaten", "engine ties", "seed"].map((x) => h("th", null, x)))),
      h("tbody", null, rows.map((r, i) => h("tr", null,
        h("td", null, String(rows.length - i)),
        h("td", null, r.outcome),
        h("td", null, String(r.score)),
        h("td", null, String(r.level)),
        h("td", null, String(r.pellets)),
        h("td", null, String(r.ghostsEaten)),
        h("td", null, String(r.engineCalls)),
        h("td", null, r.seed))))));
    body.appendChild(h("button", {
      class: "mz-chip", type: "button", onclick: () => { S.board.clear(); renderBoard(); },
    }, "clear board"));
  }

  function renderStats() {
    const body = byId("pac-stats-body");
    if (!body) return;
    body.textContent = "";
    const snap = S.meters.snapshot();
    const g = S.game;
    const rows = [
      ["frames/s (EMA)", S.fps.toFixed(0)],
      ["sim ticks", String(g ? g.ticks : 0)],
      ["decisions — weights", String(g ? g.rungs.weights : 0)],
      ["decisions — engine", String(g ? g.rungs.engine : 0)],
      ["moves — fright rule", String(g ? g.rungs.fright : 0)],
      ["moves — eyes rule", String(g ? g.rungs.eyes : 0)],
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
      "Measured in this tab only. The engine row counts genuine ties the WASM runtime answered; ",
      "fright and eyes moves are rules the code names as rules."));
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
        const outcome = g.lives <= 0 ? "game over" : "time up";
        S.board.record({
          ts: Date.now(), outcome,
          score: g.score, level: g.level,
          pellets: g.pelletsEaten, ghostsEaten: g.ghostsEaten,
          engineCalls: g.engineCalls, seed: g.cfg.seed,
          stamp: PC.pacStamp(S.cfg),
        });
        if (g.score > S.hi) S.hi = g.score;
        if (S.boardOpen) renderBoard();
      }
      return;
    }
    PC.tickGame(g, { want: S.want }, engine.status === "ready" ? engineChoose : null);
  }

  /* ---------- render ---------- */
  function drawGhost(ctx, cell, gh, fright, flashing, top, classic) {
    const cx = (gh.x + 0.5) * cell;
    const cy = top + (gh.y + 0.5) * cell;
    const r = cell * 0.42;
    const inHouse = classic ? gh.phase === "house" : gh.state === "house";
    const eyesOnly = classic ? gh.phase === "eyes" : gh.state === "eyes";
    if (inHouse || eyesOnly) {
      /* the arcade shows house ghosts as dimmed bodies; homing eyes only */
      const alpha = inHouse ? (classic ? 0.6 : 0.45) : 0.9;
      if (inHouse && !classic) {
        /* generated boards keep the old eyes-only house marker */
        ctx.globalAlpha = alpha;
      } else if (inHouse) {
        ctx.globalAlpha = alpha;
        ctx.fillStyle = fright ? PAL.fright : gh.color;
        ctx.beginPath();
        ctx.arc(cx, cy - r * 0.1, r, Math.PI, 0);
        ctx.lineTo(cx + r, cy + r * 0.75);
        ctx.lineTo(cx - r, cy + r * 0.75);
        ctx.closePath();
        ctx.fill();
      } else {
        ctx.globalAlpha = alpha;
      }
    } else {
      ctx.globalAlpha = 1;
      ctx.fillStyle = fright ? (flashing ? PAL.flash : PAL.fright) : gh.color;
      ctx.beginPath();
      ctx.arc(cx, cy - r * 0.1, r, Math.PI, 0);
      ctx.lineTo(cx + r, cy + r * 0.75);
      /* wavy skirt */
      const waves = 3;
      for (let i = 0; i < waves; i++) {
        const x0 = cx + r - (2 * r * (i + 0.5)) / waves;
        const x1 = cx + r - (2 * r * (i + 1)) / waves;
        ctx.lineTo(x0, cy + r * (i % 2 ? 0.75 : 0.45));
        ctx.lineTo(x1, cy + r * 0.75);
      }
      ctx.closePath();
      ctx.fill();
    }
    /* eyes look where the ghost is heading */
    const d = PC.DIRS[gh.dir] || { dx: 0, dy: 0 };
    for (const side of [-1, 1]) {
      ctx.fillStyle = "#f8fafc";
      ctx.beginPath();
      ctx.arc(cx + side * r * 0.34 + d.dx * r * 0.14,
        cy - r * 0.15 + d.dy * r * 0.14, r * 0.22, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = "#0f172a";
      ctx.beginPath();
      ctx.arc(cx + side * r * 0.34 + d.dx * r * 0.3,
        cy - r * 0.15 + d.dy * r * 0.3, r * 0.11, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.globalAlpha = 1;
  }

  function isWallC(b, x, y) {
    if (y < 0 || y >= b.h) return false;
    const wx = ((x % b.w) + b.w) % b.w;
    return b.grid[y * b.w + wx] === 1;
  }

  /* the arcade look: blue outline strokes tracing the wall/floor boundary,
     inset into the wall; segments extend to tile ends to join neighbors */
  function drawClassicWalls(ctx, b, cell, top) {
    const p = cell * 0.28;
    ctx.strokeStyle = "#2a3cdc";
    ctx.lineWidth = Math.max(1.5, cell * 0.09);
    ctx.lineCap = "round";
    const openAt = (x, y) => {
      if (y < 0 || y >= b.h) return false;
      const wx = ((x % b.w) + b.w) % b.w;
      return b.grid[y * b.w + wx] !== 1;       /* floor or the house door */
    };
    ctx.beginPath();
    for (let y = 0; y < b.h; y++) {
      for (let x = 0; x < b.w; x++) {
        if (!isWallC(b, x, y)) continue;
        const X = x * cell, Y = top + y * cell;
        if (openAt(x, y - 1)) {
          const x0 = isWallC(b, x - 1, y) && openAt(x - 1, y - 1) ? X : X + p;
          const x1 = isWallC(b, x + 1, y) && openAt(x + 1, y - 1) ? X + cell : X + cell - p;
          ctx.moveTo(x0, Y + p); ctx.lineTo(x1, Y + p);
        }
        if (openAt(x, y + 1)) {
          const x0 = isWallC(b, x - 1, y) && openAt(x - 1, y + 1) ? X : X + p;
          const x1 = isWallC(b, x + 1, y) && openAt(x + 1, y + 1) ? X + cell : X + cell - p;
          ctx.moveTo(x0, Y + cell - p); ctx.lineTo(x1, Y + cell - p);
        }
        if (openAt(x - 1, y)) {
          const y0 = isWallC(b, x, y - 1) && openAt(x - 1, y - 1) ? Y : Y + p;
          const y1 = isWallC(b, x, y + 1) && openAt(x - 1, y + 1) ? Y + cell : Y + cell - p;
          ctx.moveTo(X + p, y0); ctx.lineTo(X + p, y1);
        }
        if (openAt(x + 1, y)) {
          const y0 = isWallC(b, x, y - 1) && openAt(x + 1, y - 1) ? Y : Y + p;
          const y1 = isWallC(b, x, y + 1) && openAt(x + 1, y + 1) ? Y + cell : Y + cell - p;
          ctx.moveTo(X + cell - p, y0); ctx.lineTo(X + cell - p, y1);
        }
      }
    }
    ctx.stroke();
    /* the ghost-house gate */
    ctx.strokeStyle = "#ffb8de";
    ctx.lineWidth = Math.max(1.5, cell * 0.12);
    ctx.beginPath();
    for (const key of b.doorTiles || []) {
      const dx2 = key % b.w, dy2 = (key / b.w) | 0;
      ctx.moveTo(dx2 * cell + cell * 0.12, top + (dy2 + 0.5) * cell);
      ctx.lineTo((dx2 + 1) * cell - cell * 0.12, top + (dy2 + 0.5) * cell);
    }
    ctx.stroke();
  }

  /* the arcade chrome: 1UP / HIGH SCORE header, lives + level footer,
     READY! and GAME OVER under the house */
  function drawClassicHud(ctx, cell, top, g) {
    const mono = "bold " + Math.floor(cell * 0.72) + "px ui-monospace, Menlo, monospace";
    ctx.font = mono;
    ctx.textBaseline = "middle";
    ctx.fillStyle = "#f8fafc";
    ctx.textAlign = "left";
    ctx.fillText("1UP", cell, cell * 0.9);
    ctx.fillText(String(g.score), cell, cell * 1.9);
    ctx.textAlign = "center";
    ctx.fillText("HIGH SCORE", 14 * cell, cell * 0.9);
    ctx.fillText(String(Math.max(S.hi, g.score)), 14 * cell, cell * 1.9);
    const footY = (3 + 31) * cell + cell * 0.5;
    for (let i = 0; i < Math.max(0, g.lives - (g.over ? 0 : 1)); i++) {
      const cx = (2 + i * 1.7) * cell;
      ctx.fillStyle = PAL.player;
      ctx.beginPath();
      ctx.moveTo(cx, footY);
      ctx.arc(cx, footY, cell * 0.42, Math.PI * 0.2, Math.PI * 1.8);
      ctx.closePath();
      ctx.fill();
    }
    ctx.fillStyle = "#f8fafc";
    ctx.textAlign = "right";
    ctx.fillText("LEVEL " + g.level, 26 * cell, footY);
    ctx.textAlign = "center";
    if (!g.over && g.t < g.readyUntil) {
      ctx.fillStyle = PAL.player;
      ctx.fillText("READY!", 14 * cell, top + 17.5 * cell);
    }
    if (g.over) {
      ctx.fillStyle = "#ff6b6b";
      ctx.fillText(g.lives <= 0 ? "GAME  OVER" : "TIME  UP", 14 * cell, top + 17.5 * cell);
    }
  }

  function render() {
    const canvas = byId("pac-canvas");
    if (!canvas || !S.game) return;
    const g = S.game;
    const b = g.board;
    const classic = b.classic === true;
    const ctx = canvas.getContext("2d");
    const W = canvas.width;
    const cell = W / b.w;
    const top = classic ? 3 * cell : 0;
    ctx.fillStyle = PAL.bg;
    ctx.fillRect(0, 0, W, W);

    if (classic) {
      drawClassicWalls(ctx, b, cell, top);
    } else {
      ctx.fillStyle = PAL.wall;
      for (let y = 0; y < b.h; y++) {
        for (let x = 0; x < b.w; x++) {
          if (b.grid[y * b.w + x] !== 1) continue;   /* MazeCore.WALL === 1 */
          ctx.fillRect(x * cell + cell * 0.18, y * cell + cell * 0.18,
            cell * 0.64, cell * 0.64);
        }
      }
    }
    /* pellets + power pellets */
    const t = nowMs() / 1000;
    ctx.fillStyle = PAL.pellet;
    for (const key of b.pellets) {
      const x = key % b.w, y = (key / b.w) | 0;
      ctx.beginPath();
      ctx.arc((x + 0.5) * cell, top + (y + 0.5) * cell, Math.max(1.5, cell * 0.08), 0, Math.PI * 2);
      ctx.fill();
    }
    for (const key of b.powers) {
      if (g.eatenPowers.has(key)) continue;
      const x = key % b.w, y = (key / b.w) | 0;
      const pulse = 1 + 0.25 * Math.sin(t * 5);
      ctx.fillStyle = PAL.power;
      ctx.beginPath();
      ctx.arc((x + 0.5) * cell, top + (y + 0.5) * cell, cell * 0.24 * pulse, 0, Math.PI * 2);
      ctx.fill();
    }
    /* fruit: two cherries under the house, arcade schedule */
    if (classic && g.fruit) {
      const fx = 14 * cell, fy = top + 17.5 * cell;
      ctx.strokeStyle = "#4ade80";
      ctx.lineWidth = Math.max(1, cell * 0.07);
      ctx.beginPath();
      ctx.moveTo(fx - cell * 0.16, fy + cell * 0.08);
      ctx.quadraticCurveTo(fx, fy - cell * 0.3, fx + cell * 0.18, fy - cell * 0.22);
      ctx.moveTo(fx + cell * 0.16, fy + cell * 0.1);
      ctx.quadraticCurveTo(fx + cell * 0.2, fy - cell * 0.14, fx + cell * 0.18, fy - cell * 0.22);
      ctx.stroke();
      ctx.fillStyle = "#ff4d4d";
      for (const [ox, oy] of [[-0.16, 0.16], [0.16, 0.18]]) {
        ctx.beginPath();
        ctx.arc(fx + ox * cell, fy + oy * cell, cell * 0.14, 0, Math.PI * 2);
        ctx.fill();
      }
    }
    /* ghosts */
    const fright = g.t < g.frightUntil;
    const frightLeft = (g.frightUntil - g.t) / PC.TICKS_PER_S;
    for (const gh of g.ghosts) {
      drawGhost(ctx, cell, gh, fright, fright && frightLeft < 2 && Math.floor(t * 6) % 2 === 0,
        top, classic);
    }
    /* player — a wedge with an animated mouth, facing its direction */
    const p = g.player;
    const px = (p.x + 0.5) * cell;
    const py = top + (p.y + 0.5) * cell;
    const facing = { right: 0, down: Math.PI / 2, left: Math.PI, up: -Math.PI / 2 }[p.dir] || 0;
    const mouth = (0.12 + 0.16 * Math.abs(Math.sin(t * 9))) * Math.PI;
    ctx.fillStyle = PAL.player;
    ctx.beginPath();
    ctx.moveTo(px, py);
    ctx.arc(px, py, cell * 0.42, facing + mouth / 2, facing - mouth / 2);
    ctx.closePath();
    ctx.fill();

    if (classic) drawClassicHud(ctx, cell, top, g);

    /* under-row + trace line */
    const rungNames = { weights: "weights", engine: "engine", fright: "fright rule", eyes: "eyes rule" };
    const stateTxt = g.over ? (g.lives <= 0 ? "game over" : "time up")
      : g.t < (g.readyUntil || 0) ? "ready!"
      : S.running ? "running" : "paused";
    byId("pac-under").innerHTML = "";
    byId("pac-under").appendChild(DK.shell.kvRow([
      ["board", classic ? "arcade" : "generated"],
      ["state", stateTxt],
      ["score", String(g.score)],
      ["lives", "♥".repeat(Math.max(0, g.lives))],
      ["level", String(g.level)],
      ["pellets", String(b.pellets.size)],
      ["fright", fright ? frightLeft.toFixed(1) + "s" : "—"],
      ["engine ties", String(g.rungs.engine)],
    ]));
    const ld = g.lastDecision;
    if (ld) {
      byId("pac-trace").textContent = "last decision: " + ld.who + " @ (" +
        ld.at.x + "," + ld.at.y + ") → " + ld.dir + " · " + (rungNames[ld.rung] || ld.rung);
    }
  }

  /* ---------- wiring ---------- */
  function wire() {
    byId("pac-board").addEventListener("change", () => {
      syncGenerated();
      newGame();
    });
    byId("pac-play").addEventListener("click", (e) => {
      S.running = !S.running;
      e.currentTarget.textContent = S.running ? "⏸ pause" : "▶ play";
    });
    byId("pac-restart").addEventListener("click", () => newGame());
    byId("pac-seed").addEventListener("change", () => newGame());
    byId("pac-bot").addEventListener("click", (e) => {
      S.cfg.botPlayer = !(S.cfg.botPlayer === true);
      e.currentTarget.textContent = S.cfg.botPlayer ? "player: bot" : "player: human";
      if (S.game) S.game.cfg.botPlayer = S.cfg.botPlayer;
    });
    byId("pac-engine-toggle").addEventListener("click", (e) => {
      const on = engine.status === "off" || engine.status === "unavailable";
      engine.setEnabled(on);
      e.currentTarget.textContent = on ? "engine: on" : "engine: off";
    });
    byId("pac-board-btn").addEventListener("click", () => {
      S.boardOpen = true; S.statsOpen = false;
      byId("pac-stats-overlay").classList.remove("open");
      byId("pac-board-overlay").classList.add("open");
      renderBoard();
    });
    byId("pac-board-close").addEventListener("click", () => {
      S.boardOpen = false;
      byId("pac-board-overlay").classList.remove("open");
    });
    byId("pac-stats-btn").addEventListener("click", () => {
      S.statsOpen = true; S.boardOpen = false;
      byId("pac-board-overlay").classList.remove("open");
      byId("pac-stats-overlay").classList.add("open");
      renderStats();
    });
    byId("pac-stats-close").addEventListener("click", () => {
      S.statsOpen = false;
      byId("pac-stats-overlay").classList.remove("open");
    });

    /* keyboard: arrows or WASD queue a direction; space pauses; R restarts */
    const DIR_KEYS = {
      ArrowUp: "up", ArrowDown: "down", ArrowLeft: "left", ArrowRight: "right",
      w: "up", s: "down", a: "left", d: "right",
      W: "up", S: "down", A: "left", D: "right",
    };
    window.addEventListener("keydown", (e) => {
      if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
      const dir = DIR_KEYS[e.key];
      if (dir) { S.want = dir; e.preventDefault(); }
      else if (e.key === " ") {
        S.running = !S.running;
        byId("pac-play").textContent = S.running ? "⏸ pause" : "▶ play";
        e.preventDefault();
      } else if (e.key === "r" || e.key === "R") newGame();
      else if (e.key === "Escape") {
        S.boardOpen = false; S.statsOpen = false;
        byId("pac-board-overlay").classList.remove("open");
        byId("pac-stats-overlay").classList.remove("open");
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
