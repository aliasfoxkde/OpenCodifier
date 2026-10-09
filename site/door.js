/* Choose Your Door — shell.
   door-core.js owns the state, the policy weights, and the one
   decision; this file is chrome, and it is deliberately NOT a canvas:
   the decision, the confidence, the candidate scores, and the full
   trace ARE the UI. Door cards show the traits and the ladder's score
   bars; the verdict panel carries the rung badge and the why; the
   wire panel shows the exact request the engine saw. The series panel
   replays the same room sequence under all four personalities,
   ladder-only, so policy -> behavior is visible at volume. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const DC = globalThis.DoorCore;
  const root = document.getElementById("door-root");
  if (!root || !DK || !DC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const nowMs = DK.nowMs;

  /* ---------- state ---------- */
  const S = {
    cfg: Object.assign({}, DC.DEFAULT_CONFIG),
    game: null,
    engineOn: true,
    boardOpen: false, statsOpen: false,
    lastReq: null,
    lastSeries: null,
    askMs: 0,
    board: DK.sessionBoard(),
    meters: DK.meters(),
  };

  const engine = DK.engineBridge({
    onStatus: () => setChip(),
  });

  function engineChoose(request) {
    const t0 = nowMs();
    const res = engine.choose(request);
    S.askMs = nowMs() - t0;
    S.meters.sample("engine.ms", S.askMs);
    return res;
  }

  /* ---------- helpers ---------- */
  function cfgFromUI() {
    return {
      seed: S.seedKit ? S.seedKit.get() : DC.DEFAULT_CONFIG.seed,
      personality: S.cfg.personality,
      doorCount: +byId("dr-count").value,
      health: +byId("dr-health").value,
      weapon: +byId("dr-weapon").value / 100,
      knowledge: +byId("dr-knowledge").value / 100,
      torches: +byId("dr-torches").value,
      objective: byId("dr-objective").value,
      balanced: byId("dr-balanced").checked,
    };
  }

  function newGame() {
    S.cfg = cfgFromUI();
    S.game = DC.createGame(S.cfg);
    S.lastReq = null;
    syncPersonalityChips();
    render();
  }

  function decideNow() {
    const g = S.game;
    if (!g) return;
    if (g.decided) { newGame(); return; }   // decide again = a fresh room
    const t0 = nowMs();
    const d = DC.decide(g, S.engineOn ? engineChoose : null);
    void t0;
    if (d && !d.abstained && d.rung === "engine") S.meters.sample("decide.ms", S.askMs);
    const s = DC.runSummary(g);
    S.board.record({
      ts: Date.now(), outcome: s.abstained ? "abstain" : "decision",
      at: 1, mode: s.personality,
      choice: s.choice || "—", rung: s.rung,
      conf: s.confidence === null ? "—" : s.confidence.toFixed(3),
      objective: s.objective, rungs: { rule: s.rung === "rule" ? 1 : 0,
        engine: s.rung === "engine" ? 1 : 0 },
      engineCalls: s.rung === "engine" ? 1 : 0,
      seed: g.cfg.seed, stamp: g.stamp,
    });
    render();
  }

  function runSeries() {
    const n = +byId("dr-series-n").value;
    /* ladder-only by design: no engine asks, so the bars are pure policy */
    const run = (p) => DC.series(Object.assign({}, S.cfg, {
      personality: p, balanced: false,
    }), n, null);
    S.lastSeries = {
      n,
      rows: DC.PERSONALITIES.map((p) => run(p)),
      /* the demo metric: coward vs greedy, same rooms, same state */
      diverge: DC.divergence(Object.assign({}, S.cfg, { personality: "coward" }),
        Object.assign({}, S.cfg, { personality: "greedy" }), n),
      stamp: S.game ? S.game.stamp : "",
    };
    renderSeries();
  }

  function renderSeries() {
    const box = byId("dr-series-tally");
    const note = byId("dr-series-note");
    if (!box) return;
    box.textContent = "";
    if (!S.lastSeries) {
      note.textContent = "No series yet — pick a room shape and press run. "
        + "The series replays the same room sequence under every "
        + "personality, ladder-only (no engine asks), so the bars are "
        + "pure policy. Every 4th room is the balanced dead-heat setup, "
        + "counted as tie-breaks rather than argmax wins.";
      return;
    }
    const dv = S.lastSeries.diverge;
    const ties = S.lastSeries.rows[0] ? S.lastSeries.rows[0].tieBreaks : 0;
    note.textContent = S.lastSeries.n + " rooms × " + DC.PERSONALITIES.length
      + " personalities, same room sequence, ladder-only. Same state, "
      + "different weights, different behavior — the demo in one panel. "
      + "Coward and greedy pick DIFFERENT doors in " + dv.disagree
      + " of " + dv.rooms + " decided rooms (" + dv.disagreePct
      + "%). " + ties + " dead-heat rooms broke by door order (counted "
      + "separately, not as argmax wins).";
    const maxDoor = 5;
    for (const row of S.lastSeries.rows) {
      const maxC = Math.max(1, ...Object.values(row.tally));
      box.appendChild(h("div", { class: "dr-series-row" },
        h("span", { class: "dr-series-name "
          + (row.personality === S.cfg.personality ? "on" : "") },
          DC.PERSONALITY_NAME[row.personality]),
        h("span", { class: "dr-series-bars" },
          Array.from({ length: maxDoor }, (_, i) => {
            const id = "door-" + (i + 1);
            const c = row.tally[id] || 0;
            return h("span", { class: "dr-series-cell", title: id + ": " + c },
              h("span", { class: "dr-series-bar",
                style: "height:" + Math.max(2, Math.round(100 * c / maxC)) + "px" }),
              h("span", { class: "dr-series-count" }, String(c)));
          })),
        h("span", { class: "dr-series-meta" },
          "avg conf " + row.avgConfidence.toFixed(3)
          + (row.tieBreaks ? " · " + row.tieBreaks + " ties" : ""))));
    }
  }

  /* ---------- render ---------- */
  function traitRow(name, v) {
    return h("div", { class: "dr-trait" },
      h("span", { class: "dr-trait-name" }, name),
      h("span", { class: "dr-trait-bar" },
        h("span", { class: "dr-trait-fill",
          style: "width:" + Math.round(v * 100) + "%" })),
      h("span", { class: "dr-trait-val" }, v.toFixed(2)));
  }

  function doorCard(d, i, dec) {
    const chosen = dec && dec.choice === d.id;
    const prob = dec ? dec.probs[i] : null;
    const score = dec ? dec.scores[i] : null;
    return h("div", { class: "door-card" + (chosen ? " chosen" : ""),
      "data-door": d.id },
      h("div", { class: "door-head" },
        h("span", { class: "door-glyph" }, d.glyph),
        h("span", { class: "door-label" }, d.label)),
      h("div", { class: "door-traits" },
        traitRow("danger", d.danger), traitRow("reward", d.reward),
        traitRow("mystery", d.mystery), traitRow("effort", d.effort)),
      h("div", { class: "door-verdict" },
        score === null
          ? h("span", { class: "door-score dim" }, "undecided")
          : h("span", { class: "door-score" },
            "score " + score.toFixed(3) + " · p " + prob.toFixed(3)),
        chosen ? h("span", { class: "door-mark" }, "✔ chosen") : null));
  }

  function render() {
    const g = S.game;
    const doors = byId("dr-doors");
    const verdict = byId("dr-verdict");
    const under = byId("dr-under");
    const wire = byId("dr-wire");
    if (!g || !doors) return;
    const dec = g.decision;

    doors.textContent = "";
    g.doors.forEach((d, i) => doors.appendChild(doorCard(d, i, dec)));

    verdict.textContent = "";
    if (!dec) {
      verdict.appendChild(h("span", { class: "dr-v-line" },
        "The room is set. One decision awaits — press ⚖ decide."));
    } else if (dec.abstained) {
      verdict.appendChild(h("span", { class: "dr-v-badge abstain" }, "ABSTAIN"));
      verdict.appendChild(h("span", { class: "dr-v-line warn" },
        "Nobody moves."));
      verdict.appendChild(h("span", { class: "dr-v-why" }, dec.why));
    } else {
      verdict.appendChild(h("span", { class: "dr-v-badge "
        + dec.rung }, dec.rung === "engine" ? "ENGINE" : "RULE"));
      verdict.appendChild(h("span", { class: "dr-v-line" },
        DC.PERSONALITY_NAME[g.cfg.personality] + " takes " + dec.choice
        + " — confidence " + (dec.confidence * 100).toFixed(1) + "%"));
      verdict.appendChild(h("span", { class: "dr-v-why" }, dec.why));
    }

    under.textContent = "";
    const s = DC.runSummary(g);
    under.appendChild(DK.shell.kvRow([
      ["room", "seed “" + g.cfg.seed + "” · " + s.doors + " doors"
        + (g.cfg.balanced ? " · BALANCED" : "")],
      ["character", "health " + Math.round(g.cfg.health) + " · weapon "
        + g.cfg.weapon.toFixed(2) + " · knowledge " + g.cfg.knowledge.toFixed(2)
        + " · torches " + g.cfg.torches],
      ["objective", s.objective + " — " + DC.OBJECTIVE_DESC[s.objective]],
      ["personality", DC.PERSONALITY_NAME[s.personality] + " (risk "
        + s.risk + ") — " + DC.PERSONALITY_DESC[s.personality]],
      ["decision", dec
        ? (dec.abstained ? "abstained" : dec.choice) + " · rung " + dec.rung
          + " · conf " + (dec.confidence * 100).toFixed(1) + "%"
        : "not made yet"],
      ["engine", S.engineOn ? "on — " + engine.status
        : "off — the ladder decides alone"],
    ]));

    wire.textContent = "";
    const req = g.lastRequest || (dec ? null : DC.buildRequest(g));
    if (req) {
      S.lastReq = req;
      wire.appendChild(h("details", { class: "dr-wire-details" },
        h("summary", null, "the request on the wire"),
        h("pre", { class: "dr-wire-pre" },
          "state.text:\n  " + req.state.text + "\n\n"
          + "questions[0].id: " + req.questions[0].id + " (choice, "
          + req.questions[0].candidates.length + " candidates)\n"
          + "policy: " + JSON.stringify(req.policy))));
    } else {
      wire.appendChild(h("p", { class: "game-note" },
        "The engine was off — no request was built."));
    }

    byId("dr-decide").textContent = dec ? "↻ decide in a new room" : "⚖ decide";
    renderSeries();
  }

  function renderBoard() {
    const body = byId("dr-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" },
        "No rows yet — every decision (and every abstain) lands here. "
        + "Session only: close the tab and it is gone."));
      return;
    }
    body.appendChild(h("table", { class: "game-table" },
      h("thead", null, h("tr", null,
        ...["#", "outcome", "personality", "choice", "rung", "conf",
          "objective", "seed"].map((x) => h("th", null, x)))),
      h("tbody", null, rows.map((r, i) => h("tr", null,
        h("td", null, String(i + 1)),
        h("td", null, r.outcome),
        h("td", null, r.mode),
        h("td", null, String(r.choice)),
        h("td", null, r.rung),
        h("td", null, String(r.conf)),
        h("td", null, r.objective),
        h("td", null, r.seed))))));
    body.appendChild(h("button", {
      class: "mz-chip", type: "button", onclick: () => { S.board.clear(); renderBoard(); },
    }, "clear scoreboard"));
  }

  function renderStats() {
    const body = byId("dr-stats-body");
    if (!body) return;
    body.textContent = "";
    const snap = S.meters.snapshot();
    const g = S.game;
    const s = g ? DC.runSummary(g) : null;
    const rows = [
      ["doors in the room", s ? String(s.doors) : "—"],
      ["personality / risk", s ? s.personality + " / " + s.risk : "—"],
      ["objective", s ? s.objective : "—"],
      ["balanced room", g && g.cfg.balanced ? "yes — dead heat by design" : "no"],
      ["decision", s ? (s.decided
        ? (s.abstained ? "ABSTAINED — nobody moves" : s.choice + " (rung " + s.rung + ")")
        : "not made yet") : "—"],
      ["confidence", s && s.confidence !== null
        ? (s.confidence * 100).toFixed(1) + "%" : "—"],
      ["engine lane", S.engineOn ? "on" : "off — ladder only"],
      ["engine calls", s && s.rung === "engine" ? "1" : "0"],
      ["softmax temperature", String(DC.SOFTMAX_T)],
      ["series", S.lastSeries
        ? S.lastSeries.n + " rooms × " + S.lastSeries.rows.length
          + " personalities (ladder-only)" : "not run yet"],
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
  }

  function setChip() {
    const chip = byId("dr-engine-chip");
    if (!chip) return;
    const st = engine.status;
    chip.textContent = "engine: " + st;
    chip.classList.toggle("on", st === "ready");
    chip.classList.toggle("off", st === "unavailable" || st === "off");
  }

  function syncPersonalityChips() {
    document.querySelectorAll("#dr-personality .mz-chip").forEach((b) => {
      b.classList.toggle("on", b.dataset.p === S.cfg.personality);
    });
  }

  /* ---------- build ---------- */
  function slider(id, label, min, max, step, val, unit) {
    const out = h("span", { class: "game-sliderval", id: id + "-val" },
      String(val) + (unit || ""));
    const input = h("input", {
      type: "range", id, min: String(min), max: String(max), step: String(step),
      value: String(val), "aria-label": label,
    });
    input.addEventListener("input", () => { out.textContent = input.value + (unit || ""); });
    input.addEventListener("change", () => newGame());
    return h("label", { class: "game-slider" },
      h("span", { class: "game-slider-label" }, label), input, out);
  }

  function overlay(id, title, bodyId, closeId) {
    return h("div", { class: "mz-overlay", id },
      h("div", { class: "mz-overlay-card" },
        h("div", { class: "mz-overlay-head" },
          h("span", { class: "mz-overlay-title" }, title),
          h("button", { class: "mz-chip", id: closeId, type: "button" }, "✕ close")),
        h("div", { class: "mz-overlay-scroll", id: bodyId })));
  }

  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) S.cfg.seed = deepSeed;

    const bar = h("div", { class: "game-bar" },
      h("button", { class: "mz-chip", id: "dr-decide", type: "button" }, "⚖ decide"),
      h("button", { class: "mz-chip", id: "dr-restart", type: "button" }, "↺ new room"),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "dr-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "dr-seed", class: "mz-input",
          value: S.cfg.seed, size: "10" }),
        h("button", { class: "mz-chip", id: "dr-dice", type: "button",
          title: "new random seed" }, "🎲")),
      h("button", { class: "mz-chip", id: "dr-engine-toggle", type: "button" },
        "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "dr-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "dr-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "dr-stats-btn", type: "button" }, "ⓘ stats"),
      h("button", { class: "mz-chip", id: "dr-reset-params", type: "button" }, "reset params"),
      h("button", { class: "mz-chip", id: "dr-fs", type: "button" }, "⛶ full screen"),
    );

    const opts = DK.shell.accordion([
      {
        id: "character", label: "The character and the room", open: true,
        kids: [
          h("div", { class: "game-bar", id: "dr-personality",
            role: "group", "aria-label": "Personality — the policy" },
            DC.PERSONALITIES.map((p) => h("button", {
              class: "mz-chip", type: "button", "data-p": p,
              title: DC.PERSONALITY_DESC[p],
            }, DC.PERSONALITY_NAME[p]))),
          h("p", { class: "game-note", id: "dr-p-desc" },
            DC.PERSONALITY_DESC[S.cfg.personality]),
          slider("dr-health", "health", 1, 100, 1, S.cfg.health),
          slider("dr-weapon", "weapon", 0, 100, 5, Math.round(S.cfg.weapon * 100), "%"),
          slider("dr-knowledge", "knowledge", 0, 100, 5, Math.round(S.cfg.knowledge * 100), "%"),
          slider("dr-torches", "torches", 0, 5, 1, S.cfg.torches),
          slider("dr-count", "doors", 3, 5, 1, S.cfg.doorCount),
          h("div", { class: "game-bar" },
            h("label", { class: "game-slider" },
              h("span", { class: "game-slider-label" }, "objective"),
              h("select", { id: "dr-objective", class: "mz-input",
                "aria-label": "Objective" },
                DC.OBJECTIVES.map((o) => h("option",
                  { value: o, selected: o === S.cfg.objective ? "" : null }, o)))),
            h("label", { class: "game-slider" },
              h("input", { type: "checkbox", id: "dr-balanced" }),
              h("span", { class: "game-slider-label" }, "⚖ balance the doors"))),
          h("p", { class: "game-note" },
            "The personality is POLICY, not a code path: the same scorer, ",
            "the same wire request, different weights. Health and weapon ",
            "scale how hard the same danger number bites. Balancing the ",
            "doors with no objective is the canonical refusal case — the ",
            "scores dead-heat, there is no argmax, and the engine (if ",
            "asked) abstains: nobody moves. Changing anything deals a ",
            "new room."),
        ],
      },
      {
        id: "series", label: "Run the series",
        kids: [
          slider("dr-series-n", "rooms per personality", 50, 500, 50, 200),
          h("button", { class: "mz-chip", id: "dr-series-run", type: "button" },
            "▶ run the series"),
          h("div", { class: "dr-series-box", id: "dr-series-tally" }),
          h("p", { class: "game-note", id: "dr-series-note" }, "No series yet."),
        ],
      },
      {
        id: "about", label: "What decides what?",
        kids: [h("p", { class: "game-note" },
          "The character (health, weapon, knowledge, torches, objective) ",
          "is the STATE; the doors are the CANDIDATES; the personality is ",
          "the POLICY — coward, greedy, brave and steady differ only in ",
          "the weights the same request carries. The ladder scores every ",
          "door (a weighted trait sum with state modifiers) and the ",
          "confidence is a softmax over the scores. With the engine on, ",
          "the request goes over the full wire contract and the honored ",
          "answer lands on the engine rung; a refusal or a dead heat is ",
          "an abstain — nobody moves and the why is on the screen. With ",
          "the engine off, the ladder decides alone: argmax, and a dead ",
          "tie broken by door order, said aloud. Seeded and ",
          "deterministic: same state, same policy, same door."),
        ],
      },
    ]);

    const doors = h("div", { class: "door-row", id: "dr-doors",
      role: "list", "aria-label": "The doors" });
    const verdict = h("div", { class: "dr-verdict", id: "dr-verdict",
      "aria-live": "polite" },
      h("span", { class: "dr-v-line" }, "The room is set. One decision awaits."));
    const wireBox = h("div", { class: "dr-wire", id: "dr-wire" });
    const under = h("div", { class: "mz-under game-under", id: "dr-under" });

    root.appendChild(bar);
    root.appendChild(h("div", { class: "dr-stage" }, doors,
      h("div", { class: "dr-side" }, verdict, wireBox)));
    root.appendChild(under);
    root.appendChild(opts);
    root.appendChild(overlay("dr-board-overlay", "Session scoreboard",
      "dr-board-body", "dr-board-close"));
    root.appendChild(overlay("dr-stats-overlay", "Measured stats",
      "dr-stats-body", "dr-stats-close"));

    byId("dr-decide").addEventListener("click", decideNow);
    byId("dr-restart").addEventListener("click", newGame);
    byId("dr-seed").addEventListener("change", newGame);
    byId("dr-objective").addEventListener("change", newGame);
    byId("dr-balanced").addEventListener("change", newGame);
    document.querySelectorAll("#dr-personality .mz-chip").forEach((b) => {
      b.addEventListener("click", () => {
        S.cfg.personality = b.dataset.p;
        byId("dr-p-desc").textContent = DC.PERSONALITY_DESC[b.dataset.p];
        newGame();
      });
    });
    byId("dr-series-run").addEventListener("click", runSeries);
    byId("dr-engine-toggle").addEventListener("click", () => {
      S.engineOn = !S.engineOn;
      byId("dr-engine-toggle").textContent = "engine: "
        + (S.engineOn ? "on" : "off");
      render();
    });
    byId("dr-board-btn").addEventListener("click", () => {
      S.boardOpen = true; S.statsOpen = false;
      byId("dr-stats-overlay").classList.remove("open");
      byId("dr-board-overlay").classList.add("open");
      renderBoard();
    });
    byId("dr-board-close").addEventListener("click", () => {
      S.boardOpen = false;
      byId("dr-board-overlay").classList.remove("open");
    });
    byId("dr-stats-btn").addEventListener("click", () => {
      S.statsOpen = true; S.boardOpen = false;
      byId("dr-board-overlay").classList.remove("open");
      byId("dr-stats-overlay").classList.add("open");
      renderStats();
    });
    byId("dr-stats-close").addEventListener("click", () => {
      S.statsOpen = false;
      byId("dr-stats-overlay").classList.remove("open");
    });

    window.addEventListener("keydown", (e) => {
      if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
      if (e.key === "d" || e.key === "D") decideNow();
      else if (e.key === "n" || e.key === "N") newGame();
      else if (e.key === "Escape") {
        S.boardOpen = false; S.statsOpen = false;
        byId("dr-board-overlay").classList.remove("open");
        byId("dr-stats-overlay").classList.remove("open");
      }
    });

    S.seedKit = DK.shell.numericSeed(byId("dr-seed"), byId("dr-dice"), newGame);
    DK.shell.resetParams(byId("dr-reset-params"), root, newGame);
    DK.shell.fullscreen(root, byId("dr-fs"));
    engine.setEnabled(true);   /* default on: the ask is the demo */
    newGame();
    setChip();
    decideNow();               /* open decided: the verdict is the demo */
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", build);
  } else {
    build();
  }
})();
