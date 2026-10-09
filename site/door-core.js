/* Choose Your Door — core.
   The "can it decide?" rung of the suite: N doors, one character, and
   the whole reasoning revealed. Everything is the wire contract in
   miniature — the character (health, weapon, knowledge, torches,
   objective) is the STATE, the doors are the CANDIDATES, and the
   PERSONALITY is the POLICY: coward / greedy / brave / steady differ
   only in the weights the same request carries. There are no
   personality code paths: score(door) = the weighted trait sum with
   state modifiers (hurt and unarmed characters fear danger more), and
   choice confidence is a softmax over the scores. The engine lane: a
   genuine close call or ANY request when the ladder cannot separate
   the doors goes on the wire; an engine abstain (equiprobable doors +
   no objective is the canonical trigger) is a first-class outcome —
   nobody moves, and the UI says why. Engine off → the ladder decides
   alone (argmax, roster-order tie-break, and it says so).
   series() plays headless rooms so policy -> behavior is visible at
   volume. Seeded and deterministic.

   The shell (door.js) renders; this file never touches the DOM. */

(function () {
  "use strict";
  const DK = globalThis.DecisionsSDK;

  const SOFTMAX_T = 0.8;          /* confidence temperature */
  const OBJ_BOOST = 0.3;          /* objective-alignment bonus scale */

  const PERSONALITIES = ["steady", "coward", "greedy", "brave"];
  const PERSONALITY_NAME = {
    steady: "STEADY", coward: "COWARD", greedy: "GREEDY", brave: "BRAVE",
  };
  const PERSONALITY_DESC = {
    steady: "balanced weights — a bit of everything, nothing dominant",
    coward: "danger weighs triple, reward barely counts, mystery is a threat",
    greedy: "reward weighs triple; danger, mystery, effort barely register",
    brave: "SEEKS danger and mystery; effort is nothing; reward still counts",
  };

  /* policy IS the personality: same scorer, same request, different
     weights. risk rides on the wire as the request's risk field. */
  const POLICIES = {
    steady: { weights: { danger: 1, reward: 1, curiosity: 0.5, effort: 0.5 }, risk: "medium" },
    coward: { weights: { danger: 3, reward: 0.5, curiosity: 0.1, effort: 1 }, risk: "low" },
    greedy: { weights: { danger: 0.2, reward: 3, curiosity: 0.8, effort: 0.2 }, risk: "medium" },
    brave: { weights: { danger: -0.5, reward: 1.5, curiosity: 1.5, effort: 0.1 }, risk: "high" },
  };

  const OBJECTIVES = ["none", "escape", "treasure", "glory"];
  const OBJECTIVE_DESC = {
    none: "no objective — nothing biases the doors",
    escape: "escape: dull ways out align with the goal",
    treasure: "treasure: rich doors align with the goal",
    glory: "glory: dangerous doors align with the goal",
  };

  const DOOR_LABELS = [
    "The collapsed stairwell", "The gilded hall", "The mossy tunnel",
    "The guardroom", "The iron door", "The flooded crypt", "The high gallery",
    "The servant's passage", "The beast's den", "The silent archive",
  ];
  const DOOR_GLYPHS = ["🚪", "🕳", "🪜", "⚔", "💎", "🌊", "🔥", "🗝"];

  const DEFAULT_CONFIG = {
    seed: "door-one", personality: "steady", doorCount: 3,
    health: 70, weapon: 0.5, knowledge: 0.5, torches: 1,
    objective: "treasure", balanced: false,
  };

  /* ---------- config + stamp ---------- */

  function normalizeConfig(raw) {
    const c = raw || {};
    const num = (v, lo, hi, dflt) => (v === undefined || v === null
      || Number.isNaN(Number(v))) ? dflt : DK.clampf(Number(v), lo, hi, dflt);
    return {
      seed: c.seed === undefined || c.seed === null ? DEFAULT_CONFIG.seed : String(c.seed),
      personality: PERSONALITIES.includes(c.personality)
        ? c.personality : DEFAULT_CONFIG.personality,
      doorCount: Math.round(num(c.doorCount, 3, 5, DEFAULT_CONFIG.doorCount)),
      health: num(c.health, 1, 100, DEFAULT_CONFIG.health),
      weapon: num(c.weapon, 0, 1, DEFAULT_CONFIG.weapon),
      knowledge: num(c.knowledge, 0, 1, DEFAULT_CONFIG.knowledge),
      torches: Math.round(num(c.torches, 0, 5, DEFAULT_CONFIG.torches)),
      objective: OBJECTIVES.includes(c.objective) ? c.objective : DEFAULT_CONFIG.objective,
      balanced: c.balanced === undefined ? DEFAULT_CONFIG.balanced : !!c.balanced,
    };
  }

  const doorStamp = (cfg) => "door|" + cfg.seed + "|" + cfg.personality
    + "|" + cfg.doorCount + "|" + cfg.health + "|" + cfg.weapon
    + "|" + cfg.knowledge + "|" + cfg.torches + "|" + cfg.objective
    + "|" + (cfg.balanced ? 1 : 0);

  /* ---------- scenario ---------- */

  function makeScenario(cfg) {
    const rng = DK.makeRng("door-scenario:" + cfg.seed);
    const n = cfg.doorCount;
    if (cfg.balanced) {
      /* the abstention setup: identical doors, nothing to prefer */
      return Array.from({ length: n }, (_, i) => ({
        id: "door-" + (i + 1), label: "Door " + "ABCDE"[i],
        glyph: "🚪", danger: 0.5, reward: 0.5, mystery: 0, effort: 0.5,
      }));
    }
    return Array.from({ length: n }, (_, i) => {
      const danger = +rng().toFixed(3);
      /* reward leans on danger — the world pays risk, mostly */
      const reward = DK.clampf(danger * 0.7 + rng() * 0.5, 0.02, 0.98);
      return {
        id: "door-" + (i + 1),
        label: DOOR_LABELS[Math.floor(rng() * DOOR_LABELS.length)],
        glyph: DOOR_GLYPHS[Math.floor(rng() * DOOR_GLYPHS.length)],
        danger,
        reward,
        mystery: +rng().toFixed(3),
        effort: +rng().toFixed(3),
      };
    });
  }

  /* ---------- the ladder: one scorer, personality = weights ---------- */

  function scoreDoor(cfg, policy, door) {
    const w = policy.weights;
    /* state modifiers: hurt and unarmed characters fear danger more —
       the SAME weight, scaled by the state */
    const fear = w.danger * (1.25 - 0.5 * cfg.weapon) * (1.25 - 0.5 * (cfg.health / 100));
    let s = -fear * door.danger
      + w.reward * door.reward
      + w.curiosity * door.mystery * (1 - cfg.knowledge * 0.6)
      - w.effort * door.effort * (1 - Math.min(1, cfg.torches * 0.15));
    if (cfg.objective === "treasure") s += OBJ_BOOST * w.reward * door.reward;
    if (cfg.objective === "escape") s += OBJ_BOOST * Math.abs(w.danger) * (1 - door.danger);
    if (cfg.objective === "glory") s += OBJ_BOOST * Math.abs(w.danger) * door.danger;
    return +s.toFixed(6);
  }

  function softmax(scores, t) {
    const mx = Math.max(...scores);
    const ex = scores.map((s) => Math.exp((s - mx) / t));
    const tot = ex.reduce((a, b) => a + b, 0);
    return ex.map((e) => e / tot);
  }

  /* ---------- world ---------- */

  function createGame(raw) {
    const cfg = normalizeConfig(raw);
    const doors = makeScenario(cfg);
    return {
      cfg, stamp: doorStamp(cfg),
      policy: POLICIES[cfg.personality],
      doors,
      decided: false,
      decision: null,
      lastRequest: null,
    };
  }

  /* the wire request — built fresh from the same state the ladder sees */
  function buildRequest(g, scores) {
    const w = g.policy.weights;
    const probsText = Array.isArray(scores)
      ? softmax(scores, SOFTMAX_T).map((p, i) =>
        "door-" + (i + 1) + " " + p.toFixed(3)).join(", ")
      : "not scored yet";
    const doorText = g.doors.map((d) =>
      d.id + " \"" + d.label + "\" — danger " + d.danger + ", reward " + d.reward
      + ", mystery " + d.mystery + ", effort " + d.effort).join("; ");
    return {
      state: {
        text: "Choose a door. The character: health " + Math.round(g.cfg.health)
          + "/100, weapon quality " + g.cfg.weapon + ", knowledge " + g.cfg.knowledge
          + ", " + g.cfg.torches + " torch(es), objective: " + g.cfg.objective
          + ". The personality policy weighs danger " + w.danger + ", reward "
          + w.reward + ", curiosity " + w.curiosity + ", effort " + w.effort
          + ". The doors: " + doorText
          + ". The ladder's probabilities: " + probsText + ".",
        facts: {},
      },
      questions: [{
        type: "choice", id: "door-pick",
        text: "Which door does the character take?",
        candidates: g.doors.map((d) => ({
          id: d.id, description: d.label + " (danger " + d.danger
            + ", reward " + d.reward + ")",
        })),
      }],
      policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3,
        risk: g.policy.risk },
    };
  }

  /* the one decision this room is about */
  function decide(g, engineChoose) {
    if (g.decided) return g.decision;
    const scores = g.doors.map((d) => scoreDoor(g.cfg, g.policy, d));
    const probs = softmax(scores, SOFTMAX_T);
    const order = g.doors.map((_, i) => i).sort((a, b) =>
      (scores[b] - scores[a]) || (a - b));
    const top = order[0], second = order[1];
    const gap = +(scores[top] - scores[second]).toFixed(6);
    const conf = +probs[top].toFixed(4);
    const req = buildRequest(g, scores);
    g.lastRequest = req;
    const ladderWhy = "ladder: " + g.doors.map((d, i) =>
      d.id + " " + scores[i].toFixed(3)).join(", ");

    /* the ladder-alone path: argmax, roster-order tie-break, said aloud */
    if (!engineChoose) {
      g.decided = true;
      g.decision = {
        choice: g.doors[top].id, rung: "rule", confidence: conf,
        scores, probs, abstained: false,
        why: ladderWhy + (gap === 0 ? " — a dead tie, broken by door order"
          : " — top-two gap " + gap + ", argmax holds"),
      };
      return g.decision;
    }

    const answer = engineChoose(req);
    const ans = answer && answer.outcome === "accept" && Array.isArray(answer.answers)
      ? answer.answers.find((a) => a && typeof a.choice === "string") : null;
    g.decided = true;
    if (ans && g.doors.some((d) => d.id === ans.choice)) {
      const ci = g.doors.findIndex((d) => d.id === ans.choice);
      g.decision = {
        choice: ans.choice, rung: "engine", confidence: +probs[ci].toFixed(4),
        scores, probs, abstained: false,
        why: ladderWhy + " — the engine picked " + ans.choice
          + " (ladder probability " + probs[ci].toFixed(3) + ")",
      };
      return g.decision;
    }
    /* refusal is the first-class outcome: nobody moves, the why is shown */
    g.decision = {
      choice: null, rung: "abstain", confidence: conf,
      scores, probs, abstained: true,
      why: answer && answer.outcome === "abstain"
        ? "the engine ABSTAINED (ladder probability " + conf
          + (conf < 0.3 ? " < 0.30" : "") + "): the doors do not separate — "
          + (g.cfg.objective === "none" && g.cfg.balanced
            ? "equiprobable doors with no objective is the canonical "
              + "refusal case" : "no honest winner under this policy")
          + ". Nobody moves."
        : "the engine answer (outcome "
          + JSON.stringify(answer && answer.outcome) + ") named nothing live ("
          + ladderWhy + ")"
          + (gap === 0
            ? " — a dead heat: the doors are equiprobable, there is no argmax"
            : "") + " — nobody moves.",
    };
    return g.decision;
  }

  /* ---------- series: policy -> behavior at volume ---------- */

  function series(raw, count, engineChoose) {
    const cfg = normalizeConfig(raw);
    const tally = {};
    let abstains = 0, tieBreaks = 0, confSum = 0, decisions = 0;
    for (let i = 0; i < count; i++) {
      const room = createGame(Object.assign({}, cfg, {
        seed: cfg.seed + "::" + i,
        /* every 4th room is the balanced abstention setup */
        balanced: i % 4 === 3,
      }));
      const d = decide(room, engineChoose);
      decisions += 1;
      if (d.abstains || d.abstained) {
        abstains += 1;
      } else if (d.rung === "engine") {
        tally[d.choice] = (tally[d.choice] || 0) + 1;
      } else {
        /* ladder-decided: a dead heat is a tie-break, not an argmax win */
        const sorted = d.scores.slice().sort((a, b) => b - a);
        if (sorted[0] - sorted[1] === 0) {
          tieBreaks += 1;
        } else {
          tally[d.choice] = (tally[d.choice] || 0) + 1;
        }
      }
      confSum += d.confidence;
    }
    return {
      personality: cfg.personality, rooms: count, tally,
      abstains, tieBreaks,
      avgConfidence: +(confSum / Math.max(1, decisions)).toFixed(4),
      stamp: doorStamp(cfg),
    };
  }

  /* the demo metric: how often two policies pick different doors on the
     same room sequence (ladder-only, dead-heat rooms skipped) */
  function divergence(rawA, rawB, count) {
    let agree = 0, disagree = 0;
    for (let i = 0; i < count; i++) {
      const seed = (rawA.seed || DEFAULT_CONFIG.seed) + "::" + i;
      if (i % 4 === 3) continue;   /* the balanced dead-heat setup */
      const a = decide(createGame(Object.assign({}, rawA, { seed })), null);
      const b = decide(createGame(Object.assign({}, rawB, { seed })), null);
      if (a.choice === b.choice) agree += 1; else disagree += 1;
    }
    return { rooms: agree + disagree, agree, disagree,
      disagreePct: +((disagree / Math.max(1, agree + disagree)) * 100).toFixed(1) };
  }

  function runSummary(g) {
    return {
      personality: g.cfg.personality, doors: g.doors.length,
      objective: g.cfg.objective, decided: g.decided,
      choice: g.decision ? g.decision.choice : null,
      rung: g.decision ? g.decision.rung : null,
      abstained: g.decision ? g.decision.abstained : false,
      confidence: g.decision ? g.decision.confidence : null,
      risk: g.policy.risk,
    };
  }

  globalThis.DoorCore = {
    SOFTMAX_T, OBJ_BOOST, PERSONALITIES, PERSONALITY_NAME, PERSONALITY_DESC,
    POLICIES, OBJECTIVES, OBJECTIVE_DESC, DEFAULT_CONFIG,
    normalizeConfig, doorStamp, makeScenario, scoreDoor, softmax,
    createGame, buildRequest, decide, series, divergence, runSummary,
  };
})();
