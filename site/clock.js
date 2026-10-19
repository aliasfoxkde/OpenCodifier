/* Rubik's Clock shell — both faces rendered as dial grids, wheel buttons
   with an amount stepper, a real y2 flip, and the coach from clock-core.js.
   clock-core.js owns the 18-dial model, the WCA-shaped scramble, and the
   solver (Gauss-Jordan over Z₁₂ on the 14 published-order generators,
   replay-verified); this file is chrome. No engine on this page: the
   clock's group is abelian, every legal state has exactly one solution,
   and a system with no ties has nothing to arbitrate. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const CK = globalThis.ClockCore;
  const root = document.getElementById("clock-root");
  if (!root || !DK || !CK) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;

  const WHEELS = CK.MOVES; /* ["UR","DR","DL","UL","U","R","D","L","ALL"] */
  /* dial grid order for display: reading rows — the core indexes the state
     as [0..8] front row-major UL..DR, [9..17] back in the same reading */
  const DIAL_LABELS = ["UL", "UC", "UR", "CL", "CC", "CR", "DL", "DC", "DR"];

  const S = {
    cfg: { ...CK.DEFAULT_CONFIG },
    state: CK.SOLVED.slice(),   /* a fresh clock is solved; scramble is the CTA */
    flipped: false,           /* odd number of y2 applied: the back is toward you */
    history: [],
    plan: null,
    stepIdx: 0,
    auto: false, autoTimer: 0,
    wheel: "UR",              /* selected wheel */
    amount: 3,
    board: DK.sessionBoard(),
    meters: DK.meters(),
  };

  /* ---------- dial rendering ---------- */
  /* one dial: a circle, a hand rotated value*30°, the value printed under
     it. Two 3×3 grids: FRONT (state[0..8]) and BACK (state[9..17]). */
  const DIALS = []; /* { el, hand, val, side, pos } */
  const SIDE_TOWARD = "#e94b3c";
  const SIDE_AWAY = "#5a6472";

  function buildSide(side) {
    const grid = h("div", {
      class: "clk-grid", role: "group",
      "aria-label": side === "front" ? "front dials" : "back dials",
    });
    grid.style.display = "grid";
    grid.style.gridTemplateColumns = "repeat(3, 64px)";
    grid.style.gap = "10px";
    for (let i = 0; i < 9; i++) {
      const cell = h("div", { class: "clk-dial" });
      cell.style.width = "64px";
      cell.style.height = "64px";
      cell.style.borderRadius = "50%";
      cell.style.border = "2px solid " + (side === "front" ? SIDE_TOWARD : SIDE_AWAY);
      cell.style.position = "relative";
      cell.style.background = "rgba(127,127,127,0.12)";
      const hand = h("div", { class: "clk-hand" });
      hand.style.position = "absolute";
      hand.style.left = "50%";
      hand.style.top = "50%";
      hand.style.width = "3px";
      hand.style.height = "24px";
      hand.style.background = "currentColor";
      hand.style.borderRadius = "2px";
      hand.style.transformOrigin = "50% 100%";
      hand.style.marginLeft = "-1.5px";
      hand.style.marginTop = "-24px";
      const val = h("span", { class: "clk-val" });
      val.style.position = "absolute";
      val.style.right = "3px";
      val.style.bottom = "1px";
      val.style.fontSize = "11px";
      val.style.fontWeight = "700";
      val.style.opacity = "0.85";
      cell.appendChild(hand);
      cell.appendChild(val);
      grid.appendChild(cell);
      DIALS.push({ el: cell, hand, val, side, pos: i });
    }
    return grid;
  }

  function renderDials() {
    for (const d of DIALS) {
      const base = d.side === "front" ? 0 : 9;
      const v = S.state[base + d.pos];
      d.hand.style.transform = "rotate(" + (v * 30) + "deg)";
      d.val.textContent = String(v);
      const toward = (d.side === "front") !== S.flipped;
      d.el.style.opacity = toward ? "1" : "0.55";
      d.el.title = DIAL_LABELS[d.pos] + " (" + d.side + ") = " + v
        + (toward ? " — facing you" : "");
    }
  }

  function render() {
    renderDials();
    const solved = CK.verifySolved(S.state).solved;
    const chip = byId("clk-solved-chip");
    if (chip) {
      chip.textContent = solved ? "solved — all 12:00" : "scrambled";
      chip.classList.toggle("on", solved);
    }
    const laws = byId("clk-laws-chip");
    if (laws) {
      const v = CK.validateState(S.state);
      laws.textContent = v.ok ? "pair laws hold" : "laws: broken";
      laws.classList.toggle("on", v.ok);
      laws.classList.toggle("off", !v.ok);
    }
    const side = byId("clk-side-chip");
    if (side) side.textContent = S.flipped ? "back toward you" : "front toward you";
    byId("clk-progress").textContent = S.plan
      ? "step " + Math.min(S.stepIdx + 1, S.plan.steps.length) + "/" + S.plan.steps.length
      : (S.history.length ? S.history.length + " turns from the scramble" : "ready");
    const solveBtn = byId("clk-solve-btn");
    const playBtn = byId("clk-play-btn");
    if (solveBtn) solveBtn.disabled = solved;
    if (playBtn) playBtn.disabled = solved;
    for (const b of root.querySelectorAll(".clk-wheel")) {
      b.classList.toggle("on", b.getAttribute("data-wheel") === S.wheel);
    }
    renderMoveLog();
  }

  /* ---------- moves ---------- */
  function invertTok(tok) {
    if (tok === "y2") return tok;
    const m = /^(UR|DR|DL|UL|U|R|D|L|ALL)(\d+)([+-])$/.exec(tok);
    if (!m) return tok;
    const a = parseInt(m[2], 10);
    if (a === 0) return tok;
    return a <= 6 ? m[1] + a + (m[3] === "+" ? "-" : "+")
      : m[1] + (12 - a) + (m[3] === "+" ? "+" : "-");
  }

  function applyTok(tok, record, fromPlan) {
    S.state = CK.applyMove(S.state, tok);
    if (tok === "y2") S.flipped = !S.flipped;
    if (record !== false) S.history.push(tok);
    if (!fromPlan && S.plan && S.stepIdx < S.plan.steps.length) {
      S.plan = null; S.stepIdx = 0;
      coach("your turn changed the clock — the plan below no longer fits. Solve again for a fresh one.");
    }
  }
  function wheelTok(name, amount, dir) {
    return name + amount + dir;
  }
  function userTurn() {
    if (S.auto) stopAuto();
    applyTok(wheelTok(S.wheel, S.amount, "+"));
    coach(S.wheel + " turned " + S.amount + " clockwise (viewed from the face toward you).");
    render();
  }
  function userTurnBack() {
    if (S.auto) stopAuto();
    applyTok(wheelTok(S.wheel, S.amount, "-"));
    coach(S.wheel + " turned " + S.amount + " counter-clockwise.");
    render();
  }
  function userFlip() {
    if (S.auto) stopAuto();
    applyTok("y2");
    coach("y2 — the other face is toward you. Wheels now act on it.");
    render();
  }
  function undo() {
    if (S.auto) stopAuto();
    const tok = S.history.pop();
    if (!tok) return;
    S.state = CK.applyMove(S.state, invertTok(tok));
    if (tok === "y2") S.flipped = !S.flipped;
    S.plan = null; S.stepIdx = 0;
    render();
  }
  function coach(text) { byId("clk-coach").textContent = text; }

  /* ---------- solve + step-through ---------- */
  function solve() {
    if (S.auto) stopAuto();
    if (CK.verifySolved(S.state).solved) { coach("already solved — scramble it first."); return; }
    const t0 = nowMs();
    let plan = null;
    try {
      plan = CK.solve(S.state);
    } catch (err) {
      coach("the solver refused this position: " + err.message);
      render();
      return;
    }
    S.meters.sample("solve.ms", nowMs() - t0);
    S.plan = plan;
    S.stepIdx = 0;
    coach("plan ready: " + plan.wheelMoves + " wheel turns (+ y2 flips) in "
      + plan.steps.length + " steps, solved by elimination over Z₁₂. "
      + "Step through it, or let it play.");
    buildStepList();
    render();
  }
  function stepFwd() {
    if (!S.plan || S.stepIdx >= S.plan.steps.length) return;
    const st = S.plan.steps[S.stepIdx];
    applyTok(st.moves[0], undefined, true);
    render();
    coach(st.title + " — " + st.moves[0]);
    highlightStep(S.stepIdx);
    S.stepIdx++;
    if (S.stepIdx >= S.plan.steps.length) {
      S.planSteps = S.plan.steps.length;
      S.plan = null; S.stepIdx = 0;
      recordBoard();
      coach("solved. " + S.history.length + " recorded turns from the scramble. Scramble for another round.");
    }
    render();
  }
  function stepBack() {
    if (S.auto) stopAuto();
    if (!S.plan || S.stepIdx === 0) return;
    S.stepIdx--;
    const st = S.plan.steps[S.stepIdx];
    S.state = CK.applyMove(S.state, invertTok(st.moves[0]));
    if (st.moves[0] === "y2") S.flipped = !S.flipped;
    coach("stepped back: " + st.title);
    render();
  }
  function startAuto() {
    if (!S.plan) solve();
    if (!S.plan) return;
    S.auto = true;
    byId("clk-play-btn").textContent = "⏸ pause";
    tickAuto();
  }
  function tickAuto() {
    if (!S.auto) return;
    if (!S.plan) { stopAuto(); return; }
    stepFwd();
    if (S.auto) S.autoTimer = setTimeout(tickAuto, 550);
  }
  function stopAuto() {
    S.auto = false;
    clearTimeout(S.autoTimer);
    const b = byId("clk-play-btn");
    if (b) b.textContent = "▶ auto-solve";
  }
  function recordBoard() {
    S.board.record({
      ts: Date.now(), outcome: "solved",
      moves: S.history.length,
      steps: S.planSteps || 0,
      seed: S.cfg.seed,
      stamp: CK.clockStamp(S.cfg),
    });
  }

  /* ---------- step list ---------- */
  function buildStepList() {
    const list = byId("clk-steplist");
    list.textContent = "";
    if (!S.plan) return;
    S.plan.steps.forEach((st, i) => {
      const row = h("div", { class: "cube-step", "data-step": String(i) },
        h("span", { class: "mz-chip game-rung" }, "rule"),
        h("span", { class: "cube-step-title" }, st.title),
        h("span", { class: "cube-step-moves" }, st.moves[0]));
      row.addEventListener("click", () => jumpTo(i));
      list.appendChild(row);
    });
  }
  function highlightStep(i) {
    for (const el of byId("clk-steplist").children) {
      el.classList.toggle("now", el.getAttribute("data-step") === String(i));
    }
  }
  function jumpTo(i) {
    if (!S.plan || S.auto) return;
    while (S.stepIdx > i) stepBack();
    while (S.stepIdx < i) stepFwd();
  }

  /* newest on top; bounded DOM, count line keeps the truth */
  function renderMoveLog() {
    const log = byId("clk-movelog");
    if (!log) return;
    log.textContent = "";
    const hist = S.history;
    if (!hist.length) {
      log.appendChild(h("p", { class: "game-note" }, "no turns yet"));
      return;
    }
    const windowN = 120;
    const start = Math.max(0, hist.length - windowN);
    for (let i = hist.length - 1; i >= start; i--) {
      log.appendChild(h("span",
        { class: "mz-chip cube-move" + (i === hist.length - 1 ? " latest" : "") }, hist[i]));
    }
    if (start > 0) {
      log.appendChild(h("p", { class: "game-note" },
        "… " + start + " older turns (" + hist.length + " total)"));
    }
  }

  /* ---------- lifecycle ---------- */
  function newGame() {
    stopAuto();
    const raw = byId("clk-seed").value.trim();
    const seed = raw === "" ? DK.randomSeed() : String(parseInt(raw, 10) || DK.randomSeed());
    byId("clk-seed").value = seed;
    S.cfg = CK.normalizeConfig({
      seed, scrambleLen: +byId("clk-len").value,
    });
    const g = CK.createGame(S.cfg);
    S.state = g.state;
    S.flipped = false;
    S.history = [];
    S.plan = null; S.stepIdx = 0;
    coach("scrambled WCA-style: " + (S.cfg.scrambleLen) + " wheels a side around a y2 (seed "
      + seed + "). Turn wheels by hand, or ask for the elimination plan.");
    buildStepList();
    render();
  }
  function resetParams() {
    byId("clk-len").value = String(CK.DEFAULT_CONFIG.scrambleLen);
    byId("clk-seed").value = "";
    newGame();
  }

  /* ---------- build ---------- */
  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) S.cfg.seed = String(parseInt(deepSeed, 10) || DK.randomSeed());

    const wheelBtn = (name) => h("button", {
      class: "mz-chip clk-wheel", type: "button", "data-wheel": name,
      title: "select the " + name + " wheel",
    }, name);

    const bar = h("div", { class: "game-bar" },
      h("button", { class: "mz-chip", id: "clk-scramble-btn", type: "button" }, "⟲ scramble"),
      h("button", { class: "mz-chip", id: "clk-solve-btn", type: "button" }, "solve"),
      h("button", { class: "mz-chip mz-play", id: "clk-play-btn", type: "button" }, "▶ auto-solve"),
      h("button", { class: "mz-chip", id: "clk-back-btn", type: "button" }, "⏪"),
      h("button", { class: "mz-chip", id: "clk-fwd-btn", type: "button" }, "⏩"),
      h("button", { class: "mz-chip", id: "clk-undo-btn", type: "button" }, "↶ undo"),
      h("button", { class: "mz-chip", id: "clk-flip-btn", type: "button" }, "⇅ flip (y2)"),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "clk-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "clk-seed", class: "mz-input", size: "8",
          placeholder: "random", min: "0", "aria-label": "numeric seed, empty for a random one each game" }),
        h("button", { class: "mz-chip", id: "clk-dice", type: "button",
          title: "roll a new random seed" }, "🎲")),
      h("span", { class: "mz-chip game-rung", id: "clk-solved-chip" }, "scrambled"),
      h("span", { class: "mz-chip game-rung", id: "clk-laws-chip" }, "pair laws hold"),
      h("span", { class: "mz-chip game-rung", id: "clk-side-chip" }, "front toward you"),
      h("button", { class: "mz-chip", id: "clk-reset-btn", type: "button",
        title: "restore default parameters" }, "reset params"),
      h("button", { class: "mz-chip", id: "clk-fs-btn", type: "button" }, "⛶ full screen"),
      h("button", { class: "mz-chip", id: "clk-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "clk-stats-btn", type: "button" }, "ⓘ stats"),
    );

    const frontPane = h("div", { class: "clk-pane" },
      h("div", { class: "game-slider-label" }, "front"),
      buildSide("front"));
    const backPane = h("div", { class: "clk-pane" },
      h("div", { class: "game-slider-label" }, "back"),
      buildSide("back"));
    const stage = h("div", { id: "clk-stage", style: "display:flex;gap:28px;flex-wrap:wrap;justify-content:center;" },
      frontPane, backPane);
    const under = h("div", { class: "mz-under game-under" },
      h("span", { class: "mz-kv" }, h("b", { id: "clk-progress" }, "ready")));
    const coachLine = h("p", { class: "game-hintline", id: "clk-coach", "aria-live": "polite" }, "");

    const pins = h("div", { class: "clk-pins", style: "display:flex;gap:8px;flex-wrap:wrap;margin-top:10px;" },
      ...["UL", "UR", "DL", "DR"].map((p) => h("span", { class: "mz-chip game-rung on" }, "pin " + p + ": up")),
      h("span", { class: "game-note" },
        "standard solving posture — the move table's wheel-dial coupling assumes it"));

    const stepPane = h("div", null,
      h("div", { class: "cube-steplist", id: "clk-steplist" }),
      h("p", { class: "game-note" },
        "Steps appear once you press solve. Every step is a rule: the amounts ",
        "are the unique solution of a linear system over Z₁₂ — there is no ",
        "tie anywhere in a clock solve, so no engine, no roster, no random.",
        "Click a step to jump to it."));

    const opts = DK.shell.accordion([
      {
        id: "turn", label: "Turn a wheel", open: true,
        kids: [
          h("div", { class: "cube-moves", id: "clk-wheels" },
            ...WHEELS.map(wheelBtn)),
          h("label", { class: "game-slider" },
            h("span", { class: "game-slider-label" }, "turn amount (hours, 1–6)"),
            h("input", { type: "range", id: "clk-amount", min: "1", max: "6",
              step: "1", value: String(S.amount), "aria-label": "turn amount in hours" }),
            h("span", { class: "game-sliderval", id: "clk-amount-val" }, String(S.amount))),
          h("div", { class: "cube-moves" },
            h("button", { class: "mz-chip mz-play", id: "clk-cw-btn", type: "button" }, "↻ turn CW"),
            h("button", { class: "mz-chip", id: "clk-ccw-btn", type: "button" }, "↺ turn CCW")),
          h("p", { class: "game-note" },
            "Select a wheel, pick an amount, turn. Wheels act on the face ",
            "toward you — flip with y2 to reach the other side. A 6-hour turn ",
            "is the half-way mark; the core writes amounts past it as their ",
            "complement (UR8+ and UR4- are the same turn)."),
          pins,
        ],
      },
      {
        id: "setup", label: "Scramble",
        kids: [
          h("label", { class: "game-slider" },
            h("span", { class: "game-slider-label" }, "wheels per side (WCA uses 5)"),
            h("input", { type: "range", id: "clk-len", min: "1", max: "9",
              step: "1", value: String(S.cfg.scrambleLen), "aria-label": "wheels per side" }),
            h("span", { class: "game-sliderval", id: "clk-len-val" }, String(S.cfg.scrambleLen))),
          h("p", { class: "game-note" },
            "The scramble follows the WCA shape: distinct wheel moves in ",
            "canonical order, a y2, then the same for the back side, amounts ",
            "−5…+5 drawn the way the official scrambler draws them. The seed ",
            "is optional: leave it empty and every game rolls a fresh random ",
            "number, shown so the run stays reproducible."),
        ],
      },
      {
        id: "about", label: "Why no engine here?",
        kids: [h("p", { class: "game-note" },
          "Because there is nothing to decide. The clock's move group is ",
          "abelian: wheels commute, amounts add mod 12, and eliminating the ",
          "generators leaves rank 14 with exactly four consistency laws — so ",
          "every legal position has ONE solution. A tie is a decision with ",
          "several defensible answers; a clock solve has none. This is the ",
          "runtime's first principle (cheapest reliable mechanism first) ",
          "applied to itself: an exact rule beats an escalation, and an ",
          "escalation here would be theater. The engine pages — maze, pong, ",
          "the cube — have genuine ties; this page has a theorem.")],
      },
    ]);

    const moveLog = h("div", null,
      h("div", { class: "game-slider-label" }, "move history — newest first"),
      h("div", { class: "cube-movelog", id: "clk-movelog",
        "aria-live": "polite", "aria-label": "move history" },
        h("p", { class: "game-note" }, "no turns yet")));

    const main = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, stage, under, coachLine),
      h("div", { class: "game-side" }, opts, moveLog, stepPane));
    root.appendChild(h("div", { class: "game-wrap", id: "clk-wrap" }, bar, main));

    buildOverlays();
    wire();
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
    root.appendChild(overlay("clk-board-overlay", "Session scoreboard", "clk-board-body", "clk-board-close"));
    root.appendChild(overlay("clk-stats-overlay", "Measured stats", "clk-stats-body", "clk-stats-close"));
  }
  function renderBoard() {
    const body = byId("clk-board-body");
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
        ...["#", "turns", "seed", "when"].map((x) => h("th", null, x)))),
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
    const body = byId("clk-stats-body");
    if (!body) return;
    body.textContent = "";
    const m = S.meters.snapshot();
    const kv = (label, val) => h("p", { class: "game-note" }, label + " " + val);
    body.appendChild(kv("solve compute avg:", (m["solve.ms"] ? m["solve.ms"].avg.toFixed(1) : "0") + " ms"));
    body.appendChild(kv("history depth:", String(S.history.length)));
    body.appendChild(kv("flipped:", S.flipped ? "yes (back toward you)" : "no"));
  }

  /* ---------- wire ---------- */
  function wire() {
    byId("clk-scramble-btn").addEventListener("click", newGame);
    byId("clk-solve-btn").addEventListener("click", solve);
    byId("clk-play-btn").addEventListener("click", () => (S.auto ? stopAuto() : startAuto()));
    byId("clk-fwd-btn").addEventListener("click", () => { if (S.auto) stopAuto(); stepFwd(); });
    byId("clk-back-btn").addEventListener("click", stepBack);
    byId("clk-undo-btn").addEventListener("click", undo);
    byId("clk-flip-btn").addEventListener("click", userFlip);
    byId("clk-cw-btn").addEventListener("click", userTurn);
    byId("clk-ccw-btn").addEventListener("click", userTurnBack);
    byId("clk-wheels").addEventListener("click", (e) => {
      const btn = e.target.closest("button.clk-wheel");
      if (!btn) return;
      S.wheel = btn.getAttribute("data-wheel");
      render();
    });
    byId("clk-amount").addEventListener("input", () => {
      S.amount = parseInt(byId("clk-amount").value, 10) || 1;
      byId("clk-amount-val").textContent = String(S.amount);
    });
    byId("clk-dice").addEventListener("click", () => {
      byId("clk-seed").value = DK.randomSeed();
      newGame();
    });
    byId("clk-seed").addEventListener("change", newGame);
    byId("clk-len").addEventListener("input", () => {
      byId("clk-len-val").textContent = byId("clk-len").value;
    });
    byId("clk-len").addEventListener("change", newGame);
    byId("clk-reset-btn").addEventListener("click", resetParams);
    byId("clk-board-btn").addEventListener("click", () => { renderBoard(); byId("clk-board-overlay").showModal(); });
    byId("clk-board-close").addEventListener("click", () => byId("clk-board-overlay").close());
    byId("clk-stats-btn").addEventListener("click", () => { renderStats(); byId("clk-stats-overlay").showModal(); });
    byId("clk-stats-close").addEventListener("click", () => byId("clk-stats-overlay").close());
    DK.shell.fullscreen(byId("clk-wrap"), byId("clk-fs-btn"));

    /* first paint: the fresh clock IS solved — say so (scramble is the CTA) */
    render();
  }

  build();
})();
