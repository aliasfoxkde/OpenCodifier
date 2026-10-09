/* MicroWorld shell — the flagship colony sim on the Decisions SDK.
   microworld-core.js owns the deterministic world; this file is chrome:
   canvas, controls, the follow-one-creature trace, the session board
   (auto-recorded every 60 sim-seconds), the stats overlay and the engine
   bridge. The CREATURES are the decision agents (ten scored actions,
   near-ties escalated to the engine under the colony budget); the fox,
   births, deaths and stores are rules. Boots paused — the play button
   starts the colony. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const MW = globalThis.MicroWorld;
  const root = document.getElementById("microworld-root");
  if (!root || !DK || !MW) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;
  const W = MW.GW * MW.CELL, H = MW.GH * MW.CELL;
  const DT = 1 / MW.TICKS_PER_S;

  const ACTION_COLOR = {
    eat: "#34d399", drink: "#22d3ee", flee: "#ff5a4d", fight: "#fbbf24",
    explore: "#e2e8f0", rest: "#64748b", gather: "#a3e635", gohome: "#5eead4",
    follow: "#f472b6", ignore: "#475569",
  };

  /* canvas stays dark in both site themes, like the other games */
  const PAL = {
    bg: "#0a0f0d", grass: "#13291a", rich: "#265337", water: "#16324f",
    waterEdge: "#1d4266", home: "#0f3d33", homeEdge: "#5eead4",
    ink: "#e8f2ec", dim: "#7f9a8e", fox: "#ff2d2d",
    follow: "#fde047", carrying: "#f8fafc",
    legendBg: "rgba(6, 12, 9, 0.7)", legendEdge: "rgba(255, 255, 255, 0.09)",
  };
  const BOOT_HINT = "the colony is paused — every creature is waiting to think.";
  const WATCH_HINT = "watch for: the fox cornering a loner, a pack driving it off, herds forming around the stores.";

  /* ---------- state ---------- */
  const S = {
    cfg: { seed: "oc-microworld" },
    world: null,
    running: false,   /* boots paused — the play button starts the colony */
    last: 0, acc: 0, speed: 1,
    follow: null,                     /* creature id clicked in the canvas */
    pushedSeconds: 0,
    boardOpen: false, statsOpen: false,
    board: DK.sessionBoard(),
    meters: DK.meters(),
    fps: 0,
  };

  /* no importBase: the SDK's default "../wasm/" is right for a root page */
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
    return {
      seed: S.seedKit ? S.seedKit.get() : "oc-microworld",
      creatures: +byId("mw-pop").value,
      regrow: +byId("mw-regrow").value,
      budget: +byId("mw-budget").value,
    };
  }

  function newWorld() {
    S.cfg = cfgFromUI();
    S.world = MW.createWorld(S.cfg);
    S.follow = null;
    S.pushedSeconds = 0;
    render();
    trace("no decisions yet — press play, or click a creature to follow it");
    syncVeil();
    syncStatus();
  }

  function setChip() {
    const chip = byId("mw-engine-chip");
    if (!chip) return;
    const st = engine.status;
    chip.textContent = "engine: " + st;
    chip.classList.toggle("on", st === "ready");
    chip.classList.toggle("off", st === "unavailable" || st === "off");
  }

  function trace(text) {
    const el = byId("mw-trace");
    if (el) el.textContent = text;
  }

  function traceEvent(ev) {
    const lines = {
      birth: "🐣 birth #" + ev.id + " — the colony spent " + MW.BIRTH_COST + " stores",
      kill: "💀 creature #" + ev.id + " taken by the fox",
      starvation: "💀 creature #" + ev.id + " starved",
      thirst: "💀 creature #" + ev.id + " died of thirst",
      drive: "🛡️ the pack drove the fox off (creature #" + ev.id + ")",
      deposit: "📦 creature #" + ev.id + " deposited food — stores " + S.world.stores,
    };
    if (lines[ev.kind]) trace(lines[ev.kind]);
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
    input.addEventListener("change", () => newWorld());
    return h("label", { class: "game-slider" }, h("span", { class: "game-slider-label" }, label), input, out);
  }

  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) S.cfg.seed = deepSeed;

    const bar = h("div", { class: "game-bar" },
      h("button", { class: "mz-chip mz-play", id: "mw-play", type: "button" }, "▶ play"),
      h("button", { class: "mz-chip", id: "mw-restart", type: "button" }, "↺ restart"),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "mw-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "mw-seed", class: "mz-input", value: S.cfg.seed, size: "10" }),
        h("button", { class: "mz-chip", id: "mw-dice", type: "button", title: "new random seed" }, "🎲")),
      h("button", { class: "mz-chip", id: "mw-speed", type: "button" }, "speed: 1×"),
      h("button", { class: "mz-chip", id: "mw-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "mw-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "mw-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "mw-stats-btn", type: "button" }, "ⓘ stats"),
      h("button", { class: "mz-chip", id: "mw-reset-params", type: "button" }, "reset params"),
      h("button", { class: "mz-chip", id: "mw-fs", type: "button" }, "⛶ full screen"),
    );

    const canvas = h("canvas", { id: "mw-canvas", width: String(W), height: String(H),
      role: "img",
      "aria-label": "MicroWorld — a colony of creatures that each re-score ten actions about three-quarters of a second: eat, drink, flee, fight, explore, rest, gather, go home, follow, ignore. Genuine near-ties go to the OpenCodifier engine over just the tied candidates, under a colony-wide budget. The fox, births, deaths and stores are rules. Boots paused; the play button starts the colony." });
    const veil = h("div", { class: "game-veil", id: "mw-veil", role: "group",
      "aria-label": "Simulation paused" },
      h("button", { class: "game-veil-chip", type: "button", id: "mw-veil-chip" }, "▶ press play"),
      h("span", { class: "game-veil-hint", id: "mw-veil-hint" }, BOOT_HINT));
    const hint = h("p", { class: "game-hintline mw-hint" },
      WATCH_HINT + " Click a creature to read its decision table.");
    const status = h("div", { class: "game-hintline mw-status", id: "mw-status",
      "aria-live": "polite" }, "creatures: 0 · decisions: 0 · ticks: 0 · stores: 0");
    const under = h("div", { class: "mz-under game-under", id: "mw-under" });
    const traceEl = h("div", { class: "game-hintline", id: "mw-trace",
      "aria-live": "polite" }, "no decisions yet — press play, or click a creature to follow it");

    const opts = DK.shell.accordion([
      {
        id: "world", label: "The world", open: true,
        kids: [
          slider("mw-pop", "starting creatures", 8, 100, 2, 34),
          slider("mw-regrow", "grass regrowth", 0, 2, 0.25, 1, "×"),
          slider("mw-budget", "engine budget", 0, 6, 1, 2, "/s"),
          h("p", { class: "game-note" },
            "Changing anything restarts the world from tick 0 with the same seed — ",
            "runs are deterministic, so a slider change is a controlled experiment. ",
            "A newborn costs the colony 6 stores; extinction is a verdict, not an error."),
        ],
      },
      {
        id: "follow", label: "Follow one creature",
        kids: [
          h("p", { class: "game-note" },
            "Click a creature to pin its decision table to the trace line below the ",
            "canvas — its top-scored actions, the numbers behind them, and which rung ",
            "answered (rule or engine). Click empty ground or use the button to release it. ",
            "The colors are the actions: green eat, cyan drink, red flee, amber fight, ",
            "white explore, grey rest, lime gather, teal go-home, rose follow."),
          h("button", { class: "mz-chip", id: "mw-release", type: "button" },
            "release followed creature"),
        ],
      },
      {
        id: "about", label: "What decides what?",
        kids: [h("p", { class: "game-note" },
          "CREATURES decide: every ~0.75 s each one re-scores ten actions from its own ",
          "meters and its eight-cell view — eat, drink, flee, fight, explore, rest, ",
          "gather, go home, follow, ignore. Rules first: fleeing outranks everything ",
          "when the fox is within seven cells, and a pack of two or more may choose to ",
          "fight. A genuine near-tie (gap ≤ 1.5) goes to the ENGINE over just the tied ",
          "candidates — if the colony-wide batching budget has a token; a refusal falls ",
          "back to the argmax without derailing anyone. THE FOX is a rule, never a ",
          "question: it hunts the nearest creature in sight and is driven off by packs. ",
          "Births (6 stores each), starvation, thirst and the regrowing grass are all ",
          "rules. Every escalated request is auditable in the stats overlay.")]
      },
    ]);

    const main = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, hint,
        h("div", { class: "game-view" }, canvas, veil),
        status, under, traceEl),
      h("div", { class: "game-side" }, opts));
    root.appendChild(h("div", { class: "game-wrap" }, bar, main));

    buildOverlays();
    wire();
    S.seedKit = DK.shell.numericSeed(byId("mw-seed"), byId("mw-dice"), newWorld);
    DK.shell.resetParams(byId("mw-reset-params"), root, newWorld);
    DK.shell.fullscreen(root.querySelector(".game-wrap"), byId("mw-fs"));
    engine.setEnabled(true);   /* default on, like every game: ties reach the runtime */
    newWorld();
    if (!window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      requestAnimationFrame(loop);
    } else {
      render(); /* reduced motion: static world, tick on demand */
    }
  }

  /* ---------- overlays (shared chrome, microworld data) ---------- */
  function overlay(id, title, bodyId, closeId) {
    return h("dialog", { class: "mz-overlay", id },
      h("div", { class: "mz-board-card" },
        h("div", { class: "mz-overlay-head" },
          h("span", { class: "mz-overlay-title" }, title),
          h("button", { class: "mz-chip", id: closeId, type: "button" }, "✕ close")),
        h("div", { class: "mz-overlay-scroll", id: bodyId })));
  }

  function buildOverlays() {
    root.appendChild(overlay("mw-board-overlay", "Session scoreboard", "mw-board-body", "mw-board-close"));
    root.appendChild(overlay("mw-stats-overlay", "Measured stats", "mw-stats-body", "mw-stats-close"));
  }

  function renderBoard() {
    const body = byId("mw-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" },
        "No checkpoints yet — the board records a row every 60 sim-seconds. ",
        "Session only: nothing you do leaves this tab."));
      return;
    }
    const frag = h("div", null, h("table", { class: "game-table" },
      h("thead", null, h("tr", null,
        ...["#", "sim time", "pop", "births", "deaths", "fox drives", "stores", "engine asks"].map((x) => h("th", null, x)))),
      h("tbody", null, rows.map((r, i) => h("tr", null,
        h("td", null, String(rows.length - i)),
        h("td", null, r.time),
        h("td", null, String(r.pop)),
        h("td", null, String(r.births)),
        h("td", null, String(r.deaths)),
        h("td", null, String(r.drives)),
        h("td", null, String(r.stores)),
        h("td", null, String(r.engineCalls)))))));
    body.appendChild(frag);
    body.appendChild(h("button", {
      class: "mz-chip", type: "button", onclick: () => { S.board.clear(); renderBoard(); },
    }, "clear board"));
  }

  function renderStats() {
    const body = byId("mw-stats-body");
    if (!body || !S.world) return;
    body.textContent = "";
    const w = S.world;
    const snap = S.meters.snapshot();
    const rows = [
      ["frames/s (EMA)", S.fps.toFixed(0)],
      ["sim ticks", String(w.ticks)],
      ["population", String(w.creatures.length)],
      ["births / deaths", w.births + " / " + w.deaths],
      ["fox drives", String(w.drives)],
      ["colony stores", String(w.stores)],
      ["decisions → rule", String(w.ruleCalls)],
      ["decisions → engine", String(w.engineCalls)],
      ["budget tokens left", String(w.tokens)],
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
      "Measured in this tab only. The engine row counts near-tie decisions the WASM ",
      "runtime answered; with the engine off the same argmax answers every tie."));
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
      MW.tickWorld(S.world, engine.status === "ready" ? engineChoose : null);
      S.meters.sample("tick.ms", nowMs() - t0);
      S.acc -= DT;
      dirty = true;
      if (S.world.over) break;
    }
    if (S.world.over) setRunning(false);
    /* board checkpoint: a row every 60 sim-seconds */
    const seconds = Math.floor(S.world.ticks / MW.TICKS_PER_S);
    if (seconds > 0 && seconds % 60 === 0 && seconds !== S.pushedSeconds) {
      S.pushedSeconds = seconds;
      const sum = MW.runSummary(S.world);
      S.board.record({
        ts: Date.now(), time: sum.seconds.toFixed(0) + "s",
        pop: sum.pop, births: sum.births, deaths: sum.deaths,
        drives: sum.drives, stores: sum.stores, engineCalls: sum.engineCalls,
        stamp: MW.microworldStamp(S.cfg),
      });
      if (S.boardOpen) renderBoard();
    }
    if (dirty) { render(); syncUnder(); }
    const lastEvent = S.world.events.length
      ? S.world.events[S.world.events.length - 1] : null;
    if (lastEvent && lastEvent !== S.seenEvent) {
      S.seenEvent = lastEvent;
      traceEvent(lastEvent);
    }
    if (S.world.over) trace("extinction — the colony is gone. Restart for a fresh world.");
    if (S.statsOpen && (!S.meters.get("stats.draw") || t - S.meters.get("stats.draw").last > 250)) {
      S.meters.sample("stats.draw", 0);
      renderStats();
    }
    requestAnimationFrame(loop);
  }

  function syncUnder() {
    const el = byId("mw-under");
    if (!el || !S.world) return;
    const w = S.world;
    const sum = MW.runSummary(w);
    const engineShare = (sum.engineCalls + sum.ruleCalls)
      ? Math.round((100 * sum.engineCalls) / (sum.engineCalls + sum.ruleCalls)) : 0;
    el.innerHTML = "";
    el.appendChild(DK.shell.kvRow([
      ["state", w.over ? "extinct" : (S.running ? "living" : "paused")],
      ["sim time", sum.seconds.toFixed(0) + "s"],
      ["population", String(sum.pop)],
      ["stores", String(sum.stores)],
      ["births / deaths", sum.births + " / " + sum.deaths],
      ["decisions → engine", sum.engineCalls + " (" + engineShare + "%)"],
    ]));
  }

  /* ---------- render ---------- */
  function render() {
    const canvas = byId("mw-canvas");
    if (!canvas || !S.world) return;
    const ctx = canvas.getContext("2d");
    const w = S.world;
    ctx.fillStyle = PAL.bg;
    ctx.fillRect(0, 0, W, H);

    /* tiles */
    for (let y = 0; y < MW.GH; y++) {
      for (let x = 0; x < MW.GW; x++) {
        const i = y * MW.GW + x;
        const t = w.tiles[i];
        if (t === MW.TILE.WATER) {
          ctx.fillStyle = PAL.water;
          ctx.fillRect(x * MW.CELL, y * MW.CELL, MW.CELL, MW.CELL);
        } else if (t === MW.TILE.HOME) {
          ctx.fillStyle = PAL.home;
          ctx.fillRect(x * MW.CELL, y * MW.CELL, MW.CELL, MW.CELL);
        } else {
          /* plain turf stays a flat dim texture; only the rich patches
             brighten, so the food signal is the one loud thing in the field */
          const f = w.food[i] / MW.FOOD_MAX;
          if (f > 0.35) {
            ctx.fillStyle = PAL.rich;
            ctx.globalAlpha = 0.3 + 0.7 * Math.min(1, (f - 0.35) / 0.65);
          } else {
            ctx.fillStyle = PAL.grass;
            ctx.globalAlpha = 0.45;
          }
          ctx.fillRect(x * MW.CELL, y * MW.CELL, MW.CELL, MW.CELL);
          ctx.globalAlpha = 1;
        }
      }
    }
    /* home plot: the outline plus the stores themselves, drawn as a stack */
    ctx.strokeStyle = PAL.homeEdge;
    ctx.lineWidth = 1.5;
    ctx.strokeRect(w.home.x * MW.CELL + 1, w.home.y * MW.CELL + 1,
      w.home.w * MW.CELL - 2, w.home.h * MW.CELL - 2);
    ctx.lineWidth = 1;
    const stacked = Math.min(w.stores, 12);
    for (let i = 0; i < stacked; i++) {
      ctx.fillStyle = PAL.carrying;
      ctx.fillRect(w.home.x * MW.CELL + 6 + (i % 6) * 8,
        w.home.y * MW.CELL + 6 + Math.floor(i / 6) * 8, 5, 5);
    }
    ctx.fillStyle = PAL.homeEdge;
    ctx.font = "600 9px ui-monospace, Menlo, monospace";
    ctx.textAlign = "left";
    ctx.fillText("stores " + w.stores, w.home.x * MW.CELL + 2,
      w.home.y * MW.CELL + w.home.h * MW.CELL + 11);

    /* creatures: the colour IS the action; an outline keeps them readable
       on both the dark grass and the rich tiles */
    for (const c of w.creatures) {
      const cx = c.x * MW.CELL + MW.CELL / 2, cy = c.y * MW.CELL + MW.CELL / 2;
      ctx.fillStyle = ACTION_COLOR[c.action] || PAL.ink;
      ctx.beginPath();
      ctx.arc(cx, cy, 4, 0, Math.PI * 2);
      ctx.fill();
      ctx.strokeStyle = "rgba(0, 0, 0, 0.4)";
      ctx.lineWidth = 1;
      ctx.stroke();
      if (c.carrying) {
        ctx.fillStyle = PAL.carrying;
        ctx.fillRect(cx + 3, cy - 6, 3, 3);
      }
      if (S.follow === c.id) {
        ctx.strokeStyle = PAL.follow;
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(cx, cy, 7.5, 0, Math.PI * 2);
        ctx.stroke();
        ctx.lineWidth = 1;
        ctx.fillStyle = PAL.follow;
        ctx.font = "600 11px ui-monospace, Menlo, monospace";
        ctx.fillText("#" + c.id, cx + 10, cy - 8);
      }
    }

    /* the fox: ears, body, tail — a predator, not a red pixel */
    const fx = w.fox.x * MW.CELL + MW.CELL / 2, fy = w.fox.y * MW.CELL + MW.CELL / 2;
    ctx.fillStyle = PAL.fox;
    ctx.beginPath();
    ctx.moveTo(fx, fy - 6); ctx.lineTo(fx + 6.5, fy);
    ctx.lineTo(fx, fy + 6); ctx.lineTo(fx - 6.5, fy);
    ctx.closePath(); ctx.fill();
    ctx.beginPath();
    ctx.moveTo(fx - 4.5, fy - 4); ctx.lineTo(fx - 2.5, fy - 10); ctx.lineTo(fx - 0.5, fy - 5.5);
    ctx.moveTo(fx + 4.5, fy - 4); ctx.lineTo(fx + 2.5, fy - 10); ctx.lineTo(fx + 0.5, fy - 5.5);
    ctx.fill();
    ctx.strokeStyle = PAL.fox;
    ctx.lineWidth = 2;
    ctx.beginPath(); ctx.moveTo(fx - 6.5, fy + 0.5); ctx.lineTo(fx - 12, fy - 3); ctx.stroke();
    ctx.fillStyle = "rgba(255, 138, 138, 0.9)";
    ctx.font = "600 9px ui-monospace, Menlo, monospace";
    ctx.textAlign = "center";
    ctx.fillText("fox", fx, fy + 16);

    drawLegend(ctx);
    syncStatus();
    syncFollowTrace();
  }

  /* the on-canvas key: what the dots, the diamond and the tiles mean */
  function drawLegend(ctx) {
    const w = S.world;
    if (!w) return;
    const rows = [
      { kind: "dot", color: ACTION_COLOR.eat, label: "creature — its colour is its action" },
      { kind: "sq", color: PAL.carrying, label: "carrying food home" },
      { kind: "fox", color: PAL.fox, label: "the fox — a rule, never a question" },
      { kind: "tile", color: PAL.rich, label: "rich grass (food)" },
      { kind: "tile", color: PAL.water, label: "water" },
      { kind: "tile", color: PAL.home, edge: PAL.homeEdge, label: "colony stores" },
    ];
    const cw = 252, x = W - cw - 12, y = 12, rowH = 16;
    const ch = rows.length * rowH + 12;
    ctx.fillStyle = PAL.legendBg;
    ctx.strokeStyle = PAL.legendEdge;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(x + 8, y);
    ctx.arcTo(x + cw, y, x + cw, y + ch, 8);
    ctx.arcTo(x + cw, y + ch, x, y + ch, 8);
    ctx.arcTo(x, y + ch, x, y, 8);
    ctx.arcTo(x, y, x + cw, y, 8);
    ctx.closePath();
    ctx.fill();
    ctx.stroke();
    ctx.font = "600 10px ui-monospace, Menlo, monospace";
    ctx.textAlign = "left";
    rows.forEach((r, i) => {
      const cy = y + 10 + i * rowH + 5;
      if (r.kind === "dot") {
        ctx.fillStyle = r.color;
        ctx.beginPath(); ctx.arc(x + 15, cy, 4, 0, Math.PI * 2); ctx.fill();
      } else if (r.kind === "sq") {
        ctx.fillStyle = r.color;
        ctx.fillRect(x + 11, cy - 4, 7, 7);
      } else if (r.kind === "fox") {
        ctx.fillStyle = r.color;
        ctx.beginPath();
        ctx.moveTo(x + 15, cy - 5); ctx.lineTo(x + 20, cy);
        ctx.lineTo(x + 15, cy + 5); ctx.lineTo(x + 10, cy);
        ctx.closePath(); ctx.fill();
      } else {
        ctx.fillStyle = r.color;
        ctx.fillRect(x + 10, cy - 5, 10, 10);
        if (r.edge) {
          ctx.strokeStyle = r.edge;
          ctx.strokeRect(x + 10, cy - 5, 10, 10);
        }
      }
      ctx.fillStyle = PAL.dim;
      ctx.fillText(r.label, x + 28, cy + 3.5);
    });
  }

  /* the textual heartbeat: proof the world is alive without reading pixels */
  function syncStatus() {
    const el = byId("mw-status");
    const w = S.world;
    if (!el || !w) return;
    const sum = MW.runSummary(w);
    el.textContent = "creatures: " + sum.pop + " · decisions: "
      + (sum.ruleCalls + sum.engineCalls) + " (engine " + sum.engineCalls
      + ") · ticks: " + w.ticks + " · t+" + sum.seconds.toFixed(0) + "s · stores: " + sum.stores;
  }

  function syncFollowTrace() {
    if (!S.follow || !S.world) return;
    const c = S.world.creatures.find((x) => x.id === S.follow);
    if (!c) {
      trace("creature #" + S.follow + " is gone — click another to follow it");
      S.follow = null;
      return;
    }
    const top = (c.scores || []).slice(0, 3)
      .map((r) => r.id + " " + r.score.toFixed(1)).join(" · ");
    trace("#" + c.id + " " + c.action +
      " · energy " + c.energy.toFixed(0) + " water " + c.water.toFixed(0) +
      (c.carrying ? " · carrying" : "") +
      " · top: " + (top || "—") + " · rung " + c.rung);
  }

  /* ---------- running state ---------- */
  function setRunning(on) {
    S.running = on;
    if (!on) S.last = 0;
    const play = byId("mw-play");
    if (play) play.textContent = on ? "⏸ pause" : "▶ play";
    syncVeil();
  }

  function syncVeil() {
    const veil = byId("mw-veil");
    const chip = byId("mw-veil-chip");
    const hint = byId("mw-veil-hint");
    const w = S.world;
    if (!veil || !w) return;
    if (S.running) { veil.hidden = true; return; }
    veil.hidden = false;
    if (w.over) {
      chip.textContent = "↺ restart";
      hint.textContent = "extinction — the colony is gone at t+"
        + Math.floor(w.ticks / MW.TICKS_PER_S) + "s. It is a verdict, not an error: restart for a fresh world.";
    } else if (w.ticks > 0) {
      chip.textContent = "▶ resume";
      hint.textContent = "paused at t+" + Math.floor(w.ticks / MW.TICKS_PER_S) + "s — "
        + w.creatures.length + " creatures holding still, nothing lost.";
    } else {
      chip.textContent = "▶ press play";
      hint.textContent = BOOT_HINT;
    }
  }

  /* ---------- wiring ---------- */
  function wire() {
    byId("mw-play").addEventListener("click", () => setRunning(!S.running));
    byId("mw-veil").addEventListener("click", () => {
      if (S.world && S.world.over) { newWorld(); return; }
      setRunning(true);
    });
    byId("mw-restart").addEventListener("click", () => newWorld());
    byId("mw-seed").addEventListener("change", () => newWorld());
    byId("mw-speed").addEventListener("click", (e) => {
      S.speed = S.speed === 1 ? 2 : S.speed === 2 ? 4 : 1;
      e.currentTarget.textContent = "speed: " + S.speed + "×";
    });
    byId("mw-engine-toggle").addEventListener("click", (e) => {
      const on = engine.status === "off" || engine.status === "unavailable";
      engine.setEnabled(on);
      e.currentTarget.textContent = on ? "engine: on" : "engine: off";
    });
    byId("mw-board-btn").addEventListener("click", () => {
      S.boardOpen = true; S.statsOpen = false;
      byId("mw-stats-overlay").classList.remove("open");
      byId("mw-board-overlay").classList.add("open");
      renderBoard();
    });
    byId("mw-board-close").addEventListener("click", () => {
      S.boardOpen = false;
      byId("mw-board-overlay").classList.remove("open");
    });
    byId("mw-stats-btn").addEventListener("click", () => {
      S.statsOpen = true; S.boardOpen = false;
      byId("mw-board-overlay").classList.remove("open");
      byId("mw-stats-overlay").classList.add("open");
      renderStats();
    });
    byId("mw-stats-close").addEventListener("click", () => {
      S.statsOpen = false;
      byId("mw-stats-overlay").classList.remove("open");
    });
    byId("mw-release").addEventListener("click", () => {
      S.follow = null;
      trace("released — click a creature to follow another");
    });

    /* click to follow: nearest creature within half a tile, else release */
    byId("mw-canvas").addEventListener("click", (e) => {
      const rect = e.currentTarget.getBoundingClientRect();
      const gx = ((e.clientX - rect.left) / rect.width) * MW.GW;
      const gy = ((e.clientY - rect.top) / rect.height) * MW.GH;
      let best = null, bestD = Infinity;
      for (const c of S.world.creatures) {
        const d = Math.hypot(c.x + 0.5 - gx, c.y + 0.5 - gy);
        if (d < bestD) { bestD = d; best = c; }
      }
      if (best && bestD <= 1.2) {
        S.follow = best.id;
        syncFollowTrace();
      } else {
        S.follow = null;
        trace("released — click a creature to follow another");
      }
    });

    window.addEventListener("keydown", (e) => {
      if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
      if (e.key === " ") {
        setRunning(!S.running);
        e.preventDefault();
      } else if (e.key === "r" || e.key === "R") {
        newWorld();
      } else if (e.key === "Escape") {
        S.boardOpen = false; S.statsOpen = false;
        byId("mw-board-overlay").classList.remove("open");
        byId("mw-stats-overlay").classList.remove("open");
      }
    });

    setChip();
    syncUnder();   /* paused boot still shows a truthful readout, not a blank line */
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", build);
  } else {
    build();
  }
})();
