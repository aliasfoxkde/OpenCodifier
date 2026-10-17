/* Skewb shell — a corner-turning puzzle you can scramble and solve.
   skewb-core.js owns the state model; this file is chrome:
   2D facelet diagram (6 faces), move buttons,
   scramble/solve, step-through coach, session board. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSdk;
  const SC = globalThis.SkewbCore;
  const root = document.getElementById("skb-root");
  if (!root || !DK || !SC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;

  /* colours */
  const STICKER = { U: "#ffd23e", R: "#e94b3c", F: "#4da44e", D: "#f5f0e6", L: "#f97f1b", B: "#3d7de0" };
  const DARK = "#101418";
  const BG = "#0c1210";

  /* Skewb: 6 faces × 5 stickers = 30. Layout as a cross:
       [U]
     [L][F][R]
       [D]
       [B]
  Each face is a 5-sticker arrangement (a central triangle + 3 corner triangles + 1 center).
  Actually: 5 stickers per face = 1 center + 4 corner pieces.
  For the Skewb, we draw each face as a diamond shape with 5 regions. */
  const TILE = 30;

  /* Face positions: center of each face diamond in the canvas */
  const FACE_POS = {
    U: { x: 4 * TILE, y: 0 },
    L: { x: 0, y: 3 * TILE },
    F: { x: 4 * TILE, y: 3 * TILE },
    R: { x: 8 * TILE, y: 3 * TILE },
    D: { x: 4 * TILE, y: 6 * TILE },
    B: { x: 4 * TILE, y: 9 * TILE },
  };

  const S = {
    cfg: { ...SC.DEFAULT_CONFIG },
    state: null,
    scramble: [],
    history: [],
    plan: null, stepIdx: 0,
    auto: false, autoTimer: 0, speed: 1,
    board: DK.sessionBoard(),
    meters: DK.meters(),
  };

  const engine = DK.engineBridge({ onStatus: () => setChip() });

  function engineChoose(request) {
    const t0 = nowMs();
    const res = engine.choose(request);
    S.meters.sample("engine.ms", nowMs() - t0);
    return res;
  }

  /* Draw a single face as a diamond with 5 coloured regions:
     4 corner triangles around a central pentagon. */
  function drawFace(ctx, cx, cy, faceKey, fIdx, state) {
    const dim = TILE * 2.2;
    const r = dim / 2;

    /* The 5 stickers of a Skewb face: center (0) + 4 corners (1-4) */
    const fc = STICKER[faceKey];
    const faceBase = fIdx * 5;

    const colors = [
      state[faceBase]     ? (STICKER[state[faceBase]] || DARK) : DARK,     /* center */
      state[faceBase + 1] ? (STICKER[state[faceBase + 1]] || DARK) : DARK,
      state[faceBase + 2] ? (STICKER[state[faceBase + 2]] || DARK) : DARK,
      state[faceBase + 3] ? (STICKER[state[faceBase + 3]] || DARK) : DARK,
      state[faceBase + 4] ? (STICKER[state[faceBase + 4]] || DARK) : DARK,
    ];

    /* Draw diamond shape for the face */
    const diamond = (ox, oy, size, color) => {
      ctx.fillStyle = color;
      ctx.strokeStyle = "rgba(0,0,0,0.3)";
      ctx.lineWidth = 0.8;
      ctx.beginPath();
      ctx.moveTo(ox, oy - size);
      ctx.lineTo(ox + size, oy);
      ctx.lineTo(ox, oy + size);
      ctx.lineTo(ox - size, oy);
      ctx.closePath();
      ctx.fill();
      ctx.stroke();
    };

    /* Draw the central pentagon (face center sticker) */
    const pentagon = (ox, oy, size, color) => {
      ctx.fillStyle = color;
      ctx.strokeStyle = "rgba(0,0,0,0.3)";
      ctx.lineWidth = 0.8;
      ctx.beginPath();
      for (let i = 0; i < 5; i++) {
        const angle = (i * 72 - 90) * Math.PI / 180;
        const px = ox + size * Math.cos(angle);
        const py = oy + size * Math.sin(angle);
        if (i === 0) ctx.moveTo(px, py);
        else ctx.lineTo(px, py);
      }
      ctx.closePath();
      ctx.fill();
      ctx.stroke();
    };

    /* center */
    pentagon(cx, cy, r * 0.35, colors[0]);

    /* 4 corner triangles */
    const corners = [
      { dx: 0, dy: -r, label: 1 },   /* top */
      { dx: r, dy: 0, label: 2 },    /* right */
      { dx: 0, dy: r, label: 3 },    /* bottom */
      { dx: -r, dy: 0, label: 4 },   /* left */
    ];
    corners.forEach(({ dx, dy, label }) => {
      const tx = cx + dx * 0.65, ty = cy + dy * 0.65;
      const ts = r * 0.42;
      ctx.fillStyle = colors[label];
      ctx.strokeStyle = "rgba(0,0,0,0.3)";
      ctx.lineWidth = 0.8;
      ctx.beginPath();
      ctx.moveTo(tx, ty - ts);
      ctx.lineTo(tx + ts, ty + ts * 0.5);
      ctx.lineTo(tx - ts, ty + ts * 0.5);
      ctx.closePath();
      ctx.fill();
      ctx.stroke();
    });
  }

  function render() {
    const canvas = byId("skb-canvas");
    if (!canvas || !S.state) return;
    const ctx = canvas.getContext("2d");
    const W = canvas.width, Hc = canvas.height;
    ctx.clearRect(0, 0, W, Hc);

    ctx.fillStyle = BG;
    ctx.fillRect(0, 0, W, Hc);

    const fIdx = { U: 0, R: 1, F: 2, D: 3, L: 4, B: 5 };
    for (const [fk, pos] of Object.entries(FACE_POS)) {
      drawFace(ctx, pos.x, pos.y, fk, fIdx[fk], S.state);
    }

    ctx.fillStyle = "#c8dce0";
    ctx.font = "bold 13px monospace";
    ctx.textAlign = "left";
    const solved = SC.verifySolved(S.state).solved;
    ctx.fillText(solved ? "SOLVED" : "scrambled", 4, 14);
    ctx.fillText("moves: " + S.history.length, 4, 30);
    if (S.plan) {
      ctx.fillText("plan: step " + (S.stepIdx + 1) + "/" + S.plan.steps.length, 4, 46);
    }
  }

  function setChip() {
    const chip = byId("skb-engine-chip");
    if (!chip) return;
    const st = engine.status;
    chip.textContent = "engine: " + st;
    chip.classList.toggle("on", st === "ready");
  }

  function applyTok(tok) {
    S.state = SC.applyMove(S.state, tok);
    S.history.push(tok);
  }

  function solve() {
    if (S.auto) stopAuto();
    const solved = SC.verifySolved(S.state).solved;
    if (solved) { coach("already solved — scramble it first."); return; }
    coach("solver not yet implemented — try solving by hand.");
    render();
  }

  function stepFwd() {
    if (!S.plan || S.stepIdx >= S.plan.steps.length) return;
    const st = S.plan.steps[S.stepIdx];
    for (const t of st.moves) applyTok(t);
    coach(st.detail || st.title + " — " + st.moves.join(" "));
    highlightStep(S.stepIdx);
    S.stepIdx++;
    if (S.stepIdx >= S.plan.steps.length) {
      S.plan = null; S.stepIdx = 0;
      recordBoard();
      coach("solved in " + S.history.length + " moves. Scramble for another round.");
    }
    render();
  }

  function stepBack() {
    if (!S.plan || S.stepIdx === 0) return;
    S.stepIdx--;
    const st = S.plan.steps[S.stepIdx];
    for (let i = st.moves.length - 1; i >= 0; i--) {
      const t = st.moves[i];
      const inv = t.endsWith("2") ? t : t.endsWith("'") ? t[0] : t[0] + "'";
      S.state = SC.applyMove(S.state, inv);
      S.history.pop();
    }
    coach("stepped back: " + st.title);
    render();
  }

  function startAuto() {
    if (!S.plan) solve();
    if (!S.plan) return;
    S.auto = true;
    byId("skb-play-btn").textContent = "⏸ pause";
    tickAuto();
  }

  function tickAuto() {
    if (!S.auto) return;
    stepFwd();
    if (S.auto) S.autoTimer = setTimeout(tickAuto, 600 / S.speed);
  }

  function stopAuto() {
    S.auto = false;
    clearTimeout(S.autoTimer);
    const b = byId("skb-play-btn");
    if (b) b.textContent = "▶ solve";
  }

  function recordBoard() {
    S.board.record({
      ts: Date.now(), outcome: "solved",
      moves: S.history.length,
      seed: S.cfg.seed,
      stamp: SC.skewbStamp(S.cfg),
    });
  }

  function coach(text) { byId("skb-coach").textContent = text; }

  function highlightStep(i) {
    const list = byId("skb-steplist");
    if (!list) return;
    Array.from(list.children).forEach((el, idx) => {
      el.classList.toggle("now", idx === i);
    });
  }

  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) S.cfg.seed = String(parseInt(deepSeed, 10) || DK.randomSeed());

    const bar = h("div", { class: "game-bar" },
      h("button", { class: "mz-chip", id: "skb-scramble-btn", type: "button" }, "⟲ scramble"),
      h("button", { class: "mz-chip mz-play", id: "skb-solve-btn", type: "button" }, "solve"),
      h("button", { class: "mz-chip mz-play", id: "skb-play-btn", type: "button" }, "▶ solve"),
      h("button", { class: "mz-chip", id: "skb-back-btn", type: "button" }, "⏪"),
      h("button", { class: "mz-chip", id: "skb-fwd-btn", type: "button" }, "⏩"),
      h("button", { class: "mz-chip", id: "skb-undo-btn", type: "button" }, "↶ undo"),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "skb-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "skb-seed", class: "mz-input", size: "8", placeholder: "random", min: "0" })),
      h("button", { class: "mz-chip", id: "skb-dice", type: "button" }, "🎲"),
      h("span", { class: "mz-chip game-rung", id: "skb-solved-chip" }, "ready"),
      h("button", { class: "mz-chip", id: "skb-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "skb-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "skb-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "skb-fs-btn", type: "button" }, "⛶ full screen"),
    );

    /* Skewb cross layout canvas */
    const CW = 9 * TILE + 4, CH = 11 * TILE + 4;
    const canvas = h("canvas", {
      id: "skb-canvas", width: String(CW), height: String(CH),
      role: "img", "aria-label": "Skewb puzzle",
    });
    const under = h("div", { class: "mz-under game-under" },
      h("span", { class: "mz-kv" }, h("b", { id: "skb-progress" }, "ready")));

    /* move buttons: 4 axes × (clockwise + prime) */
    const axes = SC.AXES;
    const moveArea = h("div", { class: "cube-moves" },
      ...axes.flatMap(a => [
        h("button", { class: "mz-chip", type: "button", "data-move": a }, a),
        h("button", { class: "mz-chip", type: "button", "data-move": a + "'" }, a + "'"),
      ]));

    const stepPane = h("div", null,
      h("div", { class: "cube-steplist", id: "skb-steplist" }),
      h("p", { class: "game-note" }, "Steps appear once you press solve."));

    const main = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, canvas, under,
        h("p", { class: "game-hintline", id: "skb-coach", "aria-live": "polite" }, "")),
      h("div", { class: "game-side" }, moveArea, stepPane));

    root.appendChild(h("div", { class: "skb-wrap" }, bar, main));

    wire();
    S.seedKit = DK.shell.numericSeed(byId("skb-seed"), byId("skb-dice"), newGame);
    DK.shell.fullscreen(root.querySelector(".skb-wrap"), byId("skb-fs"));
    engine.setEnabled(true);
    newGame();
    render();
  }

  function newGame() {
    S.cfg = SC.normalizeConfig({ seed: S.seedKit ? S.seedKit.get() : "skewb-lbl" });
    const g = SC.createGame(S.cfg);
    S.state = g.state;
    S.history = [];
    S.scramble = g.scramble;
    S.plan = null; S.stepIdx = 0;
    if (S.auto) stopAuto();
    render();
    coach("scrambled — turn the axes or ask for a plan.");
  }

  function scramble() {
    if (S.auto) stopAuto();
    S.cfg = SC.normalizeConfig({ seed: byId("skb-seed") ? byId("skb-seed").value : DK.randomSeed() });
    const g = SC.createGame(S.cfg);
    S.state = g.state;
    S.history = [];
    S.scramble = g.scramble;
    S.plan = null; S.stepIdx = 0;
    render();
    coach("scrambled with seed " + S.cfg.seed + " — " + g.scramble.join(" "));
  }

  function wire() {
    byId("skb-scramble-btn").addEventListener("click", scramble);
    byId("skb-solve-btn").addEventListener("click", solve);
    byId("skb-play-btn").addEventListener("click", startAuto);
    byId("skb-back-btn").addEventListener("click", stepBack);
    byId("skb-fwd-btn").addEventListener("click", stepFwd);
    byId("skb-undo-btn").addEventListener("click", () => {
      if (S.auto) stopAuto();
      const tok = S.history.pop();
      if (!tok) return;
      const inv = tok.endsWith("2") ? tok : tok.endsWith("'") ? tok[0] : tok[0] + "'";
      S.state = SC.applyMove(S.state, inv);
      S.plan = null; S.stepIdx = 0;
      render();
    });
    byId("skb-engine-toggle").addEventListener("click", () => {
      engine.setEnabled(!engine._enabled);
      byId("skb-engine-toggle").textContent = "engine: " + (engine._enabled ? "on" : "off");
    });
    byId("skb-board-btn").addEventListener("click", () => {
      renderBoard(); byId("skb-board-overlay") && byId("skb-board-overlay").showModal();
    });
    byId("skb-fs-btn") && byId("skb-fs-btn").addEventListener("click", () => {
      const el = root.querySelector(".skb-wrap");
      if (el && el.requestFullscreen) el.requestFullscreen();
    });

    document.querySelectorAll("[data-move]").forEach(btn => {
      btn.addEventListener("click", () => {
        const tok = btn.getAttribute("data-move");
        if (S.auto) stopAuto();
        if (S.plan) { S.plan = null; S.stepIdx = 0; }
        applyTok(tok);
        coach("you: " + tok);
        render();
      });
    });
  }

  function renderBoard() {
    const body = byId("skb-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" }, "No solved puzzles yet."));
      return;
    }
    const tbl = h("table", { class: "mz-table" },
      h("thead", null, h("tr", null,
        ...["#", "moves", "seed"].map(x => h("th", null, x)))),
      h("tbody", null, rows.slice().reverse().map((r, i) => h("tr", null,
        h("td", null, String(i + 1)),
        h("td", null, String(r.moves)),
        h("td", null, r.seed)))));
    body.appendChild(tbl);
    body.appendChild(h("button", {
      class: "mz-chip", type: "button", onclick: () => { S.board.clear(); renderBoard(); }
    }, "clear board"));
  }

  build();
})();
