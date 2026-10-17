/* Pac-maze shell — the Playground Pacman game on the Decisions SDK.
   pac-core.js owns the deterministic sim; this file is chrome: canvas,
   controls, the ghost personality editors, the session-only scoreboard,
   the stats overlay, and the engine bridge. Ghost personalities score four
   candidate intents (pursue / intercept / patrol / retreat) at every tile
   arrival; genuine ties escalate to the engine. The player is keyboard or
   the bot. No DOM beyond the canvas; all chrome is in this file. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSdk;
  const PC = globalThis.PacCore;
  const root = document.getElementById("pac-root");
  if (!root || !DK || !PC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;

  /* canvas palette — dark in both site themes */
  const PAL = {
    bg: "#030c10",
    wall: "#1a3a5c",
    pellet: "#e8dcc8",
    power: "#f0e8d0",
    player: "#ffe84a",
    ghostFright: "#2244ff",
    ghostEyes: "#ffffff",
    ghostHouse: "#2a1040",
    text: "#c8dce0",
    dim: "#6888a0",
    teal: "#5eead4",
    green: "#34d399",
  };

  /* tile size — adapts to board dimensions */
  const TILE = 18;
  let CW = 0, CH = 0;  /* canvas dimensions, set after board is known */

  /* ---------- state ---------- */
  const S = {
    cfg: { ...PC.DEFAULT_CONFIG },
    game: null,
    running: false,
    last: 0, acc: 0,
    playerInput: null,   /* { want: "up"|"down"|"left"|right" } */
    keys: {},
    rung: { formula: 0, engine: 0, last: "formula" },
    boardOpen: false, statsOpen: false,
    board: DK.sessionBoard(),
    meters: DK.meters(),
    fps: 0,
  };

  const engine = DK.engineBridge({ onStatus: () => setChip() });

  function engineChoose(request) {
    const t0 = nowMs();
    const res = engine.choose(request);
    S.meters.sample("engine.ms", nowMs() - t0);
    if (res) { S.rung.engine += 1; S.rung.last = "engine"; }
    else { S.rung.formula += 1; S.rung.last = "formula"; }
    return res;
  }

  /* ---------- helpers ---------- */
  function cfgFromUI() {
    return {
      seed: S.seedKit ? S.seedKit.get() : "oc-pac",
      board: byId("pac-board-type") ? byId("pac-board-type").value : "classic",
      cells: PC.DEFAULT_CONFIG.cells,
      braidPct: PC.DEFAULT_CONFIG.braidPct,
      ghosts: parseInt(byId("pac-ghosts").value, 10),
      ghostSpeed: parseFloat(byId("pac-ghost-speed").value),
      playerSpeed: parseFloat(byId("pac-player-speed").value),
      frightS: parseFloat(byId("pac-fright").value),
      lives: parseInt(byId("pac-lives").value, 10),
      botPlayer: byId("pac-bot") ? byId("pac-bot").checked : false,
    };
  }

  function newGame() {
    S.cfg = PC.normalizeConfig(cfgFromUI());
    S.game = PC.createGame(S.cfg);
    S.rung = { formula: 0, engine: 0, last: "formula" };
    S.playerInput = null;
    resizeCanvas();
    render();
  }

  function resizeCanvas() {
    if (!S.game) return;
    CW = S.game.board.w * TILE;
    CH = S.game.board.h * TILE;
    const c = byId("pac-canvas");
    if (c) { c.width = CW; c.height = CH; }
  }

  function setChip() {
    const chip = byId("pac-engine-chip");
    if (!chip) return;
    const st = engine.status;
    chip.textContent = "engine: " + st;
    chip.classList.toggle("on", st === "ready");
    chip.classList.toggle("off", st === "unavailable" || st === "off");
  }

  /* ---------- keyboard ---------- */
  const KEY_MAP = {
    ArrowUp: "up", ArrowDown: "down", ArrowLeft: "left", ArrowRight: "right",
    w: "up", s: "down", a: "left", d: "right",
    W: "up", S: "down", A: "left", D: "right",
    " ": "pause",
  };

  function handleKey(e) {
    const action = KEY_MAP[e.key];
    if (!action) return;
    if (action === "pause") {
      togglePlayPause();
      return;
    }
    if (!S.game || S.game.over || S.cfg.botPlayer) return;
    e.preventDefault();
    S.playerInput = { want: action };
  }

  /* ---------- game loop ---------- */
  function loop(ts) {
    if (!S.running) return;
    const dt = ts - (S.last || ts);
    S.last = ts;

    /* tick at TICKS_PER_S regardless of frame rate */
    S.acc += dt;
    const msPerTick = 1000 / PC.TICKS_PER_S;
    while (S.acc >= msPerTick) {
      if (!S.game.over) {
        PC.tickGame(S.game, S.playerInput, engineChoose);
        S.playerInput = null;
      }
      S.acc -= msPerTick;
    }

    render();

    if (S.game.over) {
      S.running = false;
      byId("pac-play-btn").textContent = "▶ play";
      recordBoard();
    }

    if (S.running) requestAnimationFrame(loop);
  }

  function togglePlayPause() {
    if (!S.game) { newGame(); }
    if (S.game.over) { newGame(); }
    S.running = !S.running;
    byId("pac-play-btn").textContent = S.running ? "⏸ pause" : "▶ play";
    if (S.running) {
      S.last = 0; S.acc = 0;
      requestAnimationFrame(loop);
    }
  }

  function recordBoard() {
    if (!S.game) return;
    const sum = PC.runSummary(S.game);
    S.board.record({
      ts: Date.now(),
      outcome: S.game.over ? (S.game.lives > 0 ? "win" : "gameover") : "running",
      score: sum.score,
      level: sum.level,
      lives: sum.lives,
      pellets: sum.pelletsEaten + "/" + sum.pelletsTotal,
      engineCalls: sum.engineCalls,
      seed: S.cfg.seed,
      stamp: PC.pacStamp(S.cfg),
    });
  }

  /* ---------- render ---------- */
  function render() {
    const canvas = byId("pac-canvas");
    if (!canvas || !S.game) return;
    const ctx = canvas.getContext("2d");
    ctx.clearRect(0, 0, CW, CH);

    const { board, player: p, ghosts, t, frightUntil } = S.game;
    const W = board.w, H = board.h;
    const fright = t < frightUntil;

    /* background */
    ctx.fillStyle = PAL.bg;
    ctx.fillRect(0, 0, CW, CH);

    /* walls + pellets */
    for (let y = 0; y < H; y++) {
      for (let x = 0; x < W; x++) {
        const key = y * W + x;
        const gx = x * TILE, gy = y * TILE;
        if (board.grid[key] === 1) {
          ctx.fillStyle = PAL.wall;
          ctx.fillRect(gx, gy, TILE, TILE);
        } else {
          const pelletR = TILE * 0.13;
          if (board.pellets.has(key)) {
            ctx.fillStyle = PAL.pellet;
            ctx.beginPath();
            ctx.arc(gx + TILE / 2, gy + TILE / 2, pelletR, 0, Math.PI * 2);
            ctx.fill();
          } else if (board.powers.includes(key)) {
            ctx.fillStyle = PAL.power;
            ctx.beginPath();
            ctx.arc(gx + TILE / 2, gy + TILE / 2, pelletR * 2.2, 0, Math.PI * 2);
            ctx.fill();
          }
        }
      }
    }

    /* ghost house */
    const hw = 5 * TILE, hh = 3 * TILE;
    const hx = Math.floor(W / 2) * TILE - hw / 2, hy = 11 * TILE;
    ctx.fillStyle = PAL.ghostHouse;
    ctx.fillRect(hx, hy, hw, hh);

    /* entity interpolation: each entity has (x,y) = tile, (px,py) = previous tile,
       prog = 0..1 progress from px,py toward x,y */
    function entPx(e) { return (e.px + (e.x - e.px) * e.prog) * TILE + TILE / 2; }
    function entPy(e) { return (e.py + (e.y - e.py) * e.prog) * TILE + TILE / 2; }
    const PR = TILE * 0.38;  /* player radius */

    /* player */
    const ppx = entPx(p), ppy = entPy(p);
    ctx.fillStyle = PAL.player;
    ctx.beginPath();
    ctx.arc(ppx, ppy, PR, 0, Math.PI * 2);
    ctx.fill();
    /* eyes — direction indicator */
    const eyeOffsets = { left: [-3, 0], right: [3, 0], up: [0, -3], down: [0, 3] };
    const [ex, ey] = eyeOffsets[p.dir] || [0, 0];
    ctx.fillStyle = "#222";
    ctx.beginPath();
    ctx.arc(ppx + ex, ppy + ey, PR * 0.22, 0, Math.PI * 2);
    ctx.fill();

    /* ghosts */
    for (const gh of ghosts) {
      const gpx = entPx(gh), gpy = entPy(gh);
      if (gpx === undefined) continue;
      const GR = TILE * 0.40;
      let color = gh.color;
      if (fright && gh.phase !== "eyes") color = PAL.ghostFright;
      else if (gh.phase === "eyes") color = PAL.ghostEyes;
      ctx.fillStyle = color;
      ctx.beginPath();
      ctx.arc(gpx, gpy, GR, Math.PI, 0);  /* top semi */
      /* body */
      const bx = gpx - GR, by = gpy;
      const bw = GR * 2, bh = GR;
      ctx.fillRect(bx, by, bw, bh);
      /* skirt */
      const skirtW = GR * 0.5;
      for (let i = 0; i < 3; i++) {
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.arc(bx + skirtW * (i * 2 + 1), by + bh, skirtW * 0.9, 0, Math.PI);
        ctx.fill();
      }
      /* eyes on body */
      if (gh.phase !== "eyes") {
        ctx.fillStyle = "#fff";
        ctx.beginPath();
        ctx.arc(gpx - GR * 0.3, gpy - GR * 0.1, GR * 0.2, 0, Math.PI * 2);
        ctx.arc(gpx + GR * 0.3, gpy - GR * 0.1, GR * 0.2, 0, Math.PI * 2);
        ctx.fill();
        ctx.fillStyle = "#113";
        ctx.beginPath();
        ctx.arc(gpx - GR * 0.3, gpy - GR * 0.1, GR * 0.1, 0, Math.PI * 2);
        ctx.arc(gpx + GR * 0.3, gpy - GR * 0.1, GR * 0.1, 0, Math.PI * 2);
        ctx.fill();
      }
    }

    /* HUD */
    ctx.fillStyle = PAL.text;
    ctx.font = "bold 13px monospace";
    ctx.textAlign = "left";
    ctx.fillText("score " + S.game.score, 4, 14);
    ctx.fillText("lives " + S.game.lives, 4, 28);
    ctx.fillText("level " + S.game.level, 4, 42);
    ctx.textAlign = "right";
    ctx.fillText("pellets " + (board.pelletsTotal - board.pellets.size) + "/" + board.pelletsTotal, CW - 4, 14);

    if (S.game.over) {
      ctx.fillStyle = "rgba(0,0,0,0.55)";
      ctx.fillRect(0, 0, CW, CH);
      ctx.fillStyle = PAL.player;
      ctx.font = "bold 22px monospace";
      ctx.textAlign = "center";
      ctx.fillText(S.game.lives > 0 ? "YOU WIN!" : "GAME OVER", CW / 2, CH / 2 - 12);
      ctx.fillStyle = PAL.dim;
      ctx.font = "14px monospace";
      ctx.fillText("score: " + S.game.score + "   seed: " + S.cfg.seed, CW / 2, CH / 2 + 14);
    }
  }

  /* ---------- UI chrome helpers ---------- */
  function slider(id, label, min, max, step, val, unit) {
    const out = h("span", { class: "pac-sliderval", id: id + "-val" }, String(val) + (unit || ""));
    const input = h("input", {
      type: "range", id, min: String(min), max: String(max), step: String(step), value: String(val),
      "aria-label": label,
    });
    input.addEventListener("input", () => { out.textContent = input.value + (unit || ""); });
    input.addEventListener("change", () => newGame());
    return h("label", { class: "pac-slider" },
      h("span", { class: "pac-slider-label" }, label), input, out);
  }

  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) S.cfg.seed = deepSeed;

    /* top bar */
    const bar = h("div", { class: "game-bar" },
      h("button", { class: "mz-chip mz-play", id: "pac-play-btn", type: "button" }, "▶ play"),
      h("button", { class: "mz-chip", id: "pac-restart", type: "button" }, "↺ restart"),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "pac-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "pac-seed", class: "mz-input", value: S.cfg.seed, size: "10" }),
        h("button", { class: "mz-chip", id: "pac-dice", type: "button", title: "new random seed" }, "🎲")),
      h("span", { class: "mz-chip game-rung", id: "pac-rung-chip" }, "—"),
      h("button", { class: "mz-chip", id: "pac-bot-btn", type: "button" }, "bot: " + (S.cfg.botPlayer ? "on" : "off")),
      h("button", { class: "mz-chip", id: "pac-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "pac-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "pac-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "pac-stats-btn", type: "button" }, "ⓘ stats"),
      h("button", { class: "mz-chip", id: "pac-reset-params", type: "button" }, "reset params"),
      h("button", { class: "mz-chip", id: "pac-fs", type: "button" }, "⛶ full screen"),
    );

    /* canvas + under-readout */
    const canvas = h("canvas", {
      id: "pac-canvas", role: "img",
      "aria-label": "Pac-maze — eat pellets, avoid ghosts, use the decision engine to break ties",
      tabIndex: "0",
    });
    canvas.addEventListener("keydown", handleKey);
    const under = h("div", { class: "mz-under game-under", id: "pac-under" },
      h("span", { class: "mz-kv" }, h("b", { id: "pac-hint" }, "arrows / WASD to move; space to pause")));

    /* options accordion */
    const opts = DK.shell.accordion([
      {
        id: "game", label: "Game", open: true,
        kids: [
          h("label", { class: "pac-slider" },
            h("span", { class: "pac-slider-label" }, "board"),
            h("select", { id: "pac-board-type", "aria-label": "board type" },
              h("option", { value: "classic" }, "arcade \u2014 classic layout"),
              h("option", { value: "generated" }, "generated \u2014 SDK maze"))),
          slider("pac-ghosts", "ghosts", 1, 6, 1, 4),
          slider("pac-lives", "lives", 1, 5, 1, 3),
          h("label", { class: "pac-slider" },
            h("span", { class: "pac-slider-label" }, "bot player"),
            h("input", { type: "checkbox", id: "pac-bot", "aria-label": "bot player" })),
        ],
      },
      {
        id: "physics", label: "Physics",
        kids: [
          slider("pac-player-speed", "player speed", 2, 9, 0.5, 5.4),
          slider("pac-ghost-speed", "ghost speed", 1.5, 9, 0.5, 4.2),
          slider("pac-fright", "fright duration (s)", 2, 15, 1, 7),
        ],
      },
      {
        id: "about", label: "What decides what",
        kids: [h("p", { class: "game-note" },
          "Ghosts score four candidate intents (pursue, intercept, patrol, retreat) ",
          "at every tile arrival using the weight knobs below. Genuine ties — two ",
          "directions scoring identically — escalate to the OpenCodifier engine on ",
          "the wire. Frightened retreat is a rule, never a choice. You or the bot ",
          "control the player; ghosts are fully AI-driven.")],
      },
    ]);

    const main = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, canvas, under),
      h("div", { class: "game-side" }, opts));
    root.appendChild(h("div", { class: "pac-wrap" }, bar, main));

    /* overlays */
    root.appendChild(buildOverlay("pac-board-overlay", "Session scoreboard", "pac-board-body", "pac-board-close"));
    root.appendChild(buildOverlay("pac-stats-overlay", "Measured stats", "pac-stats-body", "pac-stats-close"));

    wire();
    S.seedKit = DK.shell.numericSeed(byId("pac-seed"), byId("pac-dice"), newGame);
    DK.shell.resetParams(byId("pac-reset-params"), root, newGame);
    DK.shell.fullscreen(root.querySelector(".pac-wrap"), byId("pac-fs"));
    engine.setEnabled(true);
    newGame();
    if (!window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      /* loop starts on play button press */
    } else {
      render();
    }
  }

  function buildOverlay(id, title, bodyId, closeId) {
    return h("dialog", { class: "mz-overlay", id },
      h("div", { class: "mz-board-card" },
        h("div", { class: "mz-overlay-head" },
          h("span", { class: "mz-overlay-title" }, title),
          h("button", { class: "mz-chip", id: closeId, type: "button" }, "✕ close")),
        h("div", { class: "mz-overlay-scroll", id: bodyId })));
  }

  function wire() {
    byId("pac-play-btn").addEventListener("click", togglePlayPause);
    byId("pac-restart").addEventListener("click", () => { S.running = false; newGame(); });
    byId("pac-bot-btn").addEventListener("click", () => {
      S.cfg.botPlayer = !S.cfg.botPlayer;
      byId("pac-bot-btn").textContent = "bot: " + (S.cfg.botPlayer ? "on" : "off");
      if (byId("pac-bot")) byId("pac-bot").checked = S.cfg.botPlayer;
      newGame();
    });
    byId("pac-engine-toggle").addEventListener("click", () => {
      engine.setEnabled(!engine._enabled);
      byId("pac-engine-toggle").textContent = "engine: " + (engine._enabled ? "on" : "off");
    });
    byId("pac-board-btn").addEventListener("click", () => {
      renderBoard(); byId("pac-board-overlay").showModal();
    });
    byId("pac-stats-btn").addEventListener("click", () => {
      renderStats(); byId("pac-stats-overlay").showModal();
    });
    byId("pac-board-close") && byId("pac-board-close").addEventListener("click", () => byId("pac-board-overlay").close());
    byId("pac-stats-close") && byId("pac-stats-close").addEventListener("click", () => byId("pac-stats-overlay").close());
    document.addEventListener("keydown", handleKey);
  }

  function renderBoard() {
    const body = byId("pac-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" },
        "No finished games yet — play one and it lands here. Session only: nothing leaves this tab."));
      return;
    }
    const tbl = h("table", { class: "mz-table" },
      h("thead", null, h("tr", null,
        ...["#", "outcome", "score", "level", "pellets", "seed"].map((x) => h("th", null, x)))),
      h("tbody", null, rows.slice().reverse().map((r, i) => h("tr", null,
        h("td", null, String(i + 1)),
        h("td", null, r.outcome),
        h("td", null, String(r.score)),
        h("td", null, String(r.level)),
        h("td", null, r.pellets),
        h("td", null, r.seed)))));
    body.appendChild(tbl);
    body.appendChild(h("button", {
      class: "mz-chip", type: "button", onclick: () => { S.board.clear(); renderBoard(); },
    }, "clear board"));
  }

  function renderStats() {
    const body = byId("pac-stats-body");
    if (!body) return;
    body.textContent = "";
    const m = S.meters.all();
    const rows = Object.entries(m).sort((a, b) => a[0].localeCompare(b[0]));
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" }, "No stats yet — play for a bit."));
      return;
    }
    const tbl = h("table", { class: "mz-table" },
      h("thead", null, h("tr", null, h("th", null, "metric"), h("th", null, "value"))),
      h("tbody", null, rows.map(([k, v]) => h("tr", null,
        h("td", null, k), h("td", null, String(v))))));
    body.appendChild(tbl);
  }

  build();
})();
