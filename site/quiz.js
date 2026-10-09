/* Ask the Engine shell — the quiz on the Decisions SDK.
   quiz-core.js owns the deck model, the wire contract, grading and scoring;
   this file is chrome: the three modes (speed-run / audit / compete), the
   JSON deck editor with validation and the explicit opt-in local save, the
   trace reveal, the session-only scoreboard, and the stats overlay. Every
   engine answer on this page is a real choice request — the engine can be
   wrong and the board shows it. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const QC = globalThis.QuizCore;
  const root = document.getElementById("quiz-root");
  if (!root || !DK || !QC) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const fmtMs = DK.fmtMs;
  const fmtPct = DK.fmtPct;

  const MODE_LABEL = { speed: "speed-run", audit: "audit", compete: "compete" };
  const OUTCOME_LABEL = {
    accept: "", abstain: "abstained", unavailable: "engine unavailable",
  };

  /* ---------- state ---------- */
  const S = {
    cfg: { seed: "oc-quiz", mode: "speed" },
    deck: QC.BUILT_IN_DECK,
    engineQz: null,            /* the engine's session (every mode) */
    youQz: null,               /* your session (compete only) */
    phase: "idle",             /* idle | revealed | verdict | shown */
    askedAt: 0,                /* human answer timing (compete) */
    lastRow: null,
    boardOpen: false, statsOpen: false, deckOpen: false,
    recorded: false,
    board: DK.sessionBoard(),
    meters: DK.meters(),
  };

  /* no importBase: the SDK's default "../wasm/" is right for a root page */
  const engine = DK.engineBridge({
    onStatus: () => setChip(),
  });

  /* ---------- sessions ---------- */

  function newRun() {
    S.cfg = {
      seed: S.seedKit ? S.seedKit.get() : "oc-quiz",
      mode: byId("quiz-mode").value,
    };
    S.engineQz = QC.makeQuiz(S.deck, S.cfg);
    S.youQz = S.cfg.mode === "compete" ? QC.makeQuiz(S.deck, S.cfg) : null;
    S.phase = "idle";
    S.lastRow = null;
    S.recorded = false;
    if (S.cfg.mode === "speed") stepSpeed();
    render();
  }

  function engineChoose(qz) {
    const row = QC.askEngine(qz, engine.status === "ready" ? engine : null, (r) => {
      if (r.ms !== null) S.meters.sample("engine.ms", r.ms);
    });
    S.meters.count("questions." + row.outcome);
    S.lastRow = row;
    return row;
  }

  /* the engine loads asynchronously; a run that starts at page load must
     wait for the WASM instead of grading question 1 "unavailable" */
  async function engineReady() {
    if (engine.status === "ready") return true;
    if (engine.status === "loading") return (await engine.ensure()) !== null;
    return false;                       /* off or unavailable: ask anyway */
  }

  async function askWhenReady(qz) {
    await engineReady();
    return engineChoose(qz);
  }

  /* speed-run: the engine answers the moment a question appears */
  async function stepSpeed() {
    const qz = S.engineQz;
    if (QC.finished(qz)) { finishRun("engine"); render(); return; }
    S.phase = "asking";
    render();
    await askWhenReady(qz);
    S.phase = "revealed";
    render();
  }

  /* ---------- run end: the honest board row ---------- */

  function finishRun(who) {
    if (S.recorded) return null;         /* one honest row per run */
    S.recorded = true;
    const qz = who === "you" ? S.youQz : S.engineQz;
    const s = QC.scoreSummary(qz, who);
    const row = {
      ts: Date.now(),
      mode: MODE_LABEL[S.cfg.mode], who,
      deck: S.deck.name,
      asked: s.asked, answered: s.answered,
      accuracy: s.accuracy === null ? "—" : Math.round(s.accuracy * 100) + "%",
      abstained: s.abstained,
      median: s.medianMs === null ? "—" : Math.round(s.medianMs) + " ms",
      seed: S.cfg.seed,
    };
    if (S.cfg.mode === "compete" && who === "you") {
      const se = QC.scoreSummary(S.engineQz, "engine");
      row.vsEngine = se.accuracy === null ? "—"
        : Math.round(se.accuracy * 100) + "% (" + se.correct + "/" + se.answered + ")";
      row.accuracy = s.accuracy === null ? "—" : Math.round(s.accuracy * 100) + "% (" + s.correct + "/" + s.answered + ")";
    }
    S.board.record(row);
    return row;
  }

  function setChip() {
    const chip = byId("quiz-engine-chip");
    if (!chip) return;
    const st = engine.status;
    chip.textContent = "engine: " + st;
    chip.classList.toggle("on", st === "ready");
    chip.classList.toggle("off", st === "unavailable" || st === "off");
  }

  /* ---------- build ---------- */

  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) S.cfg.seed = deepSeed;

    const bar = h("div", { class: "game-bar" },
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "quiz-mode", class: "game-slider-label" }, "mode "),
        h("select", { id: "quiz-mode", class: "mz-input" },
          ...Object.entries(MODE_LABEL).map(([v, label]) =>
            h("option", { value: v, selected: v === S.cfg.mode ? "" : null }, label)))),
      h("span", { class: "game-seed-wrap" },
        h("label", { for: "quiz-seed", class: "game-slider-label" }, "seed "),
        h("input", { type: "number", id: "quiz-seed", class: "mz-input", value: S.cfg.seed, size: "10" }),
        h("button", { class: "mz-chip", id: "quiz-dice", type: "button", title: "new random seed" }, "🎲")),
      h("button", { class: "mz-chip", id: "quiz-restart", type: "button" }, "↺ new run"),
      h("button", { class: "mz-chip", id: "quiz-next", type: "button" }, "next →"),
      h("button", { class: "mz-chip", id: "quiz-deck-btn", type: "button" }, "▦ deck editor"),
      h("button", { class: "mz-chip", id: "quiz-engine-toggle", type: "button" }, "engine: on"),
      h("span", { class: "mz-chip game-rung", id: "quiz-engine-chip" }, "engine: off"),
      h("button", { class: "mz-chip", id: "quiz-board-btn", type: "button" }, "★ scoreboard"),
      h("button", { class: "mz-chip", id: "quiz-stats-btn", type: "button" }, "ⓘ stats"),
      h("button", { class: "mz-chip", id: "quiz-reset-params", type: "button" }, "reset params"),
      h("button", { class: "mz-chip", id: "quiz-fs", type: "button" }, "⛶ full screen"),
    );

    const stage = h("div", { id: "quiz-stage", "aria-live": "polite" });
    const under = h("div", { class: "mz-under game-under", id: "quiz-under" });
    const hint = h("div", { class: "game-hintline", id: "quiz-hint" }, "no question yet");

    const opts = DK.shell.accordion([
      {
        id: "rules", label: "How it answers", open: true,
        kids: [h("p", { class: "game-note" },
          "Every answer is a real choice request to the engine compiled to WASM ",
          "in this tab — question text in the state, your options as candidates, ",
          "zero ML, zero network. The keyed answer travels with the deck; the ",
          "engine's pick is shown next to it and agreement is scored honestly. ",
          "An abstention is the runtime refusing to guess — an outcome, never a ",
          "wrong answer, and it is counted as its own thing.")],
      },
      {
        id: "modes", label: "The three modes",
        kids: [h("p", { class: "game-note" },
          "Speed-run: the engine answers as fast as the questions stream in, ",
          "scored live. Audit: step through question → engine answer + ",
          "confidence + execution trace → keyed answer → your verdict. ",
          "Compete: you answer first (keys 1–6), then the engine gets the same ",
          "question — accuracy and median speed land on the scoreboard."),
          h("p", { class: "game-note" },
            "Keyboard: 1–6 answer (compete) · Y/N verdict (audit) · N next · ",
            "R new run · Esc closes overlays.")],
      },
      {
        id: "honest", label: "Why the engine will lose",
        kids: [h("p", { class: "game-note" },
          "The engine's decision rungs are deterministic text reasoning — ",
          "lexical scoring, calibration, confidence gates — not a language ",
          "model. Quiz questions are deliberately outside what those rungs can ",
          "know: the misses, the abstentions, and the low-confidence guesses ",
          "ARE the demo. Watch the confidence report next to a wrong answer; ",
          "the calibration is the honest part.")],
      },
    ]);

    const main = h("div", { class: "game-layout" },
      h("div", { class: "game-center" }, stage, under, hint),
      h("div", { class: "game-side" }, opts));
    root.appendChild(h("div", { class: "game-wrap" }, bar, main));

    buildOverlays();
    wire();
    S.seedKit = DK.shell.numericSeed(byId("quiz-seed"), byId("quiz-dice"), newRun);
    DK.shell.resetParams(byId("quiz-reset-params"), root);
    DK.shell.fullscreen(root.querySelector(".game-wrap"), byId("quiz-fs"));
    engine.setEnabled(true);
    newRun();
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
    root.appendChild(overlay("quiz-board-overlay", "Session scoreboard", "quiz-board-body", "quiz-board-close"));
    root.appendChild(overlay("quiz-stats-overlay", "Measured stats", "quiz-stats-body", "quiz-stats-close"));
    root.appendChild(overlay("quiz-deck-overlay", "Deck editor — JSON, validated, local-only", "quiz-deck-body", "quiz-deck-close"));
  }

  /* ---------- stage renderers ---------- */

  function optionButton(q, i, disabled) {
    const b = h("button", { class: "quiz-opt", type: "button", disabled: disabled ? "" : null },
      h("span", { class: "quiz-opt-key" }, String(i + 1)),
      h("span", null, q.options[i]));
    b.addEventListener("click", () => answerYou(i));
    return b;
  }

  function questionCard(qz, q, interactive) {
    return h("div", { class: "quiz-q" },
      h("div", { class: "quiz-q-meta" }, qz.deck.name + " · " + q.id),
      h("div", { class: "quiz-q-text" }, q.text),
      h("div", { class: "quiz-opts" },
        ...q.options.map((_, i) => optionButton(q, i, !interactive))));
  }

  function outcomeTag(row) {
    const cls = row.outcome === "accept" ? (row.correct ? "right" : "wrong")
      : row.outcome === "abstain" ? "abstain" : "abstain";
    const txt = row.outcome === "accept"
      ? (row.correct ? "right" : "wrong")
      : OUTCOME_LABEL[row.outcome];
    return h("span", { class: "quiz-tag " + cls }, txt);
  }

  /* the engine's answer panel: pick, confidence, agreement with the key,
     and (audit mode) the real execution trace from the response */
  function answerPanel(row, q, withTrace) {
    const kids = [
      h("div", { class: "quiz-a-head" },
        h("span", null, "the engine says: "),
        h("strong", null, row.pick === null ? "—" : row.pickLabel), " ",
        outcomeTag(row),
        row.confidence !== null ? h("span", { class: "quiz-conf" },
          " · confidence " + fmtPct(row.confidence)) : null,
        h("span", { class: "quiz-conf" }, " · " + fmtMs(row.ms) + " in-tab"),
      ),
    ];
    if (withTrace && row.raw && row.raw.trace && Array.isArray(row.raw.trace.entries)) {
      kids.push(h("details", { class: "quiz-trace" },
        h("summary", null, "execution trace — " + row.raw.trace.entries.length + " deterministic steps"),
        h("ol", null, ...row.raw.trace.entries.map((en) => h("li", null,
          h("code", null, en.node || "?"), " ",
          h("span", { class: "quiz-trace-detail" }, JSON.stringify(en.detail || {})))))));
      const c = row.raw.confidence;
      if (c) {
        kids.push(h("p", { class: "game-note" },
          "confidence report: calibrated " + num4(c.calibrated_confidence) +
          " · top " + num4(c.top_probability) + " · margin " + num4(c.margin) +
          " · entropy " + num4(c.entropy) + " · OOD " + num4(c.ood_score) +
          " · verifier agreement " + num4(c.verifier_agreement)));
      }
    }
    return h("div", { class: "quiz-answer" }, ...kids);
  }

  const num4 = (v) => (typeof v === "number" ? v.toFixed(4) : "—");

  /* ---------- render ---------- */

  function render() {
    const stage = byId("quiz-stage");
    if (!stage) return;
    stage.textContent = "";
    const mode = S.cfg.mode;
    const qz = S.engineQz;
    if (!qz) return;

    if (QC.finished(qz)) {
      if (mode === "compete") { finishRun("you"); renderCompeteDone(stage); }
      else { renderDone(stage, qz, "engine"); }
      renderUnder();
      return;
    }

    const q = QC.current(qz);

    if (mode === "speed") {
      stage.appendChild(questionCard(qz, q, false));
      if (S.phase === "asking") {
        stage.appendChild(h("p", { class: "game-note" }, "asking the engine…"));
      } else {
        const row = S.lastRow && S.lastRow.qid === q.id ? S.lastRow : qz.results[qz.results.length - 1];
        if (row) stage.appendChild(answerPanel(row, q, false));
        stage.appendChild(h("button", { class: "mz-chip quiz-nextbtn", id: "quiz-nextbtn", type: "button" }, "next question →"));
        byId("quiz-nextbtn").addEventListener("click", advance);
      }
    } else if (mode === "audit") {
      stage.appendChild(questionCard(qz, q, false));
      if (S.phase === "asking") {
        stage.appendChild(h("p", { class: "game-note" }, "asking the engine…"));
      } else if (S.phase === "idle") {
        stage.appendChild(h("button", { class: "mz-chip quiz-nextbtn", id: "quiz-askbtn", type: "button" }, "ask the engine →"));
        byId("quiz-askbtn").addEventListener("click", async () => {
          S.phase = "asking";
          render();
          await askWhenReady(qz);
          S.phase = "revealed";
          render();
        });
      } else if (S.phase === "revealed") {
        const row = qz.results[qz.results.length - 1];
        stage.appendChild(answerPanel(row, q, true));
        stage.appendChild(h("button", { class: "mz-chip quiz-nextbtn", id: "quiz-keybtn", type: "button" }, "show the keyed answer →"));
        byId("quiz-keybtn").addEventListener("click", () => { S.phase = "verdict"; render(); });
      } else if (S.phase === "verdict") {
        const row = qz.results[qz.results.length - 1];
        stage.appendChild(h("div", { class: "quiz-key" },
          h("span", { class: "quiz-q-meta" }, "the deck's keyed answer"),
          h("div", { class: "quiz-q-text" }, q.options[q.key])));
        const engineRight = row.pick === q.key;
        stage.appendChild(h("p", { class: "game-note" },
          engineRight ? "The engine's pick matches the key." : "The engine's pick does NOT match the key."));
        stage.appendChild(h("div", { class: "quiz-verdicts" },
          h("button", { class: "mz-chip", id: "quiz-agree", type: "button" }, "verdict: the engine had it right (Y)"),
          h("button", { class: "mz-chip", id: "quiz-disagree", type: "button" }, "verdict: the engine had it wrong (N)")));
        byId("quiz-agree").addEventListener("click", () => auditVerdict(true));
        byId("quiz-disagree").addEventListener("click", () => auditVerdict(false));
      } else if (S.phase === "judged") {
        const v = qz.results[qz.results.length - 1];
        stage.appendChild(h("p", { class: "quiz-judged" },
          "verdict recorded: " + (v.agreed ? "engine right" : "engine wrong") +
          " — your judgment " + (v.correct ? "matched the key" : "did NOT match the key")));
        stage.appendChild(h("button", { class: "mz-chip quiz-nextbtn", id: "quiz-nextbtn", type: "button" }, "next question →"));
        byId("quiz-nextbtn").addEventListener("click", advance);
      }
    } else if (mode === "compete") {
      renderCompete(stage, q);
    }
    renderUnder();
  }

  function auditVerdict(agree) {
    const v = QC.verdictAudit(S.engineQz, agree);
    S.phase = "judged";
    if (v) S.meters.count("audit.verdicts");
    render();
  }

  function advance() {
    const qz = S.engineQz;
    QC.next(qz);
    S.phase = "idle";
    S.lastRow = null;
    if (S.cfg.mode === "speed" && !QC.finished(qz)) stepSpeed();
    else if (QC.finished(qz) && S.cfg.mode === "speed") finishRun("engine");
    render();
  }

  /* compete: you first (timed), then the engine gets the same question */
  function renderCompete(stage, q) {
    stage.appendChild(questionCard(S.youQz, q, S.phase === "idle"));
    if (S.phase === "idle") {
      S.askedAt = DK.nowMs();
      stage.appendChild(h("p", { class: "game-note" }, "your answer first — click an option or press 1–" + q.options.length));
    } else if (S.phase === "asking") {
      stage.appendChild(h("p", { class: "game-note" }, "your answer is in — asking the engine the same question…"));
    } else if (S.phase === "shown" && S.youQz.results.length && S.engineQz.results.length) {
      const yourRow = S.youQz.results[S.youQz.results.length - 1];
      const engRow = S.engineQz.results[S.engineQz.results.length - 1];
      stage.appendChild(h("div", { class: "quiz-answer" },
        h("div", { class: "quiz-a-head" },
          h("span", null, "you said: "), h("strong", null, yourRow.pickLabel), " ",
          h("span", { class: "quiz-tag " + (yourRow.correct ? "right" : "wrong") },
            yourRow.correct ? "right" : "wrong"),
          h("span", { class: "quiz-conf" }, " · " + fmtMs(yourRow.ms)))));
      stage.appendChild(answerPanel(engRow, q, true));
      stage.appendChild(h("p", { class: "game-note" },
        "keyed answer: " + q.options[q.key] + " — you " +
        (yourRow.correct ? "1" : "0") + ", engine " + (engRow.correct ? "1" : "0") + "."));
      stage.appendChild(h("button", { class: "mz-chip quiz-nextbtn", id: "quiz-nextbtn", type: "button" }, "next question →"));
      byId("quiz-nextbtn").addEventListener("click", advance);
    }
  }

  async function answerYou(idx) {
    if (S.cfg.mode !== "compete" || S.phase !== "idle") return;
    const ms = DK.nowMs() - S.askedAt;
    QC.answerHuman(S.youQz, idx, ms);
    S.meters.sample("you.ms", ms);
    S.phase = "asking";                 /* engine row doesn't exist yet */
    render();
    await askWhenReady(S.engineQz);
    S.phase = "shown";
    render();
  }

  function renderCompeteDone(stage) {
    const sy = QC.scoreSummary(S.youQz, "you");
    const se = QC.scoreSummary(S.engineQz, "engine");
    renderDone(stage, S.youQz, "you", [
      h("p", { class: "quiz-vs" },
        "you " + pct(sy.accuracy) + " (" + sy.correct + "/" + sy.answered + " answered, median " +
        fmtMs(sy.medianMs) + ")  vs  engine " + pct(se.accuracy) + " (" +
        se.correct + "/" + se.answered + " answered, " + se.abstained + " abstained, median " +
        fmtMs(se.medianMs) + ")")]);
  }

  const pct = (a) => (a === null ? "—" : Math.round(a * 100) + "%");

  function renderDone(stage, qz, who, extra) {
    const s = QC.scoreSummary(qz, who);
    stage.appendChild(h("div", { class: "quiz-done" },
      h("div", { class: "quiz-q-text" }, "run complete — " + MODE_LABEL[S.cfg.mode]),
      h("p", { class: "game-note" },
        s.asked + " questions · " + s.answered + " answered · " + s.correct + " right · " +
        s.wrong + " wrong · " + s.abstained + " abstained" +
        (s.answered ? " · accuracy " + pct(s.accuracy) + " over answered" : "") +
        " · best streak " + s.bestStreak + " · median " + fmtMs(s.medianMs)),
      ...(extra || []),
      h("button", { class: "mz-chip quiz-nextbtn", id: "quiz-againbtn", type: "button" }, "↺ new run (R)")));
    byId("quiz-againbtn").addEventListener("click", newRun);
  }

  function renderUnder() {
    const under = byId("quiz-under");
    if (!under) return;
    const qz = S.engineQz;
    if (!qz) return;
    const se = QC.scoreSummary(qz, "engine");
    const sy = S.youQz ? QC.scoreSummary(S.youQz, "you") : null;
    const row = S.lastRow || qz.results[qz.results.length - 1] || null;
    const em = S.meters.get("engine.ms");
    under.innerHTML = "";
    under.appendChild(DK.shell.kvRow([
      ["mode", MODE_LABEL[S.cfg.mode]],
      ["question", QC.finished(qz) ? "done" : (qz.i + 1) + "/" + qz.order.length],
      ["deck", S.deck.name.length > 26 ? S.deck.name.slice(0, 25) + "…" : S.deck.name],
      ["engine pick", row ? (row.pick === null ? "—" : row.pickLabel) : "—"],
      ["engine score", se.correct + "/" + se.answered + (se.abstained ? " (" + se.abstained + " abst)" : "")],
      ...(sy ? [["you score", sy.correct + "/" + sy.answered]] : []),
      ["engine latency", em ? fmtMs(em.ema) + " ema" : "—"],
    ]));
    if (row) {
      byId("quiz-hint").textContent = "last engine request: " + row.qid + " · " + row.outcome +
        (row.pick !== null ? " → " + row.pickLabel + (row.correct === null ? "" : row.correct ? " (right)" : " (wrong)") : "") +
        " · " + fmtMs(row.ms) + " · full trace in audit mode";
    }
  }

  /* ---------- overlays: board, stats, deck editor ---------- */

  function renderBoard() {
    const body = byId("quiz-board-body");
    if (!body) return;
    body.textContent = "";
    const rows = S.board.all();
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" },
        "No finished runs yet — answer through a mode and the run lands here. ",
        "Session only: close the tab and it is gone."));
      return;
    }
    body.appendChild(h("table", { class: "game-table" },
      h("thead", null, h("tr", null,
        ...["#", "mode", "who", "deck", "asked", "accuracy", "abstained", "median", "seed"].map((x) => h("th", null, x)))),
      h("tbody", null, rows.map((r, i) => h("tr", null,
        h("td", null, String(rows.length - i)),
        h("td", null, r.mode),
        h("td", null, r.who),
        h("td", null, r.deck),
        h("td", null, String(r.asked)),
        h("td", null, r.accuracy + (r.vsEngine ? " vs " + r.vsEngine : "")),
        h("td", null, String(r.abstained)),
        h("td", null, r.median),
        h("td", null, r.seed))))));
  }

  function renderStats() {
    const body = byId("quiz-stats-body");
    if (!body) return;
    body.textContent = "";
    const snap = S.meters.snapshot();
    const em = snap["engine.ms"];
    const rows = [
      ["engine latency", em ? fmtMs(em.avg) + " ema · " + fmtMs(em.worst) + " worst · " + em.count + " calls" : "no calls yet"],
      ["your answer time", snap["you.ms"] ? fmtMs(snap["you.ms"].avg) + " ema" : "—"],
      ["questions answered by engine", String((snap["questions.accept"] || {}).count || 0)],
      ["engine abstentions", String((snap["questions.abstain"] || {}).count || 0)],
      ["engine unavailable", String((snap["questions.unavailable"] || {}).count || 0)],
      ["audit verdicts", String((snap["audit.verdicts"] || {}).count || 0)],
      ["deck", S.deck.name + " · " + S.deck.questions.length + " questions"],
      ["engine status", engine.status],
    ];
    body.appendChild(h("dl", { class: "pg-kv" },
      ...rows.flatMap(([k, v]) => [h("dt", null, k), h("dd", null, v)])));
    if (S.engineQz) {
      const se = QC.scoreSummary(S.engineQz, "engine");
      body.appendChild(h("p", { class: "game-note" },
        "this session (engine): " + se.correct + "/" + se.answered + " right over answered" +
        (se.abstained ? ", " + se.abstained + " abstained" : "") +
        " — accuracy over ANSWERED questions; abstentions are counted, never upscored."));
    }
  }

  const TEMPLATE = {
    name: "My deck",
    questions: [
      { text: "Which option names a primary colour?", options: ["crimson", "orange", "blue", "teal"], key: 2 },
      { text: "2 + 2 equals what?", options: ["3", "4"], key: 1 },
    ],
  };

  function renderDeck() {
    const body = byId("quiz-deck-body");
    if (!body || body.dataset.built) return;
    body.dataset.built = "1";
    body.appendChild(h("p", { class: "game-note" },
      "The deck is JSON: { name, questions: [{ text, options, key }] } — 2 to 6 options ",
      "per question, exactly one keyed by index, non-empty text. Edit to add, ",
      "change, or delete questions; validate, then load it into the session. ",
      "Saving writes to THIS browser's localStorage only, when you click save — ",
      "nothing is sent anywhere (the site has no server)."));
    const txt = h("textarea", { id: "quiz-deck-text", class: "quiz-deck-text",
      "aria-label": "deck JSON", spellcheck: "false" });
    txt.value = QC.deckToJSON(S.deck);
    const msg = h("p", { class: "game-note", id: "quiz-deck-msg" }, "deck: " + S.deck.name +
      " · " + S.deck.questions.length + " questions");
    const btn = (id, label) => h("button", { class: "mz-chip", id, type: "button" }, label);
    body.appendChild(h("div", { class: "quiz-deck-actions" },
      btn("quiz-deck-validate", "validate"),
      btn("quiz-deck-load", "load into session"),
      btn("quiz-deck-export", "export current deck"),
      btn("quiz-deck-template", "insert template"),
      btn("quiz-deck-save", "save to this browser"),
      btn("quiz-deck-restore", "load saved"),
      btn("quiz-deck-builtin", "restore built-in")));
    body.appendChild(txt);
    body.appendChild(msg);

    const parsed = () => QC.deckFromJSON(txt.value);
    const say = (t) => { msg.textContent = t; };

    byId("quiz-deck-validate").addEventListener("click", () => {
      const r = parsed();
      say(r.ok ? "valid — " + r.deck.questions.length + " questions"
        : "INVALID: " + r.errors.join(" · "));
    });
    byId("quiz-deck-load").addEventListener("click", () => {
      const r = parsed();
      if (!r.ok) { say("not loaded — INVALID: " + r.errors.join(" · ")); return; }
      S.deck = r.deck;
      txt.value = QC.deckToJSON(S.deck);
      say("loaded — " + S.deck.name + " · " + S.deck.questions.length + " questions; new run started");
      newRun();
    });
    byId("quiz-deck-export").addEventListener("click", () => {
      txt.value = QC.deckToJSON(S.deck);
      say("exported the current deck into the editor");
    });
    byId("quiz-deck-template").addEventListener("click", () => {
      txt.value = JSON.stringify(TEMPLATE, null, 2);
      say("template inserted — edit, validate, load");
    });
    byId("quiz-deck-save").addEventListener("click", () => {
      const r = parsed();
      if (!r.ok) { say("not saved — INVALID: " + r.errors.join(" · ")); return; }
      const ok = QC.quizSaveDeck(r.deck);
      say(ok ? "saved to this browser only (localStorage key " + QC.DECK_KEY + ")"
        : "storage unavailable in this browser — nothing saved");
    });
    byId("quiz-deck-restore").addEventListener("click", () => {
      const d = QC.quizLoadDeck();
      if (!d) { say("no valid saved deck in this browser"); return; }
      S.deck = d;
      txt.value = QC.deckToJSON(d);
      say("loaded the saved deck — " + d.name + " · " + d.questions.length + " questions; new run started");
      newRun();
    });
    byId("quiz-deck-builtin").addEventListener("click", () => {
      S.deck = QC.BUILT_IN_DECK;
      txt.value = QC.deckToJSON(S.deck);
      say("built-in deck restored; new run started");
      newRun();
    });
  }

  /* ---------- wiring ---------- */

  function wire() {
    byId("quiz-restart").addEventListener("click", newRun);
    byId("quiz-seed").addEventListener("change", newRun);
    byId("quiz-mode").addEventListener("change", newRun);
    byId("quiz-next").addEventListener("click", () => {
      if (S.cfg.mode === "speed") advance();
      else if (S.cfg.mode === "audit") {
        if (S.phase === "idle" && byId("quiz-askbtn")) byId("quiz-askbtn").click();
        else if (S.phase === "revealed" && byId("quiz-keybtn")) byId("quiz-keybtn").click();
        else if (S.phase === "verdict") auditVerdict(true);
        else if (S.phase === "judged" && byId("quiz-nextbtn")) byId("quiz-nextbtn").click();
      } else if (S.cfg.mode === "compete" && byId("quiz-nextbtn")) byId("quiz-nextbtn").click();
    });
    byId("quiz-deck-btn").addEventListener("click", () => {
      S.deckOpen = true;
      byId("quiz-deck-overlay").classList.add("open");
      renderDeck();
    });
    byId("quiz-deck-close").addEventListener("click", () => {
      S.deckOpen = false;
      byId("quiz-deck-overlay").classList.remove("open");
    });
    byId("quiz-engine-toggle").addEventListener("click", (e) => {
      const on = engine.status === "off" || engine.status === "unavailable";
      engine.setEnabled(on);
      e.currentTarget.textContent = on ? "engine: on" : "engine: off";
    });
    byId("quiz-board-btn").addEventListener("click", () => {
      S.boardOpen = true; S.statsOpen = false;
      byId("quiz-stats-overlay").classList.remove("open");
      byId("quiz-board-overlay").classList.add("open");
      renderBoard();
    });
    byId("quiz-board-close").addEventListener("click", () => {
      S.boardOpen = false;
      byId("quiz-board-overlay").classList.remove("open");
    });
    byId("quiz-stats-btn").addEventListener("click", () => {
      S.statsOpen = true; S.boardOpen = false;
      byId("quiz-board-overlay").classList.remove("open");
      byId("quiz-stats-overlay").classList.add("open");
      renderStats();
    });
    byId("quiz-stats-close").addEventListener("click", () => {
      S.statsOpen = false;
      byId("quiz-stats-overlay").classList.remove("open");
    });

    window.addEventListener("keydown", (e) => {
      if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
      const qz = S.engineQz;
      if (e.key === "Escape") {
        S.boardOpen = false; S.statsOpen = false; S.deckOpen = false;
        for (const id of ["quiz-board-overlay", "quiz-stats-overlay", "quiz-deck-overlay"]) {
          byId(id).classList.remove("open");
        }
        return;
      }
      if (/^[1-6]$/.test(e.key) && S.cfg.mode === "compete" && S.phase === "idle" && qz && !QC.finished(qz)) {
        const q = QC.current(qz);
        const idx = Number(e.key) - 1;
        if (idx < q.options.length) { answerYou(idx); e.preventDefault(); }
      } else if ((e.key === "y" || e.key === "Y") && S.cfg.mode === "audit" && S.phase === "verdict") {
        auditVerdict(true); e.preventDefault();
      } else if ((e.key === "n" || e.key === "N") && S.cfg.mode === "audit" && S.phase === "verdict") {
        auditVerdict(false); e.preventDefault();
      } else if ((e.key === "n" || e.key === "N") && byId("quiz-nextbtn")) {
        byId("quiz-nextbtn").click(); e.preventDefault();
      } else if (e.key === "r" || e.key === "R") {
        newRun(); e.preventDefault();
      }
    });

    setChip();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", build);
  } else {
    build();
  }
})();
