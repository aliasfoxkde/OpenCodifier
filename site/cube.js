/* Twisty-cube shell — the N×N family (2×2 … 21×21) plus the 3×3 shape mods,
   with the coach that shows its decisions. Two verified cores sit underneath:
   cube-core.js owns the battle-tested 3×3 (54-facelet LBL solver), and
   cube-n-core.js owns everything else — 2×2 with its own BFS-permuted coach,
   and 4×4…21×21 as validated free play (the piece laws are checked on every
   move; a solver claim for big cubes would be a lie, so none is made).
   Rendering follows the same split: a CSS-3D cube you can orbit at 2×2–3×3,
   and a fast 2D net on canvas from 4×4 up (a 21×21 has 2,646 stickers —
   DOM-per-sticker dies there). Wide turns (Uw, 3Uw…) get a width stepper at
   sizes where they exist. Engine bridge, session board, and meters are
   unchanged: genuine ties go to the Decisions SDK engine, everything else
   is a rule. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const C = globalThis.CubeCore;
  const NC = globalThis.CubeNCore;
  const root = document.getElementById("cube-root");
  if (!root || !DK || !C || !NC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;

  /* ---------- shared geometry & palette ---------- */
  const CUBIE = 44;          /* cubie face size in px (3D view) */
  const GAP = 3;
  const PITCH = CUBIE + GAP;
  const HALF = CUBIE / 2;
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
  const FACES = C.FACES;
  const NORMAL = C.NORMAL;

  /* ---------- state ---------- */
  const S = {
    n: 3,                     /* cube size, 2..21 */
    K: null,                  /* active core facade (makeK) */
    width: 1,                 /* turn width in layers (wide turns) */
    cfg: { ...C.DEFAULT_CONFIG },
    state: null,
    scramble: [],
    history: [],
    plan: null,
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

  /* ---------- core facade: one interface over the two verified cores.
     n === 3 goes to cube-core (the battle-tested 54-facelet LBL solver);
     every other size goes to cube-n-core, which coaches 2×2 and validates
     the piece laws everywhere. Nothing here guesses. ---------- */
  /* shell-cubie lattice coords: the cores that publish CUBIE_COORDS are
     authoritative; cube-core (n=3) gets the classic 26 derived here */
  function shellCoords(core, n) {
    if (core.CUBIE_COORDS) return core.CUBIE_COORDS;
    const m = n - 1;
    const out = [];
    for (let x = -m; x <= m; x += 2) {
      for (let y = -m; y <= m; y += 2) {
        for (let z = -m; z <= m; z += 2) {
          if (Math.max(Math.abs(x), Math.abs(y), Math.abs(z)) === m) out.push([x, y, z]);
        }
      }
    }
    return out;
  }
  function makeK(n) {
    if (n === 3) {
      return {
        n, core: C, maxDepth: 1, mods: true,
        coords: shellCoords(C, n),
        solver: "lbl",
        newState(seed, len) {
          return C.createGame(C.normalizeConfig({ seed, scrambleLen: len }));
        },
        solve(state, chooser) { return C.solveLBL(state, chooser); },
        stamp(cfg) { return C.cubeStamp(cfg); },
        /* cube-core reports { ok, checks: [{ name, ok }] }; normalized here */
        laws(state) {
          const v = C.validateState(state);
          const bad = (v.checks || []).find((c) => !c.ok);
          return { legal: !!v.ok, issue: bad ? bad.name : null };
        },
      };
    }
    const core = NC.createCore(n);
    return {
      n, core, maxDepth: core.maxDepth, mods: false,
      coords: shellCoords(core, n),
      solver: n === 2 ? "n2" : null,
      newState(seed, len) {
        return core.createGame({ seed, scrambleLen: len });
      },
      solve(state, chooser) {
        const r = core.solve(state, chooser);
        return r && !r.refused ? { steps: r.steps, ties: [], moves: r.moves } : null;
      },
      stamp(cfg) { return "cubeN|" + n + "|" + cfg.seed + "|" + cfg.scrambleLen; },
      /* cube-n-core reports { legal, issues: [{ check }] } */
      laws(state) {
        const v = core.validateState(state);
        return { legal: !!v.legal, issue: v.issues && v.issues[0] ? v.issues[0].check : null };
      },
    };
  }

  /* ---------- 3D cube (n = 2, 3) ---------- */
  /* shape mods: mirror & ghost — same 3x3 algebra, same solver — only each
     cubie's EXTENT differs. Sizes are fixed per position signature (a real
     mirror/ghost cube is one fixed physical shape), so rendering stays
     deterministic and the solve plan applies verbatim. scale3d keeps every
     piece centered on its slot. */
  const SIG_KEY = (x, y, z) => (x ? "1" : "0") + (y ? "1" : "0") + (z ? "1" : "0")
    + (x > 0 ? "+" : x < 0 ? "-" : "") + (y > 0 ? "+" : y < 0 ? "-" : "") + (z > 0 ? "+" : z < 0 ? "-" : "");
  const MIRROR_SIZES = {
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
  /* sudokube: a fixed, deterministic numbering skin — each face shows 1..9
     exactly once (a solved sudoku box). Pure cosmetics: same algebra, same
     plan; the numbers ride the stickers when the cube turns. */
  const sudokuNum = (f, a, b) => ((a * 3 + b + FACES.indexOf(f)) % 9) + 1;
  /* grid position of a sticker on its face (row a, col b) — shared by the
     sudokube skin and the net renderer. F is the reference frame; each net
     face folds out around it. m = n - 1 (lattice half-width). */
  function faceGrid(f, x, y, z, m) {
    if (f === "F") return [m - y, x + m];
    if (f === "B") return [m - y, m - x];
    if (f === "R") return [m - y, m - z];
    if (f === "L") return [m - y, z + m];
    if (f === "U") return [z + m, x + m];
    return [m - z, x + m]; /* D */
  }

  let STICKER_DIVS = []; /* { el, idx, f, a, b } */
  let FACE_RING = {};

  function buildCube() {
    const m = S.n - 1;
    const cube = h("div", { class: "cube3d" + (S.cfg.cubeType !== "classic" ? " cube3d-mod" : ""), id: "cube3d" });
    STICKER_DIVS = [];
    FACE_RING = {};
    for (const f of FACES) FACE_RING[f] = [];
    for (const [rx, ry, rz] of S.K.coords) {
      /* raw lattice coords (-m..m step 2) -> css units (-1..1) */
      const x = rx / m, y = ry / m, z = rz / m;
      const [sx, sy, sz] = cubieScale(x, y, z);
      const cubie = h("div", {
        class: "cube3d-cubie",
        style: "transform: translate3d(" + (x * PITCH) + "px,"
          + (-y * PITCH) + "px," + (z * PITCH) + "px)"
          + (sx !== 1 || sy !== 1 || sz !== 1 ? " scale3d(" + sx + "," + sy + "," + sz + ")" : ""),
      });
      for (const f of FACES) {
        const nv = NORMAL[f];
        const isOuter = nv[0] * rx + nv[1] * ry + nv[2] * rz === m;
        const idx = isOuter ? S.K.core.faceletAt(f, rx, ry, rz) : -1;
        /* every face of the cubie boxes it: each sits HALF out along its
           normal (inward-facing ones end up plane-coincident with the
           neighbor's — identically dark, so no visible z-fight) */
        const div = h("div", {
          class: "cube3d-face" + (isOuter ? "" : " inner"),
          style: "transform: " + FACE_ROT[f] + "translateZ(" + HALF + "px)",
          "data-idx": String(idx),
        });
        if (isOuter) {
          const [a, b] = faceGrid(f, rx, ry, rz, m);
          STICKER_DIVS.push({ el: div, idx, f, a, b });
          FACE_RING[f].push(div);
        }
        cubie.appendChild(div);
      }
      cube.appendChild(cubie);
    }
    return cube;
  }

  function rebuildGeometry() {
    const cube = byId("cube3d");
    if (!cube) return;
    cube.classList.toggle("cube3d-mod", S.cfg.cubeType !== "classic");
    /* children are the cubies in CUBIE_COORDS order */
    const kids = cube.children;
    const m = S.n - 1;
    const coords = S.K.coords;
    for (let i = 0; i < coords.length; i++) {
      const [rx, ry, rz] = coords[i];
      const x = rx / m, y = ry / m, z = rz / m;
      const [sx, sy, sz] = cubieScale(x, y, z);
      kids[i].style.transform = "translate3d(" + (x * PITCH) + "px,"
        + (-y * PITCH) + "px," + (z * PITCH) + "px)"
        + (sx !== 1 || sy !== 1 || sz !== 1 ? " scale3d(" + sx + "," + sy + "," + sz + ")" : "");
    }
  }

  function applySkin() {
    /* sudokube numbering rides the sticker divs; classic strips it back off */
    const on = S.cfg.cubeType === "sudoku";
    for (const { el, f, a, b } of STICKER_DIVS) {
      if (on) {
        el.textContent = String(sudokuNum(f, a, b));
        el.style.display = "flex";
        el.style.alignItems = "center";
        el.style.justifyContent = "center";
        el.style.fontWeight = "700";
        el.style.fontSize = "19px";
        el.style.color = (f === "U" || f === "D") ? "#101418" : "#ffffff";
      } else if (el.textContent) {
        el.textContent = "";
        el.style.display = "";
        el.style.fontWeight = "";
        el.style.fontSize = "";
        el.style.color = "";
      }
    }
  }

  /* ---------- 2D net (n ≥ 4) ---------- */
  /* A 21×21 has 2,646 live stickers — one DOM node each is a dead browser.
     The net is a canvas: six faces in a cross, redrawn per move. Same
     facelet indices, same truth, cheap paint. */
  let netCtx = null;
  let netGeom = null; /* { cell, gap, off: {face: [px, px]}, size } */

  function netLayout() {
    const n = S.n;
    const cell = Math.max(5, Math.min(30, Math.floor(500 / (4 * n))));
    const gap = 1;
    const off = {
      U: [n * cell + 2 * gap, gap],
      L: [gap, n * cell + 2 * gap],
      F: [n * cell + 2 * gap, n * cell + 2 * gap],
      R: [2 * n * cell + 3 * gap, n * cell + 2 * gap],
      B: [3 * n * cell + 4 * gap, n * cell + 2 * gap],
      D: [n * cell + 2 * gap, 2 * n * cell + 3 * gap],
    };
    return { cell, gap, off, w: 4 * n * cell + 5 * gap, h: 3 * n * cell + 4 * gap };
  }

  function buildNet() {
    netGeom = netLayout();
    const cv = h("canvas", {
      id: "cube-net", width: String(netGeom.w), height: String(netGeom.h),
      "aria-label": S.n + "×" + S.n + " cube, unfolded net view",
      role: "img",
    });
    cv.style.maxWidth = "100%";
    cv.style.height = "auto";
    cv.style.display = "block";
    cv.style.margin = "0 auto";
    cv.style.borderRadius = "10px";
    cv.style.background = "rgba(0,0,0,0.25)";
    netCtx = cv.getContext("2d");
    return cv;
  }

  function drawNet() {
    if (!netCtx || !netGeom) return;
    const { cell, gap, off } = netGeom;
    const m = S.n - 1;
    netCtx.clearRect(0, 0, netGeom.w, netGeom.h);
    /* direct enumeration: for each face, iterate the shell cubies and keep
       the stickers that exist, mapping to (a,b) via faceGrid — one query
       per sticker, no grid inversion needed */
    for (const f of FACES) {
      const nv = NORMAL[f];
      const [ox, oy] = off[f];
      for (const p of S.K.coords) {
        if (nv[0] * p[0] + nv[1] * p[1] + nv[2] * p[2] !== m) continue;
        const [a, b] = faceGrid(f, p[0], p[1], p[2], m);
        const idx = S.K.core.faceletAt(f, p[0], p[1], p[2]);
        netCtx.fillStyle = STICKER[S.state[idx]] || DARK;
        netCtx.fillRect(ox + b * (cell + gap), oy + a * (cell + gap), cell, cell);
      }
    }
  }

  /* face flash on the net: outline the turned face's cells (the same
     approximation the 3D view makes with its face ring — a turn moves
     stickers among many faces, but the turned face is the anchor) */
  let netFlashTimer = 0;
  function flashNet(face) {
    if (!netCtx || !netGeom) return;
    const { cell, gap, off } = netGeom;
    const [ox, oy] = off[face];
    netCtx.strokeStyle = "#ffffff";
    netCtx.lineWidth = 2;
    for (let a = 0; a < S.n; a++) {
      for (let b = 0; b < S.n; b++) {
        netCtx.strokeRect(ox + b * (cell + gap) + 1, oy + a * (cell + gap) + 1, cell - 2, cell - 2);
      }
    }
    clearTimeout(netFlashTimer);
    netFlashTimer = setTimeout(drawNet, 190);
  }

  /* ---------- render ---------- */
  function render() {
    const is3d = S.n <= 3;
    if (is3d) {
      for (const { el, idx } of STICKER_DIVS) {
        el.style.background = STICKER[S.state[idx]] || DARK;
      }
    } else {
      drawNet();
    }
    const solved = S.K.core.verifySolved(S.state).solved;
    const chip = byId("cube-solved-chip");
    if (chip) {
      chip.textContent = solved ? "solved" : "scrambled";
      chip.classList.toggle("on", solved);
    }
    const laws = byId("cube-laws-chip");
    if (laws) {
      const v = S.K.laws(S.state);
      laws.textContent = v.legal ? "piece laws hold" : "laws: " + v.issue;
      laws.classList.toggle("on", v.legal);
      laws.classList.toggle("off", !v.legal);
    }
    byId("cube-progress").textContent = S.plan
      ? "step " + Math.min(S.stepIdx + 1, S.plan.steps.length) + "/" + S.plan.steps.length
      : (S.history.length ? S.history.length + " moves from the scramble" : "ready");
    const solveBtn = byId("cube-solve-btn");
    const playBtn = byId("cube-play-btn");
    if (solveBtn) solveBtn.disabled = solved || !S.K.solver;
    if (playBtn) playBtn.disabled = solved || !S.K.solver;
    renderSteps();
  }

  /* ---------- moves ---------- */
  function tokInfo(tok) {
    /* number-first SiGN: [1-21]?[URFDLB]w?[2']? */
    const mt = /^(\d+)?([URFDLB])(w)?(2|')?$/.exec(tok);
    if (!mt) return null;
    return {
      face: mt[2],
      width: mt[1] ? parseInt(mt[1], 10) : (mt[3] ? 2 : 1),
    };
  }
  function invertTok(tok) {
    if (tok.endsWith("2")) return tok;
    if (tok.endsWith("'")) return tok.slice(0, -1);
    return tok + "'";
  }

  function applyTok(tok, record) {
    S.state = S.K.core.applyMove(S.state, tok);
    if (record !== false) S.history.push(tok);
    if (S.n <= 3) flash(tok[0]);
    else {
      const info = tokInfo(tok);
      if (info) flashNet(info.face);
    }
    renderMoveLog();
  }

  /* newest on top, newest highlighted; the rail scrolls so long solves
     never blow the layout up */
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
    const inv = invertTok(tok);
    S.state = S.K.core.applyMove(S.state, inv);
    if (S.n <= 3) flash(inv[0]);
    S.plan = null; S.stepIdx = 0;
    render();
  }
  let flashTimer = 0;
  function flash(face) {
    for (const el of FACE_RING[face] || []) el.classList.add("flash");
    clearTimeout(flashTimer);
    flashTimer = setTimeout(() => {
      for (const f of FACES) for (const el of FACE_RING[f]) el.classList.remove("flash");
    }, 190);
  }
  function coach(text) { byId("cube-coach").textContent = text; }

  /* ---------- solve + step-through ---------- */
  function solve() {
    if (S.auto) stopAuto();
    if (!S.K.solver) {
      coach(S.n + "×" + S.n + " has no coach — free play with live piece-law "
        + "validation. The solver covers 2×2 and 3×3; pretending otherwise "
        + "would be a lie in a solver that verifies every move.");
      return;
    }
    const solved = S.K.core.verifySolved(S.state).solved;
    if (solved) { coach("already solved — scramble it first."); return; }
    const t0 = nowMs();
    S.plan = S.K.solve(S.state, engine.status === "ready" ? engineChoose : undefined);
    S.meters.sample("solve.ms", nowMs() - t0);
    S.stepIdx = 0;
    if (!S.plan) {
      coach("the solver refused this position (" + S.n + "×" + S.n
        + ") — that should be impossible for a legal scramble; it is logged, not hidden.");
      render();
      return;
    }
    const ties = S.plan.ties ? S.plan.ties.filter((t) => t.source === "engine").length : 0;
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
      const inv = invertTok(st.moves[i]);
      S.state = S.K.core.applyMove(S.state, inv);
      if (S.n <= 3) flash(inv[0]);
    }
    coach("stepped back: " + st.title);
    render();
  }
  function startAuto() {
    if (!S.K.solver) { solve(); return; }
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
      n: S.n,
      moves: S.history.length,
      steps: S.planSteps || 0,
      seed: S.cfg.seed,
      stamp: S.K.stamp(S.cfg),
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
  function readSize() {
    const sel = byId("cube-size");
    const n = sel ? parseInt(sel.value, 10) : 3;
    return Number.isInteger(n) && n >= 2 && n <= 21 ? n : 3;
  }

  function newGame() {
    stopAuto();
    const raw = byId("cube-seed").value.trim();
    /* optional numeric seed: empty means a fresh random number every game */
    const seed = raw === "" ? DK.randomSeed() : String(parseInt(raw, 10) || DK.randomSeed());
    byId("cube-seed").value = seed;
    /* shape mods are 3×3 physical puzzles: any other size plays classic */
    S.cfg = {
      seed, scrambleLen: +byId("cube-len").value,
      cubeType: S.K.mods ? S.cfg.cubeType : "classic",
    };
    const g = S.K.newState(seed, S.cfg.scrambleLen);
    S.state = g.state;
    S.scramble = g.scramble.slice();
    S.history = [];
    S.plan = null; S.stepIdx = 0; S.solvedAt = 0;
    const what = S.K.solver
      ? "Turn it by hand, or ask for the " + (S.n === 3 ? "layer-by-layer" : "beginner") + " plan."
      : "Free play — every position is checked against the piece laws as you move.";
    coach("scrambled with " + S.scramble.length + " turns (seed " + seed + "). " + what);
    buildStepList();
    renderMoveLog();
    render();
  }

  function resetParams() {
    byId("cube-len").value = String(S.K.core.defaultScrambleLen
      ? S.K.core.defaultScrambleLen(S.n) : C.DEFAULT_CONFIG.scrambleLen);
    byId("cube-seed").value = "";
    byId("cube-type").value = "classic";
    newGame();
  }

  /* rebuild everything below the bar when the size changes: cores,
     renderers, and controls all follow n */
  function rebuildForSize(n) {
    S.n = n;
    S.K = makeK(n);
    S.width = 1;
    const stage = byId("cube-stage");
    stage.textContent = "";
    stage.appendChild(n <= 3
      ? h("div", { class: "cube3d-scene", id: "cube-scene" },
          h("div", { class: "cube3d-orbit" }, buildCube()))
      : buildNet());
    if (n <= 3) {
      wireOrbit(byId("cube-scene"));
      applyOrbit();
      applySkin();
    }
    buildMovePad();
    /* shape mods exist only at 3×3: reflect that in the type select */
    const typeSel = byId("cube-type");
    for (const opt of typeSel.options) {
      if (opt.value !== "classic") opt.disabled = !S.K.mods;
    }
    if (!S.K.mods && typeSel.value !== "classic") typeSel.value = "classic";
    S.cfg.cubeType = typeSel.value;
    const widthWrap = byId("cube-width-wrap");
    widthWrap.hidden = S.K.maxDepth < 2;
    if (S.K.maxDepth >= 2) {
      const w = byId("cube-width");
      w.max = String(S.K.maxDepth);
      w.value = "1";
      byId("cube-width-val").textContent = "1";
    }
    const note = byId("cube-solver-note");
    note.textContent = S.K.solver
      ? (n === 2
        ? "2×2 runs its own verified coach: layer build, corner orientation, "
          + "then a shortest-search finish over the 24 top permutations."
        : "3×3 runs the battle-tested layer-by-layer coach on the 54-facelet core.")
      : n + "×" + n + ": free play. Every move is validated against the piece "
        + "laws (color counts, piece multiplicity, corner twist, edge flip) — "
        + "the coach covers 2×2 and 3×3, and a fake plan for big cubes would "
        + "be a lie. Watch the laws chip stay green while you scramble.";
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
    const el = byId("cube3d");
    if (el) el.style.transform =
      "rotateX(" + S.orbit.rx + "deg) rotateY(" + S.orbit.ry + "deg)";
  }

  /* ---------- build ---------- */
  function moveBtn(tok) {
    return h("button", { class: "mz-chip cube-move", type: "button" }, tok);
  }
  function wideTok(face, suffix) {
    if (S.width === 1) return face + suffix;
    return (S.width > 2 ? String(S.width) : "") + face + "w" + suffix;
  }
  /* the move pad regenerates whenever size or turn width changes */
  function buildMovePad() {
    const pad = byId("cube-moves");
    pad.textContent = "";
    for (const f of ["U", "R", "F", "D", "L", "B"]) {
      pad.appendChild(moveBtn(wideTok(f, "")));
      pad.appendChild(moveBtn(wideTok(f, "'")));
      pad.appendChild(moveBtn(wideTok(f, "2")));
    }
    const note = byId("cube-moves-note");
    note.textContent = S.width === 1
      ? "Each button turns one face a quarter turn clockwise (viewed from "
        + "that face), counter-clockwise (prime), or a half turn (2). "
        + (S.n <= 3 ? "Drag the cube to orbit." : "")
      : "Buttons turn " + S.width + " layer" + (S.width > 1 ? "s" : "") + " at once (SiGN wide "
        + "moves" + (S.width > 2 ? ", number-first" : "") + ").";
  }

  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) S.cfg.seed = String(parseInt(deepSeed, 10) || DK.randomSeed());

    const sizeSel = h("select", { id: "cube-size", class: "mz-input", "aria-label": "cube size" });
    for (let n = 2; n <= 21; n++) {
      const label = n + "×" + n + (n === 2 ? " (coach)" : n === 3 ? " (coach + mods)" : " (free play)");
      sizeSel.appendChild(h("option", { value: String(n) }, label));
    }
    sizeSel.value = "3";

    const bar = h("div", { class: "game-bar" },
      sizeSel,
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
      h("span", { class: "mz-chip game-rung", id: "cube-laws-chip" }, "piece laws hold"),
      h("button", { class: "mz-chip", id: "cube-reset-btn", type: "button",
        title: "restore default parameters" }, "reset params"),
      h("button", { class: "mz-chip", id: "cube-fs-btn", type: "button" }, "⛶ full screen"),
      h("button", { class: "mz-chip", id: "cube-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "cube-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "cube-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "cube-stats-btn", type: "button" }, "ⓘ stats"),
    );

    const stage = h("div", { id: "cube-stage" });
    const under = h("div", { class: "mz-under game-under" },
      h("span", { class: "mz-kv" }, h("b", { id: "cube-progress" }, "ready")));
    const coachLine = h("p", { class: "game-hintline", id: "cube-coach", "aria-live": "polite" }, "");
    const solverNote = h("p", { class: "game-note", id: "cube-solver-note" }, "");

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
          h("div", { class: "cube-moves", id: "cube-moves" }),
          h("p", { class: "game-note", id: "cube-moves-note" }, ""),
          h("div", { class: "game-slider", id: "cube-width-wrap", hidden: "" },
            h("span", { class: "game-slider-label" }, "turn width (wide moves)"),
            h("input", { type: "range", id: "cube-width", min: "1", max: "2",
              step: "1", value: "1", "aria-label": "turn width in layers" }),
            h("span", { class: "game-sliderval", id: "cube-width-val" }, "1")),
        ],
      },
      {
        id: "setup", label: "Advanced — shape, scramble & solve",
        kids: [
          h("label", { class: "game-field" },
            h("span", { class: "game-slider-label" }, "cube type (3×3)"),
            h("select", { id: "cube-type", "aria-label": "cube shape mod" },
              h("option", { value: "classic" }, "classic — the standard cube"),
              h("option", { value: "mirror" }, "mirror — staggered piece depths, same algebra"),
              h("option", { value: "ghost" }, "ghost — irregular piece depths, same algebra"),
              h("option", { value: "sudoku" }, "sudokube — numbered stickers, same algebra")),
            h("span", { class: "game-note" },
              "Mirror, ghost, and sudokube are 3×3 skins: the move algebra, ",
              "scramble, and the whole solver are IDENTICAL — only the piece ",
              "geometry (or the sticker print) differs, exactly like the ",
              "physical puzzles. Other sizes play classic. Pyraminx, Skewb, ",
              "and Clock are different puzzle families with their own ",
              "verified cores and pages; megaminx, square-1, fisher, and ",
              "windmill need cut lines this renderer doesn't have — not ",
              "shipped, and pretending otherwise would be a lie in a solver ",
              "that verifies every move.")),
          h("label", { class: "game-slider" },
            h("span", { class: "game-slider-label" }, "scramble length"),
            h("input", { type: "range", id: "cube-len", min: "1", max: "40",
              step: "1", value: String(S.cfg.scrambleLen), "aria-label": "scramble length" }),
            h("span", { class: "game-sliderval", id: "cube-len-val" }, String(S.cfg.scrambleLen))),
          h("p", { class: "game-note" },
            "The seed is optional: leave it empty and every game rolls a fresh ",
            "random number (shown in the bar so the run stays reproducible — ",
            "type it back in to replay the exact scramble). Scrambles use the ",
            "full legal turn set for the chosen size, wide turns included."),
        ],
      },
      {
        id: "about", label: "What decides what?",
        kids: [h("p", { class: "game-note" },
          "The 3×3 solver is layer-by-layer: cross, first-layer corners, middle ",
          "edges, then edge orientation, corner orientation, corner ",
          "permutation, edge permutation, and a final alignment. The 2×2 coach ",
          "builds a layer, orients corners with the (R' D' R D) loop, then ",
          "searches a shortest finish over the 24 top permutations. Almost all ",
          "of it is RULE — the method is fixed and every candidate move is ",
          "verified by simulation before it is played. The genuine ties are ",
          "the set-ups: when several alignments reach the next phase in the ",
          "same number of moves, the engine picks one (and a refusal falls ",
          "back to roster order without derailing the solve). Every move the ",
          "plan plays is checked against the facelet model — the coach can't ",
          "lie you into a wrong cube.")],
      },
    ]);

    const moveLog = h("div", null,
      h("div", { class: "game-slider-label" }, "move history — newest first"),
      h("div", { class: "cube-movelog", id: "cube-movelog",
        "aria-live": "polite", "aria-label": "move history" },
        h("p", { class: "game-note" }, "no moves yet")));

    const main = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, stage, under, coachLine, solverNote),
      h("div", { class: "game-side" }, opts, moveLog, stepPane));
    root.appendChild(h("div", { class: "game-wrap", id: "cube-wrap" }, bar, main));

    buildOverlays();
    wire();
    engine.setEnabled(true);
    S.K = makeK(readSize());
    rebuildForSize(S.n);
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
        ...["#", "size", "moves", "seed", "when"].map((x) => h("th", null, x)))),
      h("tbody", null, rows.map((r, i) => h("tr", null,
        h("td", null, String(rows.length - i)),
        h("td", null, r.n ? r.n + "×" + r.n : "3×3"),
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
    body.appendChild(kv("cube size:", S.n + "×" + S.n + " (turn width " + S.width + ")"));
    body.appendChild(kv("engine calls:", String(m["engine.calls"] || 0)));
    body.appendChild(kv("engine latency avg:", (m["engine.ms"] ? m["engine.ms"].avg.toFixed(1) : "0") + " ms"));
    body.appendChild(kv("solve compute avg:", (m["solve.ms"] ? m["solve.ms"].avg.toFixed(1) : "0") + " ms"));
    body.appendChild(kv("history depth:", String(S.history.length)));
  }

  /* ---------- wire ---------- */
  function wire() {
    byId("cube-size").addEventListener("change", () => {
      rebuildForSize(readSize());
      newGame();
    });
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
    byId("cube-width").addEventListener("input", () => {
      S.width = parseInt(byId("cube-width").value, 10) || 1;
      byId("cube-width-val").textContent = String(S.width);
      buildMovePad();
    });
    byId("cube-type").addEventListener("change", () => {
      /* a shape mod is physical geometry (or print): apply to the CURRENT
         cube without rescrambling — state, history, and any plan stay valid */
      S.cfg.cubeType = byId("cube-type").value;
      if (S.n <= 3) {
        rebuildGeometry();
        applySkin();
        render();
      }
      coach(S.cfg.cubeType + " skin applied — same algebra, same plan.");
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
    /* the move pad regenerates on size/width changes — delegate clicks */
    byId("cube-moves").addEventListener("click", (e) => {
      const btn = e.target.closest("button.cube-move");
      if (btn) userMove(btn.textContent);
    });
    DK.shell.fullscreen(byId("cube-wrap"), byId("cube-fs-btn"));
    setChip();
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
