/* OpenCodifier — Ask the Engine, core (DOM-free).
   §5.6 of the Decisions-SDK plan: a multiple-choice quiz where EVERY engine
   answer is a real choice request over the option candidates — no local
   simulation dressed up as engine output. This module owns:

   - the deck model: { name, questions: [{ text, options, key }] }, with
     validation (2–6 options, exactly one keyed, non-empty text). The deck
     carries the keyed answers; the engine's pick is displayed next to them
     and agreement is scored honestly — the engine CAN be wrong;
   - the wire contract: every request carries state {text, facts:{}},
     question {text, type:"choice", candidates:[{id, description}]},
     policy. A bare {questions} request makes the WASM decide() throw
     (pinned finding from the racer — see FINDINGS.md);
   - grading + scoring: accuracy over ANSWERED questions, abstain rate
     (abstention is an outcome, not an error — it is never scored wrong),
     streaks, median answer time;
   - deterministic seeded question order so sessions replay;
   - deck import/export round-trip and the EXPLICIT opt-in localStorage
     save (the one planned exception to session-only data — labelled
     local-only in the shell).

   The shell (quiz.js) renders; this file never touches the DOM. */

(function () {
  "use strict";
  const DK = globalThis.DecisionsSDK;

  const MIN_OPTIONS = 2;
  const MAX_OPTIONS = 6;
  const MODES = ["speed", "audit", "compete"];
  const POLICY = { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" };
  const DEFAULT_CONFIG = { seed: "oc-quiz", mode: "speed" };

  const optionId = (i) => "option-" + i;
  const optionIndexOf = (id) => {
    if (typeof id !== "string" || id.indexOf("option-") !== 0) return -1;
    const n = Number(id.slice("option-".length));
    return Number.isInteger(n) && n >= 0 ? n : -1;
  };

  /* ---------- config ---------- */

  function normalizeConfig(raw) {
    const c = raw || {};
    const mode = MODES.indexOf(c.mode) !== -1 ? c.mode : DEFAULT_CONFIG.mode;
    return {
      seed: c.seed === undefined || c.seed === null ? DEFAULT_CONFIG.seed : String(c.seed),
      mode,
    };
  }

  const deckStamp = (deck, cfg) =>
    "quiz|" + (deck ? deck.name : "?") + "|" + (deck ? deck.questions.length : 0) +
    "|" + cfg.seed + "|" + cfg.mode;

  /* ---------- deck: validation, normalization, import/export ---------- */

  /* every problem the editor can produce, named — the shell shows these
     strings verbatim next to the invalid JSON */
  function validateDeck(deck) {
    const errors = [];
    if (!deck || typeof deck !== "object" || Array.isArray(deck)) {
      return ["deck must be an object like { name, questions: [...] }"];
    }
    if (typeof deck.name !== "string" || !deck.name.trim()) errors.push("name must be a non-empty string");
    if (!Array.isArray(deck.questions) || deck.questions.length === 0) {
      errors.push("questions must be a non-empty array");
      return errors;
    }
    deck.questions.forEach((q, i) => {
      const at = "question " + (i + 1) + ": ";
      if (!q || typeof q !== "object") { errors.push(at + "not an object"); return; }
      if (typeof q.text !== "string" || !q.text.trim()) errors.push(at + "text is empty");
      if (!Array.isArray(q.options)) { errors.push(at + "options missing"); return; }
      if (q.options.length < MIN_OPTIONS) errors.push(at + "needs at least " + MIN_OPTIONS + " options");
      if (q.options.length > MAX_OPTIONS) errors.push(at + "allows at most " + MAX_OPTIONS + " options");
      if (q.options.some((o) => typeof o !== "string" || !o.trim())) {
        errors.push(at + "every option needs non-empty text");
      }
      if (!Number.isInteger(q.key)) errors.push(at + "key must be an integer index");
      else if (q.key < 0 || q.key >= q.options.length) errors.push(at + "key out of range");
    });
    return errors;
  }

  /* validated decks get stable ids q1..qn and trimmed text; ids (not
     indexes) travel through results so a re-shuffle cannot mislabel them */
  function normalizeDeck(raw) {
    const errors = validateDeck(raw);
    if (errors.length) return { ok: false, errors };
    return {
      ok: true, errors: [],
      deck: {
        name: raw.name.trim(),
        questions: raw.questions.map((q, i) => ({
          id: "q" + (i + 1),
          text: q.text.trim(),
          options: q.options.map((o) => o.trim()),
          key: q.key,
        })),
      },
    };
  }

  function deckToJSON(deck) {
    return JSON.stringify({
      name: deck.name,
      questions: deck.questions.map((q) => ({ text: q.text, options: q.options, key: q.key })),
    }, null, 2);
  }

  /* parse → validate in one step; the editor's only entry point for
     imported text (and the save file, which is the same format) */
  function deckFromJSON(text) {
    let raw = null;
    try { raw = JSON.parse(text); } catch (e) { return { ok: false, errors: ["not valid JSON: " + e.message] }; }
    return normalizeDeck(raw);
  }

  /* explicit opt-in persistence — the ONLY planned localStorage use beyond
     the session board, labelled local-only in the shell. Storage is
     injected so tests run without a browser. */
  function defaultStorage() {
    return typeof localStorage !== "undefined" ? localStorage : null;
  }
  const DECK_KEY = "oc-quiz-deck";
  function quizSaveDeck(deck, storage) {
    const st = storage || defaultStorage();
    if (!st) return false;
    try { st.setItem(DECK_KEY, deckToJSON(deck)); return true; } catch (e) { return false; }
  }
  function quizLoadDeck(storage) {
    const st = storage || defaultStorage();
    if (!st) return null;
    let text = null;
    try { text = st.getItem(DECK_KEY); } catch (e) { return null; }
    if (!text) return null;
    const parsed = deckFromJSON(text);
    return parsed.ok ? parsed.deck : null;
  }

  /* ---------- the built-in deck ----------
     General knowledge + logic puzzles sized for the engine's lexical rungs
     (zero-ML). Includes cognitive-reflection traps (bat-and-ball) and pure
     arithmetic — the engine's misses and abstentions ARE part of the demo;
     the scoreboard shows them without cosmetic upscoring. */

  const BUILT_IN_DECK = normalizeDeck({
    name: "Built-in: knowledge & logic",
    questions: [
      { text: "What is the capital of France?", options: ["Paris", "Lyon", "Marseille", "Bordeaux"], key: 0 },
      { text: "Which number is prime?", options: ["15", "21", "17", "27"], key: 2 },
      { text: "What is 7 × 8?", options: ["54", "56", "48", "64"], key: 1 },
      { text: "If all bloops are razzies and all razzies are lazzies, then all bloops are definitely what?",
        options: ["lazzies", "razzies", "neither", "cannot be determined"], key: 0 },
      { text: "Which planet is known as the Red Planet?", options: ["Venus", "Mars", "Jupiter", "Mercury"], key: 1 },
      { text: "What comes next in the sequence 2, 4, 8, 16?", options: ["24", "20", "32", "18"], key: 2 },
      { text: "Which word is a synonym of \"rapid\"?", options: ["slow", "swift", "heavy", "loud"], key: 1 },
      { text: "How many legs does a spider have?", options: ["6", "8", "10", "4"], key: 1 },
      { text: "Which of these is a citrus fruit?", options: ["lemon", "potato", "onion", "apple"], key: 0 },
      { text: "A bat and a ball cost $1.10 in total. The bat costs $1.00 more than the ball. How much does the ball cost?",
        options: ["$0.10", "$0.05", "$0.15", "$1.10"], key: 1 },
      { text: "Which language has the most native speakers worldwide?",
        options: ["English", "Spanish", "Mandarin Chinese", "Hindi"], key: 2 },
      { text: "At what temperature does water boil at sea level?", options: ["90 °C", "100 °C", "110 °C", "80 °C"], key: 1 },
    ],
  }).deck;

  /* ---------- sessions ---------- */

  /* seeded Fisher-Yates over question indexes: same seed, same order */
  function shuffledOrder(n, seed) {
    const rng = DK.makeRng("order:" + seed);
    const order = Array.from({ length: n }, (_, i) => i);
    for (let i = n - 1; i > 0; i--) {
      const j = Math.floor(rng() * (i + 1));
      const t = order[i]; order[i] = order[j]; order[j] = t;
    }
    return order;
  }

  function makeQuiz(deck, raw) {
    const cfg = normalizeConfig(raw);
    return {
      cfg, deck,
      order: shuffledOrder(deck.questions.length, cfg.seed),
      i: 0,                 /* index into order */
      results: [],          /* every graded answer, newest last */
    };
  }

  const current = (qz) => qz.deck.questions[qz.order[qz.i]];
  const finished = (qz) => qz.i >= qz.order.length;

  /* the full wire contract — the same shape the server runtime speaks */
  function buildRequest(q) {
    return {
      state: {
        text: "Quiz question " + q.id + ": " + q.text +
          " Pick the single best option from the candidates.",
        facts: {},
      },
      questions: [{
        type: "choice",
        id: "quiz-" + q.id,
        text: q.text,
        candidates: q.options.map((o, i) => ({ id: optionId(i), description: o })),
      }],
      policy: POLICY,
    };
  }

  /* grade one raw bridge result against the keyed deck. outcome:
     - "accept" — the engine picked a candidate; correct is a real boolean
     - "abstain" — the runtime refused to guess (never scored wrong)
     - "unavailable" — bridge down / no parseable answer (as abstain, but
       labelled so the stats overlay can show engine health honestly) */
  function grade(raw, q) {
    const base = { pick: null, correct: null, confidence: null, outcome: "unavailable", raw };
    if (!raw || !Array.isArray(raw.answers)) return base;
    const ans = raw.answers[0];
    const outcome = raw.outcome;
    if (outcome !== "accept" || !ans || typeof ans.choice !== "string") {
      return Object.assign(base, { outcome: outcome === "abstain" ? "abstain" : "unavailable" });
    }
    const idx = optionIndexOf(ans.choice);
    if (idx < 0 || idx >= q.options.length) {
      return Object.assign(base, { outcome: "unavailable" });   /* not a candidate we offered */
    }
    return {
      pick: idx,
      correct: idx === q.key,
      confidence: typeof ans.confidence === "number" ? ans.confidence : null,
      outcome: "accept",
      raw,
    };
  }

  function record(qz, q, who, graded, ms) {
    const row = {
      qid: q.id, who,
      pick: graded.pick,
      pickLabel: graded.pick === null ? null : q.options[graded.pick],
      key: q.key, keyLabel: q.options[q.key],
      correct: graded.correct, confidence: graded.confidence,
      outcome: graded.outcome, ms: ms === undefined ? null : ms,
      raw: graded.raw,
    };
    qz.results.push(row);
    return row;
  }

  /* the engine's turn: one real bridge call, measured */
  function askEngine(qz, bridge, msNow) {
    const q = current(qz);
    const t0 = DK.nowMs();
    const raw = bridge ? bridge.choose(buildRequest(q)) : null;
    const ms = DK.nowMs() - t0;
    const row = record(qz, q, "engine", grade(raw, q), ms);
    if (typeof msNow === "function") msNow(row);
    return row;
  }

  /* the human's turn (compete): pick an option index, timed by the shell */
  function answerHuman(qz, optionIdx, ms) {
    const q = current(qz);
    const graded = optionIdx === q.key
      ? { pick: optionIdx, correct: true, confidence: 1, outcome: "accept", raw: null }
      : { pick: optionIdx, correct: false, confidence: 1, outcome: "accept", raw: null };
    return record(qz, q, "you", graded, ms === undefined ? null : ms);
  }

  /* audit mode: the human's verdict on the engine's answer, made AFTER the
     key is shown — agree means "the engine had it right" */
  function verdictAudit(qz, agree) {
    /* only an actual answer is judgeable — an abstention has no pick to
       agree or disagree with */
    const lastEngine = [...qz.results].reverse().find(
      (r) => r.who === "engine" && r.outcome === "accept" && r.pick !== null);
    if (!lastEngine) return null;
    const row = {
      qid: lastEngine.qid, who: "audit",
      pick: lastEngine.pick, pickLabel: lastEngine.pickLabel,
      key: lastEngine.key, keyLabel: lastEngine.keyLabel,
      /* the audit row's "correct" is the HUMAN's judgment accuracy: did
         their agree/disagree verdict match the keyed truth */
      correct: agree === (lastEngine.pick === lastEngine.key),
      agreed: agree, engineCorrect: lastEngine.pick === lastEngine.key,
      confidence: lastEngine.confidence, outcome: lastEngine.outcome,
      ms: null, raw: null,
    };
    qz.results.push(row);
    return row;
  }

  const next = (qz) => { if (!finished(qz)) qz.i += 1; };

  /* ---------- scoring ---------- */

  function median(xs) {
    const v = xs.filter((x) => typeof x === "number" && isFinite(x)).slice().sort((a, b) => a - b);
    if (!v.length) return null;
    const mid = Math.floor(v.length / 2);
    return v.length % 2 ? v[mid] : (v[mid - 1] + v[mid]) / 2;
  }

  function scoreSummary(qz, who) {
    const rows = qz.results.filter((r) => r.who === who);
    const answered = rows.filter((r) => r.outcome === "accept" && r.pick !== null);
    const correct = answered.filter((r) => r.correct === true).length;
    let streak = 0, best = 0;
    for (const r of rows) {
      if (r.outcome === "accept" && r.pick !== null) {
        streak = r.correct ? streak + 1 : 0;
        if (streak > best) best = streak;
      }
    }
    return {
      asked: rows.length,
      answered: answered.length,
      correct,
      wrong: answered.length - correct,
      abstained: rows.length - answered.length,
      accuracy: answered.length ? correct / answered.length : null,
      abstainRate: rows.length ? (rows.length - answered.length) / rows.length : null,
      bestStreak: best,
      medianMs: median(rows.map((r) => r.ms)),
    };
  }

  globalThis.QuizCore = {
    MIN_OPTIONS, MAX_OPTIONS, MODES, POLICY, DEFAULT_CONFIG, DECK_KEY,
    BUILT_IN_DECK,
    optionId, optionIndexOf,
    normalizeConfig, deckStamp,
    validateDeck, normalizeDeck, deckToJSON, deckFromJSON,
    quizSaveDeck, quizLoadDeck,
    makeQuiz, current, finished, buildRequest, grade,
    record, askEngine, answerHuman, verdictAudit, next,
    median, scoreSummary,
  };
})();
