/* Pyraminx shell — a tetrahedron puzzle you can scramble and solve.
   pyraminx-core.js owns the state model and solver; this file is chrome:
   2D facelet diagram (4 faces laid flat as triangles), move buttons,
   scramble/solve, step-through coach with rung chips, session board. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSdk;
  const PC = globalThis.PyraminxCore;
  const root = document.getElementById("pyr-root");
  if (!root || !DK || !PC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;

  /* colours matching the cube palette */
  const STICKER = { U: "#ffd23e", L: "#e94b3c", R: "#4da44e", B: "#3d7de0" };
  const DARK = "#101418";
  const BG = "#0c1210";

  /* 2D canvas: four triangles in a diamond layout.
     U at top, L at bottom-left, R at bottom-right, B at top-back.
     Each triangle is 3×3 grid of small triangles pointing outward. */
  const TILE = 28;
  const H = TILE * Math.sqrt(3) / 2;  /* height of equilateral triangle side TILE */

  /* Positions of each face's top-left corner in the canvas (centers) */
  const FACE_POS = {
    U: { x: 3 * TILE, y: 0 },
    L: { x: 0, y: 3 * H },
    R: { x: 6 * TILE, y: 3 * H },
    B: { x: 3 * TILE, y: 3 * H },  /* B sits behind, lower on screen */
  };

  const S = {
    cfg: { ...PC.DEFAULT_CONFIG },
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

  /* facelet index for face f (0..3) and local index i (0..8) */
  function faceletIdx(f, i) { return f * 9 + i; }

  /* Draw one triangular face as a grid of small triangles.
     Each facelet i (0..8) maps to a sub-triangle within the face triangle. */
  function drawFace(ctx, cx, cy, faceKey, fIdx, state) {
    const fc = STICKER[faceKey];
    const dim = TILE * 0.88;

    /* face triangle: point up for U, point down for others? No - all face outward.
       We draw each face as an upward-pointing triangle subdivided into 9 upward
       mini-triangles (3 along each edge = 9 tips). */
    const tri = (x1, y1, x2, y2, x3, y3, color) => {
      ctx.fillStyle = color;
      ctx.beginPath();
      ctx.moveTo(x1, y1);
      ctx.lineTo(x2, y2);
      ctx.lineTo(x3, y3);
      ctx.closePath();
      ctx.fill();
      ctx.strokeStyle = "rgba(0,0,0,0.25)";
      ctx.lineWidth = 0.5;
      ctx.stroke();
    };

    /* The 9 mini-triangles form the face grid:
       row 0: tip (0)
       row 1: tips 1,2
       row 2: tips 3,4,5
       row 3: tips 6,7,8 (base)
       Each mini-triangle is upright.
       Side of the face = dim. Mini height = dim/3. */
    const mh = dim / 3;

    /* origin of this face's bounding triangle */
    const bx = cx - dim / 2, by = cy;

    for (let row = 0; row < 4; row++) {
      const n = row + 1;  /* number of mini-triangles in this row */
      const rowY = by - (3 - row) * mh;  /* y of the TOP of this row */
      const totalW = n * (dim / 3);
      const startX = bx + (dim - totalW) / 2;
      for (let col = 0; col < n; col++) {
        const miniIdx = (row * (row + 1)) / 2 + col;  /* triangular number */
        const si = faceletIdx(fIdx, miniIdx);
        const color = state[si] !== undefined ? (STICKER[state[si]] || DARK) : DARK;
        const tx = startX + col * (dim / 3) + (dim / 6);
        const ty = rowY;
        /* Draw upright mini-triangle */
        tri(tx, ty, tx - dim / 6, ty + mh, tx + dim / 6, ty + mh, color);
      }
    }
  }

  function render() {
    const canvas = byId("pyr-canvas");
    if (!canvas || !S.state) return;
    const ctx = canvas.getContext("2d");
    const W = canvas.width, Hc = canvas.height;
    ctx.clearRect(0, 0, W, Hc);

    /* Background */
    ctx.fillStyle = BG;
    ctx.fillRect(0, 0, W, Hc);

    /* Draw each face */
    const fIdx = { U: 0, L: 1, R: 2, B: 3 };
    for (const [fk, pos] of Object.entries(FACE_POS)) {
      drawFace(ctx, pos.x, pos.y, fk, fIdx[fk], S.state);
    }

    /* HUD */
    ctx.fillStyle = "#c8dce0";
    ctx.font = "bold 13px monospace";
    ctx.textAlign = "left";
    const solved = PC.verifySolved(S.state).solved;
    ctx.fillText(solved ? "SOLVED" : "scrambled", 4, 14);
    ctx.fillText("moves: " + S.history.length, 4, 30);

    if (S.plan) {
      ctx.fillText("plan: step " + (S.stepIdx + 1) + "/" + S.plan.steps.length, 4, 46);
    }
  }

  function setChip() {
    const chip = byId("pyr-engine-chip");
    if (!chip) return;
    const st = engine.status;
    chip.textContent = "engine: " + st;
    chip.classList.toggle("on", st === "ready");
  }

  function applyTok(tok) {
    S.state = PC.applyMove(S.state, tok);
    S.history.push(tok);
    flashFace(tok[0]);
  }

  let flashTimer = null;
  function flashFace(face) {
    /* simple flash: re-render with a brief highlight on the face */
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
      S.state = PC.applyMove(S.state, inv);
      S.history.pop();
    }
    coach("stepped back: " + st.title);
    render();
  }

  function startAuto() {
    if (!S.plan) solve();
    if (!S.plan) return;
    S.auto = true;
    byId("pyr-play-btn").textContent = "⏸ pause";
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
    const b = byId("pyr-play-btn");
    if (b) b.textContent = "▶ solve";
  }

  function solve() {
    if (S.auto) stopAuto();
    const solved = PC.verifySolved(S.state).solved;
    if (solved) { coach("already solved — scramble it first."); return; }
    /* basic solver: greedy from current state using MOVEs */
    /* TODO: implement layer-by-layer solver */
    coach("solver not yet implemented — try scrambling and solving by hand.");
    render();
  }

  function recordBoard() {
    S.board.record({
      ts: Date.now(), outcome: "solved",
      moves: S.history.length,
      seed: S.cfg.seed,
      stamp: PC.pyraminxStamp(S.cfg),
    });
  }

  function coach(text) { byId("pyr-coach").textContent = text; }

  function highlightStep(i) {
    const list = byId("pyr-steplist");
    if (!list) return;
    Array.from(list.children).forEach((el, idx) => {
      el.classList.toggle("now", idx === i);
    });
  }

  function moveBtn(tok) {
    return h("button", { class: "mz-chip", type: "button" }, tok);
  }

  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) S.cfg.seed = String(parseInt(deepSeed, 10) || DK.randomSeed());

    /* top bar */
    const bar = h("div", { class: "game-bar" },
      h("button", { class: "mz-chip", id: "pyr-scramble-btn", type: "button" }, "⟲ scramble"),
      h("button", { class: "mz-chip mz-play", id: "pyr-solve-btn", type: "button" }, "solve"),
      h("button", { class: "mz-chip mz-play", id: "pyr-play-btn", type: "button" }, "▶ solve"),
      h("button", { class: "mz-chip", id: "pyr-back-btn", type: "button" }, "⏪"),
      h("button", { class: "mz-chip", id: "pyr-fwd-btn", type: "button" }, "⏩"),
      h("button", { class: "mz-chip", id: "pyr-undo-btn", type: "button" }, "↶ undo"),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "pyr-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "pyr-seed", class: "mz-input", size: "8",
          placeholder: "random", min: "0" })),
      h("button", { class: "mz-chip", id: "pyr-dice", type: "button" }, "🎲"),
      h("span", { class: "mz-chip game-rung", id: "pyr-solved-chip" }, "ready"),
      h("button", { class: "mz-chip", id: "pyr-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "pyr-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "pyr-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "pyr-fs-btn", type: "button" }, "⛶ full screen"),
    );

    /* canvas */
    const CW = 9 * TILE + 8, CH = 6 * H + 4;
    const canvas = h("canvas", {
      id: "pyr-canvas", width: String(CW), height: String(CH),
      role: "img", "aria-label": "Pyraminx puzzle",
    });
    const under = h("div", { class: "mz-under game-under" },
      h("span", { class: "mz-kv" }, h("b", { id: "pyr-progress" }, "ready")));

    /* move buttons */
    const moveArea = h("div", { class: "cube-moves" },
      ...["U", "U'", "L", "L'", "R", "R'", "B", "B'"].flatMap(t => [
        h("button", { class: "mz-chip", type: "button", "data-move": t }, t),
      ]));

    /* step list */
    const stepPane = h("div", null,
      h("div", { class: "cube-steplist", id: "pyr-steplist" }),
      h("p", { class: "game-note" }, "Steps appear once you press solve."));

    const main = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, canvas, under, coachEl()),
      h("div", { class: "game-side" }, moveArea, stepPane));

    root.appendChild(h("div", { class: "pyr-wrap" }, bar, main));

    wire();
    S.seedKit = DK.shell.numericSeed(byId("pyr-seed"), byId("pyr-dice"), newGame);
    DK.shell.fullscreen(root.querySelector(".pyr-wrap"), byId("pyr-fs"));
    engine.setEnabled(true);
    newGame();
    render();
  }

  function coachEl() {
    return h("p", { class: "game-hintline", id: "pyr-coach", "aria-live": "polite" }, "");
  }

  function newGame() {
    S.cfg = PC.normalizeConfig({ seed: S.seedKit ? S.seedKit.get() : "pyraminx-lbl" });
    const g = PC.createGame(S.cfg);
    S.state = g.state;
    S.history = [];
    S.scramble = g.scramble;
    S.plan = null; S.stepIdx = 0;
    if (S.auto) stopAuto();
    render();
    coach("scrambled — turn faces or ask for a plan.");
  }

  function scramble() {
    if (S.auto) stopAuto();
    S.cfg = PC.normalizeConfig({ seed: byId("pyr-seed") ? byId("pyr-seed").value : DK.randomSeed() });
    const g = PC.createGame(S.cfg);
    S.state = g.state;
    S.history = [];
    S.scramble = g.scramble;
    S.plan = null; S.stepIdx = 0;
    render();
    coach("scrambled with seed " + S.cfg.seed + " — " + g.scramble.join(" "));
  }

  function wire() {
    byId("pyr-scramble-btn").addEventListener("click", scramble);
    byId("pyr-solve-btn").addEventListener("click", solve);
    byId("pyr-play-btn").addEventListener("click", startAuto);
    byId("pyr-back-btn").addEventListener("click", stepBack);
    byId("pyr-fwd-btn").addEventListener("click", stepFwd);
    byId("pyr-undo-btn").addEventListener("click", () => {
      if (S.auto) stopAuto();
      const tok = S.history.pop();
      if (!tok) return;
      const inv = tok.endsWith("2") ? tok : tok.endsWith("'") ? tok[0] : tok[0] + "'";
      S.state = PC.applyMove(S.state, inv);
      S.plan = null; S.stepIdx = 0;
      render();
    });
    byId("pyr-engine-toggle").addEventListener("click", () => {
      engine.setEnabled(!engine._enabled);
      byId("pyr-engine-toggle").textContent = "engine: " + (engine._enabled ? "on" : "off");
    });
    byId("pyr-board-btn").addEventListener("click", () => {
      renderBoard(); byId("pyr-board-overlay") && byId("pyr-board-overlay").showModal();
    });
    byId("pyr-fs-btn") && byId("pyr-fs-btn").addEventListener("click", () => {
      const el = root.querySelector(".pyr-wrap");
      if (el && el.requestFullscreen) el.requestFullscreen();
    });

    /* move buttons */
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

    document.addEventListener("keydown", e => {
      const map = { u: "U", l: "L", r: "R", b: "B", U: "U'", L: "L'", R: "R'", B: "B'" };
      const tok = map[e.key];
      if (!tok) return;
      if (S.auto) stopAuto();
      if (S.plan) { S.plan = null; S.stepIdx = 0; }
      applyTok(tok);
      render();
    });
  }

  function renderBoard() {
    const body = byId("pyr-board-body");
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
