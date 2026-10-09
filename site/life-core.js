/* Life core — Conway's Game of Life on the Decisions SDK: pure rules,
   classification features, and seed proposals. No DOM, no timers — the
   board is a Uint8Array, deterministic given (seed, rule, dimensions),
   unit-testable under `node --test` like pong-core.js (import for side
   effect, read the global; sdk/decisions-sdk.js must load first).

   Honesty contract (docs/planning/playground/DECISIONS-SDK-PLAN.md §5.2):
   - Life itself is pure rules — the engine NEVER toggles a cell.
   - The engine's only jobs: classify what the board is doing (a verdict
     chip, advisory — the local heuristic produces the same verdicts without
     it) and break genuine ties between equally-interesting proposed seeds.
   - The classifier reads features (population, births/deaths trend, cycle
     period, drift) that are computed here, so the chip can always show
     which rung answered: heuristic or engine. */

"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;

  /* ---------- rules ---------- */

  const PRESETS = {
    life: { label: "Life (B3/S23)", born: [3], survive: [2, 3] },
    highlife: { label: "HighLife (B36/S23)", born: [3, 6], survive: [2, 3] },
    seeds: { label: "Seeds (B2/S)", born: [2], survive: [] },
    daynight: { label: "Day & Night (B3678/S34678)", born: [3, 6, 7, 8], survive: [3, 4, 6, 7, 8] },
    maze: { label: "Maze (B3/S12345)", born: [3], survive: [1, 2, 3, 4, 5] },
    thirtyfour: { label: "34 Life (B34/S34)", born: [3, 4], survive: [3, 4] },
  };

  function ruleToSets(born, survive) {
    const b = new Array(9).fill(false);
    const s = new Array(9).fill(false);
    for (const n of born) if (n >= 0 && n <= 8) b[n] = true;
    for (const n of survive) if (n >= 0 && n <= 8) s[n] = true;
    return { born: b, survive: s };
  }

  function ruleLabel(born, survive) {
    return "B" + born.join("") + "/S" + survive.join("");
  }

  /* parse a rule string like "B3/S23" or "3/23"; hostile input falls back
     to classic Life — the sandbox always has a valid rule */
  function normalizeRule(raw) {
    const fallback = ruleToSets(PRESETS.life.born, PRESETS.life.survive);
    if (typeof raw !== "string") {
      return { ...fallback, label: ruleLabel(PRESETS.life.born, PRESETS.life.survive) };
    }
    const m = /^\s*(?:B|b)?([0-8]*)\s*\/\s*(?:S|s)?([0-8]*)\s*$/.exec(raw);
    if (!m) {
      return { ...fallback, label: ruleLabel(PRESETS.life.born, PRESETS.life.survive) };
    }
    const born = Array.from(new Set(m[1].split("").map(Number)));
    const survive = Array.from(new Set(m[2].split("").map(Number)));
    const sets = ruleToSets(born, survive);
    return { ...sets, label: ruleLabel(born.sort(), survive.sort()) };
  }

  /* ---------- world ---------- */

  function normalizeConfig(raw) {
    const c = raw || {};
    return {
      cols: DK.clamp(c.cols, 16, 200, 96),
      rows: DK.clamp(c.rows, 16, 140, 56),
      wrap: !!c.wrap,
      rule: normalizeRule(c.rule),
      seed: String(c.seed || "oc-life"),
      density: DK.clampf(c.density, 0.05, 0.6, 0.28),
      maxGens: DK.clamp(c.maxGens, 100, 200000, 20000),
    };
  }

  function createWorld(cfg) {
    const c = normalizeConfig(cfg);
    return {
      cfg: c,
      cells: new Uint8Array(c.cols * c.rows),
      gen: 0,
      pop: 0,
      /* rolling cycle memory: shape key -> last generation + bbox origin */
      recent: new Map(),
      period: 0,          /* 0 = none detected yet */
      periodAt: null,     /* features snapshot when the cycle closed */
      births: 0, deaths: 0, changed: 0,
    };
  }

  function idx(w, x, y) { return y * w.cfg.cols + x; }

  function inBounds(w, x, y) {
    return x >= 0 && y >= 0 && x < w.cfg.cols && y < w.cfg.rows;
  }

  function setCell(w, x, y, v) {
    if (!inBounds(w, x, y)) return;
    const i = idx(w, x, y);
    if (w.cells[i] === (v ? 1 : 0)) return;
    w.pop += v ? 1 : -1;
    w.cells[i] = v ? 1 : 0;
  }

  function getCell(w, x, y) {
    if (w.cfg.wrap) {
      x = ((x % w.cfg.cols) + w.cfg.cols) % w.cfg.cols;
      y = ((y % w.cfg.rows) + w.cfg.rows) % w.cfg.rows;
      return w.cells[idx(w, x, y)];
    }
    return inBounds(w, x, y) ? w.cells[idx(w, x, y)] : 0;
  }

  function clearWorld(w) {
    w.cells.fill(0);
    w.gen = 0; w.pop = 0;
    w.recent.clear(); w.period = 0; w.periodAt = null;
    w.births = 0; w.deaths = 0; w.changed = 0;
  }

  /* stable key for cycle detection — translation-NORMALIZED: the alive
     pattern is cropped to its bounding box, so a spaceship translating
     across a torus yields the same shape keys an oscillator does, and the
     two separate by how far the bounding-box ORIGIN moves per cycle.
     Returns { key, ox, oy }; ox/oy are the bbox origin. */
  function worldKey(w) {
    const { cols, rows } = w.cfg;
    let minX = cols, minY = rows, maxX = -1, maxY = -1;
    for (let y = 0; y < rows; y++) {
      const off = y * cols;
      for (let x = 0; x < cols; x++) {
        if (w.cells[off + x]) {
          if (x < minX) minX = x;
          if (x > maxX) maxX = x;
          if (y < minY) minY = y;
          if (y > maxY) maxY = y;
        }
      }
    }
    if (maxX < 0) return { key: "empty", ox: 0, oy: 0 };
    let s = w.pop + ":";
    for (let y = minY; y <= maxY; y++) {
      let row = "";
      const off = y * cols;
      for (let x = minX; x <= maxX; x++) row += w.cells[off + x];
      s += row + "|";
    }
    return { key: DK.hashSeed(s).toString(36) + ":" + w.pop, ox: minX, oy: minY };
  }

  /* forget any detected cycle (the caller changed the board out-of-band) */
  function resetCycle(w) {
    w.recent.clear();
    w.period = 0;
    w.periodAt = null;
  }

  function centroid(w) {
    let sx = 0, sy = 0, n = 0;
    for (let y = 0; y < w.cfg.rows; y++) {
      for (let x = 0; x < w.cfg.cols; x++) {
        if (w.cells[y * w.cfg.cols + x]) { sx += x; sy += y; n += 1; }
      }
    }
    return n ? { x: sx / n, y: sy / n } : { x: 0, y: 0 };
  }

  /* one generation under the world's rule; returns the step's own deltas */
  function step(w) {
    const { cols, rows } = w.cfg;
    const { born, survive } = w.cfg.rule;
    const next = new Uint8Array(cols * rows);
    let pop = 0, births = 0, deaths = 0, changed = 0;
    for (let y = 0; y < rows; y++) {
      for (let x = 0; x < cols; x++) {
        let n = 0;
        for (let dy = -1; dy <= 1; dy++) {
          for (let dx = -1; dx <= 1; dx++) {
            if (dx === 0 && dy === 0) continue;
            n += getCell(w, x + dx, y + dy);
          }
        }
        const alive = w.cells[y * cols + x];
        const after = alive ? (survive[n] ? 1 : 0) : (born[n] ? 1 : 0);
        if (after) pop += 1;
        if (after && !alive) births += 1;
        if (!after && alive) deaths += 1;
        if (after !== alive) changed += 1;
        next[y * cols + x] = after;
      }
    }
    w.cells = next;
    w.pop = pop;
    w.gen += 1;
    w.births = births; w.deaths = deaths; w.changed = changed;

    /* rolling-window cycle detection: the current (translation-normalized)
       shape vs the last 64 generations. Self-correcting by construction —
       a spaceship that crashes into ash re-reads as still — and cheap
       relative to the step. periodAt holds the bbox-origin DISPLACEMENT
       over the cycle: zero for an oscillator, the spaceship's per-cycle
       travel otherwise. */
    const sk = worldKey(w);
    const prev = w.recent.get(sk.key);
    w.recent.set(sk.key, { gen: w.gen, ox: sk.ox, oy: sk.oy });
    for (const [k, v] of w.recent) {
      if (v.gen < w.gen - 64) w.recent.delete(k);
    }
    if (prev) {
      w.period = w.gen - prev.gen;
      w.periodAt = { x: sk.ox - prev.ox, y: sk.oy - prev.oy };
    } else {
      w.period = 0;
      w.periodAt = null;
    }
    return { births, deaths, changed };
  }

  /* ---------- patterns ---------- */

  /* cells as [x, y] offsets; hint shown in the library */
  const PATTERNS = [
    { name: "glider", hint: "the smallest spaceship", cells: [[1, 0], [2, 1], [0, 2], [1, 2], [2, 2]] },
    { name: "LWSS", hint: "lightweight spaceship", cells: [[1, 0], [4, 0], [0, 1], [0, 2], [4, 2], [0, 3], [1, 3], [2, 3], [3, 3]] },
    { name: "blinker", hint: "period-2 oscillator", cells: [[0, 1], [1, 1], [2, 1]] },
    { name: "pulsar", hint: "period-3 oscillator", cells: (() => {
      const art = [
        "..OOO...OOO..",
        ".............",
        "O....O.O....O",
        "O....O.O....O",
        "O....O.O....O",
        "..OOO...OOO..",
        ".............",
        "..OOO...OOO..",
        "O....O.O....O",
        "O....O.O....O",
        "O....O.O....O",
        ".............",
        "..OOO...OOO..",
      ];
      const pts = [];
      art.forEach((row, y) => {
        for (let x = 0; x < row.length; x++) if (row[x] === "O") pts.push([x, y]);
      });
      return pts;
    })() },
    { name: "r-pentomino", hint: "small seed, long chaos", cells: [[1, 0], [2, 0], [0, 1], [1, 1], [1, 2]] },
    { name: "acorn", hint: "7 cells, 5206 generations", cells: [[1, 0], [3, 1], [0, 2], [1, 2], [4, 2], [5, 2], [6, 2]] },
    { name: "block", hint: "still life", cells: [[0, 0], [1, 0], [0, 1], [1, 1]] },
    { name: "glider gun", hint: "Gosper's factory", cells: [[24, 0], [22, 1], [24, 1], [12, 2], [13, 2], [20, 2], [21, 2], [34, 2], [35, 2], [11, 3], [15, 3], [20, 3], [21, 3], [34, 3], [35, 3], [0, 4], [1, 4], [10, 4], [16, 4], [20, 4], [21, 4], [0, 5], [1, 5], [10, 5], [14, 5], [16, 5], [17, 5], [22, 5], [24, 5], [10, 6], [16, 6], [24, 6], [11, 7], [15, 7], [12, 8], [13, 8]] },
  ];

  function stampPattern(w, pattern, ox, oy) {
    for (const [dx, dy] of pattern.cells) setCell(w, ox + dx, oy + dy, 1);
  }

  function randomSoup(w, density, rng) {
    const r = rng || DK.makeRng(w.cfg.seed + ":soup:" + w.gen);
    const d = density === undefined ? w.cfg.density : density;
    for (let i = 0; i < w.cells.length; i++) w.cells[i] = r() < d ? 1 : 0;
    w.pop = 0;
    for (let i = 0; i < w.cells.length; i++) w.pop += w.cells[i];
    w.recent.clear(); w.period = 0; w.periodAt = null;
  }

  /* ---------- classification features + heuristic ---------- */

  /* features over the recent window; caller keeps the window small */
  function features(w, popWindow) {
    const c = centroid(w);
    const hist = popWindow || [];
    const ago = hist.length > 12 ? hist[hist.length - 13] : (hist[0] || w.pop);
    const trend = ago > 0 ? w.pop / ago : (w.pop > 0 ? 2 : 1);
    let drift = 0;
    if (w.periodAt) {
      drift = Math.hypot(w.periodAt.x, w.periodAt.y);
    }
    return {
      gen: w.gen,
      pop: w.pop,
      births: w.births,
      deaths: w.deaths,
      activity: w.changed,
      trend: Math.round(trend * 100) / 100,
      period: w.period,
      drift: Math.round(drift * 100) / 100,
      density: Math.round((w.pop / (w.cfg.cols * w.cfg.rows)) * 1000) / 1000,
    };
  }

  const LABELS = ["extinct", "still", "oscillator", "spaceship", "exploding", "dying", "mixed"];

  /* the local read of the features — what the chip shows when the engine
     is off, abstains, or says nothing useful */
  function classify(f) {
    if (f.pop === 0) return { label: "extinct", conf: 0.99 };
    if (f.period === 1) return { label: "still", conf: 0.97 };
    if (f.period >= 2 && f.period <= 64) {
      return f.drift > 0.5
        ? { label: "spaceship", conf: 0.9 }
        : { label: "oscillator", conf: 0.9 };
    }
    if (f.trend > 1.6) return { label: "exploding", conf: 0.7 };
    if (f.trend < 0.5) return { label: "dying", conf: 0.75 };
    return { label: "mixed", conf: 0.5 };
  }

  /* the engine's view of the same features — a real choice question over
     the labelled trajectories; abstention is welcome */
  function engineRequest(f) {
    const why = {
      extinct: "population is zero",
      still: "a period-1 cycle was detected",
      oscillator: "a short cycle (" + f.period + " gens) with no drift",
      spaceship: "a cycle with the pattern drifting (" + f.drift + " cells)",
      exploding: "population is growing fast (×" + f.trend + " over 12 gens)",
      dying: "population is collapsing (×" + f.trend + " over 12 gens)",
      mixed: "no cycle and no clear trend",
    };
    return {
      state: {
        text: "Life board at generation " + f.gen + ": population " + f.pop +
          ", births " + f.births + ", deaths " + f.deaths + ", changed cells " +
          f.activity + ", population trend x" + f.trend + ", detected cycle " +
          (f.period || "none") + ", pattern drift " + f.drift + " cells.",
        facts: {},
      },
      questions: [{
        type: "choice", id: "board-verdict",
        text: "Which trajectory best describes this board?",
        candidates: LABELS.map((id) => ({ id, description: why[id] })),
      }],
      policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" },
    };
  }

  /* read the engine's answer; anything other than a known label (including
     abstention) returns null so the caller falls back to the heuristic */
  function applyEngineAnswer(res) {
    if (!res || res.outcome !== "accept") return null;
    const ans = res.answers && res.answers[0];
    const id = ans && ans.choice;
    return LABELS.includes(id) ? { label: id, conf: ans.confidence || null } : null;
  }

  /* ---------- seed proposals ---------- */

  /* candidate seeds: small random soups in a sub-box; scored by a measured
     "interest" profile — death and static fill are boring, sustained
     structure is interesting. Returns candidates sorted by interest with
     the score tie left visible for the engine to break. */
  function seedCandidates(cfg, n, seedSalt) {
    const c = normalizeConfig(cfg);
    const rng = DK.makeRng(c.seed + ":proposals:" + (seedSalt || ""));
    const boxW = Math.min(24, c.cols);
    const boxH = Math.min(16, c.rows);
    const out = [];
    for (let k = 0; k < (n || 3); k++) {
      const cells = [];
      const density = 0.25 + rng() * 0.25;
      for (let y = 0; y < boxH; y++) {
        for (let x = 0; x < boxW; x++) {
          if (rng() < density) cells.push([x, y]);
        }
      }
      const probe = createWorld({ cols: c.cols, rows: c.rows, wrap: c.wrap, rule: c.rule, seed: c.seed });
      for (const [x, y] of cells) setCell(probe, x + ((c.cols - boxW) >> 1), y + ((c.rows - boxH) >> 1), 1);
      const interest = measureInterest(probe);
      out.push({ id: "soup-" + (k + 1), cells, interest: interest.score, profile: interest.profile });
    }
    out.sort((a, b) => b.interest - a.interest);
    return out;
  }

  /* simulate a probe for up to 90 gens and score the trajectory:
     extinction and instant stillness score low, sustained activity with
     population in a healthy band scores high */
  function measureInterest(probe) {
    const pops = [];
    let act = 0;
    for (let i = 0; i < 90 && probe.gen < probe.cfg.maxGens; i++) {
      step(probe);
      pops.push(probe.pop);
      act += probe.changed;
    }
    const maxPop = Math.max.apply(null, pops);
    const minPop = Math.min.apply(null, pops);
    const last = pops[pops.length - 1];
    const stillEarly = probe.period > 0 && probe.period <= 2 && probe.gen < 20;
    let score;
    if (last === 0) score = 0.05;                       /* died: boring */
    else if (stillEarly) score = 0.15;                  /* froze: boring */
    else {
      const alive = Math.min(1, last / (probe.cfg.cols * probe.cfg.rows * 0.12));
      const variety = Math.min(1, (maxPop - minPop) / Math.max(1, maxPop));
      const motion = Math.min(1, act / (probe.gen * probe.cfg.cols * probe.cfg.rows * 0.02));
      score = Math.round((0.4 * variety + 0.35 * motion + 0.25 * alive) * 100) / 100;
    }
    return { score, profile: { maxPop, minPop, last, changed: act, gens: probe.gen, period: probe.period } };
  }

  /* the genuine tie: top two candidates within epsilon → the engine's call */
  function pickSeed(cands, epsilon) {
    const eps = epsilon === undefined ? 0.02 : epsilon;
    if (cands.length < 2) return { best: cands[0], tie: null };
    const sorted = cands.slice().sort((a, b) => b.interest - a.interest);
    if (Math.abs(sorted[0].interest - sorted[1].interest) < eps) {
      return { best: sorted[0], tie: [sorted[0], sorted[1]] };
    }
    return { best: sorted[0], tie: null };
  }

  function seedRequest(cands, tiePair) {
    return {
      state: {
        text: "Two proposed Life seeds scored equally interesting (" +
          tiePair.map((s) => s.interest).join(" vs ") + "). Populations: " +
          tiePair.map((s) => s.profile.last).join(" vs ") +
          " after 90 generations. Pick one to load.",
        facts: {},
      },
      questions: [{
        type: "choice", id: "seed-pick",
        text: "Which seed should the sandbox load?",
        candidates: tiePair.map((s) => ({
          id: s.id,
          description: s.profile.last + " cells alive after 90 gens, peak " + s.profile.maxPop,
        })),
      }],
      policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" },
    };
  }

  /* stamp — verdicts only compare like-for-like */
  function experimentStamp(cfg) {
    const c = normalizeConfig(cfg);
    return [c.cols + "x" + c.rows, c.wrap ? "wrap" : "bounded",
      c.rule.label, c.seed].join("|");
  }

  /* ---------- exports ---------- */
  globalThis.LifeCore = {
    PRESETS, normalizeRule, normalizeConfig,
    createWorld, clearWorld, setCell, getCell, inBounds, step, worldKey,
    resetCycle, centroid,
    PATTERNS, stampPattern, randomSoup,
    features, LABELS, classify, engineRequest, applyEngineAnswer,
    seedCandidates, pickSeed, seedRequest, measureInterest, experimentStamp,
  };
})();
