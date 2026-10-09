/* The Prisoner's Dilemma — core.
   Iterated prisoner's dilemma on the Decisions SDK: two agents decide
   COOPERATE/DEFECT each round from the interaction history. Seven roster
   strategies (trusting, suspicious, tit-for-tat, retaliatory grim
   trigger, forgiving tit-for-two-tats, pavlov win-stay-lose-shift,
   random) plus the ENGINE AGENT, which asks the runtime per round from
   the recent history — under a hard ask budget per match; past the
   budget (or on refusal) it falls back to the mood rule (copy the
   opponent's recent cooperate-rate) and the rung says so. Noise garbles
   moves in transmission: the opponent SEES the flipped move while the
   scorer uses the true one. The payoff matrix (R/S/T/P) is editable and
   every score flows through it. Headless tournament() plays the full
   round-robin. Seeded and deterministic: the random strategy and every
   noise draw come from the match's own rng stream.

   The shell (dilemma.js) renders; this file never touches the DOM. */

(function () {
  "use strict";
  const DK = globalThis.DecisionsSDK;

  const LOG_MAX = 200;
  const HIST_EVERY = 10;              /* rounds per history sample */
  const MOOD_WINDOW = 8;              /* rounds the mood rule looks back */

  const ACTIONS = ["cooperate", "defect"];
  const ACTION_DESC = {
    cooperate: "pay the cost of cooperation — mutual cooperation pays R each round",
    defect: "take the temptation payoff T — but the future remembers",
  };

  /* the classic matrix, as mine-vs-theirs payoffs */
  const DEFAULT_MATRIX = { cc: 3, cd: 0, dc: 5, dd: 1 };
  const MATRIX_LABEL = {
    cc: "R — both cooperate", cd: "S — I cooperate, they defect",
    dc: "T — I defect, they cooperate", dd: "P — both defect",
  };

  const ROSTER = ["engine", "trusting", "titfortat", "grim", "forgiving",
    "pavlov", "suspicious", "random"];
  const STRATEGY_NAME = {
    engine: "ENGINE AGENT", trusting: "TRUSTING", titfortat: "TIT-FOR-TAT",
    grim: "GRIM TRIGGER", forgiving: "TIT-FOR-TWO-TATS", pavlov: "PAVLOV",
    suspicious: "SUSPICIOUS", random: "RANDOM",
  };
  const STRATEGY_DESC = {
    engine: "the OpenCodifier runtime decides from the interaction history, under the ask budget",
    trusting: "always cooperates, whatever happens",
    titfortat: "cooperates first, then plays whatever the opponent last played",
    grim: "cooperates until the opponent defects once — then defects forever",
    forgiving: "defects only after the opponent defected twice in a row",
    pavlov: "win-stay, lose-shift: repeats after a good round, switches after a bad one",
    suspicious: "always defects, whatever happens",
    random: "a fair coin every round (the match's own seeded coin)",
  };

  const DEFAULT_CONFIG = {
    seed: "pd-axelrod", rounds: 200, noise: 0, askBudget: 4,
    left: "engine", right: "titfortat", engineInTournament: true,
    matrix: Object.assign({}, DEFAULT_MATRIX),
  };

  /* ---------- config + stamp ---------- */

  function normalizeConfig(raw) {
    const c = raw || {};
    /* 0 is a real value for every knob: explicit checks, no `||` */
    const num = (v, lo, hi, dflt) => (v === undefined || v === null
      || Number.isNaN(Number(v))) ? dflt : DK.clampf(Number(v), lo, hi, dflt);
    const m = c.matrix || {};
    const cell = (v, dflt) => (v === undefined || v === null
      || Number.isNaN(Number(v))) ? dflt : Math.max(-9, Math.min(9, Number(v)));
    const left = ROSTER.includes(c.left) ? c.left : DEFAULT_CONFIG.left;
    const right = ROSTER.includes(c.right) ? c.right : DEFAULT_CONFIG.right;
    return {
      seed: c.seed === undefined || c.seed === null ? DEFAULT_CONFIG.seed : String(c.seed),
      rounds: Math.round(num(c.rounds, 10, 500, DEFAULT_CONFIG.rounds)),
      noise: num(c.noise, 0, 0.5, DEFAULT_CONFIG.noise),
      askBudget: Math.round(num(c.askBudget, 0, 10, DEFAULT_CONFIG.askBudget)),
      left,
      right,
      engineInTournament: c.engineInTournament === undefined
        ? DEFAULT_CONFIG.engineInTournament : !!c.engineInTournament,
      matrix: {
        cc: cell(m.cc, DEFAULT_MATRIX.cc), cd: cell(m.cd, DEFAULT_MATRIX.cd),
        dc: cell(m.dc, DEFAULT_MATRIX.dc), dd: cell(m.dd, DEFAULT_MATRIX.dd),
      },
    };
  }

  const dilemmaStamp = (cfg) => "pd|" + cfg.seed + "|" + cfg.rounds
    + "|" + cfg.noise + "|" + cfg.askBudget + "|" + cfg.left + "|" + cfg.right
    + "|" + (cfg.engineInTournament ? 1 : 0)
    + "|" + cfg.matrix.cc + "," + cfg.matrix.cd
    + "," + cfg.matrix.dc + "," + cfg.matrix.dd;

  /* ---------- roster ---------- */

  /* every strategy sees only what noise let through: `st.seen` is the
     opponent's moves AS TRANSMITTED. ctx = { rng, R } */
  function step(strategy, st, ctx) {
    const seen = st.seen;
    switch (strategy) {
      case "trusting": return "cooperate";
      case "suspicious": return "defect";
      case "random": return ctx.rng() < 0.5 ? "cooperate" : "defect";
      case "titfortat":
        return seen.length === 0 ? "cooperate" : seen[seen.length - 1];
      case "grim":
        return seen.includes("defect") ? "defect" : "cooperate";
      case "forgiving": {
        const n = seen.length;
        return n >= 2 && seen[n - 1] === "defect" && seen[n - 2] === "defect"
          ? "defect" : "cooperate";
      }
      case "pavlov": {
        if (st.mine.length === 0) return "cooperate";
        /* stay on a win (payoff >= R), shift on a loss */
        return st.lastPayoff >= ctx.R ? st.mine[st.mine.length - 1]
          : (st.mine[st.mine.length - 1] === "cooperate" ? "defect" : "cooperate");
      }
      default: return "cooperate";
    }
  }

  /* the mood rule: the engine agent's no-token / refused-answer fallback */
  function moodRule(st) {
    const seen = st.seen.slice(-MOOD_WINDOW);
    if (!seen.length) return "cooperate";
    const coop = seen.filter((m) => m === "cooperate").length;
    return coop * 2 >= seen.length ? "cooperate" : "defect";
  }

  /* ---------- world ---------- */

  function mkAgent(id, side, cfg) {
    return {
      id, side,
      seen: [],            /* opponent's moves as this agent received them */
      mine: [],            /* this agent's true moves */
      lastPayoff: 0,
      rng: DK.makeRng("dilemma:" + cfg.seed + ":" + side),
    };
  }

  function createGame(raw) {
    const cfg = normalizeConfig(raw);
    const rng = DK.makeRng("dilemma:" + cfg.seed);
    return {
      cfg, stamp: dilemmaStamp(cfg), rng,
      agents: { left: mkAgent(cfg.left, "left", cfg), right: mkAgent(cfg.right, "right", cfg) },
      round: 0,
      scores: { left: 0, right: 0 },
      moves: [],           /* { round, a, b, sa, sb, fa, fb, pa, pb } true
                              moves a/b, as-seen sa/sb, flips fa/fb, payoffs */
      tally: { left: { cc: 0, cd: 0, dc: 0, dd: 0 }, right: { cc: 0, cd: 0, dc: 0, dd: 0 } },
      flips: 0, decisions: 0,
      rungs: { rule: 0, engine: 0 },
      engineCalls: 0, tokens: cfg.askBudget,
      history: [],
      log: [], over: false,
      lastDecision: null,
    };
  }

  function log(g, kind, note) {
    g.log.push({ round: g.round, kind, note });
    if (g.log.length > LOG_MAX) g.log.shift();
  }

  const payoff = (matrix, mine, theirs) =>
    mine === "cooperate"
      ? (theirs === "cooperate" ? matrix.cc : matrix.cd)
      : (theirs === "cooperate" ? matrix.dc : matrix.dd);

  /* ---------- the engine agent's ask ---------- */

  function engineDecide(g, me, engineChoose) {
    const opp = me.side === "left" ? g.agents.right : g.agents.left;
    const recent = me.seen.slice(-MOOD_WINDOW);
    const coop = recent.filter((m) => m === "cooperate").length;
    const stateText = "Iterated prisoner's dilemma, round " + (g.round + 1)
      + " of " + g.cfg.rounds + ". I am the " + me.side + " agent; the "
      + "opponent (" + STRATEGY_NAME[opp.id] + ") last played "
      + (opp.mine.length ? opp.mine[opp.mine.length - 1] : "nothing yet")
      + ". Payoffs R=" + g.cfg.matrix.cc + " S=" + g.cfg.matrix.cd
      + " T=" + g.cfg.matrix.dc + " P=" + g.cfg.matrix.dd
      + ", transmission noise " + g.cfg.noise + ". Last " + MOOD_WINDOW
      + " rounds (mine/theirs-as-received): "
      + (recent.length
        ? me.mine.slice(-MOOD_WINDOW).map((m, i) =>
          (m === "cooperate" ? "C" : "D") + (recent[i] === "cooperate" ? "C" : "D")).join(" ")
        : "none yet")
      + ". Their cooperate rate over that window: "
      + (recent.length ? (coop / recent.length).toFixed(2) : "n/a")
      + ". Score me " + g.scores[me.side] + ", them " + g.scores[me.side === "left" ? "right" : "left"] + ".";

    if (g.tokens < 1 || !engineChoose) {
      const move = moodRule(me);
      g.decisions += 1;
      g.rungs.rule += 1;
      g.lastDecision = { side: me.side, action: move, rung: "rule",
        why: g.tokens < 1 ? "ask budget spent — the mood rule holds" : "no bridge — the mood rule holds" };
      return move;
    }

    g.tokens -= 1;
    g.engineCalls += 1;
    const req = {
      state: { text: stateText, facts: {} },
      questions: [{
        type: "choice", id: "pd-move",
        text: "Cooperate or defect this round?",
        candidates: ACTIONS.map((a) => ({ id: a, description: ACTION_DESC[a] })),
      }],
      policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" },
    };
    const answer = engineChoose(req);
    const ans = answer && answer.outcome === "accept" && Array.isArray(answer.answers)
      ? answer.answers.find((a) => a && typeof a.choice === "string") : null;
    g.decisions += 1;
    if (ans && ACTIONS.includes(ans.choice)) {
      g.rungs.engine += 1;
      g.lastDecision = { side: me.side, action: ans.choice, rung: "engine",
        why: "honored — history on the wire" };
      return ans.choice;
    }
    /* refusal or malformed: the mood rule holds, the rung says so — this
       game has no captain to escalate to, the engine agent IS a player */
    g.rungs.rule += 1;
    g.lastDecision = { side: me.side, action: moodRule(me), rung: "rule",
      why: answer && answer.outcome === "abstain"
        ? "engine abstained — the mood rule holds"
        : "engine answer named nothing live — the mood rule holds" };
    return g.lastDecision.action;
  }

  function decide(g, side, engineChoose) {
    if (g.over) return null;
    const me = g.agents[side];
    if (me.id === "engine") return engineDecide(g, me, engineChoose);
    const move = step(me.id, me, { rng: me.rng, R: g.cfg.matrix.cc });
    g.decisions += 1;
    g.rungs.rule += 1;
    g.lastDecision = { side, action: move, rung: "rule", why: STRATEGY_DESC[me.id] };
    return move;
  }

  /* ---------- tick: one round ---------- */

  function tickGame(g, engineChoose) {
    if (g.over) return;
    g.round += 1;

    const a = decide(g, "left", engineChoose);
    const b = decide(g, "right", engineChoose);

    /* noise garbles TRANSMISSION: the opponent sees the flip, the scorer
       uses the true move */
    const flip = (move) => (g.rng() < g.cfg.noise
      ? (move === "cooperate" ? "defect" : "cooperate") : move);
    const sa = flip(a), sb = flip(b);
    const fa = sa !== a, fb = sb !== b;
    if (fa || fb) {
      g.flips += (fa ? 1 : 0) + (fb ? 1 : 0);
      log(g, "noise", (fa ? "left's " + a : "") + (fa && fb ? "; " : "")
        + (fb ? "right's " + b : "") + " garbled in transmission");
    }

    const pa = payoff(g.cfg.matrix, a, b);
    const pb = payoff(g.cfg.matrix, b, a);
    g.scores.left += pa;
    g.scores.right += pb;

    const la = g.agents.left, ra = g.agents.right;
    la.mine.push(a); la.seen.push(sb); la.lastPayoff = pa;
    ra.mine.push(b); ra.seen.push(sa); ra.lastPayoff = pb;
    g.tally.left[a[0] + b[0]] += 1;
    g.tally.right[b[0] + a[0]] += 1;
    g.moves.push({ round: g.round, a, b, sa, sb, fa, fb, pa, pb });
    if (g.moves.length > 2000) g.moves.shift();

    if (g.round === 1) {
      log(g, "match", STRATEGY_NAME[la.id] + " vs " + STRATEGY_NAME[ra.id]
        + " — first moves in");
    }

    /* the tail sample only when the boundary doesn't already land there */
    if (g.round % HIST_EVERY === 0
      || (g.round === g.cfg.rounds && g.round % HIST_EVERY !== 0)) {
      const win = g.moves.slice(-HIST_EVERY);
      g.history.push({
        r: g.round,
        coopL: win.filter((m) => m.a === "cooperate").length / win.length,
        coopR: win.filter((m) => m.b === "cooperate").length / win.length,
        avg: (win.reduce((s, m) => s + m.pa + m.pb, 0) / win.length) / 2,
      });
      if (g.history.length > 400) g.history.shift();
    }

    if (g.round >= g.cfg.rounds) {
      g.over = true;
      const w = g.scores.left === g.scores.right ? "a draw"
        : (g.scores.left > g.scores.right ? "left" : "right") + " wins";
      log(g, "end", "match over after " + g.round + " rounds — " + w
        + " (" + g.scores.left + " to " + g.scores.right + ")");
    }
  }

  /* ---------- summary ---------- */

  function runSummary(g) {
    const coop = (side) => {
      const t = g.tally[side];
      const tot = t.cc + t.cd + t.dc + t.dd;
      return tot ? +((t.cc + t.cd) / tot).toFixed(3) : 0;
    };
    return {
      round: g.round, rounds: g.cfg.rounds, over: g.over,
      left: g.cfg.left, right: g.cfg.right,
      scores: Object.assign({}, g.scores),
      coopRate: { left: coop("left"), right: coop("right") },
      tally: { left: Object.assign({}, g.tally.left), right: Object.assign({}, g.tally.right) },
      flips: g.flips, decisions: g.decisions,
      rungs: Object.assign({}, g.rungs),
      engineCalls: g.engineCalls,
      tokens: g.tokens,
      noise: g.cfg.noise,
      history: g.history.length, logEntries: g.log.length,
    };
  }

  /* ---------- tournament: headless round-robin ---------- */

  function tournament(raw, engineChoose) {
    const cfg = normalizeConfig(raw);
    const roster = ROSTER.filter((s) => s !== "engine"
      || cfg.engineInTournament);
    const table = [];
    const matches = [];
    let interactions = 0, decisions = 0, engineCalls = 0, flips = 0;
    for (let i = 0; i < roster.length; i++) {
      for (let j = i + 1; j < roster.length; j++) {
        const m = createGame({
          seed: cfg.seed + "::" + roster[i] + "::" + roster[j],
          rounds: cfg.rounds, noise: cfg.noise, askBudget: cfg.askBudget,
          left: roster[i], right: roster[j],
          matrix: cfg.matrix,
        });
        while (!m.over) tickGame(m, roster[i] === "engine" || roster[j] === "engine"
          ? engineChoose : null);
        const s = runSummary(m);
        interactions += m.round;
        decisions += s.decisions;
        engineCalls += s.engineCalls;
        flips += s.flips;
        table.push({ a: roster[i], b: roster[j], sa: s.scores.left, sb: s.scores.right,
          coopA: s.coopRate.left, coopB: s.coopRate.right });
        matches.push(m);
      }
    }
    /* cooperation rate over time, aggregated across every pair */
    const series = [];
    const winLen = HIST_EVERY;
    for (let r = winLen; r <= cfg.rounds; r += winLen) {
      let coop = 0, tot = 0;
      for (const m of matches) {
        const win = m.moves.slice(r - winLen, r);
        if (!win.length) continue;
        coop += win.filter((x) => x.a === "cooperate").length
          + win.filter((x) => x.b === "cooperate").length;
        tot += win.length * 2;
      }
      series.push({ r, coop: tot ? +(coop / tot).toFixed(3) : 0 });
    }
    return {
      roster, table, series,
      interactions, decisions, engineCalls, flips,
      stamp: dilemmaStamp(cfg),
    };
  }

  globalThis.DilemmaCore = {
    LOG_MAX, HIST_EVERY, MOOD_WINDOW, ACTIONS, ACTION_DESC,
    DEFAULT_MATRIX, MATRIX_LABEL, ROSTER, STRATEGY_NAME, STRATEGY_DESC,
    DEFAULT_CONFIG, normalizeConfig, dilemmaStamp, step, moodRule,
    createGame, tickGame, runSummary, decide, payoff, tournament,
  };
})();
