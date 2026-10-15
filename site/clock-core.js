/* Rubik's Clock solver — core.
   The WCA Rubik's Clock as 18 dials mod 12 (front 0..8 row-major from the
   front, back 9..17 row-major from the BACK view, so the halves swap
   under y2 exactly like the WCA scrambler's own state encoding). On top
   of the moves: a validator that refuses unreachable states WITH the
   named reason (dial range, the four corner-pair laws), a deterministic
   solver that runs the published method shape — front edge wheels +
   ALL, y2, back edge wheels + ALL, y2, then the four corner wheels —
   with every amount solved exactly in Z_12 and every applied step
   replay-verified through the public move algebra, seeded WCA-style
   scrambles, and a solved verifier with two independent reads.

   The move table is DERIVED from the documented pin mechanism, never
   hand-transcribed: each move names the pins that are UP, the front
   dials that turn are the union of "the four dials around each up
   pin", and the reverse side sees exactly the up-pin corner dials,
   turned the opposite way. The derived table is checked at load
   against the reference rows of the WCA scrambler itself.

   Sources for the mechanism, the notation and the method shape:
   - WCA scrambler of record (move rows + scramble shape):
     https://github.com/thewca/tnoodle-lib  —
     scrambles/src/main/java/org/worldcubeassociation/tnoodle/puzzle/ClockPuzzle.java
     (the `moves` table and `generateRandomMoves`)
   - Pin-config semantics ("UR = only UR pin is up", "that dial turns
     only the 4 mini clocks around this pin", "All up: any dial turns
     all 9 mini clocks", y2 = flip about the y axis):
     https://www.speedsolving.com/wiki/index.php/Clock_notation
   - Mechanism and method shape (front/back corner clocks connected,
     Abelian group, 12^14 positions, beginner method outline):
     https://www.jaapsch.net/puzzles/clock.htm

   The shell (clock.js) renders the dials; this file never touches the
   DOM. */

