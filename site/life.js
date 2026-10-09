/* Life shell — the Playground Life sandbox on the Decisions SDK.
   life-core.js owns the rules and the classification features; this file is
   chrome: canvas + painting, rule controls, the pattern library, the verdict
   chip (with its rung), the seed proposer, and the session-only experiment
   log. The rules NEVER ask the engine — the engine only reads the board and
   breaks ties between equally-interesting proposed seeds. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const LC = globalThis.LifeCore;
  const root = document.getElementById("life-root");
  if (!root || !DK || !LC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;

  /* canvas stays dark in both site themes, like the maze and pong */
  const PAL = {
    bg: "#04100d", grid: "rgba(94,234,212,0.07)", wall: "rgba(94,234,212,0.25)",
    cell: "#5eead4", fade: "rgba(94,234,212,0.35)",
  };

  /* ---------- state ---------- */
  const S = {
    cfg: { cols: 96, rows: 56, wrap: false, rule: "B3/S23", seed: "oc-life" },
    world: null,
    speed: 20,             /* generations per second */
    classifyEvery: 15,
    density: 0.28,
    running: false,   /* boots paused — the soup waits for play */
    popWindow: [],
    verdict: { label: "—", conf: null, rung: "none", at: -1 },
    rungCount: { heuristic: 0, engine: 0 },
    last: 0, acc: 0, stepMs: 0,
    paintValue: 1, painting: false,
    genCounter: 0, genWindowCount: 0, genWindowAt: 0, fpsGens: 0,
    boardOpen: false, statsOpen: false,
    board: DK.sessionBoard(),
    meters: DK.meters(),
    fps: 0,
  };

  /* no importBase: the SDK resolves its own wasm URL correctly */
  const engine = DK.engineBridge({
    onStatus: (st) => {
      const chip = byId("life-engine-chip");
      if (!chip) return;
      chip.textContent = "engine: " + st;
      chip.classList.toggle("on", st === "ready");
      chip.classList.toggle("off", st === "unavailable" || st === "off");
    },
  });

  function engineChoose(request) {
    const t0 = nowMs();
    const res = engine.choose(request);
    S.meters.sample("engine.ms", nowMs() - t0);
    if (res) S.rungCount.engine += 1;
    else S.rungCount.heuristic += 1;
    return res;
  }

  /* ---------- world lifecycle ---------- */
  function seedStr() {
    /* numeric-only: the shell kit keeps the box honest (random fill-in) */
    return S.seedKit ? S.seedKit.get() : (S.cfg.seed || "oc-life");
  }
  function newWorld(keepCells) {
    const prev = S.world;
    S.world = LC.createWorld(S.cfg);
    if (keepCells && prev && prev.cfg.cols === S.world.cfg.cols &&
        prev.cfg.rows === S.world.cfg.rows) {
      S.world.cells.set(prev.cells);
      S.world.pop = prev.pop;
    }
    S.popWindow = [];
    S.verdict = { label: "—", conf: null, rung: "none", at: -1 };
    S.acc = 0;
  }

  function loadSeed(cells) {
    newWorld(false);
    const w = S.world;
    const ox = (w.cfg.cols >> 1) - 6;
    const oy = (w.cfg.rows >> 1) - 6;
    for (const [x, y] of cells) LC.setCell(w, ox + x, oy + y, 1);
    S.running = false;   /* boots paused — the soup waits for play */
    syncPlayBtn();
    render();
    renderUnder();
  }

  function cfgFromUI() {
    const wrap = byId("life-wrap").checked;
    S.speed = +byId("life-speed").value;
    S.classifyEvery = +byId("life-classify").value;
    S.density = +byId("life-density").value / 100;
    const [cols, rows] = byId("life-size").value.split("x").map(Number);
    const ruleRaw = byId("life-rule-custom").value.trim() || byId("life-rule-preset").value;
    const changed = S.cfg.cols !== cols || S.cfg.rows !== rows ||
      S.cfg.rule !== ruleRaw || S.cfg.wrap !== wrap;
    S.cfg.wrap = wrap;
    if (changed) {
      S.cfg.cols = cols; S.cfg.rows = rows;
      S.cfg.rule = ruleRaw;
      newWorld(false);
    }
  }

  /* ---------- classification (the honest engine part) ---------- */
  function readBoard() {
    const w = S.world;
    const f = LC.features(w, S.popWindow);
    const local = LC.classify(f);
    let verdict = { ...local, rung: "heuristic" };
    if (engine.status === "ready") {
      const res = engineChoose(LC.engineRequest(f));
      const fromEngine = LC.applyEngineAnswer(res);
      if (fromEngine) {
        verdict = { label: fromEngine.label, conf: fromEngine.conf, rung: "engine" };
      }
    }
    S.verdict = { ...verdict, at: w.gen, features: f };
    renderVerdict();
  }

  function renderVerdict() {
    const v = S.verdict;
    const chip = byId("life-verdict");
    if (!chip) return;
    const conf = typeof v.conf === "number" ? " · " + Math.round(v.conf * 100) + "%" : "";
    chip.textContent = "verdict: " + v.label + conf + " · " + v.rung;
    chip.classList.toggle("on", v.rung === "engine");
    const line = byId("life-features");
    if (line && v.features) {
      const f = v.features;
      line.textContent = "gen " + f.gen + " · pop " + f.pop +
        " · +" + f.births + "/-" + f.deaths + " · cycle " + (f.period || "—") +
        " · trend ×" + f.trend + " · drift " + f.drift;
    }
  }

  function recordExperiment(source) {
    const v = S.verdict;
    if (!v.features) return;
    S.board.record({
      ts: nowMs(),
      rule: S.world.cfg.rule.label,
      seed: seedStr(),
      gen: v.features.gen,
      pop: v.features.pop,
      verdict: v.label,
      rung: v.rung,
      source,
    });
  }

  /* ---------- seed proposals ---------- */
  function suggestSeed() {
    cfgFromUI();
    const t0 = nowMs();
    const cands = LC.seedCandidates(
      { cols: S.cfg.cols, rows: S.cfg.rows, wrap: S.cfg.wrap, rule: S.cfg.rule, seed: seedStr() },
      3, String(Date.now()));
    S.meters.sample("propose.ms", nowMs() - t0);
    let pick = LC.pickSeed(cands);
    let source = "top score";
    if (pick.tie && engine.status === "ready") {
      const res = engineChoose(LC.seedRequest(cands, pick.tie));
      const ans = res && res.outcome === "accept" && res.answers && res.answers[0];
      const id = ans && ans.choice;
      const chosen = pick.tie.find((s) => s.id === id);
      if (chosen) { pick = { best: chosen, tie: null }; source = "engine tie-break"; }
      else source = "tie fell back to top score";
    }
    loadSeed(pick.best.cells);
    readBoardSoon = 2;   /* classify once the soup has breathed a little */
    recordExperiment(source + " · " + pick.best.id);
  }

  let readBoardSoon = 0;

  /* ---------- rendering ---------- */
  function render() {
    const canvas = byId("life-canvas");
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    const w = S.world;
    const cw = canvas.width / w.cfg.cols;
    const ch = canvas.height / w.cfg.rows;
    ctx.fillStyle = PAL.bg;
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    if (cw >= 5) {
      ctx.strokeStyle = PAL.grid;
      ctx.lineWidth = 1;
      ctx.beginPath();
      for (let x = 1; x < w.cfg.cols; x++) { ctx.moveTo(x * cw, 0); ctx.lineTo(x * cw, canvas.height); }
      for (let y = 1; y < w.cfg.rows; y++) { ctx.moveTo(0, y * ch); ctx.lineTo(canvas.width, y * ch); }
      ctx.stroke();
    }
    ctx.fillStyle = PAL.cell;
    for (let y = 0; y < w.cfg.rows; y++) {
      const off = y * w.cfg.cols;
      for (let x = 0; x < w.cfg.cols; x++) {
        if (w.cells[off + x]) {
          ctx.fillRect(x * cw, y * ch, Math.max(1, cw - 0.6), Math.max(1, ch - 0.6));
        }
      }
    }
    /* bounded boards show their wall; toroidal boards show none */
    if (!w.cfg.wrap) {
      ctx.strokeStyle = PAL.wall;
      ctx.setLineDash([6, 6]);
      ctx.strokeRect(0.5, 0.5, canvas.width - 1, canvas.height - 1);
      ctx.setLineDash([]);
    }
  }

  /* the SDK's kvRow takes one array of [label, value] pairs and renders
     them as a single .mz-under row */
  function kvRow(pairs) { return DK.shell.kvRow(pairs); }

  function renderUnder() {
    const w = S.world;
    const under = byId("life-under");
    if (!under) return;
    under.textContent = "";
    under.appendChild(kvRow([
      ["state", S.running ? "running" : "paused"],
      ["generation", String(w.gen)],
      ["population", String(w.pop)],
      ["rule", w.cfg.rule.label + (w.cfg.wrap ? " · toroidal" : " · bounded")],
      ["gens/s", String(S.fpsGens || 0)],
    ]));
  }

  /* ---------- overlays (shared chrome, life data) ---------- */
  function overlay(id, title, bodyId, closeId) {
    return h("dialog", { class: "mz-overlay", id },
      h("div", { class: "mz-board-card" },
        h("div", { class: "mz-overlay-head" },
          h("span", { class: "mz-overlay-title" }, title),
          h("button", { class: "mz-chip", id: closeId, type: "button" }, "✕ close")),
        h("div", { class: "mz-overlay-scroll", id: bodyId })));
  }

  function buildOverlays() {
    const board = overlay("life-board-overlay", "Experiment log (session)", "life-board-body", "life-board-close");
    const stats = overlay("life-stats-overlay", "Measured stats", "life-stats-body", "life-stats-close");
    root.appendChild(board);
    root.appendChild(stats);
  }

  function renderBoard() {
    const body = byId("life-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" },
        "No experiments recorded yet. Run the classifier, then press “record experiment” — or propose a seed, which records itself."));
      return;
    }
    const t = h("table", { class: "game-table" },
      h("thead", {}, h("tr", {},
        h("th", {}, "when"), h("th", {}, "rule"), h("th", {}, "seed"),
        h("th", {}, "gen"), h("th", {}, "pop"), h("th", {}, "verdict"),
        h("th", {}, "rung"), h("th", {}, "source"))));
    const tbody = h("tbody", {});
    rows.slice().reverse().forEach((r) => {
      tbody.appendChild(h("tr", {},
        h("td", {}, new Date(r.ts).toLocaleTimeString()),
        h("td", {}, r.rule), h("td", {}, r.seed), h("td", {}, String(r.gen)),
        h("td", {}, String(r.pop)), h("td", {}, r.verdict),
        h("td", {}, r.rung), h("td", {}, r.source)));
    });
    t.appendChild(tbody);
    body.appendChild(t);
    body.appendChild(h("div", { style: "margin-top:0.8rem" },
      h("button", { class: "mz-chip", type: "button", onclick: () => { S.board.clear(); renderBoard(); } },
        "clear session log")));
  }

  function renderStats() {
    const body = byId("life-stats-body");
    if (!body) return;
    body.textContent = "";
    const snap = S.meters.snapshot();
    const rows = [
      ["frames/s", String(S.fps)],
      ["generations/s (measured)", String(S.fpsGens || 0)],
      ["verdicts — heuristic / engine", S.rungCount.heuristic + " / " + S.rungCount.engine],
      ["experiments logged", String(S.board.all().length)],
      ["engine status", engine.status],
    ];
    if (snap["step.ms"]) rows.push(["step ms — avg / worst",
      DK.fmtMs(snap["step.ms"].avg) + " / " + DK.fmtMs(snap["step.ms"].worst)]);
    if (snap["engine.ms"]) rows.push(["engine call — avg / worst",
      DK.fmtMs(snap["engine.ms"].avg) + " / " + DK.fmtMs(snap["engine.ms"].worst)]);
    if (snap["propose.ms"]) rows.push(["seed proposal — avg", DK.fmtMs(snap["propose.ms"].avg)]);
    body.appendChild(h("div", { class: "mz-stats-grid" }, rows.map(([k, v]) =>
      h("div", { class: "mz-stats-row" },
        h("span", { class: "mz-stats-label" }, k),
        h("span", { class: "mz-stats-value" }, v)))));
    body.appendChild(h("p", { class: "game-note" },
      "Measured in this tab only. The verdict rows count which rung answered each classification — the engine is welcome to abstain, and the heuristic verdict is what shows when it does."));
  }

  /* ---------- main loop ---------- */
  function tick(ms) {
    if (!S.running) { requestAnimationFrame(tick); return; }   /* paused: no work */
    const dt = Math.min(0.25, (ms - S.last) / 1000 || 0);
    S.last = ms;
    S.fps = Math.round(1 / Math.max(1e-6, dt));
    if (S.running) {
      S.acc += dt * S.speed;
      let steps = 0;
      const t0 = nowMs();
      while (S.acc >= 1 && steps < 8) {
        LC.step(S.world);
        S.popWindow.push(S.world.pop);
        if (S.popWindow.length > 24) S.popWindow.shift();
        S.acc -= 1;
        steps += 1;
      }
      if (steps) {
        S.stepMs = nowMs() - t0;
        S.meters.sample("step.ms", S.stepMs);
        S.genCounter += steps;
      }
      if (ms - S.genWindowAt >= 1000) {
        S.fpsGens = Math.round(((S.genCounter - S.genWindowCount) * 1000) / (ms - S.genWindowAt));
        S.genWindowCount = S.genCounter;
        S.genWindowAt = ms;
      }
      if (S.world.gen % S.classifyEvery === 0) readBoard();
      if (readBoardSoon > 0 && --readBoardSoon === 0) readBoard();
      if (S.world.gen >= S.world.cfg.maxGens) S.running = false;
    }
    render();
    renderUnder();
    requestAnimationFrame(tick);
  }

  function syncPlayBtn() {
    const btn = byId("life-play");
    if (btn) btn.textContent = S.running ? "⏸ pause" : "▶ play";
  }

  /* ---------- painting ---------- */
  function cellFromEvent(e) {
    const canvas = byId("life-canvas");
    const rect = canvas.getBoundingClientRect();
    const x = Math.floor(((e.clientX - rect.left) / rect.width) * S.world.cfg.cols);
    const y = Math.floor(((e.clientY - rect.top) / rect.height) * S.world.cfg.rows);
    return { x, y };
  }

  function paintAt(e) {
    const { x, y } = cellFromEvent(e);
    LC.setCell(S.world, x, y, S.paintValue);
    LC.resetCycle(S.world);        /* painting invalidates a detected cycle */
    S.popWindow = [];              /* ...and the trend window */
  }

  /* ---------- build ---------- */
  function slider(id, label, em, min, max, step, val) {
    return h("div", { class: "game-slider" },
      h("label", { for: id, class: "game-slider-label" }, label, " ",
        h("em", {}, em)),
      h("input", { type: "range", id, min: String(min), max: String(max), step: String(step), value: String(val) }),
      h("span", { class: "game-sliderval", id: id + "-val" }, String(val)));
  }

  function bindSlider(id) {
    const el = byId(id);
    el.addEventListener("input", () => {
      byId(id + "-val").textContent = el.value;
      cfgFromUI();
    });
  }

  function patternBtn(p) {
    return h("button", {
      class: "mz-chip", type: "button", title: p.hint,
      onclick: () => { loadSeed(p.cells); recordExperiment("pattern: " + p.name); },
    }, p.name);
  }

  function build() {
    const params = new URLSearchParams(window.location.search);
    if (params.get("seed")) S.cfg.seed = params.get("seed");
    if (params.get("rule")) S.cfg.rule = params.get("rule");

    /* top bar */
    const bar = h("div", { class: "game-bar" },
      h("button", { class: "mz-chip mz-play", id: "life-play", type: "button" }, "▶ play"),
      h("button", { class: "mz-chip", id: "life-step", type: "button" }, "⏭ step"),
      h("button", { class: "mz-chip", id: "life-clear", type: "button" }, "clear"),
      h("button", { class: "mz-chip", id: "life-soup", type: "button" }, "random soup"),
      h("button", { class: "mz-chip", id: "life-suggest", type: "button" }, "✨ propose a seed"),
      h("button", { class: "mz-chip", id: "life-read", type: "button" }, "read the board"),
      h("button", { class: "mz-chip", id: "life-record", type: "button" }, "record experiment"),
      h("span", { class: "mz-chip game-rung", id: "life-verdict" }, "verdict: —"),
      h("span", { class: "mz-chip game-rung", id: "life-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "life-board-btn", type: "button" }, "★ experiments"),
      h("button", { class: "mz-chip", id: "life-stats-btn", type: "button" }, "ⓘ stats"),
      h("button", { class: "mz-chip", id: "life-reset-params", type: "button" }, "reset params"),
      h("button", { class: "mz-chip", id: "life-fs", type: "button" }, "⛶ full screen"),
    );

    /* seed deep link input */
    const seedWrap = h("span", { class: "game-seed-wrap" },
      h("label", { for: "life-seed", class: "game-slider-label" }, "seed "),
      h("input", { type: "number", id: "life-seed", class: "mz-input", value: S.cfg.seed, size: "10" }),
      h("button", { class: "mz-chip", id: "life-dice", type: "button", title: "new random seed" }, "🎲"));


    const canvas = h("canvas", { id: "life-canvas", class: "game-canvas", width: "960", height: "560", role: "img",
      "aria-label": "Game of Life board — paint cells, run rules, read the engine's verdict" });
    const under = h("div", { class: "mz-under game-under", id: "life-under" });
    const featuresLine = h("div", { class: "game-hintline", id: "life-features",
      "aria-live": "polite" }, "no read yet — press “read the board”");

    /* options accordion */
    const ruleOpts = Object.values(LC.PRESETS).map((p) => {
      const canon = "B" + p.born.join("") + "/S" + p.survive.join("");
      return h("option", { value: canon, selected: canon === "B3/S23" ? "" : null },
        p.label);
    });
    const sizeOpts = ["64x40", "96x56", "120x72"].map((v) =>
      h("option", { value: v, selected: v === "96x56" ? "" : null }, v));

    const opts = DK.shell.accordion([
      {
        id: "board", label: "Board", open: true,
        kids: [
          h("div", { class: "game-slider" },
            h("label", { for: "life-size", class: "game-slider-label" }, "grid size"),
            h("select", { id: "life-size", class: "mz-input" }, sizeOpts)),
          h("div", { class: "game-slider" },
            h("label", { for: "life-rule-preset", class: "game-slider-label" }, "rule preset"),
            h("select", { id: "life-rule-preset", class: "mz-input" }, ruleOpts)),
          h("div", { class: "game-slider" },
            h("label", { for: "life-rule-custom", class: "game-slider-label" }, "custom rule ", h("em", {}, "B…/S… — blank uses the preset")),
            h("input", { type: "text", id: "life-rule-custom", class: "mz-input", placeholder: "B3/S23", size: "12" })),
          h("label", { class: "game-slider-label", style: "display:flex;gap:0.45rem;align-items:center" },
            h("input", { type: "checkbox", id: "life-wrap" }), "toroidal (edges wrap)"),
        ],
      },
      {
        id: "run", label: "Run", open: false,
        kids: [
          slider("life-speed", "speed", "generations/s", 1, 60, 1, S.speed),
          slider("life-classify", "classify every", "generations", 5, 60, 5, S.classifyEvery),
          slider("life-density", "soup density", "%", 5, 60, 5, Math.round(S.density * 100)),
        ],
      },
      {
        id: "patterns", label: "Pattern library", open: false,
        kids: [
          h("p", { class: "game-note" }, "Stamp a classic pattern at the center — the verdict chip tells you what it became."),
          h("div", { class: "game-rewards" }, LC.PATTERNS.map(patternBtn)),
        ],
      },
      {
        id: "about", label: "What decides what?", open: false,
        kids: [
          h("p", { class: "game-note" },
            "The rules are pure code — the engine never touches a cell. What you can watch: every ",),
          h("p", { class: "game-note" },
            "classify run reads the same features you see under the court (population, births, cycle period, trend) and returns a verdict chip — heuristic when the engine is off, engine + confidence when it answers, and the local verdict if it abstains. “Propose a seed” scores three candidate soups and, when two tie as equally interesting, that exact tie goes to the engine."),
        ],
      },
    ]);

    const about = h("div", { class: "card game-about" },
      h("h3", {}, "Reading a sandbox honestly"),
      h("p", {}, "Life is a zero-player game: the rules decide everything, deterministically, forever — the engine is never the player here. It is the observer: it classifies what the code already did, and it breaks ties between equally-interesting proposed seeds. Every verdict names its rung, and every logged experiment keeps rule, seed, and verdict together so the claim can be replayed on this very board."));

    const layout = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, canvas, under, featuresLine),
      h("div", { class: "game-side" }, opts, seedWrap));

    root.appendChild(bar);
    root.appendChild(layout);
    root.appendChild(about);
    buildOverlays();

    S.seedKit = DK.shell.numericSeed(byId("life-seed"), byId("life-dice"));
    DK.shell.resetParams(byId("life-reset-params"), root);
    DK.shell.fullscreen(root, byId("life-fs"));

    newWorld(false);
    const p = params.get("pattern");
    if (p) {
      const pat = LC.PATTERNS.find((x) => x.name.toLowerCase() === p.toLowerCase());
      if (pat) { loadSeed(pat.cells); }
    }
    engine.setEnabled(true);   /* default on: verdicts reach the runtime */

    /* open alive: an empty court steps but shows nothing, so the landing
       state is a running soup (a ?pattern= link still loads that pattern) */
    if (!params.get("pattern")) {
      LC.randomSoup(S.world, S.density, DK.makeRng((seedStr()) + ":boot"));
      readBoardSoon = 2;
    }

    /* wire */
    byId("life-play").addEventListener("click", () => {
      S.running = !S.running;
      syncPlayBtn();
    });
    byId("life-step").addEventListener("click", () => {
      S.running = false; syncPlayBtn();
      LC.step(S.world); S.popWindow.push(S.world.pop);
      if (S.popWindow.length > 24) S.popWindow.shift();
      readBoard();
      render();
      renderUnder();
    });
    byId("life-clear").addEventListener("click", () => { newWorld(false); render(); renderVerdict(); });
    byId("life-soup").addEventListener("click", () => {
      cfgFromUI();
      LC.randomSoup(S.world, S.density, DK.makeRng((seedStr()) + ":soup:" + S.world.gen));
      S.popWindow = [];
      S.running = true; syncPlayBtn();
    });
    byId("life-suggest").addEventListener("click", suggestSeed);
    byId("life-read").addEventListener("click", readBoard);
    byId("life-record").addEventListener("click", () => recordExperiment("manual"));
    byId("life-seed").addEventListener("change", () => { /* affects soup/proposal rng only */ });
    byId("life-size").addEventListener("change", cfgFromUI);
    byId("life-rule-preset").addEventListener("change", cfgFromUI);
    byId("life-rule-custom").addEventListener("change", () => {
      const raw = byId("life-rule-custom").value.trim();
      if (!raw) return;
      const norm = LC.normalizeRule(raw);
      byId("life-rule-custom").value = norm.label;   /* hostile input lands here */
      cfgFromUI();
    });
    byId("life-wrap").addEventListener("change", cfgFromUI);
    ["life-speed", "life-classify", "life-density"].forEach(bindSlider);

    byId("life-board-btn").addEventListener("click", () => {
      S.boardOpen = true; S.statsOpen = false;
      byId("life-stats-overlay").classList.remove("open");
      byId("life-board-overlay").classList.add("open");
      renderBoard();
    });
    byId("life-board-close").addEventListener("click", () => {
      S.boardOpen = false;
      byId("life-board-overlay").classList.remove("open");
    });
    byId("life-stats-btn").addEventListener("click", () => {
      S.statsOpen = true; S.boardOpen = false;
      byId("life-board-overlay").classList.remove("open");
      byId("life-stats-overlay").classList.add("open");
      renderStats();
    });
    byId("life-stats-close").addEventListener("click", () => {
      S.statsOpen = false;
      byId("life-stats-overlay").classList.remove("open");
    });

    /* painting */
    const canvasEl = byId("life-canvas");
    canvasEl.addEventListener("pointerdown", (e) => {
      S.painting = true;
      const { x, y } = cellFromEvent(e);
      S.paintValue = LC.getCell(S.world, x, y) ? 0 : 1;
      paintAt(e);
      canvasEl.setPointerCapture(e.pointerId);
    });
    canvasEl.addEventListener("pointermove", (e) => { if (S.painting) paintAt(e); });
    canvasEl.addEventListener("pointerup", () => { S.painting = false; });

    /* keyboard: space pauses, N steps, C clears, S soups */
    window.addEventListener("keydown", (e) => {
      if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
      if (e.key === " ") { S.running = !S.running; syncPlayBtn(); e.preventDefault(); }
      else if (e.key === "n" || e.key === "N") byId("life-step").click();
      else if (e.key === "c" || e.key === "C") byId("life-clear").click();
      else if (e.key === "s" || e.key === "S") byId("life-soup").click();
      else if (e.key === "Escape") {
        S.boardOpen = false; S.statsOpen = false;
        byId("life-board-overlay").classList.remove("open");
        byId("life-stats-overlay").classList.remove("open");
      }
    });

    const reduce = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduce) { S.running = false; syncPlayBtn(); }
    syncPlayBtn();
    render(); renderUnder();   /* paused boot draws its first frame, then idles */
    requestAnimationFrame((t) => { S.last = t; requestAnimationFrame(tick); });
  }

  build();
})();
