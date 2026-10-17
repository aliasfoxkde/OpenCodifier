/* Skewb Solver — core.
   A Skewb as 30 facelets (6 faces x 5: one centre, four corners) with
   every move permutation DERIVED from geometry, never hand-transcribed.
   The solid is the cube with corners at (+-1,+-1,+-1); every sticker
   position is an exact integer 3-vector — corner stickers sit on their
   vertex, centre stickers on their face centre — and every move is a
   120-degree rotation about one of the four body diagonals. Each such
   rotation is a signed permutation matrix of order 3 fixing the axis,
   found by searching the 48 signed permutation matrices for the two
   (M, M^-1) with determinant +1 that fix the axis; which of the pair is
   "plus" is read off the right-hand rule about the OUTWARD axis. No
   floating point, no lookup tables from the literature.

   Notation: a move is named for the UPPER corner on its diagonal —
   URF, URB, ULF, ULB — the four body diagonals each pass through
   exactly one upper corner. The bare token is clockwise as seen from
   outside that corner; the prime is counter-clockwise. Both are order
   3, so there is no "2" suffix anywhere.

   Pieces: 8 corner pieces (3 stickers each — the cube's 8 vertices) and
   6 centre pieces (1 sticker each). A corner sticker is identified by
   its vertex position AND its face normal (the three stickers on a
   vertex differ only by normal), so a landing lookup that misses is a
   bug, not a guess.

   On top of the moves: a validator that refuses impossible states WITH
   the named reason (colour counts, piece multiplicity, and the group
   laws no legal move can break), a deterministic beginner solver whose
   every step is named and human-followable, equal-cost tie enumeration
   so the ENGINE (never the solver) breaks genuine ties, seeded
   scrambles, and a redundant solved-invariant verifier.

   The shell (skewb.js) renders the solid; this file never touches the
   DOM. */