(function () {
  "use strict";
  const DK = globalThis.DecisionsSDK;

  /* ---------- dials: layout, corners, pin geometry ---------- */

  const N = 18;
  const FRONT = 0, BACK = 9;          /* first index of each face's block */
  const SOLVED = new Array(N).fill(0);

  /* corner dial indices, front view: 0 UL, 2 UR, 6 DL, 8 DR */
  const FRONT_CORNERS = { UL: 0, UR: 2, DL: 6, DR: 8 };
  /* the same physical corner seen from the BACK view is mirrored
     left-right: back index = 9 + 3*row + (2 - col). */
  function backIndexOf(frontCorner) {
    const row = Math.floor(frontCorner / 3), col = frontCorner % 3;
    return BACK + row * 3 + (2 - col);
  }
  const CORNER_PAIRS = { UL: [0, 11], UR: [2, 9], DL: [6, 17], DR: [8, 15] };

  /* the four front dials around each corner pin (corner, two edges,
     center) — the wiki's "the 4 mini clocks around this pin" */
  const AROUND = {
    UL: [0, 1, 3, 4],
    UR: [1, 2, 4, 5],
    DL: [3, 4, 6, 7],
    DR: [4, 5, 7, 8],
  };

  /* which pins are UP for each move (wiki: "UR = Only UR pin is up",
     "U = Only UL and UR pins are up", "ALL = All pins are up") */
  const PINS_UP = {
    UR: ["UR"], DR: ["DR"], DL: ["DL"], UL: ["UL"],
    U: ["UL", "UR"], R: ["UR", "DR"], D: ["DR", "DL"], L: ["DL", "UL"],
    ALL: ["UL", "UR", "DL", "DR"],
  };

  const MOVES = ["UR", "DR", "DL", "UL", "U", "R", "D", "L", "ALL"];

  /* ---------- move table, derived then pinned to the reference ---------- */

  /* Front dials turn +1 (clockwise from the front); the reverse side
     turns the up-pin corner dials -1 (the same physical turn seen from
     behind). Jaap: "the front and back corner clocks are connected"
     and, for a corner turned next to an up button, "none of the clocks
     on the reverse side move, except for the corner clocks." */
  function deriveMove(name) {
    const v = new Array(N).fill(0);
    const seen = {};
    for (const pin of PINS_UP[name]) {
      for (const d of AROUND[pin]) { v[d] = 1; seen[d] = true; }
      v[backIndexOf(FRONT_CORNERS[pin])] = -1;
    }
    return v;
  }

  const MOVE_VEC = {};
  for (const m of MOVES) MOVE_VEC[m] = deriveMove(m);

  /* Reference rows transcribed from the WCA scrambler of record
     (tnoodle-lib ClockPuzzle.java, the `moves` int[][]). The derived
     table must equal these exactly — a mismatch is a load-time abort,
     not a silently wrong puzzle. */
  const REFERENCE = {
    UR: [0,1,1,0,1,1,0,0,0,  -1,0,0,0,0,0,0,0,0],
    DR: [0,0,0,0,1,1,0,1,1,   0,0,0,0,0,0,-1,0,0],
    DL: [0,0,0,1,1,0,1,1,0,   0,0,0,0,0,0,0,0,-1],
    UL: [1,1,0,1,1,0,0,0,0,   0,0,-1,0,0,0,0,0,0],
    U:  [1,1,1,1,1,1,0,0,0,  -1,0,-1,0,0,0,0,0,0],
    R:  [0,1,1,0,1,1,0,1,1,  -1,0,0,0,0,0,-1,0,0],
    D:  [0,0,0,1,1,1,1,1,1,   0,0,0,0,0,0,-1,0,-1],
    L:  [1,1,0,1,1,0,1,1,0,   0,0,-1,0,0,0,0,0,-1],
    ALL:[1,1,1,1,1,1,1,1,1,  -1,0,-1,0,0,0,-1,0,-1],
  };
  for (const m of MOVES) {
    if (MOVE_VEC[m].some((x, i) => x !== REFERENCE[m][i])) {
      throw new Error("derived move table disagrees with the WCA reference for " + m);
    }
  }

  /* ---------- move token algebra ---------- */

  /* wheel tokens: NAME [digits] SIGN ("UR3+", "DL+", "ALL8-", "UR4-");
     a bare sign means one hour; y2 flips the puzzle about the y axis. */
  const MOVE_RE = /^(UR|DR|DL|UL|U|R|D|L|ALL)(\d*)([+-])$/;

  function parseMove(tok) {
    if (tok === "y2") return { name: "y2", amount: 0 };
    const m = MOVE_RE.exec(tok);
    if (!m) throw new Error("bad move token: " + tok);
    const digits = m[2] === "" ? 1 : Number(m[2]);
    if (!Number.isInteger(digits) || digits < 0) {
      throw new Error("bad amount in move token: " + tok);
    }
    const signed = m[3] === "+" ? digits : -digits;
    return { name: m[1], amount: ((signed % 12) + 12) % 12 };
  }

  function applyVec(state, v, amount) {
    return state.map((x, i) => ((x + amount * v[i]) % 12 + 12) % 12);
  }

  function swapHalves(state) {
    return state.slice(BACK).concat(state.slice(FRONT, BACK));
  }

  function applyMove(state, tok) {
    const { name, amount } = parseMove(tok);
    if (name === "y2") return swapHalves(state);
    return applyVec(state, MOVE_VEC[name], amount);
  }

  function applyAlg(state, alg) {
    const toks = Array.isArray(alg)
      ? alg
      : String(alg).trim().split(/\s+/).filter(Boolean);
    let s = state;
    for (const tok of toks) s = applyMove(s, tok);
    return s;
  }

  /* the y2 image of a move vector: flipping the puzzle then applying a
     move equals applying the swapped-half vector then flipping */
  const swapVec = (v) => v.map((_, i) => v[(i + BACK) % N]);

  /* ---------- validator: refuse unreachable states, with reasons ---------- */

  /* Reachable set. The nine wheel generators (plus their y2 images)
     span a rank-14 subgroup of Z_12^18 — elimination over the mod-12
     generator matrix leaves exactly four independent constraints, so
     the puzzle has 12^14 reachable positions, which is Jaap's published
     count. The four constraints say: at every physical corner, the
     front dial and the back dial are negatives mod 12. Every state
     passing them IS reachable (the law-legal set also has 12^14
     elements, and the reachable set is contained in it), and the
     solver realizes that reachability constructively. */
  const PAIR_LAWS = ["UL", "UR", "DL", "DR"];

  function validateState(state) {
    const checks = [];
    const add = (name, ok, reason) => checks.push({ name, ok, reason: ok ? "" : reason });

    if (!Array.isArray(state) || state.length !== N) {
      add("shape", false, "state must be " + N + " dial readings, front 0..8 then back 9..17");
      return { ok: false, checks };
    }
    add("shape", true, "");

    for (let i = 0; i < N; i++) {
      const d = state[i];
      add("dial-" + i, Number.isInteger(d) && d >= 0 && d <= 11,
        "dial " + i + " reads " + String(d) + " — every dial is an integer 0..11 (hours past 12, mod 12)");
    }

    for (const k of PAIR_LAWS) {
      const [f, b] = CORNER_PAIRS[k];
      const sum = (((state[f] + state[b]) % 12) + 12) % 12;
      add("corner-pair-" + k, sum === 0,
        "corner " + k + ": front dial " + f + " (" + state[f] + ") + back dial " + b
        + " (" + state[b] + ") = " + sum + " mod 12 — the two dials of one physical corner"
        + " are negatives, so no pin-legal sequence reaches this state");
    }

    return { ok: checks.every((c) => c.ok), checks };
  }

  /* Two reads of "solved": the flat dial string and the per-face loop.
     For an additive puzzle they reduce to the same arithmetic; both are
     kept as independent code paths so a layout slip cannot pass one
     while passing the other. */
  function verifySolved(state) {
    const byDials = state.length === N && state.every((d) => d === 0);
    let byFace = Array.isArray(state) && state.length === N;
    for (let face = 0; face < 2 && byFace; face++) {
      for (let i = 0; i < 9; i++) {
        if (state[face * 9 + i] !== 0) { byFace = false; break; }
      }
    }
    return { solved: byDials && byFace, byDials, byFace };
  }

  /* ---------- solved state, config, scrambles ---------- */

  const DEFAULT_CONFIG = {
    seed: "clock-wca", scrambleLen: 6, engineTies: true,
  };

  function normalizeConfig(raw) {
    const c = raw || {};
    const num = (v, lo, hi, dflt) => (v === undefined || v === null
      || Number.isNaN(Number(v))) ? dflt : DK.clampf(Number(v), lo, hi, dflt);
    return {
      seed: c.seed === undefined || c.seed === null ? DEFAULT_CONFIG.seed : String(c.seed),
      /* wheel moves per side; the WCA scramble uses 9 distinct types
         front (UR DR DL UL U R D L ALL) and 5 on the back */
      scrambleLen: Math.round(num(c.scrambleLen, 1, 9, DEFAULT_CONFIG.scrambleLen)),
      engineTies: c.engineTies === undefined ? DEFAULT_CONFIG.engineTies : !!c.engineTies,
    };
  }

  const clockStamp = (cfg) => "clock|" + cfg.seed + "|" + cfg.scrambleLen
    + "|" + (cfg.engineTies ? 1 : 0);

  /* WCA-style scramble: front section (distinct move types, canonical
     UR DR DL UL U R D L ALL order), y2, back section. Amounts follow
     the scrambler of record: uniform -5..+5, sign from the raw draw
     (ClockPuzzle.java: turn = nextInt(12)-5, clockwise = turn >= 0). */
  function makeScramble(cfg) {
    const rng = DK.makeRng("clock-scramble:" + cfg.seed);
    const pick = (n) => {
      const idx = MOVES.map((_, i) => i);
      for (let i = idx.length - 1; i > 0; i--) {
        const j = Math.floor(rng() * (i + 1));
        [idx[i], idx[j]] = [idx[j], idx[i]];
      }
      return idx.slice(0, n).sort((a, b) => a - b).map((i) => MOVES[i]);
    };
    const token = (name) => {
      const raw = Math.floor(rng() * 12) - 5;
      return name + Math.abs(raw) + (raw >= 0 ? "+" : "-");
    };
    const section = (n) => pick(n).map(token);
    return section(cfg.scrambleLen).concat(["y2"], section(cfg.scrambleLen));
  }

  /* ---------- solver ---------- */

  /* The published method shape, as the scrambler itself moves:
     front edge wheels (U L R D ALL), y2, back edge wheels (U L R D
     ALL), y2, then the four corner wheels (UL UR DL DR). 14 unknowns.
     The clock is abelian and additive mod 12 (Jaap: "the order that
     the moves are performed is unimportant"), so those 14 vectors form
     a basis of the whole reachable subgroup (rank 14 — the same
     elimination that proves the four pair laws): for every law-legal
     state the linear system has exactly one solution, computed here by
     Gauss-Jordan over Z_12. Every nonzero table entry is ±1, a unit
     mod 12, so elimination never stalls on a non-invertible pivot.

     Termination is by construction, not by descent: the plan is a
     fixed 14-slot sequence whose amounts are solved up front, and
     every applied token is replayed through the public move algebra
     with the running state checked for law-legality — a divergence or
     an illegal intermediate refuses with a named error. */

  const SOLVE_PLAN = [
    { phase: "edges-front", title: "front edge wheels (U L R D ALL)",
      names: ["U", "L", "R", "D", "ALL"], flipped: false },
    { phase: "edges-back", title: "back edge wheels (U L R D ALL)",
      names: ["U", "L", "R", "D", "ALL"], flipped: true },
    { phase: "corners", title: "corner wheels (UL UR DL DR)",
      names: ["UL", "UR", "DL", "DR"], flipped: false },
  ];

  const SOLVE_VECS = [];
  for (const p of SOLVE_PLAN) {
    for (const name of p.names) {
      SOLVE_VECS.push(p.flipped ? swapVec(MOVE_VEC[name]) : MOVE_VEC[name].slice());
    }
  }

  function solveAmounts(state) {
    /* dial d equation: sum_i VEC[i][d] * t_i = -state[d]  (mod 12) */
    const rows = [];
    for (let d = 0; d < N; d++) {
      rows.push(SOLVE_VECS.map((v) => ((v[d] % 12) + 12) % 12)
        .concat([(((-state[d]) % 12) + 12) % 12]));
    }
    const width = SOLVE_VECS.length + 1;
    const rowOf = new Array(SOLVE_VECS.length).fill(-1);
    let row = 0;
    const invOf = (p) => {
      for (let q = 1; q < 12; q++) if ((p * q) % 12 === 1) return q;
      return 0; /* unreachable for ±1 pivots */
    };
    for (let c = 0; c < width - 1 && row < rows.length; c++) {
      let ri = -1;
      for (let i = row; i < rows.length; i++) {
        if (rows[i][c] % 12 !== 0) { ri = i; break; }
      }
      if (ri < 0) continue;
      [rows[row], rows[ri]] = [rows[ri], rows[row]];
      const inv = invOf(((rows[row][c] % 12) + 12) % 12);
      for (let j = 0; j < width; j++) rows[row][j] = (rows[row][j] * inv) % 12;
      for (let i = 0; i < rows.length; i++) {
        if (i === row || rows[i][c] % 12 === 0) continue;
        const f = rows[i][c];
        for (let j = 0; j < width; j++) {
          rows[i][j] = ((rows[i][j] - f * rows[row][j]) % 12 + 12) % 12;
        }
      }
      rowOf[c] = row;
      row++;
    }
    for (const r of rows) {
      if (r.slice(0, width - 1).every((x) => ((x % 12) + 12) % 12 === 0)
        && ((r[width - 1] % 12) + 12) % 12 !== 0) {
        throw new Error("state is not in the reachable orbit: the pair laws are violated");
      }
    }
    const amounts = new Array(SOLVE_VECS.length).fill(0);
    for (let c = 0; c < width - 1; c++) {
      if (rowOf[c] >= 0) amounts[c] = ((rows[rowOf[c]][width - 1] % 12) + 12) % 12;
    }
    return amounts;
  }

  /* amount -> token: 1..6 write "+", 7..11 write their complement with
     "-" (UR8+ and UR4- are the same turn) */
  function amountToken(name, amount) {
    const a = ((amount % 12) + 12) % 12;
    if (a === 0) return null;
    return a <= 6 ? name + a + "+" : name + (12 - a) + "-";
  }

  function solve(state) {
    const verdict = validateState(state);
    if (!verdict.ok) {
      const bad = verdict.checks.filter((c) => !c.ok).map((c) => c.name).join(", ");
      throw new Error("cannot solve: state fails validation (" + bad + ")");
    }
    if (verifySolved(state).solved) {
      return { moves: [], steps: [], final: state.slice(), wheelMoves: 0,
        verified: verifySolved(state), legal: true };
    }

    const amounts = solveAmounts(state);

    /* build the token plan: skip zero amounts (no-ops); a flipped
       section plays as y2, section, y2 */
    const plan = [];
    let k = 0;
    for (const p of SOLVE_PLAN) {
      const toks = [];
      for (const name of p.names) {
        const tok = amountToken(name, amounts[k++]);
        if (tok) toks.push({ tok, phase: p.phase, title: p.title });
      }
      if (toks.length) {
        if (p.flipped) plan.push({ tok: "y2", phase: p.phase + "-flip", title: "y2 flip" });
        plan.push(...toks);
        if (p.flipped) plan.push({ tok: "y2", phase: p.phase + "-flip", title: "y2 flip back" });
      }
    }

    /* replay-verify: every token through the public algebra, every
       intermediate law-legal, the end all-zeros */
    let cur = state.slice();
    const steps = [];
    const moves = [];
    for (let i = 0; i < plan.length; i++) {
      const item = plan[i];
      cur = applyMove(cur, item.tok);
      const legal = validateState(cur).ok;
      if (!legal) {
        throw new Error("step " + i + " (" + item.tok + ") produced an unreachable state");
      }
      moves.push(item.tok);
      steps.push({ phase: item.phase, title: item.title, moves: [item.tok],
        stateAfter: cur.slice() });
    }
    const verified = verifySolved(cur);
    if (!verified.solved) {
      throw new Error("solve plan did not reach all-zeros (replay diverged from the algebra)");
    }
    const wheelMoves = moves.filter((t) => t !== "y2").length;
    return { moves, steps, final: cur, wheelMoves, verified, legal: true };
  }

  /* ---------- game ---------- */

  function createGame(cfgRaw) {
    const cfg = normalizeConfig(cfgRaw);
    const scramble = makeScramble(cfg);
    const state = applyAlg(SOLVED, scramble);
    return { cfg, stamp: clockStamp(cfg), scramble, state,
      solvedStart: verifySolved(state).solved };
  }

  globalThis.ClockCore = {
    N, FRONT, BACK, SOLVED, MOVES, MOVE_VEC, REFERENCE,
    FRONT_CORNERS, CORNER_PAIRS, AROUND, PINS_UP, PAIR_LAWS, SOLVE_PLAN,
    backIndexOf, swapVec, swapHalves,
    parseMove, applyMove, applyAlg, applyVec,
    validateState, verifySolved, solve, solveAmounts,
    DEFAULT_CONFIG, normalizeConfig, clockStamp, makeScramble, createGame,
  };
})();
