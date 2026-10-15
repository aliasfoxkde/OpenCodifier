/* RPSLS++ — core.
   Rock-paper-scissors-lizard-spock on an EXTENDABLE move graph: every
   pair of moves resolves to exactly one winner (a tournament graph),
   the base five sit on the classic cycle, and a new move may join only
   by beating exactly (n-1)/2 of an odd-sized roster — the validator
   refuses anything else, with a reason. The opponent is a pattern
   hunter: frequency and markov-1 tables over your move history feed an
   expected-value computation, four predictors (freq, markov, mirror,
   random) are scored against what actually happened, and the bot plays
   the EV argmax. When the top two EVs land inside the tie band the
   close call escalates to the engine over the full wire contract (the
   history table rides in state.text); engine off, out of tokens, or a
   refusal → the same EV argmax decides as a rule, on the record. In
   bot-vs-bot mode both brains are rules — a pure watch mode with no
   engine lane. Seeded and deterministic for any fixed move sequence.

   The shell (rpsls.js) renders; this file never touches the DOM. */

(function () {
  "use strict";
  const DK = globalThis.DecisionsSDK;

  const LOG_MAX = 200;
  const META_DECAY = 0.9;         /* predictor score decay per round */
  const EXPLORE = 0.05;           /* seeded deviation from the argmax */
  const MARKOV_MIN = 3;           /* opponent moves before markov votes */

  /* the classic cycle: each move beats the next two, loses to the
     previous two — a regular tournament on 5 vertices */
  const BASE_MOVES = [
    { id: "rock", label: "Rock", glyph: "✊" },
    { id: "paper", label: "Paper", glyph: "✋" },
    { id: "scissors", label: "Scissors", glyph: "✌" },
    { id: "lizard", label: "Lizard", glyph: "🦎" },
    { id: "spock", label: "Spock", glyph: "🖖" },
  ];

  const PREDICTORS = ["freq", "markov", "mirror", "random"];
  const PREDICTOR_NAME = {
    freq: "FREQUENCY", markov: "MARKOV-1", mirror: "MIRROR", random: "RANDOM",
  };

  const DEFAULT_CONFIG = {
    seed: "rpsls-hunter", mode: "you",          /* "you" | "bots" */
    botRounds: 60, askBudget: 6, band: 0.15,
  };

  /* ---------- config + stamp ---------- */

  function normalizeConfig(raw) {
    const c = raw || {};
    const num = (v, lo, hi, dflt) => (v === undefined || v === null
      || Number.isNaN(Number(v))) ? dflt : DK.clampf(Number(v), lo, hi, dflt);
    return {
      seed: c.seed === undefined || c.seed === null ? DEFAULT_CONFIG.seed : String(c.seed),
      mode: c.mode === "bots" ? "bots" : "you",
      botRounds: Math.round(num(c.botRounds, 10, 500, DEFAULT_CONFIG.botRounds)),
      askBudget: Math.round(num(c.askBudget, 0, 30, DEFAULT_CONFIG.askBudget)),
      band: num(c.band, 0.01, 1, DEFAULT_CONFIG.band),
    };
  }

  const rpslsStamp = (cfg) => "rpsls|" + cfg.seed + "|" + cfg.mode
    + "|" + cfg.botRounds + "|" + cfg.askBudget + "|" + cfg.band;

  /* ---------- graph ---------- */

  /* rules map: "a|b" (sorted ids) -> winner id. Built from `beats` sets. */
  function buildRules(moves) {
    const rules = {};
    for (let i = 0; i < moves.length; i++) {
      for (let j = i + 1; j < moves.length; j++) {
        const a = moves[i].id, b = moves[j].id;
        const aBeats = (moves[i].beats || []).includes(b);
        const bBeats = (moves[j].beats || []).includes(a);
        const key = a < b ? a + "|" + b : b + "|" + a;
        rules[key] = aBeats && !bBeats ? a : bBeats && !aBeats ? b : null;
      }
    }
    return rules;
  }

  /* every pair must resolve to exactly one winner; fairness = every
     move beats the same count it loses to. Returns {ok, reason}. */
  function validateGraph(moves) {
    const ids = moves.map((m) => m.id);
    if (new Set(ids).size !== ids.length) return { ok: false, reason: "duplicate move id" };
    for (const m of moves) {
      if (!m.beats || !m.beats.length) return { ok: false, reason: m.id + " beats nothing" };
      for (const b of m.beats) {
        if (b === m.id) return { ok: false, reason: m.id + " beats itself" };
        if (!ids.includes(b)) return { ok: false, reason: m.id + " beats unknown move " + b };
      }
    }
    for (const key of Object.keys(buildRules(moves))) {
      if (buildRules(moves)[key] === null) {
        return { ok: false, reason: key + " does not resolve (both or neither beat each other)" };
      }
    }
    const n = ids.length;
    const maxDiff = n % 2 === 1 ? 0 : 1;   /* exact regularity is only
        achievable on odd counts; even counts allow +-1 */
    for (const m of moves) {
      const wins = m.beats.length;
      const losses = ids.filter((x) => x !== m.id
        && (moves.find((o) => o.id === x).beats || []).includes(m.id)).length;
      if (Math.abs(wins - losses) > maxDiff) {
        return { ok: false, reason: m.id + " beats " + wins + " but loses to " + losses
          + " — too unfair for a " + n + "-move roster" };
      }
    }
    return { ok: true, reason: "every pair resolves, all moves fair" };
  }

  const baseGraph = () => BASE_MOVES.map((m, i) => ({
    id: m.id, label: m.label, glyph: m.glyph,
    beats: [(i + 1) % 5, (i + 2) % 5].map((k) => BASE_MOVES[k].id),
  }));

  /* a fair add: the roster must be odd, the newcomer beats exactly
     floor(n/2) of it and loses to the rest — the graph is REORIENTED,
     with the winners gaining the newcomer in their own beats, and the
     validator sees the full rewritten graph */
  function reoriented(moves, spec) {
    return moves
      .map((m) => (spec.beats.includes(m.id) ? m
        : Object.assign({}, m, { beats: m.beats.concat([spec.id]) })))
      .concat([{ id: spec.id, beats: spec.beats.slice() }]);
  }

  function checkAdd(moves, spec) {
    const n = moves.length;
    if (n % 2 === 0) {
      return { ok: false, reason: "the roster is even (" + n + ") — a fair graph needs an odd count" };
    }
    const k = Math.floor(n / 2);
    if (!spec.id || /\s/.test(spec.id)) return { ok: false, reason: "the move needs a one-word id" };
    if (moves.some((m) => m.id === spec.id)) return { ok: false, reason: spec.id + " already exists" };
    if (!Array.isArray(spec.beats) || spec.beats.length !== k) {
      return { ok: false, reason: "a fair add beats exactly " + k + " of the " + n + " existing moves" };
    }
    if (spec.beats.some((b) => !moves.some((m) => m.id === b))) {
      return { ok: false, reason: "the beats list names an unknown move" };
    }
    return validateGraph(reoriented(moves, spec));
  }

  /* ---------- resolution ---------- */

  function result(rules, mine, theirs) {
    if (mine === theirs) return "tie";
    const key = mine < theirs ? mine + "|" + theirs : theirs + "|" + mine;
    return rules[key] === mine ? "win" : "lose";
  }

  /* ---------- predictions + EV ---------- */

  /* the bot's model of the OPPONENT's next move, as a distribution */
  function predict(oppHistory, rng) {
    const p = {};
    for (const m of oppHistory) p[m] = (p[m] || 0) + 1;
    const freq = Object.assign({}, p);
    const markov = {};
    if (oppHistory.length >= MARKOV_MIN) {
      const last = oppHistory[oppHistory.length - 1];
      for (let i = 0; i + 1 < oppHistory.length; i++) {
        if (oppHistory[i] === last) markov[oppHistory[i + 1]] = (markov[oppHistory[i + 1]] || 0) + 1;
      }
    }
    const normalize = (d) => {
      const tot = Object.values(d).reduce((s, x) => s + x, 0);
      const out = {};
      for (const k of Object.keys(d)) out[k] = d[k] / tot;
      return out;
    };
    return {
      freq: Object.keys(freq).length ? normalize(freq) : {},
      markov: Object.keys(markov).length ? normalize(markov) : null,
      mirror: oppHistory.length ? oppHistory[oppHistory.length - 1] : null,
      random: null,            /* the coin speaks at vote time */
    };
  }

  /* each predictor names the move IT would play; scored against what
     actually happened so the meta layer learns who to trust */
  function predictorVotes(g, side, pred) {
    const opp = side === "left" ? g.histories.right : g.histories.left;
    const moves = g.moves.map((m) => m.id);
    const counterOf = (m) => {
      if (!m) return null;
      /* the move that beats m: scan the rules */
      for (const cand of moves) {
        if (cand !== m && result(g.rules, cand, m) === "win") return cand;
      }
      return moves[0];
    };
    const blended = {};
    for (const k of Object.keys(pred.freq)) blended[k] = (blended[k] || 0) + 0.5 * pred.freq[k];
    if (pred.markov) {
      for (const k of Object.keys(pred.markov)) {
        blended[k] = (blended[k] || 0) + 0.5 * pred.markov[k];
      }
    }
    const argmax = (d) => {
      let best = null, bv = -1;
      for (const k of Object.keys(d)) {
        if (d[k] > bv) { bv = d[k]; best = k; }
      }
      return best;
    };
    return {
      freq: counterOf(argmax(pred.freq)),
      markov: pred.markov ? counterOf(argmax(pred.markov)) : null,
      mirror: pred.mirror ? pred.mirror : null,
      random: moves[Math.floor(g.rng() * moves.length)],
      blended,
    };
  }

  /* EV of playing m against a predicted distribution: +1 win, 0 tie, -1 loss */
  function evOf(g, m, dist) {
    let ev = 0;
    for (const k of Object.keys(dist)) {
      ev += dist[k] * (result(g.rules, m, k) === "win" ? 1
        : result(g.rules, m, k) === "tie" ? 0 : -1);
    }
    return ev;
  }

  /* ---------- the bot brain ---------- */

  function botDecide(g, side, engineChoose) {
    const moves = g.moves.map((m) => m.id);
    const opp = side === "left" ? g.histories.right : g.histories.left;
    const pred = predict(opp, g.rng);
    const votes = predictorVotes(g, side, pred);
    /* EV over the blended distribution (frequency + markov when mature);
       deterministic tie-break: first move in roster order */
    const dist = Object.keys(votes.blended).length ? votes.blended
      : Object.fromEntries(moves.map((m) => [m, 1 / moves.length]));
    const evs = {};
    for (const m of moves) evs[m] = +evOf(g, m, dist).toFixed(6);
    const order = moves.slice().sort((x, y) =>
      (evs[y] - evs[x]) || (moves.indexOf(x) - moves.indexOf(y)));
    const best = order[0], second = order[1];
    const gap = +(evs[best] - evs[second]).toFixed(6);
    g.decisions += 1;

    /* a little seeded exploration: the hunter occasionally deviates
       from its own argmax so it cannot be counter-looped into a fixed
       rhythm — this is also where the seed actually enters the match */
    if (g.rng() < EXPLORE) {
      const alt = moves[Math.floor(g.rng() * moves.length)];
      g.rungs.rule += 1;
      g.lastDecision = { side, action: alt, rung: "rule",
        why: "explored — the seeded coin deviated from the argmax (" + best + ")",
        evs, gap, votes };
      return alt;
    }

    /* the escalation lane: a genuinely close call goes on the wire */
    if (gap < g.cfg.band && engineChoose && g.tokens >= 1 && g.cfg.mode === "you") {
      g.tokens -= 1;
      g.engineCalls += 1;
      const table = [];
      const n = Math.min(12, g.histories[side === "left" ? "right" : "left"].length);
      for (let i = g.histories[side === "left" ? "right" : "left"].length - n;
        i < g.histories[side === "left" ? "right" : "left"].length; i++) {
        table.push((side === "left" ? g.histories.right[i] : g.histories.left[i])
          + " vs " + (side === "left" ? g.histories.left[i] : g.histories.right[i]));
      }
      const stateText = "Rock-paper-scissors-lizard-spock on an extendable graph"
        + " (moves: " + moves.join(", ") + "), round " + (g.round + 1)
        + ". I am the " + side + " bot. Score me " + g.scores[side] + ", them "
        + g.scores[side === "left" ? "right" : "left"] + ". Opponent's last "
        + table.length + " moves (theirs vs mine): " + (table.join("; ") || "none yet")
        + ". My predictors read their history as: frequency-counter "
        + (votes.freq || "n/a") + ", markov-counter " + (votes.markov || "n/a")
        + ", mirror " + (votes.mirror || "n/a") + ". Expected values: "
        + moves.map((m) => m + " " + evs[m]).join(", ")
        + ". The top two are " + (gap < g.cfg.band ? "nearly tied" : "separated")
        + " — pick my move.";
      const req = {
        state: { text: stateText, facts: {} },
        questions: [{
          type: "choice", id: "rpsls-move",
          text: "The expected values are nearly tied — which move should the " + side + " bot play?",
          candidates: moves.map((m) => ({
            id: m, description: "play " + m + " (EV " + evs[m] + ")",
          })),
        }],
        policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" },
      };
      const answer = engineChoose(req);
      const ans = answer && answer.outcome === "accept" && Array.isArray(answer.answers)
        ? answer.answers.find((a) => a && typeof a.choice === "string") : null;
      if (ans && moves.includes(ans.choice)) {
        g.rungs.engine += 1;
        g.lastDecision = { side, action: ans.choice, rung: "engine",
          why: "honored — close EVs (" + gap + ") went to the engine", evs, gap, votes };
        return ans.choice;
      }
      /* refusal or malformed: the EV argmax holds, the rung says so */
      g.rungs.rule += 1;
      g.lastDecision = { side, action: best, rung: "rule",
        why: answer && answer.outcome === "abstain"
          ? "engine abstained — the EV argmax holds (" + best + ")"
          : "engine answer named nothing live — the EV argmax holds (" + best + ")",
        evs, gap, votes };
      return best;
    }

    /* the rule path: argmax with the roster-order tie-break */
    g.rungs.rule += 1;
    g.lastDecision = { side, action: best, rung: "rule",
      why: gap < g.cfg.band
        ? "close EVs but no engine lane — argmax on " + best
        : "EV gap " + gap + " is decisive — argmax on " + best,
      evs, gap, votes };
    return best;
  }

  /* ---------- world ---------- */

  function createGame(raw, movesOverride) {
    const cfg = normalizeConfig(raw);
    const moves = movesOverride || baseGraph();
    const v = validateGraph(moves);
    if (!v.ok) throw new Error("invalid move graph: " + v.reason);
    return {
      cfg, stamp: rpslsStamp(cfg),
      rng: DK.makeRng("rpsls:" + cfg.seed),
      moves, rules: buildRules(moves),
      round: 0, over: false,
      scores: { left: 0, right: 0, ties: 0 },
      streak: { side: null, count: 0, best: { left: 0, right: 0 } },
      histories: { left: [], right: [] },
      meta: {},                 /* predictor -> running score, per side */
      usage: {},                /* move id -> plays, both sides summed */
      flips: 0, decisions: 0,
      rungs: { rule: 0, engine: 0 },
      engineCalls: 0, tokens: cfg.askBudget,
      log: [], lastDecision: null,
    };
  }

  function log(g, kind, note) {
    g.log.push({ round: g.round, kind, note });
    if (g.log.length > LOG_MAX) g.log.shift();
  }

  /* one round. In "you" mode playerMove is the human's pick (or the
     scripted pick in tests); in "bots" mode both sides are brains. */
  function playRound(g, playerMove, engineChoose) {
    if (g.over) return;
    g.round += 1;
    const left = g.cfg.mode === "you" ? playerMove : botDecide(g, "left", null);
    const right = botDecide(g, "right", g.cfg.mode === "you" ? engineChoose : null);
    const r = result(g.rules, left, right);
    g.histories.left.push(left);
    g.histories.right.push(right);
    g.usage[left] = (g.usage[left] || 0) + 1;
    g.usage[right] = (g.usage[right] || 0) + 1;
    if (r === "tie") {
      g.scores.ties += 1;
      log(g, "round", left + " mirrors " + right + " — a tie");
    } else {
      const winner = r === "win" ? "left" : "right";
      g.scores[winner] += 1;
      const winMove = r === "win" ? left : right;
      const loseMove = r === "win" ? right : left;
      const beats = g.moves.find((m) => m.id === winMove).label.toLowerCase();
      const loses = g.moves.find((m) => m.id === loseMove).label.toLowerCase();
      log(g, "round", (winner === "left" ? "left" : "right") + " wins: "
        + beats + " beats " + loses);
      /* streak bookkeeping */
      if (g.streak.side === winner) {
        g.streak.count += 1;
      } else {
        g.streak.side = winner;
        g.streak.count = 1;
      }
      g.streak.best[winner] = Math.max(g.streak.best[winner], g.streak.count);
    }
    /* meta layer: score every predictor by what its counter would have
       done against the opponent's actual move */
    for (const side of ["left", "right"]) {
      const oppMove = side === "left" ? right : left;
      g.meta[side] = g.meta[side] || {};
      const pr = predict(side === "left" ? g.histories.right.slice(0, -1)
        : g.histories.left.slice(0, -1), g.rng);
      const vt = predictorVotes(g, side, pr);
      for (const pName of PREDICTORS) {
        const pick = vt[pName];
        if (!pick) continue;
        const o = result(g.rules, pick, oppMove);
        const delta = o === "win" ? 1 : o === "tie" ? 0 : -1;
        g.meta[side][pName] = META_DECAY * (g.meta[side][pName] || 0) + delta;
      }
    }
    if (g.cfg.mode === "bots" && g.round >= g.cfg.botRounds) {
      g.over = true;
      log(g, "end", "bot duel over after " + g.round + " rounds — left "
        + g.scores.left + ", right " + g.scores.right + ", ties " + g.scores.ties);
    }
  }

  /* a fair extension, validated; returns the reason on refusal */
  function addMove(g, spec) {
    const check = checkAdd(g.moves, spec);
    if (!check.ok) {
      log(g, "graph", "add refused: " + check.reason);
      return check;
    }
    g.moves = reoriented(g.moves, spec).map((m) => (m.id === spec.id
      ? { id: m.id, label: spec.label || spec.id, glyph: spec.glyph || "➕", beats: m.beats }
      : m));
    g.rules = buildRules(g.moves);
    log(g, "graph", spec.id + " joined the graph, beating "
      + spec.beats.join(", "));
    return check;
  }

  function runSummary(g) {
    return {
      round: g.round, over: g.over, mode: g.cfg.mode,
      moves: g.moves.map((m) => m.id),
      scores: Object.assign({}, g.scores),
      streak: { side: g.streak.side, count: g.streak.count,
        best: Object.assign({}, g.streak.best) },
      usage: Object.assign({}, g.usage),
      rungs: Object.assign({}, g.rungs),
      engineCalls: g.engineCalls, tokens: g.tokens,
      decisions: g.decisions,
      history: g.histories.left.length,
      logEntries: g.log.length,
    };
  }

  globalThis.RpslsCore = {
    LOG_MAX, META_DECAY, MARKOV_MIN, EXPLORE, BASE_MOVES, PREDICTORS, PREDICTOR_NAME,
    DEFAULT_CONFIG, normalizeConfig, rpslsStamp, buildRules, validateGraph,
    baseGraph, checkAdd, result, predict, predictorVotes, evOf, botDecide,
    createGame, playRound, addMove, runSummary,
  };
})();
