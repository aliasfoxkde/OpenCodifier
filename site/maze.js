/* Maze Solver — Fog of War (the try.html playground's playable demo).
 *
 * Shell over site/maze-core.js (pure logic, node-testable). This file owns
 * everything the DOM implies: DPR canvas, cyber-grid fog renderer, keyboard
 * + d-pad input, the rAF loop, mode lifecycle (bot / play / versus), the
 * session-only leaderboard, the drag-and-drop rubric editor, and the one
 * place the real WASM engine is consulted — junction tie-breaks between
 * equal-cost frontier choices. Everything else the bot does is the local
 * decision ladder; the trace panel says which rung decided every step.
 *
 * Honesty rules mirrored from the page copy: the leaderboard never persists
 * (page memory only), god-mode runs are marked assisted and excluded from
 * verdicts, and the engine never decides anything a cheaper rung already
 * decided — it only breaks exact ties.
 */
(function () {
  "use strict";

  const M = globalThis.MazeCore;
  const root = document.getElementById("maze-root");
  if (!M || !root) return;

  const REDUCED = window.matchMedia &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  /* ---------- tiny DOM helpers (no innerHTML with dynamic data) ---------- */

  function h(tag, attrs) {
    const node = document.createElement(tag);
    if (attrs) {
      for (const k of Object.keys(attrs)) {
        const v = attrs[k];
        if (v === null || v === undefined) continue;
        if (k === "class") node.className = v;
        else if (k === "text") node.textContent = v;
        else if (k === "html") node.innerHTML = v; /* static markup only */
        else if (k.slice(0, 2) === "on") node.addEventListener(k.slice(2), v);
        else node.setAttribute(k, v);
      }
    }
    for (let i = 2; i < arguments.length; i++) {
      const kid = arguments[i];
      if (kid === null || kid === undefined) continue;
      node.appendChild(typeof kid === "string" ? document.createTextNode(kid) : kid);
    }
    return node;
  }

  const fmtPct = (x) => Math.round(x * 100) + "%";
  const fmtTime = (s) => {
    const t = Math.max(0, Math.floor(s));
    return Math.floor(t / 60) + ":" + String(t % 60).padStart(2, "0");
  };

  /* ---------- flavor lines (deterministic pools, core picks per seed) ---------- */

  const FIGHT_LINES = [
    "You lock antlers with the denizen. It is mostly posture.",
    "A brief, dignified scuffle. Nobody wins; one of you leaves.",
    "The monster checks its watch. You check yours. Two seconds pass.",
    "Static combat: all of the drama, none of the hit points.",
  ];
  const AGGRO_LINES = [
    "Something heard you. It has opinions.",
    "Eyes in the dark. They are moving this way.",
    "The maze has a landlord, and you are late on rent.",
  ];
  const LOSE_LINES = [
    "The maze keeps the crown. Try another seed.",
    "Out of time — the exit ages another day behind its wall.",
  ];

  function pickLine(pool, rng) {
    return pool[Math.floor(rng() * pool.length)];
  }

  /* ---------- state ---------- */

  const S = {
    mode: "bot",            /* bot | play | versus */
    god: false,
    paused: false,
    maze: null,
    bot: null,              /* run state, runnerType "bot" */
    human: null,            /* run state, runnerType "human" */
    recorded: null,         /* Set of run states already on the board */
    rubric: JSON.parse(JSON.stringify(M.DEFAULT_RUBRIC)),
    rows: [],               /* leaderboard rows, newest first */
    trail: [],              /* ghost trail of {x, y} */
    trace: [],              /* recent bot decisions */
    shake: 0,
    escapeFlash: 0,
    humanCool: 0,
    heldDir: null,
    queuedDir: null,        /* a tap shorter than one frame still steps */
    engine: null,
    engineOn: false,
    engineBusy: false,
    engineFails: 0,
    audioOn: false,
    actx: null,
    msgRng: M.makeRng("shell-messages"),
    lastFrame: 0,
  };

  /* ---------- canvas palette (fixed dungeon theme — the maze screen stays
     dark in both site themes, like a terminal; DOM text uses site tokens) --- */

  const PAL = {
    bg: "#060d0c",          /* the void: never seen */
    floorLit: "#0e211f",    /* currently visible floor */
    floorMem: "#0a1716",    /* remembered floor */
    wallLit: "#134e4a",     /* visible wall face */
    wallMem: "#0e2a30",     /* remembered wall */
    grid: "rgba(94, 234, 212, 0.05)",
    ink: "#d8e6e0",
    teal: "#5eead4",
    you: "#67e8f9",
    exit: "#5eead4",
    loot: "#fbbf24",
    chest: "#f59e0b",
    monster: "#f87171",
  };

  /* ---------- audio (lazy, off by default) ---------- */

  function beep(freq, dur, type, gain) {
    if (!S.audioOn) return;
    try {
      if (!S.actx) S.actx = new (window.AudioContext || window.webkitAudioContext)();
      const t = S.actx.currentTime;
      const osc = S.actx.createOscillator();
      const vol = S.actx.createGain();
      osc.type = type || "square";
      osc.frequency.value = freq;
      vol.gain.setValueAtTime(0, t);
      vol.gain.linearRampToValueAtTime(gain || 0.04, t + 0.005);
      vol.gain.exponentialRampToValueAtTime(0.0001, t + dur);
      osc.connect(vol).connect(S.actx.destination);
      osc.start(t);
      osc.stop(t + dur + 0.02);
    } catch (err) { /* audio is a garnish; never let it break the run */ }
  }

  const SFX = {
    move: () => beep(220, 0.03, "square", 0.015),
    loot: () => { beep(660, 0.08, "triangle"); setTimeout(() => beep(880, 0.1, "triangle"), 70); },
    chest: () => { beep(520, 0.1, "triangle"); setTimeout(() => beep(780, 0.14, "triangle"), 90); },
    fight: () => { beep(110, 0.25, "sawtooth", 0.06); },
    aggro: () => beep(90, 0.18, "sawtooth", 0.05),
    escape: () => {
      beep(523, 0.12, "triangle");
      setTimeout(() => beep(659, 0.12, "triangle"), 110);
      setTimeout(() => beep(784, 0.2, "triangle"), 220);
    },
    over: () => { beep(200, 0.3, "sawtooth", 0.05); setTimeout(() => beep(140, 0.4, "sawtooth", 0.05), 180); },
  };

  /* ---------- engine (WASM tie-breaks only) ---------- */

  function ensureEngine() {
    if (S.engine || S.engineBusy) return Promise.resolve(S.engine);
    S.engineBusy = true;
    setEngineStatus("loading");
    return import("./wasm/opencodifier_wasm.js")
      .then((mod) => mod.default().then(() => mod))
      .then((mod) => {
        S.engine = new mod.WasmEngine();
        S.engineBusy = false;
        setEngineStatus("ready");
        return S.engine;
      })
      .catch(() => {
        S.engineBusy = false;
        S.engineOn = false;
        byId("mz-engine-on").checked = false;
        setEngineStatus("unavailable");
        return null;
      });
  }

  function setEngineStatus(kind) {
    const el = byId("mz-engine-status");
    if (!el) return;
    el.textContent = kind === "ready" ? "engine ready"
      : kind === "loading" ? "loading engine…"
      : kind === "unavailable" ? "engine unavailable — local ties"
      : "engine off — local ties";
    el.dataset.kind = kind;
  }

  /* The one engine call site: an exact tie between frontier first-steps
     becomes a real `choice` request over direction candidates. Any failure
     or abstain falls back to the ladder's own pick — the maze never stops. */
  function engineTiebreak(cands, decision, state) {
    if (!S.engineOn || !S.engine) return null;
    const here = state.pos;
    const name = (d) => d.name;
    try {
      const req = {
        state: {
          text: "Maze junction. The scout reached (" + here.x + "," + here.y +
            ") with " + decision.note + ". " +
            "Known corridors: " + frontierDigest(state) +
            ". Steps taken: " + state.steps + ".",
          facts: {},
        },
        questions: [{
          type: "choice",
          id: "tie",
          text: "Which way should the scout turn at this equal-cost junction?",
          candidates: cands.map((d) => ({
            id: name(d),
            description: DIR_DESC[d.name] + " — equal-cost frontier route",
          })),
        }],
        policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" },
        metadata: { request_id: "maze-tiebreak", limits: ENGINE_LIMITS },
      };
      const out = JSON.parse(S.engine.decide(JSON.stringify(req)));
      const ans = (out.answers && out.answers[0]) || (out.results && out.results[0]) || out;
      const id = ans && ans.choice;
      const conf = typeof (ans && ans.confidence) === "number" ? ans.confidence : null;
      const chosen = cands.find((d) => name(d) === String(id));
      if (!chosen) return null;
      pushTrace("engine.tiebreak", DIR_DESC[chosen.name] +
        (conf !== null ? " · " + Math.round(conf * 100) + "% conf" : " · engine"), true);
      return chosen;
    } catch (err) {
      S.engineFails++;
      if (S.engineFails > 5) { S.engineOn = false; setEngineStatus("unavailable"); }
      return null;
    }
  }

  const DIR_DESC = {
    N: "head north", E: "head east", S: "head south", W: "head west",
  };
  const ENGINE_LIMITS = {
    max_input_bytes: 1048576,
    max_questions: 32,
    max_candidates: 256,
    max_graph_nodes: 128,
    max_execution_time: { secs: 2, nanos: 0 },
    max_retrieval_results: 64,
  };

  function frontierDigest(state) {
    /* a compact, honest picture of what the scout can see */
    const knownFloors = countOf(state.known, M.KNOWN_FLOOR);
    const unknown = state.maze.w * state.maze.h - knownFloors - countOf(state.known, M.KNOWN_WALL);
    return knownFloors + " known floor cells, " + unknown + " unexplored";
  }

  function countOf(arr, val) {
    let n = 0;
    for (let i = 0; i < arr.length; i++) if (arr[i] === val) n++;
    return n;
  }

  function hooks() {
    return { tiebreak: engineTiebreak };
  }

  /* ---------- run lifecycle ---------- */

  function readConfig() {
    return {
      seed: byId("mz-seed").value.trim() || "opencodifier",
      cells: parseInt(byId("mz-cells").value, 10),
      braidPct: parseInt(byId("mz-braid").value, 10),
      vision: parseInt(byId("mz-vision").value, 10),
      botSpeed: parseInt(byId("mz-bot-speed").value, 10),
      timeLimitS: parseInt(byId("mz-timer").value, 10) || 0,
      lootEnabled: byId("mz-loot-on").checked,
      lootCount: parseInt(byId("mz-loot-count").value, 10),
      chestsEnabled: byId("mz-chest-on").checked,
      chestCount: parseInt(byId("mz-chest-count").value, 10),
      monstersEnabled: byId("mz-mon-on").checked,
      monsterCount: parseInt(byId("mz-mon-count").value, 10),
      monsterSpeed: parseInt(byId("mz-mon-speed").value, 10),
      aggroRange: parseInt(byId("mz-aggro").value, 10),
      wanderRandomness: parseInt(byId("mz-wander").value, 10),
    };
  }

  function newGame(keepSeed) {
    const cfg = readConfig();
    if (!keepSeed && !cfg.seed) byId("mz-seed").value = cfg.seed = randomSeed();
    S.maze = M.generateMaze(cfg);
    S.bot = M.createRunState(S.maze, cfg, "bot");
    S.human = M.createRunState(S.maze, cfg, "human");
    S.recorded = new Set();
    S.trail = [];
    S.trace = [];
    S.heldDir = null;
    S.queuedDir = null;
    S.humanCool = 0;
    S.shake = 0;
    S.escapeFlash = 0;
    S.paused = false;
    byId("mz-pause").textContent = "pause";
    drawBoardOnce();
    updateHud();
    renderTrace();
    byId("mz-verdict").textContent = "";
    byId("mz-verdict").className = "mz-verdict";
    log("New maze “" + cfg.seed + "” — " + S.maze.w + "×" + S.maze.h +
      ", optimal " + S.maze.optimalSteps + " steps.");
  }

  function randomSeed() {
    return "mz-" + Math.random().toString(36).slice(2, 8);
  }

  function activeRuns() {
    if (S.mode === "versus") return [S.bot, S.human];
    if (S.mode === "play") return [S.human];
    return [S.bot];
  }

  /* the run whose fog we draw */
  function viewRun() {
    if (S.mode === "bot") return S.bot;
    return S.human;
  }

  /* ---------- event handling per tick ---------- */

  function handleEvents(state, events) {
    for (const e of events) {
      if (e.type === "move") {
        if (state === S.bot) {
          S.trail.push({ x: e.to.x, y: e.to.y });
          if (S.trail.length > 30) S.trail.shift();
          if (e.rung !== "human") {
            pushTrace(e.rung, e.note);
            if (S.mode !== "play") byId("mz-rung").textContent = e.rung;
          }
        }
        if (state === S.human && S.mode !== "bot") SFX.move();
      } else if (e.type === "loot") {
        log("Loot secured (" + e.collected + "/" + totalLoot(state) + ").");
        if (state === viewRun()) SFX.loot();
      } else if (e.type === "chest-start") {
        log("Opening a chest…");
      } else if (e.type === "chest-open") {
        log("Chest: " + e.message);
        if (state === viewRun()) SFX.chest();
      } else if (e.type === "fight-start") {
        log(pickLine(FIGHT_LINES, S.msgRng));
        if (!REDUCED) S.shake = 1;
        if (state === viewRun()) SFX.fight();
      } else if (e.type === "aggro") {
        if (state === viewRun()) { log(pickLine(AGGRO_LINES, S.msgRng)); SFX.aggro(); }
      } else if (e.type === "fight-end") {
        log("The denizen slinks off. Fight " + e.fights + " settled.");
      } else if (e.type === "escaped") {
        log(state === S.human && S.mode !== "bot"
          ? "YOU found the exit in " + fmtTime(state.timeS) + "!"
          : "The bot found the exit in " + fmtTime(state.timeS) + ".");
        if (state === viewRun()) { S.escapeFlash = 1; SFX.escape(); }
        recordRun(state);
      } else if (e.type === "abstain") {
        log("The bot abstains — no honest route left. Refusing to guess is the point.");
        recordRun(state);
      } else if (e.type === "game-over") {
        log(pickLine(LOSE_LINES, S.msgRng));
        if (state === viewRun()) SFX.over();
        recordRun(state);
      }
    }
    updateHud();
    updateVerdict();
  }

  function totalLoot(state) {
    return state.config.lootEnabled ? state.loot.length : 0;
  }

  function recordRun(state) {
    if (!S.recorded) S.recorded = new Set();
    if (S.recorded.has(state)) return;
    S.recorded.add(state);
    const summary = M.runSummary(state);
    S.rows.unshift({
      summary,
      stamp: M.configStamp(state.config),
      botSpeed: state === S.bot ? state.config.botSpeed : null,
      score: M.scoreRun(summary, S.rubric),
    });
    if (S.rows.length > 30) S.rows.pop();
    renderBoard();
  }

  /* ---------- HUD ---------- */

  function byId(id) { return document.getElementById(id); }

  function updateHud() {
    if (!S.maze || !S.bot || !S.human) return; /* pre-newGame (boot order) */
    const view = viewRun();
    const coords = byId("mz-coords");
    if (coords) coords.textContent = "x " + view.pos.x + " · y " + view.pos.y;
    const st = byId("mz-status");
    if (st) {
      st.textContent = view.finished
        ? (view.escaped ? "escaped" : view.abstained ? "abstained" : "game over")
        : S.paused ? "paused"
        : view.fight ? "fighting " + view.fight.remaining.toFixed(1) + "s"
        : view.chest ? "opening chest " + view.chest.remaining.toFixed(1) + "s"
        : S.mode === "play" ? "exploring" : "bot: " + (view.lastDecision ? view.lastDecision.rung : "thinking");
      st.dataset.state = view.finished ? (view.escaped ? "good" : "bad") : "live";
    }
    const tl = byId("mz-time-left");
    if (tl) {
      if (view.config.timeLimitS > 0) {
        tl.textContent = "time left " + fmtTime(view.config.timeLimitS - view.timeS);
        tl.dataset.low = String(view.config.timeLimitS - view.timeS < 30);
      } else {
        tl.textContent = "elapsed " + fmtTime(view.timeS);
        tl.dataset.low = "false";
      }
    }
    /* scoreboard */
    const you = byId("mz-score-you");
    const bot = byId("mz-score-bot");
    if (S.mode === "versus") {
      you.textContent = "YOU " + M.liveScore(S.human, S.rubric);
      bot.textContent = "BOT " + M.liveScore(S.bot, S.rubric);
      you.className = "mz-score mz-you" + (S.human.finished ? " done" : "");
      bot.className = "mz-score mz-bot" + (S.bot.finished ? " done" : "");
      you.parentElement.dataset.active = S.human.finished ? "no" : "yes";
    } else {
      const active = viewRun();
      const label = S.mode === "bot" ? "BOT" : "YOU";
      you.textContent = label + " " + M.liveScore(active, S.rubric);
      bot.textContent = "versus mode: off";
      you.className = "mz-score mz-you";
      bot.className = "mz-score mz-bot muted";
    }
  }

  function updateVerdict() {
    const el = byId("mz-verdict");
    if (!el || S.mode !== "versus") return;
    const you = S.rows.find((r) => r.summary.runner === "human");
    const bot = S.rows.find((r) => r.summary.runner === "bot");
    if (!you || !bot || you.stamp !== bot.stamp || you.botSpeed !== bot.botSpeed) {
      if (you && bot && you.stamp !== bot.stamp) {
        el.textContent = "no verdict — runs used different configs";
        el.className = "mz-verdict warn";
      }
      return;
    }
    const v = M.resolveWinner([you.summary, bot.summary], S.rubric);
    if (v.winner === "none") { el.textContent = "no verdict — " + v.reason; }
    else if (v.winner === "tie") { el.textContent = "draw — " + v.reason; }
    else {
      const who = v.winner === "human" ? "YOU win" : "BOT wins";
      el.textContent = who + " — by " + v.by + (v.reason ? " (" + v.reason + ")" : "");
    }
    el.className = "mz-verdict show";
  }

  /* ---------- message log ---------- */

  function log(text) {
    const box = byId("mz-log");
    if (!box) return;
    const row = h("div", { class: "mz-log-line" }, text);
    box.insertBefore(row, box.firstChild);
    while (box.children.length > 8) box.removeChild(box.lastChild);
  }

  function pushTrace(rung, note, engine) {
    S.trace.unshift({ rung, note, engine: !!engine });
    if (S.trace.length > 14) S.trace.pop();
    renderTrace();
  }

  function renderTrace() {
    const box = byId("mz-trace");
    if (!box) return;
    box.textContent = "";
    for (const t of S.trace) {
      box.appendChild(h("div", { class: "mz-trace-line" },
        h("span", { class: "mz-chip mz-rung-chip" + (t.engine ? " engine" : "") }, t.rung),
        h("span", { class: "mz-trace-note" }, t.note)));
    }
    if (!S.trace.length) {
      box.appendChild(h("div", { class: "mz-trace-note muted" },
        "the bot's decision ladder will appear here"));
    }
  }

  /* ---------- leaderboard ---------- */

  function renderBoard() {
    const body = byId("mz-board-body");
    if (!body) return;
    body.textContent = "";
    for (const row of S.rows) {
      const s = row.summary;
      const badge = s.runner === "bot" ? "BOT" : "YOU";
      const result = s.escaped ? (s.assisted ? "escaped (god)" : "escaped")
        : s.abstained ? "abstained" : s.timedOut ? "time up" : "unfinished";
      body.appendChild(h("tr", { class: s.escaped && !s.assisted ? "" : "mz-dim" },
        h("td", null, h("span", { class: "mz-chip " + (s.runner === "bot" ? "mz-bot" : "mz-you") }, badge)),
        h("td", null, result),
        h("td", null, s.escaped ? fmtTime(s.timeS) : "—"),
        h("td", null, String(s.steps)),
        h("td", null, fmtPct(s.explored)),
        h("td", null, s.lootEnabled ? s.lootCollected + "/" + s.lootTotal : "0"),
        h("td", null, s.chestsEnabled ? String(s.coins) : "0"),
        h("td", null, s.chestsEnabled ? s.chestOpened + "/" + s.chestTotal : "0"),
        h("td", null, s.monstersEnabled || s.chestsEnabled ? String(s.fights) : "0"),
        h("td", { class: "mz-score-cell" }, String(row.score.score))));
    }
  }

  /* ---------- rubric editor ---------- */

  function renderPalette() {
    const pal = byId("mz-palette");
    if (!pal) return;
    pal.textContent = "";
    for (const tag of M.TAGS) {
      pal.appendChild(h("span", {
        class: "mz-chip mz-palette-chip",
        draggable: "true",
        title: "drag into the rubric (or click to append)",
        ondragstart: (e) => e.dataTransfer.setData("text/plain", tag),
        onclick: () => addCriterion(tag),
      }, tag));
    }
  }

  function addCriterion(tag) {
    if (S.rubric.criteria.some((c) => c.tag === tag)) return;
    S.rubric.criteria.push({ tag, weight: defaultWeight(tag) });
    renderRubric();
  }

  function defaultWeight(tag) {
    const def = M.DEFAULT_RUBRIC.criteria.find((c) => c.tag === tag);
    return def ? def.weight : 1;
  }

  function renderRubric() {
    const list = byId("mz-rubric-list");
    if (!list) return;
    list.textContent = "";
    for (const c of S.rubric.criteria) {
      list.appendChild(h("div", { class: "mz-rubric-row" },
        h("span", { class: "mz-chip" }, c.tag),
        h("span", { class: "mz-rubric-w" }, String(c.weight)),
        h("button", {
          class: "ctl ctl-small", type: "button", "aria-label": "decrease weight of " + c.tag,
          onclick: () => { c.weight = Math.max(-3, c.weight - 1); renderRubric(); },
        }, "−"),
        h("button", {
          class: "ctl ctl-small", type: "button", "aria-label": "increase weight of " + c.tag,
          onclick: () => { c.weight = Math.min(5, c.weight + 1); renderRubric(); },
        }, "+"),
        h("button", {
          class: "ctl ctl-small mz-remove", type: "button", "aria-label": "remove " + c.tag,
          onclick: () => {
            S.rubric.criteria = S.rubric.criteria.filter((x) => x.tag !== c.tag);
            renderRubric();
          },
        }, "×")));
    }
    if (!S.rubric.criteria.length) {
      list.appendChild(h("div", { class: "mz-trace-note muted" },
        "empty rubric — drop tags here"));
    }
    byId("mz-rubric-json").value = JSON.stringify(S.rubric, null, 2);
    renderBoard();
    updateHud();
  }

  function wireRubricEditor() {
    const drop = byId("mz-rubric-list");
    drop.addEventListener("dragover", (e) => { e.preventDefault(); drop.classList.add("mz-over"); });
    drop.addEventListener("dragleave", () => drop.classList.remove("mz-over"));
    drop.addEventListener("drop", (e) => {
      e.preventDefault();
      drop.classList.remove("mz-over");
      addCriterion(e.dataTransfer.getData("text/plain"));
    });
    byId("mz-rubric-apply").addEventListener("click", () => {
      try {
        S.rubric = M.normalizeRubric(JSON.parse(byId("mz-rubric-json").value));
        renderRubric();
        log("Rubric applied — scores rescore live.");
      } catch (err) {
        log("Rubric JSON rejected; sanitized instead. " + err.message);
        S.rubric = M.normalizeRubric(null);
        renderRubric();
      }
    });
    byId("mz-rubric-reset").addEventListener("click", () => {
      S.rubric = JSON.parse(JSON.stringify(M.DEFAULT_RUBRIC));
      renderRubric();
      log("Rubric reset to defaults.");
    });
  }

  /* ---------- rendering ---------- */

  const canvas = h("canvas", {
    id: "mz-canvas",
    role: "img",
    "aria-label": "Maze view: fog of war, loot, chests, monsters and runners",
  });
  const ctx = canvas.getContext("2d");

  function sizeCanvas() {
    const px = parseInt(byId("mz-size").value, 10);
    const dpr = window.devicePixelRatio || 1;
    canvas.style.width = px + "px";
    canvas.style.height = px + "px";
    canvas.width = Math.round(px * dpr);
    canvas.height = Math.round(px * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  }

  function drawBoardOnce() { /* placeholder so tests and first paint agree */
    if (!S.maze) return;
    render();
  }

  function render() {
    if (!S.maze) return;
    const px = canvas.clientWidth || parseInt(byId("mz-size").value, 10);
    const cell = px / S.maze.w;
    const view = viewRun();
    const god = S.god;

    ctx.save();
    if (S.shake > 0 && !REDUCED) {
      ctx.translate((Math.random() - 0.5) * 6 * S.shake, (Math.random() - 0.5) * 6 * S.shake);
      S.shake = Math.max(0, S.shake - 0.04);
    }

    ctx.fillStyle = PAL.bg;
    ctx.fillRect(0, 0, px, px);

    /* cells */
    for (let y = 0; y < S.maze.h; y++) {
      for (let x = 0; x < S.maze.w; x++) {
        const i = y * S.maze.w + x;
        const known = view.known[i];
        const wall = S.maze.grid[i] === M.WALL;
        if (god) {
          ctx.fillStyle = wall ? PAL.wallLit : PAL.floorLit;
        } else if (view.visible[i]) {
          ctx.fillStyle = wall ? PAL.wallLit : PAL.floorLit;
        } else if (known === M.KNOWN_FLOOR) {
          ctx.fillStyle = PAL.floorMem;
        } else if (known === M.KNOWN_WALL) {
          ctx.fillStyle = PAL.wallMem;
        } else {
          continue; /* unknown: leave the void */
        }
        ctx.fillRect(x * cell, y * cell, cell + 0.5, cell + 0.5);
      }
    }

    /* cyber-grid over remembered/visible floor */
    ctx.strokeStyle = PAL.grid;
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (let x = 0; x <= S.maze.w; x++) {
      ctx.moveTo(x * cell, 0);
      ctx.lineTo(x * cell, px);
    }
    for (let y = 0; y <= S.maze.h; y++) {
      ctx.moveTo(0, y * cell);
      ctx.lineTo(px, y * cell);
    }
    ctx.stroke();

    const visibleOrGod = (x, y) => {
      if (god) return true;
      const i = y * S.maze.w + x;
      return view.visible[i] === 1 || view.known[i] === M.KNOWN_FLOOR;
    };
    const isVisibleNow = (x, y) => god || view.visible[y * S.maze.w + x] === 1;

    /* exit (only once discovered, or god) */
    if (visibleOrGod(S.maze.exit.x, S.maze.exit.y)) {
      drawGlyph(S.maze.exit.x, S.maze.exit.y, cell, PAL.exit, "exit");
    }

    /* loot + chests (static — remembered position is truth) */
    for (const l of view.loot) {
      if (l.taken || !visibleOrGod(l.x, l.y)) continue;
      drawGlyph(l.x, l.y, cell, PAL.loot, "loot");
    }
    for (const c of view.chests) {
      if (c.opened || !visibleOrGod(c.x, c.y)) continue;
      drawGlyph(c.x, c.y, cell, PAL.chest, "chest");
    }

    /* monsters: only when currently visible — they move */
    for (const mo of view.monsters) {
      if (!isVisibleNow(mo.x, mo.y)) continue;
      drawMonster(mo, cell);
    }

    /* ghost trail + ghost (versus / bot mode) */
    if (S.mode !== "play") {
      ctx.save();
      for (let i = 0; i < S.trail.length; i++) {
        const t = S.trail[i];
        ctx.globalAlpha = 0.25 * (i / S.trail.length);
        ctx.fillStyle = PAL.teal;
        ctx.fillRect(t.x * cell + cell * 0.3, t.y * cell + cell * 0.3, cell * 0.4, cell * 0.4);
      }
      ctx.restore();
      if (S.mode === "versus" && !S.bot.finished) drawRunner(S.bot, cell, true);
    }

    /* the viewed runner */
    drawRunner(view, cell, false);

    /* fight + chest fx */
    if (view.fight) {
      const m = view.monsters.find((x) => x.id === view.fight.monsterId) || view.pos;
      const fx = m.x !== undefined ? m : view.pos;
      pulse(fx.x, fx.y, cell, REDUCED ? 0.4 : (0.5 + 0.5 * Math.sin(Date.now() / 60)), "#f87171");
      drawBubble(view.pos.x, view.pos.y, cell, " fight " + Math.ceil(view.fight.remaining) + "s ");
    }
    if (view.chest) {
      pulse(view.pos.x, view.pos.y, cell, REDUCED ? 0.3 : (0.4 + 0.4 * Math.sin(Date.now() / 90)), PAL.chest);
      drawBubble(view.pos.x, view.pos.y, cell, " opening… ");
    }

    /* danger vignette: an aggro monster within 3 cells of the viewed runner */
    if (!REDUCED && !view.finished) {
      let danger = 0;
      for (const mo of view.monsters) {
        if (!mo.aggro) continue;
        const d = Math.max(Math.abs(mo.x - view.pos.x), Math.abs(mo.y - view.pos.y));
        if (d <= 3) danger = Math.max(danger, 1 - d / 4);
      }
      if (danger > 0) {
        const g = ctx.createRadialGradient(px / 2, px / 2, px * 0.3, px / 2, px / 2, px * 0.72);
        g.addColorStop(0, "rgba(248,113,113,0)");
        g.addColorStop(1, "rgba(248,113,113," + (0.16 * danger * (0.6 + 0.4 * Math.sin(Date.now() / 300))).toFixed(3) + ")");
        ctx.fillStyle = g;
        ctx.fillRect(0, 0, px, px);
      }
    }

    /* escape flash */
    if (S.escapeFlash > 0) {
      ctx.fillStyle = "rgba(94,234,212," + (0.22 * S.escapeFlash).toFixed(3) + ")";
      ctx.fillRect(0, 0, px, px);
      S.escapeFlash = Math.max(0, S.escapeFlash - 0.02);
    }

    /* paused veil */
    if (S.paused) {
      ctx.fillStyle = "rgba(6,13,12,0.55)";
      ctx.fillRect(0, 0, px, px);
      ctx.fillStyle = PAL.ink;
      ctx.font = "600 " + Math.max(16, px / 22) + "px system-ui, sans-serif";
      ctx.textAlign = "center";
      ctx.fillText("paused — space resumes", px / 2, px / 2);
    }

    /* god badge */
    if (god) {
      ctx.fillStyle = "rgba(248,113,113,0.85)";
      ctx.font = "700 " + Math.max(10, px / 46) + "px system-ui, sans-serif";
      ctx.textAlign = "left";
      ctx.fillText("GOD VIEW — runs while this is on score nothing", 8, px - 8);
    }

    ctx.restore();
  }


  function cellCenter(x, y, cell) {
    return [(x + 0.5) * cell, (y + 0.5) * cell];
  }

  function drawGlyph(x, y, cell, color, kind) {
    const [cx, cy] = cellCenter(x, y, cell);
    ctx.fillStyle = color;
    if (kind === "exit") {
      const r = cell * 0.38;
      ctx.strokeStyle = color;
      ctx.lineWidth = Math.max(1.5, cell * 0.12);
      ctx.strokeRect(cx - r, cy - r, r * 2, r * 2);
      ctx.beginPath();
      ctx.arc(cx, cy, r * 0.4, 0, Math.PI * 2);
      ctx.fill();
    } else if (kind === "loot") {
      ctx.beginPath();
      ctx.moveTo(cx, cy - cell * 0.3);
      ctx.lineTo(cx + cell * 0.24, cy);
      ctx.lineTo(cx, cy + cell * 0.3);
      ctx.lineTo(cx - cell * 0.24, cy);
      ctx.closePath();
      ctx.fill();
    } else {
      const w = cell * 0.5;
      ctx.fillRect(cx - w / 2, cy - w * 0.32, w, w * 0.64);
      ctx.strokeStyle = "rgba(6,13,12,0.9)";
      ctx.lineWidth = 1;
      ctx.strokeRect(cx - w / 2, cy - w * 0.05, w, 1.5);
    }
  }

  function drawMonster(mo, cell) {
    const [cx, cy] = cellCenter(mo.x, mo.y, cell);
    const r = cell * 0.32;
    if (mo.aggro && !REDUCED) {
      ctx.fillStyle = "rgba(248,113,113," + (0.2 + 0.15 * Math.sin(Date.now() / 140)).toFixed(3) + ")";
      ctx.beginPath();
      ctx.arc(cx, cy, r * 1.9, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.fillStyle = PAL.monster;
    ctx.beginPath();
    ctx.arc(cx, cy, r, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillStyle = "#060d0c";
    ctx.beginPath();
    ctx.arc(cx - r * 0.35, cy - r * 0.15, r * 0.18, 0, Math.PI * 2);
    ctx.arc(cx + r * 0.35, cy - r * 0.15, r * 0.18, 0, Math.PI * 2);
    ctx.fill();
  }

  function drawRunner(state, cell, ghost) {
    const [cx, cy] = cellCenter(state.pos.x, state.pos.y, cell);
    const r = cell * 0.34;
    ctx.save();
    if (ghost) {
      ctx.globalAlpha = 0.5;
      ctx.shadowColor = PAL.teal;
      ctx.shadowBlur = REDUCED ? 0 : 12;
      ctx.fillStyle = PAL.teal;
    } else {
      ctx.shadowColor = PAL.you;
      ctx.shadowBlur = REDUCED ? 0 : 8;
      ctx.fillStyle = PAL.you;
    }
    ctx.beginPath();
    ctx.arc(cx, cy, r, 0, Math.PI * 2);
    ctx.fill();
    /* facing tick */
    ctx.shadowBlur = 0;
    ctx.strokeStyle = "#060d0c";
    ctx.lineWidth = Math.max(1, cell * 0.08);
    ctx.beginPath();
    ctx.moveTo(cx, cy);
    const d = state.lastDecision && state.lastDecision.dir;
    const dx = d ? d.dx : 0;
    const dy = d ? d.dy : 1;
    ctx.lineTo(cx + dx * r * 1.4, cy + dy * r * 1.4);
    ctx.stroke();
    if (!ghost) {
      ctx.fillStyle = "#060d0c";
      ctx.beginPath();
      ctx.arc(cx - r * 0.3, cy - r * 0.2, r * 0.16, 0, Math.PI * 2);
      ctx.arc(cx + r * 0.3, cy - r * 0.2, r * 0.16, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.restore();
  }

  function pulse(x, y, cell, alpha, color) {
    const [cx, cy] = cellCenter(x, y, cell);
    ctx.strokeStyle = color;
    ctx.globalAlpha = alpha;
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.arc(cx, cy, cell * (0.5 + 0.35 * alpha), 0, Math.PI * 2);
    ctx.stroke();
    ctx.globalAlpha = 1;
  }

  function drawBubble(x, y, cell, text) {
    const [cx, cy] = cellCenter(x, y, cell);
    ctx.font = "600 " + Math.max(10, cell * 0.9) + "px system-ui, sans-serif";
    ctx.textAlign = "center";
    const w = ctx.measureText(text).width + 10;
    ctx.fillStyle = "rgba(6,13,12,0.85)";
    ctx.fillRect(cx - w / 2, cy - cell * 1.4, w, cell * 0.9);
    ctx.fillStyle = PAL.ink;
    ctx.fillText(text, cx, cy - cell * 0.75);
  }

  /* ---------- main loop ---------- */

  function frame(t) {
    const dt = Math.min(0.1, Math.max(0, (t - S.lastFrame) / 1000));
    S.lastFrame = t;
    if (!S.paused && S.maze) {
      const hk = hooks();
      for (const run of activeRuns()) {
        if (run.finished) continue;
        let humanMove = null;
        if (run === S.human && S.mode !== "bot") {
          S.humanCool -= dt;
          const want = S.queuedDir
            || (S.heldDir && S.humanCool <= 0 ? S.heldDir : null);
          if (want && !run.fight && !run.chest) {
            humanMove = want;
            S.humanCool = 1 / 12;
          }
          S.queuedDir = null; /* taps fire once; holds re-arm via key repeat */
        }
        const evs = M.tickRun(run, dt, humanMove, hk);
        if (evs.length) handleEvents(run, evs);
      }
    }
    render();
    requestAnimationFrame(frame);
  }

  /* ---------- input ---------- */

  const KEY_DIRS = {
    ArrowUp: "N", KeyW: "N",
    ArrowRight: "E", KeyD: "E",
    ArrowDown: "S", KeyS: "S",
    ArrowLeft: "W", KeyA: "W",
  };

  function dirByName(name) {
    return M.DIRS.find((d) => d.name === name) || null;
  }

  /* letter keys arrive as e.code ("KeyW"); some keyboards/IMEs only carry
     e.key ("w") — accept both, arrows share their name across both */
  const KEY_ALIASES = { w: "KeyW", a: "KeyA", s: "KeyS", d: "KeyD" };

  function dirNameFromEvent(e) {
    return KEY_DIRS[e.code] || KEY_DIRS[KEY_ALIASES[e.key]] || KEY_DIRS[e.key] || null;
  }

  function onKeyDown(e) {
    const tag = (e.target && e.target.tagName) || "";
    if (/^(INPUT|TEXTAREA|SELECT)$/.test(tag)) return;
    const dirName = dirNameFromEvent(e);
    if (e.code === "Space" && tag === "BUTTON") return; /* let the button have it */
    if (dirName) {
      e.preventDefault();
      if (S.mode === "bot") return;
      const dir = dirByName(dirName);
      S.heldDir = dir;
      /* queue the step so a tap shorter than one frame is not dropped —
         keyup clears the hold, never the queue */
      S.queuedDir = dir;
      S.humanCool = 0;
      return;
    }
    if (e.code === "Space" || e.key === " ") {
      e.preventDefault();
      togglePause();
    } else if (e.code === "KeyG") {
      toggleGod();
    } else if (e.code === "KeyR") {
      newGame(true);
    } else if (e.code === "KeyB") {
      setMode("bot");
    }
  }

  function onKeyUp(e) {
    const dirName = dirNameFromEvent(e);
    if (dirName && S.heldDir && S.heldDir.name === dirName) {
      S.heldDir = null;
    }
  }

  function togglePause() {
    if (!S.maze) return;
    S.paused = !S.paused;
    byId("mz-pause").textContent = S.paused ? "resume" : "pause";
    updateHud();
  }

  function toggleGod() {
    S.god = !S.god;
    byId("mz-god").setAttribute("aria-pressed", String(S.god));
    byId("mz-god").classList.toggle("on", S.god);
    /* honest scoring: a human run finished after peeking is marked assisted */
    if (S.god && S.human && !S.human.finished) {
      S.human.assisted = true;
      log("God view on — your current run will be marked assisted.");
    }
  }

  function setMode(mode) {
    S.mode = mode;
    for (const b of ["bot", "play", "versus"]) {
      byId("mz-mode-" + b).classList.toggle("on", b === mode);
      byId("mz-mode-" + b).setAttribute("aria-pressed", String(b === mode));
    }
    /* mode changes restart both runs on the same maze: clean clocks, clean
       comparison — finished runs stay on the board with their old stamp */
    if (S.maze) {
      const cfg = readConfig();
      S.bot = M.createRunState(S.maze, cfg, "bot");
      S.human = M.createRunState(S.maze, cfg, "human");
      S.recorded = new Set();
      S.trail = [];
      S.trace = [];
      S.heldDir = null;
      S.queuedDir = null;
      S.paused = false;
      byId("mz-pause").textContent = "pause";
      if (mode === "play") log("Your run — WASD or arrows. The bot waits its turn.");
      if (mode === "versus") log("Versus — race the ghost. It sees only its own fog.");
      if (mode === "bot") log("Spectating the bot. You see exactly what it knows.");
      updateHud();
    }
  }

  /* ---------- UI construction ---------- */

  function buildUI() {
    const wrap = h("div", { class: "mz" });

    /* scoreboard bar */
    const scorebar = h("div", { class: "mz-scorebar", id: "mz-scorebar" },
      h("span", { class: "mz-score mz-you", id: "mz-score-you" }, "YOU 0"),
      h("span", { class: "mz-verdict", id: "mz-verdict" }),
      h("span", { class: "mz-score mz-bot muted", id: "mz-score-bot" }, "versus mode: off"));

    /* canvas column */
    const stage = h("div", { class: "mz-stage" }, canvas,
      h("div", { class: "mz-under" },
        h("span", { id: "mz-coords", class: "mz-kv" }, "x 0 · y 0"),
        h("span", { id: "mz-status", class: "mz-kv", "data-state": "live" }, "idle"),
        h("span", { id: "mz-time-left", class: "mz-kv" }, "elapsed 0:00"),
        h("span", { id: "mz-rung", class: "mz-chip mz-rung-chip" }, "—")));

    /* d-pad (touch) */
    const dpad = h("div", { class: "mz-dpad", "aria-label": "movement pad" },
      h("button", { class: "ctl", "data-dir": "N", type: "button", "aria-label": "move north" }, "▲"),
      h("button", { class: "ctl", "data-dir": "W", type: "button", "aria-label": "move west" }, "◀"),
      h("button", { class: "ctl", "data-dir": "S", type: "button", "aria-label": "move south" }, "▼"),
      h("button", { class: "ctl", "data-dir": "E", type: "button", "aria-label": "move east" }, "▶"));

    /* ---- control panel ---- */
    const panel = h("div", { class: "mz-panel" });

    /* modes + core actions */
    panel.appendChild(h("div", { class: "mz-group" },
      h("div", { class: "mz-group-title" }, "run"),
      h("div", { class: "mz-btnrow" },
        h("button", { id: "mz-mode-bot", class: "ctl on", type: "button" }, "watch bot"),
        h("button", { id: "mz-mode-play", class: "ctl", type: "button" }, "play"),
        h("button", { id: "mz-mode-versus", class: "ctl", type: "button" }, "versus")),
      h("div", { class: "mz-btnrow" },
        h("button", { id: "mz-generate", class: "ctl", type: "button" }, "generate map"),
        h("button", { id: "mz-restart", class: "ctl", type: "button" }, "restart (R)"),
        h("button", { id: "mz-pause", class: "ctl", type: "button" }, "pause"),
        h("button", { id: "mz-god", class: "ctl", type: "button", "aria-pressed": "false" }, "god (G)")),
      h("label", { class: "mz-field" }, "seed ",
        h("input", { id: "mz-seed", type: "text", value: "opencodifier", spellcheck: "false" })),
      h("label", { class: "mz-field" }, "preset ",
        h("select", { id: "mz-preset" },
          h("option", { value: "" }, "— pick a difficulty —"),
          h("option", { value: "cozy" }, "cozy — small, safe"),
          h("option", { value: "standard" }, "standard — the demo default"),
          h("option", { value: "hard" }, "hard — 5 hunters, 5 min"),
          h("option", { value: "night" }, "maze night — 25×25, 4 min")))));

    /* world knobs */
    panel.appendChild(h("div", { class: "mz-group" },
      h("div", { class: "mz-group-title" }, "world"),
      rangeField("mz-vision", "line of sight", 5, 15, 9, 1, " grids"),
      rangeField("mz-cells", "maze size", 9, 29, 17, 2, " cells"),
      rangeField("mz-braid", "loopiness (braiding)", 0, 30, 10, 5, "%"),
      rangeField("mz-bot-speed", "bot speed", 2, 60, 14, 1, " steps/s"),
      numberField("mz-timer", "time limit (s, 0 = off)", 0, 3600, 0),
      h("label", { class: "mz-field" }, "canvas ",
        h("select", { id: "mz-size" },
          h("option", { value: "480" }, "480 × 480"),
          h("option", { value: "600", selected: "selected" }, "600 × 600"),
          h("option", { value: "720" }, "720 × 720")))));

    /* objectives */
    panel.appendChild(h("div", { class: "mz-group" },
      h("div", { class: "mz-group-title" }, "objectives"),
      checkField("mz-loot-on", "loot", true),
      rangeField("mz-loot-count", "loot piles", 0, 16, 8, 1, ""),
      checkField("mz-chest-on", "chests (1 s to open: coins 0–5 or a fight)", true),
      rangeField("mz-chest-count", "chests", 0, 8, 3, 1, ""),
      checkField("mz-mon-on", "monsters (2 s static fight on contact)", true),
      rangeField("mz-mon-count", "monsters", 0, 10, 3, 1, ""),
      rangeField("mz-mon-speed", "monster speed", 1, 5, 2, 1, " cells/s"),
      rangeField("mz-aggro", "aggro range", 2, 10, 5, 1, " cells"),
      rangeField("mz-wander", "wander randomness", 0, 100, 40, 10, "%")));

    /* engine */
    panel.appendChild(h("div", { class: "mz-group" },
      h("div", { class: "mz-group-title" }, "engine"),
      h("label", { class: "mz-check" },
        h("input", { id: "mz-engine-on", type: "checkbox" }),
        h("span", null, "WASM engine breaks junction ties"),
        h("span", { id: "mz-engine-status", class: "mz-engine-status", "data-kind": "off" }, "engine off — local ties")),
      h("div", { class: "mz-trace", id: "mz-trace", "aria-live": "polite" })));

    /* log */
    panel.appendChild(h("div", { class: "mz-group" },
      h("div", { class: "mz-group-title" }, "field notes"),
      h("div", { id: "mz-log", class: "mz-log", "aria-live": "polite" })));

    /* rubric editor */
    panel.appendChild(h("div", { class: "mz-group" },
      h("div", { class: "mz-group-title" }, "scoring rubric (drag or click)"),
      h("div", { id: "mz-palette", class: "mz-palette" }),
      h("div", { id: "mz-rubric-list", class: "mz-rubric-list" }),
      h("textarea", { id: "mz-rubric-json", class: "mz-json", rows: "7", spellcheck: "false",
        "aria-label": "scoring rubric as JSON" }),
      h("div", { class: "mz-btnrow" },
        h("button", { id: "mz-rubric-apply", class: "ctl", type: "button" }, "apply JSON"),
        h("button", { id: "mz-rubric-reset", class: "ctl", type: "button" }, "reset")),
      h("p", { class: "mz-note" },
        "score = round(100 · Σ weightᵢ · normᵢ / Σ |weightᵢ|); norms are per-map pars; ",
        "negative weights are penalty axes (fights is a badness norm: f/(1+f)).")));

    /* leaderboard */
    panel.appendChild(h("div", { class: "mz-group" },
      h("div", { class: "mz-group-title" }, "leaderboard (this session only)"),
      h("table", { class: "mz-board" },
        h("thead", null, h("tr", null,
          h("th", null, "who"), h("th", null, "result"), h("th", null, "time"),
          h("th", null, "steps"), h("th", null, "explored"), h("th", null, "loot"),
          h("th", null, "coins"), h("th", null, "chests"), h("th", null, "fights"),
          h("th", null, "score"))),
        h("tbody", { id: "mz-board-body" })),
      h("p", { class: "mz-note" }, "god-view runs score nothing. Disabled objectives report 0."),
      h("label", { class: "mz-check" },
        h("input", { id: "mz-audio", type: "checkbox" }),
        h("span", null, "sound effects (off by default)"))));

    wrap.appendChild(scorebar);
    const main = h("div", { class: "mz-main" }, stage, panel);
    wrap.appendChild(main);
    wrap.appendChild(dpad);
    return wrap;
  }

  function rangeField(id, label, min, max, value, step, suffix) {
    return h("label", { class: "mz-field" }, label + " ",
      h("input", { id, type: "range", min: String(min), max: String(max), value: String(value), step: String(step) }),
      h("output", { "data-for": id }, value + suffix));
  }

  function numberField(id, label, min, max, value) {
    return h("label", { class: "mz-field" }, label + " ",
      h("input", { id, type: "number", min: String(min), max: String(max), value: String(value) }));
  }

  function checkField(id, label, checked) {
    return h("label", { class: "mz-check" },
      h("input", { id, type: "checkbox", checked: checked ? "checked" : null }),
      h("span", null, label));
  }

  /* ---------- wiring ---------- */

  function wire() {
    byId("mz-generate").addEventListener("click", () => newGame(false));
    byId("mz-restart").addEventListener("click", () => newGame(true));
    byId("mz-pause").addEventListener("click", togglePause);
    byId("mz-god").addEventListener("click", toggleGod);
    for (const m of ["bot", "play", "versus"]) {
      byId("mz-mode-" + m).addEventListener("click", () => setMode(m));
    }
    byId("mz-size").addEventListener("change", () => { sizeCanvas(); render(); });
    byId("mz-audio").addEventListener("change", (e) => {
      S.audioOn = e.target.checked;
      if (S.audioOn) beep(440, 0.08, "triangle", 0.03);
    });
    byId("mz-engine-on").addEventListener("change", (e) => {
      S.engineOn = e.target.checked;
      if (S.engineOn) ensureEngine();
    });

    /* live knobs: vision applies to both runs mid-flight (it is in the fair
       stamp); bot speed is the bot's playback clock */
    byId("mz-vision").addEventListener("input", () => {
      const v = parseInt(byId("mz-vision").value, 10);
      for (const run of [S.bot, S.human]) if (run) run.config.vision = v;
      syncOutputs();
      if (S.maze) { for (const run of [S.bot, S.human]) M.updateVision(run); }
    });
    byId("mz-bot-speed").addEventListener("input", () => {
      if (S.bot) S.bot.config.botSpeed = parseInt(byId("mz-bot-speed").value, 10);
      syncOutputs();
    });

    wireRubricEditor();

    /* d-pad: pointer events so touch + mouse both work */
    for (const btn of root.querySelectorAll(".mz-dpad .ctl")) {
      const dir = dirByName(btn.dataset.dir);
      btn.addEventListener("pointerdown", (e) => {
        e.preventDefault();
        if (S.mode === "bot") setMode("versus");
        S.heldDir = dir;
        S.humanCool = 0;
      });
      const stop = () => { if (S.heldDir === dir) S.heldDir = null; };
      btn.addEventListener("pointerup", stop);
      btn.addEventListener("pointerleave", stop);
      btn.addEventListener("pointercancel", stop);
    }

    /* presets */
    const PRESETS = {
      cozy: { cells: 13, braidPct: 10, vision: 11, botSpeed: 12, timeLimitS: 0,
        lootEnabled: true, lootCount: 6, chestsEnabled: true, chestCount: 2,
        monstersEnabled: false, monsterCount: 0 },
      standard: { cells: 17, braidPct: 10, vision: 9, botSpeed: 14, timeLimitS: 0,
        lootEnabled: true, lootCount: 8, chestsEnabled: true, chestCount: 3,
        monstersEnabled: true, monsterCount: 3, monsterSpeed: 2, aggroRange: 5, wanderRandomness: 40 },
      hard: { cells: 21, braidPct: 15, vision: 9, botSpeed: 16, timeLimitS: 300,
        lootEnabled: true, lootCount: 10, chestsEnabled: true, chestCount: 4,
        monstersEnabled: true, monsterCount: 5, monsterSpeed: 3, aggroRange: 6, wanderRandomness: 50 },
      night: { cells: 25, braidPct: 20, vision: 7, botSpeed: 18, timeLimitS: 240,
        lootEnabled: true, lootCount: 12, chestsEnabled: true, chestCount: 5,
        monstersEnabled: true, monsterCount: 7, monsterSpeed: 3, aggroRange: 7, wanderRandomness: 60 },
    };
    byId("mz-preset").addEventListener("change", (e) => {
      const p = PRESETS[e.target.value];
      if (!p) return;
      applyControls(p);
      newGame(true);
      log("Preset applied — same seed, new parameters.");
    });

    function applyControls(p) {
      const set = (id, v) => { byId(id).value = String(v); };
      if (p.cells !== undefined) set("mz-cells", p.cells);
      if (p.braidPct !== undefined) set("mz-braid", p.braidPct);
      if (p.vision !== undefined) set("mz-vision", p.vision);
      if (p.botSpeed !== undefined) set("mz-bot-speed", p.botSpeed);
      if (p.timeLimitS !== undefined) set("mz-timer", p.timeLimitS);
      if (p.lootEnabled !== undefined) byId("mz-loot-on").checked = p.lootEnabled;
      if (p.lootCount !== undefined) set("mz-loot-count", p.lootCount);
      if (p.chestsEnabled !== undefined) byId("mz-chest-on").checked = p.chestsEnabled;
      if (p.chestCount !== undefined) set("mz-chest-count", p.chestCount);
      if (p.monstersEnabled !== undefined) byId("mz-mon-on").checked = p.monstersEnabled;
      if (p.monsterCount !== undefined) set("mz-mon-count", p.monsterCount);
      if (p.monsterSpeed !== undefined) set("mz-mon-speed", p.monsterSpeed);
      if (p.aggroRange !== undefined) set("mz-aggro", p.aggroRange);
      if (p.wanderRandomness !== undefined) set("mz-wander", p.wanderRandomness);
      syncOutputs();
    }

    function syncOutputs() {
      for (const out of root.querySelectorAll("output[data-for]")) {
        const inp = byId(out.dataset.for);
        const suffix = (out.textContent.match(/[^\d\-+]+$/) || [""])[0];
        out.textContent = inp.value + suffix;
      }
    }
    for (const inp of root.querySelectorAll(".mz-field input[type=range]")) {
      inp.addEventListener("input", syncOutputs);
    }

    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("resize", () => { sizeCanvas(); });

    /* deep link: ?maze=<seed> (and optional &mode=bot|play|versus) */
    try {
      const q = new URLSearchParams(window.location.search);
      const seed = q.get("maze");
      if (seed) byId("mz-seed").value = seed;
      const mode = q.get("mode");
      if (mode && ["bot", "play", "versus"].includes(mode)) setMode(mode);
    } catch (err) { /* query parsing is best-effort */ }

    /* theme can flip at runtime (site toggle); re-read periodically, cheap */
  }

  /* ---------- boot ---------- */

  root.appendChild(buildUI());
  sizeCanvas();
  wire();
  renderPalette();
  renderRubric();
  renderTrace();
  newGame(false);
  requestAnimationFrame((t) => { S.lastFrame = t; requestAnimationFrame(frame); });
})();
