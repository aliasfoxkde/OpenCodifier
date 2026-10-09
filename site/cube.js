/* Rubik's Cube shell — a real 3D cube you can orbit, scramble, and turn,
   with the LBL coach from cube-core.js stepping through its own solve.
   cube-core.js owns the 54-facelet model and the deterministic solver;
   this file is chrome: the CSS-3D cube (26 cubies, 54 live stickers),
   drag-orbit, move buttons + undo, the step-through coach with rung
   chips, the session board, and the engine bridge. Genuine ties (which
   of several equal-length setups to use) go to the engine; everything
   else — the method itself, the cross, the inserts — is a rule. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const C = globalThis.CubeCore;
  const root = document.getElementById("cube-root");
  if (!root || !DK || !C) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;

  /* ---------- 3D cube geometry (CSS coordinates: x right, y DOWN, z out) */
  const CUBIE = 44;          /* cubie face size in px */
  const GAP = 3;             /* grid gap */
  const PITCH = CUBIE + GAP; /* grid step */
  const HALF = CUBIE / 2;
  /* plane rotation per face: maps the div's +z onto the face's outward
     normal in css space (world x -> css x, world y -> css -y, world z -> css z) */
  const FACE_ROT = {
    F: "", B: "rotateY(180deg) ",
    R: "rotateY(90deg) ", L: "rotateY(-90deg) ",
    U: "rotateX(90deg) ", D: "rotateX(-90deg) ",
  };
  const STICKER = {
    U: "#ffd23e", R: "#e94b3c", F: "#4da44e",
    D: "#f5f0e6", L: "#f97f1b", B: "#3d7de0",
  };
  const DARK = "#101418";
  /* the 26 cubie positions (center skipped) */
  const CUBIES = [];
  for (const x of [-1, 0, 1]) for (const y of [-1, 0, 1]) for (const z of [-1, 0, 1]) {
    if (!x && !y && !z) continue;
    CUBIES.push([x, y, z]);
  }
  /* face divs that carry a real sticker: (cubie, face, facelet index) */
  const STICKER_DIVS = []; /* { el, idx } */
  const FACE_RING = {};    /* face -> [divs] for the turn flash */
  for (const f of C.FACES) FACE_RING[f] = [];

  /* ---------- state ---------- */
  const S = {
    cfg: { ...C.DEFAULT_CONFIG },
    state: null,
    scramble: [],
    history: [],              /* every token applied since the scramble */
    plan: null,               /* solveLBL result for the step-through */
    stepIdx: 0,
    auto: false, autoTimer: 0, speed: 1,
    solvedAt: 0,
    orbit: { rx: -24, ry: -32 },
    dragging: false, dragMoved: false, px: 0, py: 0,
    board: DK.sessionBoard(),
    meters: DK.meters(),
  };
  const engine = DK.engineBridge({ onStatus: () => setChip() });

  function engineChoose(request) {
    const t0 = nowMs();
    const res = engine.choose(request);
    S.meters.sample("engine.ms", nowMs() - t0);
    if (res && res.outcome === "accept" && res.answers
      && res.answers[0] && typeof res.answers[0].choice === "number") {
      return res.answers[0].choice;
    }
    return null; /* refusal and outage both fall back to roster order */
  }

  /* ---------- cube build ---------- */
  /* ---------- shape mods: mirror & ghost ----------
     Same 3x3 algebra, same solver — only each cubie's EXTENT differs. Sizes
     are fixed per position signature (a real mirror/ghost cube is one fixed
     physical shape), so rendering stays deterministic and the solve plan
     applies verbatim. scale3d keeps every piece centered on its slot. */
  const SIG_KEY = (x, y, z) => (x ? "1" : "0") + (y ? "1" : "0") + (z ? "1" : "0")
    + (x > 0 ? "+" : x < 0 ? "-" : "") + (y > 0 ? "+" : y < 0 ? "-" : "") + (z > 0 ? "+" : z < 0 ? "-" : "");
  const MIRROR_SIZES = {
    /* signature -> [sx, sy, sz]: centers thin, edges split thin/wide,
       corners carry the visible stagger a mirror cube is loved for */
    "0++": [0.62, 1.02, 1.02], "0--": [0.72, 0.92, 0.92],
    "0+-": [0.66, 1.04, 0.90], "0-+": [0.70, 0.90, 1.04],
    "+0+": [1.02, 0.62, 1.02], "-0-": [0.92, 0.72, 0.92],
    "+0-": [1.04, 0.66, 0.90], "-0+": [0.90, 0.70, 1.04],
    "++0": [1.02, 1.02, 0.62], "--0": [0.92, 0.92, 0.72],
    "+-0": [1.04, 0.90, 0.66], "-+0": [0.90, 1.04, 0.70],
    "+++": [1.18, 0.98, 1.08], "++-": [1.10, 1.20, 0.88],
    "+-+": [0.90, 0.84, 1.22], "+--": [1.14, 0.90, 0.94],
    "-++": [0.86, 1.14, 1.00], "-+-": [1.06, 0.88, 1.16],
    "--+": [0.94, 1.10, 0.86], "---": [1.22, 1.06, 0.92],
  };
  const GHOST_SIZES = {
    /* deliberately irregular — the ghost cube's charm is that nothing
       matches; one axis per piece is deliberately oversized */
    "0++": [0.58, 1.10, 0.94], "0--": [0.80, 0.86, 1.12],
    "0+-": [0.70, 0.96, 1.06], "0-+": [0.62, 1.14, 0.88],
    "+0+": [1.08, 0.60, 0.98], "-0-": [0.88, 0.78, 1.06],
    "+0-": [0.96, 0.70, 1.12], "-0+": [1.10, 0.84, 0.86],
    "++0": [1.00, 1.08, 0.62], "--0": [0.84, 0.94, 0.74],
    "+-0": [1.12, 0.88, 0.68], "-+0": [0.78, 1.12, 0.82],
    "+++": [1.24, 0.92, 1.04], "++-": [1.04, 1.22, 0.84],
    "+-+": [0.88, 0.80, 1.26], "+--": [1.18, 0.94, 0.90],
    "-++": [0.82, 1.18, 0.96], "-+-": [1.08, 0.84, 1.20],
    "--+": [0.90, 1.06, 0.82], "---": [1.26, 1.10, 0.88],
  };
  const cubieScale = (x, y, z) => {
    if (S.cfg.cubeType === "mirror") return MIRROR_SIZES[SIG_KEY(x, y, z)] || [1, 1, 1];
    if (S.cfg.cubeType === "ghost") return GHOST_SIZES[SIG_KEY(x, y, z)] || [1, 1, 1];
    return [1, 1, 1];
  };

  function buildCube() {
    const cube = h("div", { class: "cube3d" + (S.cfg.cubeType !== "classic" ? " cube3d-mod" : ""), id: "cube3d" });
    for (const [x, y, z] of CUBIES) {
      const [sx, sy, sz] = cubieScale(x, y, z);
      const cubie = h("div", {
        class: "cube3d-cubie",
        style: "transform: translate3d(" + (x * PITCH) + "px,"
          + (-y * PITCH) + "px," + (z * PITCH) + "px)"
          + (sx !== 1 || sy !== 1 || sz !== 1 ? " scale3d(" + sx + "," + sy + "," + sz + ")" : ""),
      });
      for (const f of C.FACES) {
        const idx = C.faceletAt(f, x, y, z);
        const isOuter = (f === "U" && y === 1) || (f === "D" && y === -1)
          || (f === "R" && x === 1) || (f === "L" && x === -1)
          || (f === "F" && z === 1) || (f === "B" && z === -1);
        /* every face of the cubie boxes it: each sits HALF out along its
           normal (inward-facing ones end up plane-coincident with the
           neighbor's — identically dark, so no visible z-fight) */
        const div = h("div", {
          class: "cube3d-face" + (isOuter ? "" : " inner"),
          style: "transform: " + FACE_ROT[f] + "translateZ(" + HALF + "px)",
          "data-idx": String(idx),
        });
        if (isOuter) { STICKER_DIVS.push({ el: div, idx }); FACE_RING[f].push(div); }
        cubie.appendChild(div);
      }
      cube.appendChild(cubie);
    }
    return cube;
  }

  function rebuildGeometry() {
    /* shape mods re-space the pieces without touching state or history */
    const cube = byId("cube3d");
    if (!cube) return;
    cube.classList.toggle("cube3d-mod", S.cfg.cubeType !== "classic");
    const kids = cube.children;
    for (let i = 0; i < CUBIES.length; i++) {
      const [x, y, z] = CUBIES[i];
      const [sx, sy, sz] = cubieScale(x, y, z);
      const t = "translate3d(" + (x * PITCH) + "px," + (-y * PITCH) + "px," + (z * PITCH) + "px)"
        + (sx !== 1 || sy !== 1 || sz !== 1 ? " scale3d(" + sx + "," + sy + "," + sz + ")" : "");
      kids[i].style.transform = t;
    }
  }

  function render() {
    for (const { el, idx } of STICKER_DIVS) {
      el.style.background = STICKER[S.state[idx]] || DARK;
    }
    const solved = C.verifySolved(S.state).solved;
    const chip = byId("cube-solved-chip");
    if (chip) {
      chip.textContent = solved ? "solved" : "scrambled";
      chip.classList.toggle("on", solved);
    }
    byId("cube-progress").textContent = S.plan
      ? "step " + Math.min(S.stepIdx + 1, S.plan.steps.length) + "/" + S.plan.steps.length
      : (S.history.length ? S.history.length + " moves from the scramble" : "ready");
    byId("cube-solve-btn").disabled = solved;
    renderSteps();
  }

  /* ---------- moves ---------- */
  function applyTok(tok, record) {
    S.state = C.applyMove(S.state, tok);
    if (record !== false) S.history.push(tok);
    flash(tok[0]);
    renderMoveLog();
  }

  /* newest on top, newest highlighted; the rail scrolls (themed scrollbar)
     so long solves never blow the layout up */
  function renderMoveLog() {
    const log = byId("cube-movelog");
    if (!log) return;
    log.textContent = "";
    const hist = S.history;
    if (!hist.length) {
      log.appendChild(h("p", { class: "game-note" }, "no moves yet"));
      return;
    }
    const windowN = 120; /* bound the DOM; the count line keeps the truth */
    const start = Math.max(0, hist.length - windowN);
    for (let i = hist.length - 1; i >= start; i--) {
      const chip = h("span", { class: "mz-chip cube-move" + (i === hist.length - 1 ? " latest" : "") },
        hist[i]);
      log.appendChild(chip);
    }
    if (start > 0) {
      log.appendChild(h("p", { class: "game-note" },
        "… " + start + " older moves (" + hist.length + " total)"));
    }
  }
  function userMove(tok) {
    if (S.auto) stopAuto();
    applyTok(tok);
    if (S.plan && S.stepIdx < S.plan.steps.length) {
      /* a hand move mid-plan invalidates the rest of the plan honestly */
      S.plan = null; S.stepIdx = 0;
      coach("your move changed the cube — the plan below no longer fits. Solve again for a fresh one.");
    }
    render();
  }
  function undo() {
    if (S.auto) stopAuto();
    const tok = S.history.pop();
    if (!tok) return;
    const inv = tok.endsWith("2") ? tok : tok.endsWith("'") ? tok[0] : tok[0] + "'";
    S.state = C.applyMove(S.state, inv);
    flash(inv[0]);
    S.plan = null; S.stepIdx = 0;
    render();
  }
  let flashTimer = 0;
  function flash(face) {
    for (const el of FACE_RING[face]) el.classList.add("flash");
    clearTimeout(flashTimer);
    flashTimer = setTimeout(() => {
      for (const f of C.FACES) for (const el of FACE_RING[f]) el.classList.remove("flash");
    }, 190);
  }
  function coach(text) { byId("cube-coach").textContent = text; }

  /* ---------- solve + step-through ---------- */
  function solve() {
    if (S.auto) stopAuto();
    const solved = C.verifySolved(S.state).solved;
    if (solved) { coach("already solved — scramble it first."); return; }
    const t0 = nowMs();
    S.plan = C.solveLBL(S.state, engine.status === "ready" ? engineChoose : undefined);
    S.meters.sample("solve.ms", nowMs() - t0);
    S.stepIdx = 0;
    const ties = S.plan.ties.filter((t) => t.source === "engine").length;
    coach("plan ready: " + S.plan.moves.length + " moves in " + S.plan.steps.length
      + " steps (" + ties + " engine ties). Step through it, or let it play.");
    buildStepList();
    renderMoveLog();
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
      S.planSteps = S.plan.steps.length;
      S.plan = null; S.stepIdx = 0;
      recordBoard();
      coach("solved. " + S.history.length + " moves from the scramble. Scramble for another round.");
    }
    render();
  }
  function stepBack() {
    if (S.auto) stopAuto();
    if (!S.plan || S.stepIdx === 0) return;
    S.stepIdx--;
    const st = S.plan.steps[S.stepIdx];
    for (let i = st.moves.length - 1; i >= 0; i--) {
      const t = st.moves[i];
      const inv = t.endsWith("2") ? t : t.endsWith("'") ? t[0] : t[0] + "'";
      S.state = C.applyMove(S.state, inv);
      flash(inv[0]);
    }
    coach("stepped back: " + st.title);
    render();
  }
  function startAuto() {
    if (!S.plan) solve();
    if (!S.plan) return;
    S.auto = true;
    byId("cube-play-btn").textContent = "⏸ pause";
    tickAuto();
  }
  function tickAuto() {
    if (!S.auto) return;
    if (!S.plan) { stopAuto(); return; }
    stepFwd();
    if (S.auto) S.autoTimer = setTimeout(tickAuto, 650 / S.speed);
  }
  function stopAuto() {
    S.auto = false;
    clearTimeout(S.autoTimer);
    const b = byId("cube-play-btn");
    if (b) b.textContent = "▶ auto-solve";
  }

  function recordBoard() {
    S.board.record({
      ts: Date.now(), outcome: "solved",
      moves: S.history.length,
      steps: S.planSteps || 0,
      seed: S.cfg.seed,
      stamp: C.cubeStamp(S.cfg),
    });
  }

  /* ---------- step list ---------- */
  function buildStepList() {
    const list = byId("cube-steplist");
    list.textContent = "";
    if (!S.plan) return;
    const rungFor = (step) => {
      if (!step.tie) return ["rule", "rule"];
      const src = step.tie.chosenIndex > 0 ? "engine" : (step.tie.options.length > 1 ? "roster" : "rule");
      return [src, step.tie.options.length > 1 ? "tie" : "unique"];
    };
    S.plan.steps.forEach((st, i) => {
      const [rung, kind] = rungFor(st);
      const row = h("div", { class: "cube-step", "data-step": String(i) },
        h("span", { class: "mz-chip game-rung" + (rung === "engine" ? " on" : "") }, rung),
        h("span", { class: "cube-step-title" }, st.title),
        h("span", { class: "cube-step-moves" }, st.moves.join(" ") || "—"));
      row.addEventListener("click", () => jumpTo(i));
      list.appendChild(row);
    });
  }
  function highlightStep(i) {
    for (const el of byId("cube-steplist").children) {
      el.classList.toggle("now", el.getAttribute("data-step") === String(i));
    }
  }
  function jumpTo(i) {
    if (!S.plan || S.auto) return;
    while (S.stepIdx > i) stepBack();
    while (S.stepIdx < i) stepFwd();
  }
  function renderSteps() {
    if (!S.plan) {
      const list = byId("cube-steplist");
      if (list.children.length) list.textContent = "";
      return;
    }
    highlightStep(S.stepIdx);
  }

  /* ---------- game lifecycle ---------- */
  function newGame() {
    stopAuto();
    const raw = byId("cube-seed").value.trim();
    /* optional numeric seed: empty means a fresh random number every game */
    const seed = raw === "" ? DK.randomSeed() : String(parseInt(raw, 10) || DK.randomSeed());
    byId("cube-seed").value = seed;
    S.cfg = C.normalizeConfig({
      seed, scrambleLen: +byId("cube-len").value,
      cubeType: byId("cube-type") ? byId("cube-type").value : "classic",
    });
    const g = C.createGame(S.cfg);
    S.state = g.state;
    S.scramble = g.scramble.slice();
    S.history = [];
    S.plan = null; S.stepIdx = 0; S.solvedAt = 0;
    coach("scrambled with " + S.scramble.length + " turns (seed " + seed
      + "). Turn it by hand, or ask for the layer-by-layer plan.");
    buildStepList();
    renderMoveLog();
    render();
  }
  function resetParams() {
    byId("cube-len").value = String(C.DEFAULT_CONFIG.scrambleLen);
    byId("cube-seed").value = "";
    byId("cube-type").value = "classic";
    newGame();
  }

  /* ---------- orbit ---------- */
  function wireOrbit(scene) {
    scene.addEventListener("pointerdown", (e) => {
      S.dragging = true; S.dragMoved = false; S.px = e.clientX; S.py = e.clientY;
      scene.setPointerCapture(e.pointerId);
    });
    scene.addEventListener("pointermove", (e) => {
      if (!S.dragging) return;
      const dx = e.clientX - S.px, dy = e.clientY - S.py;
      if (Math.abs(dx) + Math.abs(dy) > 3) S.dragMoved = true;
      S.orbit.ry += dx * 0.45;
      S.orbit.rx = Math.max(-80, Math.min(80, S.orbit.rx - dy * 0.45));
      S.px = e.clientX; S.py = e.clientY;
      applyOrbit();
    });
    const up = () => { S.dragging = false; };
    scene.addEventListener("pointerup", up);
    scene.addEventListener("pointercancel", up);
  }
  function applyOrbit() {
    byId("cube3d").style.transform =
      "rotateX(" + S.orbit.rx + "deg) rotateY(" + S.orbit.ry + "deg)";
  }

  /* ---------- build ---------- */
  function moveBtn(tok) {
    return h("button", { class: "mz-chip cube-move", type: "button" }, tok);
  }

  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) S.cfg.seed = String(parseInt(deepSeed, 10) || DK.randomSeed());

    const bar = h("div", { class: "game-bar" },
      h("button", { class: "mz-chip", id: "cube-scramble-btn", type: "button" }, "⟲ scramble"),
      h("button", { class: "mz-chip", id: "cube-solve-btn", type: "button" }, "solve"),
      h("button", { class: "mz-chip mz-play", id: "cube-play-btn", type: "button" }, "▶ auto-solve"),
      h("button", { class: "mz-chip", id: "cube-back-btn", type: "button" }, "⏪"),
      h("button", { class: "mz-chip", id: "cube-fwd-btn", type: "button" }, "⏩"),
      h("button", { class: "mz-chip", id: "cube-undo-btn", type: "button" }, "↶ undo"),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "cube-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "cube-seed", class: "mz-input", size: "8",
          placeholder: "random", min: "0", "aria-label": "numeric seed, empty for a random one each game" }),
        h("button", { class: "mz-chip", id: "cube-dice", type: "button",
          title: "roll a new random seed" }, "🎲")),
      h("span", { class: "mz-chip game-rung", id: "cube-solved-chip" }, "scrambled"),
      h("button", { class: "mz-chip", id: "cube-reset-btn", type: "button",
        title: "restore default parameters" }, "reset params"),
      h("button", { class: "mz-chip", id: "cube-fs-btn", type: "button" }, "⛶ full screen"),
      h("button", { class: "mz-chip", id: "cube-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "cube-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "cube-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "cube-stats-btn", type: "button" }, "ⓘ stats"),
    );

    const scene = h("div", { class: "cube3d-scene", id: "cube-scene" },
      h("div", { class: "cube3d-orbit" }, buildCube()));
    const under = h("div", { class: "mz-under game-under" },
      h("span", { class: "mz-kv" }, h("b", { id: "cube-progress" }, "ready")));
    const coachLine = h("p", { class: "game-hintline", id: "cube-coach", "aria-live": "polite" }, "");

    const stepPane = h("div", null,
      h("div", { class: "cube-steplist", id: "cube-steplist" }),
      h("p", { class: "game-note" },
        "Steps appear once you press solve. Click a step to jump to it; the ",
        "rung chip says who picked the setup when several were equal — rule ",
        "(no tie), roster (tie, nobody asked), engine (tie, the runtime chose)."));

    const opts = DK.shell.accordion([
      {
        id: "play", label: "Turn the cube", open: true,
        kids: [
          h("div", { class: "cube-moves" },
            ...["U", "R", "F", "D", "L", "B"].flatMap((f) => [
              moveBtn(f), moveBtn(f + "'"), moveBtn(f + "2"),
            ])),
          h("p", { class: "game-note" },
            "Each button turns one face a quarter turn clockwise (viewed from ",
            "that face), counter-clockwise (prime), or a half turn (2). Drag ",
            "the cube to orbit. A hand move mid-plan drops the rest of the ",
            "plan — solve again for a fresh one."),
        ],
      },
      {
        id: "setup", label: "Advanced — shape, scramble & solve",
        kids: [
          h("label", { class: "game-field" },
            h("span", { class: "game-slider-label" }, "cube type"),
            h("select", { id: "cube-type", "aria-label": "cube shape mod" },
              h("option", { value: "classic" }, "classic — the standard cube"),
              h("option", { value: "mirror" }, "mirror — staggered piece depths, same algebra"),
              h("option", { value: "ghost" }, "ghost — irregular piece depths, same algebra")),
            h("span", { class: "game-note" },
              "Mirror and ghost are shape mods: the move algebra, scramble, ",
              "and the whole layer-by-layer solver are IDENTICAL — only the ",
              "piece geometry differs, exactly like the physical puzzles. ",
              "Pyraminx and megaminx are different puzzle families with ",
              "different move laws — not shipped; pretending otherwise would ",
              "be a lie in a solver that verifies every move.")),
          h("label", { class: "game-slider" },
            h("span", { class: "game-slider-label" }, "scramble length"),
            h("input", { type: "range", id: "cube-len", min: "1", max: "40",
              step: "1", value: String(S.cfg.scrambleLen), "aria-label": "scramble length" }),
            h("span", { class: "game-sliderval", id: "cube-len-val" }, String(S.cfg.scrambleLen))),
          h("p", { class: "game-note" },
            "The seed is optional: leave it empty and every game rolls a fresh ",
            "random number (shown in the bar so the run stays reproducible — ",
            "type it back in to replay the exact scramble)."),
        ],
      },
      {
        id: "about", label: "What decides what?",
        kids: [h("p", { class: "game-note" },
          "The solver is layer-by-layer: cross, first-layer corners, middle ",
          "edges, then edge orientation, corner orientation, corner ",
          "permutation, edge permutation, and a final alignment. Almost all of ",
          "it is RULE — the method is fixed and every insert is verified by ",
          "simulation before it is played. The genuine ties are the set-ups: ",
          "when several alignments reach the next phase in the same number of ",
          "moves, the engine picks one (and a refusal falls back to roster ",
          "order without derailing the solve). Every move the plan plays is ",
          "checked against the facelet model — the coach can't lie you into a ",
          "wrong cube.")],
      },
    ]);

    const moveLog = h("div", null,
      h("div", { class: "game-slider-label" }, "move history — newest first"),
      h("div", { class: "cube-movelog", id: "cube-movelog",
        "aria-live": "polite", "aria-label": "move history" },
        h("p", { class: "game-note" }, "no moves yet")));

    const main = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, scene, under, coachLine),
      h("div", { class: "game-side" }, opts, moveLog, stepPane));
    root.appendChild(h("div", { class: "game-wrap", id: "cube-wrap" }, bar, main));

    buildOverlays();
    wire();
    engine.setEnabled(true);
    newGame();                 /* boots paused — solve/auto-solve is the play button */
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
    root.appendChild(overlay("cube-board-overlay", "Session scoreboard", "cube-board-body", "cube-board-close"));
    root.appendChild(overlay("cube-stats-overlay", "Measured stats", "cube-stats-body", "cube-stats-close"));
  }
  function renderBoard() {
    const body = byId("cube-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" },
        "No solves yet — finish a plan (step through or auto-solve) and the ",
        "run lands here. Session only."));
      return;
    }
    body.appendChild(h("table", { class: "game-table" },
      h("thead", null, h("tr", null,
        ...["#", "moves", "seed", "when"].map((x) => h("th", null, x)))),
      h("tbody", null, rows.map((r, i) => h("tr", null,
        h("td", null, String(rows.length - i)),
        h("td", null, String(r.moves)),
        h("td", null, r.seed),
        h("td", null, new Date(r.ts).toLocaleTimeString()))))));
    body.appendChild(h("button", {
      class: "mz-chip", type: "button", onclick: () => { S.board.clear(); renderBoard(); },
    }, "clear board"));
  }
  function renderStats() {
    const body = byId("cube-stats-body");
    if (!body) return;
    body.textContent = "";
    const m = S.meters.snapshot();
    const kv = (label, val) => h("p", { class: "game-note" }, label + " " + val);
    body.appendChild(kv("engine calls:", String(m["engine.calls"] || 0)));
    body.appendChild(kv("engine latency avg:", (m["engine.ms"] ? m["engine.ms"].avg.toFixed(1) : "0") + " ms"));
    body.appendChild(kv("solve compute avg:", (m["solve.ms"] ? m["solve.ms"].avg.toFixed(1) : "0") + " ms"));
    body.appendChild(kv("history depth:", String(S.history.length)));
  }

  /* ---------- wire ---------- */
  function wire() {
    byId("cube-scramble-btn").addEventListener("click", () => { newGame(); });
    byId("cube-solve-btn").addEventListener("click", solve);
    byId("cube-play-btn").addEventListener("click", () => (S.auto ? stopAuto() : startAuto()));
    byId("cube-fwd-btn").addEventListener("click", () => { if (S.auto) stopAuto(); stepFwd(); });
    byId("cube-back-btn").addEventListener("click", stepBack);
    byId("cube-undo-btn").addEventListener("click", undo);
    byId("cube-dice").addEventListener("click", () => {
      byId("cube-seed").value = DK.randomSeed();
      newGame();
    });
    byId("cube-seed").addEventListener("change", newGame);
    byId("cube-len").addEventListener("input", () => {
      byId("cube-len-val").textContent = byId("cube-len").value;
    });
    byId("cube-len").addEventListener("change", newGame);
    byId("cube-type").addEventListener("change", () => {
      /* a shape mod is physical geometry: apply to the CURRENT cube without
         rescrambling — state, history, and any plan all stay valid */
      S.cfg.cubeType = byId("cube-type").value;
      rebuildGeometry();
      coach(byId("cube-type").value + " shape applied — same algebra, same plan.");
    });
    byId("cube-reset-btn").addEventListener("click", resetParams);
    byId("cube-engine-toggle").addEventListener("click", () => {
      engine.setEnabled(!engine.enabled);
      byId("cube-engine-toggle").textContent = "engine: " + (engine.enabled ? "on" : "off");
      setChip();
    });
    byId("cube-board-btn").addEventListener("click", () => { renderBoard(); byId("cube-board-overlay").showModal(); });
    byId("cube-board-close").addEventListener("click", () => byId("cube-board-overlay").close());
    byId("cube-stats-btn").addEventListener("click", () => { renderStats(); byId("cube-stats-overlay").showModal(); });
    byId("cube-stats-close").addEventListener("click", () => byId("cube-stats-overlay").close());
    for (const btn of root.querySelectorAll(".cube-move")) {
      btn.addEventListener("click", () => userMove(btn.textContent));
    }
    wireOrbit(byId("cube-scene"));
    DK.shell.fullscreen(byId("cube-wrap"), byId("cube-fs-btn"));
    setChip();
    applyOrbit();
  }

  function setChip() {
    const chip = byId("cube-engine-chip");
    if (!chip) return;
    const st = engine.status;
    chip.textContent = "engine: " + st;
    chip.classList.toggle("on", st === "ready");
    chip.classList.toggle("off", st === "unavailable" || st === "off");
  }

  build();
})();