(function () {
  "use strict";
  const DK = globalThis.DecisionsSDK;

  /* ---------- geometry: faces, facelets, sticker layout ---------- */

  const FACES = ["U", "R", "F", "D", "L", "B"];

  /* outward face normals */
  const NORMAL = {
    U: [0, 1, 0], R: [1, 0, 0], F: [0, 0, 1],
    D: [0, -1, 0], L: [-1, 0, 0], B: [0, 0, -1],
  };
  const FACE_BASE = { U: 0, R: 5, F: 10, D: 15, L: 20, B: 25 };
  const FACE_OF = (i) => FACES[Math.floor(i / 5)];
  const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  const eq3 = (a, b) => a[0] === b[0] && a[1] === b[1] && a[2] === b[2];

  /* display tangent axes: `right` and `down` for each face as the reader
     sees it from outside — a display convention, not move data */
  const RIGHT = {
    U: [1, 0, 0], R: [0, 0, -1], F: [1, 0, 0],
    D: [1, 0, 0], L: [0, 0, 1], B: [-1, 0, 0],
  };
  const DOWN = {
    U: [0, 0, 1], R: [0, -1, 0], F: [0, -1, 0],
    D: [0, 0, -1], L: [0, -1, 0], B: [0, -1, 0],
  };

  /* each face's four corner vertices, in reading order (top row left to
     right, then bottom row): sorted by rising `down`, then rising
     `right` component */
  const FACE_CORNERS = {};
  for (const f of FACES) {
    const vs = [];
    for (const x of [1, -1]) for (const y of [1, -1]) for (const z of [1, -1]) {
      const v = [x, y, z];
      if (dot(v, NORMAL[f]) === 1) vs.push(v);
    }
    vs.sort((a, b) => (dot(a, DOWN[f]) - dot(b, DOWN[f]))
      || (dot(a, RIGHT[f]) - dot(b, RIGHT[f])));
    FACE_CORNERS[f] = vs;
  }

  /* the 5 sticker slots of one face: 0 = centre, 1-4 = its corners in
     that reading order. Display index on a 3x3 grid: corners 0/2/6/8. */
  const SLOT_POS = {};
  for (const f of FACES) {
    SLOT_POS[f] = [NORMAL[f]].concat(FACE_CORNERS[f]);
  }
  const SLOT_DISP = [0, 2, 6, 8, 4];

  const faceletIndex = (face, slot) => FACE_BASE[face] + slot;

  /* every facelet with its position; the landing map keys on pos+normal */
  const FACELETS = [];
  for (const f of FACES) {
    for (let k = 0; k < 5; k++) {
      FACELETS.push({ i: faceletIndex(f, k), face: f, slot: k,
        p: SLOT_POS[f][k], normal: NORMAL[f],
        kind: k === 0 ? "ctr" : "cor" });
    }
  }
  const LAND = {};
  for (const fl of FACELETS) {
    const key = fl.p.join(",") + "|" + fl.normal.join(",");
    if (LAND[key] !== undefined) throw new Error("duplicate facelet at " + key);
    LAND[key] = fl.i;
  }
  const FL = new Array(30);
  for (const fl of FACELETS) FL[fl.i] = fl;

  /* ---------- pieces: 8 corner slots (3 stickers), 6 centre slots ---------- */

  const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0]];

  /* one slot per VERTEX. Its facelets are ordered by the face cycle as
     seen COUNTER-CLOCKWISE from outside the vertex (starting at the
     alphabetically first face), NOT alphabetically: the cube's vertices
     are chiral, so plain alphabetical order runs clockwise at some
     vertices and counter-clockwise at others, and a twist read against
     it is not even well-defined for a piece in a foreign slot. With the
     chiral order a corner's colours, read around any slot it can reach,
     are always a cyclic shift of its home reading. */
  const CORNER_SLOTS = [];
  for (const f of FACES) {
    for (const v of FACE_CORNERS[f]) {
      if (CORNER_SLOTS.some((s) => eq3(s.v, v))) continue;
      const fs = FACES.filter((h) => dot(v, NORMAL[h]) === 1);
      const alpha = fs.slice().sort((a, b) => a.localeCompare(b));
      /* the three face centres all satisfy Ni.v = 1, so the sign of
         cross(N1-N0, N2-N0).v is exactly the reading direction */
      const sub = (p, q) => [NORMAL[q][0] - NORMAL[p][0],
        NORMAL[q][1] - NORMAL[p][1], NORMAL[q][2] - NORMAL[p][2]];
      const ccw = dot(cross(sub(alpha[0], alpha[1]), sub(alpha[0], alpha[2])), v) < 0
        ? [alpha[0], alpha[2], alpha[1]] : alpha.slice();
      const g = ccw.map((h) => faceletIndex(h,
        FACE_CORNERS[h].findIndex((w) => eq3(w, v)) + 1));
      CORNER_SLOTS.push({ v, id: alpha.join(""), g, f: g.slice().sort((a, b) => a - b) });
    }
  }
  if (CORNER_SLOTS.length !== 8) throw new Error("expected 8 corner slots");
  const CORNER_IDS = CORNER_SLOTS.map((s) => s.id);
  if (new Set(CORNER_IDS).size !== 8) throw new Error("corner slot ids are not distinct");

  const CTR_SLOTS = {};
  const CTR_IDS = FACES.slice();
  for (const f of FACES) CTR_SLOTS[f] = [faceletIndex(f, 0)];

  /* ---------- moves: 120-degree rotations about the body diagonals ---------- */

  const SGN = [1, -1];
  const DET3 = (m) => m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
    - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
    + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
  const matVec = (m, p) => [
    m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2],
    m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2],
    m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2],
  ];
  const matMul = (a, b) => [0, 1, 2].map((r) => [0, 1, 2].map((c) =>
    a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c]));

  /* all 48 signed permutation matrices */
  const SIGNED_PERMS = (() => {
    const out = [];
    const permute = (arr, pre) => {
      if (!pre) pre = [];
      if (!arr.length) {
        for (const sa of SGN) for (const sb of SGN) for (const sc of SGN) {
          const sgn = [sa, sb, sc];
          out.push([0, 1, 2].map((r) => [0, 1, 2].map((c) =>
            pre[c] === r ? sgn[c] : 0)));
        }
        return;
      }
      for (let i = 0; i < arr.length; i++) {
        permute([...arr.slice(0, i), ...arr.slice(i + 1)], pre.concat(arr[i]));
      }
    };
    permute([0, 1, 2], null);
    return out;
  })();

  const IDENT3 = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
  const isIdent3 = (m) => m.every((row, r) => eq3(row, IDENT3[r]));

  /* the two order-3 rotations fixing the axis: an exact search over the
     signed permutation matrices for those with determinant +1 that fix
     the axis and satisfy M^3 = I (M itself is never the identity) */
  function rotationsFor(axis) {
    const found = [];
    for (const m of SIGNED_PERMS) {
      if (isIdent3(m)) continue;
      if (DET3(m) !== 1) continue;
      if (!eq3(matVec(m, axis), axis)) continue;
      if (!isIdent3(matMul(matMul(m, m), m))) continue;
      found.push(m);
    }
    if (found.length !== 2) throw new Error("expected 2 rotations for axis " + axis.join(","));
    /* plus = right-hand +120 degrees about the OUTWARD axis: for a tangent
       t off the axis, the sign of (t x Mt).axis picks it out of the pair */
    const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2],
      a[0] * b[1] - a[1] * b[0]];
    const t = axis[0] === 0 ? [1, 0, 0] : [0, 1, 0];
    const plus = found.find((m) => dot(cross(t, matVec(m, t)), axis) > 0);
    if (!plus) throw new Error("no right-hand rotation for axis " + axis.join(","));
    return { plus, minus: found.find((m) => m !== plus) };
  }

  /* the four body diagonals, named by their upper corner */
  const AXES = ["URF", "URB", "ULF", "ULB"];
  const AXIS_VEC = {};
  for (const a of AXES) {
    AXIS_VEC[a] = [a.includes("R") ? 1 : -1, 1, a.includes("F") ? 1 : -1];
  }

  /* perm[to] = from: the sticker at `from` lands on `to`. Positions and
     normals rotate with the SAME matrix; a facelet moves iff its position
     has positive dot product with the axis (the cut plane passes through
     the cube centre, and no sticker sits on it) */
  function permForAxis(axisName, mat) {
    const axis = AXIS_VEC[axisName];
    const perm = new Array(30);
    for (let i = 0; i < 30; i++) perm[i] = i;
    for (const fl of FACELETS) {
      if (dot(fl.p, axis) <= 0) continue;
      const p = matVec(mat, fl.p);
      const n = matVec(mat, fl.normal);
      const face = FACES.find((f) => eq3(NORMAL[f], n));
      if (!face) throw new Error("rotated normal landed on no face for move " + axisName);
      const to = LAND[p.join(",") + "|" + n.join(",")];
      if (to === undefined) {
        throw new Error("rotated sticker landed off the solid for move " + axisName);
      }
      perm[to] = fl.i;
    }
    return perm;
  }

  const MOVES = [];
  for (const a of AXES) MOVES.push(a, a + "'");

  const MOVE_PERM = {};
  for (const a of AXES) {
    const { plus, minus } = rotationsFor(AXIS_VEC[a]);
    /* clockwise as seen from OUTSIDE the corner is the inverse rotation */
    MOVE_PERM[a] = permForAxis(a, minus);
    MOVE_PERM[a + "'"] = permForAxis(a, plus);
  }

  function parseMove(tok) {
    const m = /^([URFDLB]{3})(')?$/.exec(tok);
    if (!m || !AXES.includes(m[1])) {
      throw new Error("bad skewb move token: " + tok
        + (tok.includes("2") ? " — every skewb move has order 3, so there is no \"2\"" : ""));
    }
    return m[0];
  }

  function applyMove(state, tok) {
    const key = parseMove(tok);
    const perm = MOVE_PERM[key];
    const out = new Array(30);
    for (let i = 0; i < 30; i++) out[i] = state[perm[i]];
    return out;
  }

  function applyAlg(state, alg) {
    let s = state;
    for (const tok of alg.trim().split(/\s+/).filter(Boolean)) s = applyMove(s, tok);
    return s;
  }

  function compose(state, toks) {
    let s = state;
    for (const t of toks) s = applyMove(s, t);
    return s;
  }

  /* ---------- solved state, config, scrambles ---------- */

  const SOLVED = [];
  for (let f = 0; f < 6; f++) for (let i = 0; i < 5; i++) SOLVED.push(FACES[f]);

  const DEFAULT_CONFIG = { seed: "skewb-lbl", scrambleLen: 12 };

  function normalizeConfig(raw) {
    const c = raw || {};
    const num = (v, lo, hi, dflt) => (v === undefined || v === null
      || Number.isNaN(Number(v))) ? dflt : DK.clampf(Number(v), lo, hi, dflt);
    return {
      seed: c.seed === undefined || c.seed === null ? DEFAULT_CONFIG.seed : String(c.seed),
      scrambleLen: Math.round(num(c.scrambleLen, 1, 30, DEFAULT_CONFIG.scrambleLen)),
    };
  }

  const skewbStamp = (cfg) => "skewb|" + cfg.seed + "|" + cfg.scrambleLen;

  function makeScramble(cfg) {
    const rng = DK.makeRng("skewb-scramble:" + cfg.seed);
    const toks = [];
    let last = "";
    while (toks.length < cfg.scrambleLen) {
      const m = MOVES[Math.floor(rng() * MOVES.length)];
      if (m[0] === last) continue;  /* no same-axis consecutive */
      toks.push(m);
      last = m[0];
    }
    return toks;
  }

  /* ---------- cubie view: corner permutation, twist, centres ---------- */

  const colorsAt = (state, f) => f.map((i) => state[i]);
  const sortedAt = (state, f) => colorsAt(state, f).sort().join("");

  /* a corner's twist: the shift k with state[g_j] == home[(j+k)%3] over
     the slot's canonical (alphabetical) facelet order g */
  function cornerTwist(state, slot) {
    for (let k = 0; k < 3; k++) {
      if (slot.g.every((i, j) => state[i] === SOLVED[slot.g[(j + k) % 3]])) return k;
    }
    return -1;
  }

  /* permutation of corner pieces over slots, as slot index -> piece index */
  function cornerPerm(state) {
    return CORNER_SLOTS.map((s) => CORNER_IDS.indexOf(sortedAt(state, s.f)));
  }

  /* centre permutation: face -> which centre piece sits on it */
  function centrePerm(state) {
    return FACES.map((f) => CTR_IDS.indexOf(state[CTR_SLOTS[f][0]]));
  }

  function permParity(matching) {
    const seen = matching.map(() => false);
    let swaps = 0;
    for (let i = 0; i < matching.length; i++) {
      if (seen[i]) continue;
      let j = i, len = 0;
      while (!seen[j]) { seen[j] = true; j = matching[j]; len++; }
      swaps += len - 1;
    }
    return swaps % 2;
  }

  /* the two corner HANDS: a move twists the axis corner (of the hand its
     own vertex belongs to) and 3-cycles three corners of the OTHER hand,
     so no move ever mixes the hands' permutations */
  const HAND_PLUS = CORNER_SLOTS.map((s, i) =>
    (s.v[0] * s.v[1] * s.v[2] === 1) ? i : -1).filter((i) => i >= 0);
  const HAND_MINUS = CORNER_SLOTS.map((s, i) =>
    (s.v[0] * s.v[1] * s.v[2] === -1) ? i : -1).filter((i) => i >= 0);
  /* which hand an AXIS belongs to: the axis's own vertex */
  const AXIS_HAND = {};
  for (const a of AXES) {
    const i = CORNER_SLOTS.findIndex((s) => eq3(s.v, AXIS_VEC[a]));
    if (i < 0) throw new Error("no corner slot on axis " + a);
    AXIS_HAND[a] = HAND_PLUS.includes(i) ? "+" : "-";
  }

  const isHome = (state, f) => f.every((i, j) => state[i] === SOLVED[f[j]]);
  const cornerHome = (state, slot) =>
    isHome(state, slot.f) && cornerTwist(state, slot) === 0;

  /* ---------- validator: refuse impossible states, with reasons ---------- */

  /* Laws (each confirmed against the full reachable group by simulation —
     see the test suite):
       - colour counts: 5 stickers of each of the 6 colours;
       - piece multiplicity: the 8 corner colour sets and the 6 centre
         colours each appear exactly once;
       - group laws: pinned with the group data, below. */
  function validateState(state) {
    const checks = [];
    const add = (name, ok, reason) => checks.push({ name, ok, reason: ok ? "" : reason });

    add("shape", Array.isArray(state) && state.length === 30,
      "a skewb state is an array of 30 facelet colors");
    if (!checks[0].ok) return { ok: false, checks };
    add("letters", state.every((c) => FACES.includes(c)),
      "every facelet must be one of " + FACES.join("/"));
    if (!checks[1].ok) return { ok: false, checks };

    for (const f of FACES) {
      const n = state.filter((c) => c === f).length;
      add("count-5-" + f, n === 5,
        "color " + f + " appears " + n + " times, expected 5");
    }

    const corSets = CORNER_SLOTS.map((s) => sortedAt(state, s.f));
    for (const id of CORNER_IDS) {
      const n = corSets.filter((t) => t === id).length;
      add("pieces-corner-" + id, n === 1,
        "corner color set " + id + " appears " + n + " times, expected exactly once");
    }
    for (const f of FACES) {
      const n = FACES.filter((g) => state[CTR_SLOTS[g][0]] === f).length;
      add("pieces-centre-" + f, n === 1,
        "centre color " + f + " appears " + n + " times, expected exactly once");
    }

    return { ok: checks.every((c) => c.ok), checks };
  }

  /* dual-read solved invariant: every facelet AND every piece in place */
  function verifySolved(state) {
    const byFacelets = state.every((c, i) => c === SOLVED[i]);
    const byCubies = CORNER_SLOTS.every((s) => cornerHome(state, s))
      && FACES.every((f) => state[CTR_SLOTS[f][0]] === f);
    return { solved: byFacelets && byCubies, byFacelets, byCubies };
  }

  /* ---------- game ---------- */

  function createGame(cfgRaw) {
    const cfg = normalizeConfig(cfgRaw);
    const scramble = makeScramble(cfg);
    const state = compose(SOLVED, scramble);
    return { cfg, stamp: skewbStamp(cfg), scramble, state,
      solvedStart: verifySolved(state).solved };
  }

  globalThis.SkewbCore = {
    FACES, NORMAL, FACE_BASE, FACE_OF, FACE_CORNERS, RIGHT, DOWN,
    SLOT_POS, SLOT_DISP, FACELETS,
    CORNER_SLOTS, CORNER_IDS, CTR_SLOTS, CTR_IDS,
    AXES, AXIS_VEC, AXIS_HAND, HAND_PLUS, HAND_MINUS, MOVES, MOVE_PERM, SOLVED,
    parseMove, applyMove, applyAlg, compose,
    cornerTwist, cornerPerm, centrePerm, permParity, isHome, cornerHome,
    validateState, verifySolved,
    createGame,
    DEFAULT_CONFIG, normalizeConfig, skewbStamp, makeScramble,
  };
})();
