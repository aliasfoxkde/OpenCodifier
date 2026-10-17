/* Pyraminx Solver — core.
   A tetrahedral Pyraminx as 36 facelets (4 faces x 9) with every move
   permutation DERIVED from geometry, never hand-transcribed. The solid is
   the cube-corner tetrahedron: vertices u=(1,1,1), l=(1,-1,-1),
   r=(-1,1,-1), b=(-1,-1,1); every sticker position is written in integer
   barycentric coordinates (x9, so all arithmetic stays exact) and every
   move is a 120-degree rotation about a body axis — a signed permutation
   matrix, computed once from the axis and applied to BOTH the sticker
   position and its face normal. No floating point, no lookup tables from
   the literature.

   Notation (WCA): an uppercase letter twists the layer around that
   VERTEX — the tip, the axial center under it, and the three edges that
   touch it, together (a "double-layer turn"); the lowercase letter twists
   the bare tip. Both are of order 3, so there is no "2" suffix anywhere:
   two clockwise turns already equal one counter-clockwise.

   Pieces: 4 tips (3 stickers, one per adjacent face), 4 axial centers
   (3 stickers), 6 edges (2 stickers). Faces are named after the vertex
   they do NOT contain (face U is opposite vertex u), so every vertex
   shares its letter with the one face it is not on.

   On top of the moves: a validator that refuses impossible states WITH
   the named reason (color counts, piece multiplicity, the two laws no
   legal move can break: even edge permutation and an even number of
   flipped edges), a deterministic beginner solver — tips, first-face
   edges, then the last three edges and the center twists together over a
   precomputed distance table — whose every step is named and
   human-followable, equal-cost tie enumeration so the ENGINE (never the
   solver) breaks genuine ties, seeded scrambles, and a redundant
   solved-invariant verifier.

   The shell (pyraminx.js) renders the solid; this file never touches the
   DOM. */

(function () {
  "use strict";
  const DK = globalThis.DecisionsSDK;

  /* ---------- geometry: vertices, faces, sticker layout ---------- */

  const FACES = ["U", "L", "R", "B"];

  /* the four vertices, named by the lowercase tip move that twists them */
  const VERT = {
    U: [1, 1, 1], L: [1, -1, -1], R: [-1, 1, -1], B: [-1, -1, 1],
  };

  /* face X is the face OPPOSITE vertex x (vertex x is not on face X —
     that is the standard naming), so its outward normal is -vertex */
  const FACE_NORMAL = {};
  for (const f of FACES) FACE_NORMAL[f] = VERT[f].map((v) => -v);
  const FACE_BASE = { U: 0, L: 9, R: 18, B: 27 };
  const FACE_OF = (i) => FACES[Math.floor(i / 9)];

  /* the three faces meeting at vertex x = every face except X */
  const FACES_AT = {};
  for (const v of FACES) FACES_AT[v] = FACES.filter((f) => f !== v);
  /* face X's three corners, in canonical U,L,R,B vertex order (c0,c1,c2) */
  const FACE_CORNERS = {};
  for (const f of FACES) {
    FACE_CORNERS[f] = ["U", "L", "R", "B"].filter((v) => v !== f);
  }

  /* the 9 sticker slots of one face in integer barycentrics x9 over
     (c0,c1,c2). Read the face as a 3-row triangle reading c0,c1,c2
     left-to-right:
       row 0 (1 sticker):  the c0 tip
       row 1 (3 stickers): the c0c2 edge, the c0 center, the c0c1 edge
       row 2 (5 stickers): the c2 tip, the c2 center, the c1c2 edge,
                           the c1 center, the c1 tip
     Slot order below is by PIECE role; SLOT_DISP maps each slot to its
     position in that display order (row-major, columns by rising c1
     weight). */
  const SLOT_BARY = [
    [8, 1, 1], /* 0: tip sticker at c0        */
    [4, 4, 1], /* 1: edge sticker on c0c1     */
    [4, 1, 4], /* 2: edge sticker on c0c2     */
    [5, 2, 2], /* 3: center sticker at c0     */
    [2, 5, 2], /* 4: center sticker at c1     */
    [2, 2, 5], /* 5: center sticker at c2     */
    [1, 8, 1], /* 6: tip sticker at c1        */
    [1, 4, 4], /* 7: edge sticker on c1c2     */
    [1, 1, 8], /* 8: tip sticker at c2        */
  ];
  const SLOT_DISP = [0, 3, 1, 2, 7, 5, 8, 6, 4];
  /* role of each slot: which vertex(ices) it belongs to, as c-indices */
  const SLOT_VERTS = [[0], [0, 1], [0, 2], [0], [1], [2], [1], [1, 2], [2]];
  const SLOT_KIND = ["tip", "edge", "edge", "ctr", "ctr", "ctr", "tip", "edge", "tip"];

  /* exact sticker position: integer 3-vector = 9 x the real barycentric
     point (a*c0 + b*c1 + g*c2)/9 */
  function pos9(face, slot) {
    const [a, b, g] = SLOT_BARY[slot];
    const [c0, c1, c2] = FACE_CORNERS[face].map((v) => VERT[v]);
    return [
      a * c0[0] + b * c1[0] + g * c2[0],
      a * c0[1] + b * c1[1] + g * c2[1],
      a * c0[2] + b * c1[2] + g * c2[2],
    ];
  }

  const faceletIndex = (face, slot) => FACE_BASE[face] + SLOT_DISP[slot];

  /* every facelet with its position; the landing map keys on (face, pos) */
  const FACELETS = [];
  for (const f of FACES) {
    for (let k = 0; k < 9; k++) {
      FACELETS.push({ i: faceletIndex(f, k), face: f, slot: k,
        p9: pos9(f, k), normal: FACE_NORMAL[f],
        kind: SLOT_KIND[k], verts: SLOT_VERTS[k] });
    }
  }
  const LAND = {};
  for (const fl of FACELETS) LAND[fl.face + "|" + fl.p9.join(",")] = fl.i;
  const FL = new Array(36);
  for (const fl of FACELETS) FL[fl.i] = fl;

  /* ---------- pieces: 4 tips, 4 centers, 6 edges ---------- */

  /* piece ids are the sorted face letters the piece's stickers live on
     when solved — tip u and center u both show one sticker on each of
     faces L,R,B, so they need a kind prefix to stay distinct. */
  /* tip and center slots, by c-index within a face (see SLOT_BARY) */
  const TIP_SLOT_OF_C = [0, 6, 8];
  const CTR_SLOT_OF_C = [3, 4, 5];
  const TIP_SLOTS = {};  /* vertex -> [3 facelet indices, FACES_AT order] */
  const CTR_SLOTS = {};
  const TIP_IDS = [];
  const CTR_IDS = [];
  for (const v of FACES) {
    TIP_SLOTS[v] = [];
    CTR_SLOTS[v] = [];
    for (const f of FACES_AT[v]) {
      const ci = FACE_CORNERS[f].indexOf(v);
      TIP_SLOTS[v].push(faceletIndex(f, TIP_SLOT_OF_C[ci]));
      CTR_SLOTS[v].push(faceletIndex(f, CTR_SLOT_OF_C[ci]));
    }
    TIP_IDS.push(FACES_AT[v].slice().sort().join(""));
    CTR_IDS.push(FACES_AT[v].slice().sort().join(""));
  }

  /* the 6 edges: one per unordered vertex pair, on the two faces that
     contain both vertices */
  const EDGE_SLOTS = [];
  for (let a = 0; a < 4; a++) {
    for (let b = a + 1; b < 4; b++) {
      const va = FACES[a], vb = FACES[b];
      /* the two faces through this edge, in ascending face-letter order —
         the slot's canonical order (face-letter order is B<L<R<U, NOT the
         U,L,R,B list order) */
      const fs = FACES_AT[va].filter((f) => FACES_AT[vb].includes(f)).sort();
      const verts = [va, vb].sort();
      const want = verts.join("");
      const g = [];  /* facelets in face-letter order: the slot's canonical order */
      for (const face of fs) {
        const k = SLOT_BARY.findIndex((bary, i) => SLOT_KIND[i] === "edge"
          && SLOT_VERTS[i].map((x) => FACE_CORNERS[face][x]).sort().join("") === want);
        if (k < 0) throw new Error("no edge slot for " + verts.join(""));
        g.push(faceletIndex(face, k));
      }
      EDGE_SLOTS.push({ id: fs.slice().sort().join(""), verts,
        g, f: g.slice().sort((x, y) => x - y) });
    }
  }
  const EDGE_IDS = EDGE_SLOTS.map((s) => s.id);

  /* the layer of uppercase moves about vertex v: tip v + center v + the
     three edges through v. Lowercase v moves tip v alone. Membership is
     by PIECE (vertex set), not by a dot-product test: in a tetrahedron
     the edge midpoints are not separated by any plane through the axis. */
  const LAYER_UPPER = {};
  const LAYER_TIP = {};
  for (const v of FACES) {
    const set = [];
    for (const i of TIP_SLOTS[v]) set.push(i);
    for (const i of CTR_SLOTS[v]) set.push(i);
    for (const e of EDGE_SLOTS) {
      if (e.verts.includes(v)) for (const i of e.f) set.push(i);
    }
    LAYER_UPPER[v] = set.sort((a, b) => a - b);
    LAYER_TIP[v] = TIP_SLOTS[v].slice().sort((a, b) => a - b);
  }

  /* ---------- moves: 120-degree rotations, exact integer matrices ---------- */

  /* Right-hand +120 degrees about axis s=(+-1,+-1,+-1): the image of each
     basis vector e is (-e + (s x e) + (s.e) s) / 2 — always integral
     because |s|^2 = 3. Clockwise as seen from OUTSIDE the vertex (the
     notation's direction) is the inverse = the transpose matrix. */
  function plusMat(s) {
    const cols = [];
    for (const e of [[1, 0, 0], [0, 1, 0], [0, 0, 1]]) {
      const se = s[0] * e[0] + s[1] * e[1] + s[2] * e[2];
      const cx = [s[1] * e[2] - s[2] * e[1],
        s[2] * e[0] - s[0] * e[2],
        s[0] * e[1] - s[1] * e[0]];
      cols.push([(-e[0] + cx[0] + s[0] * se) / 2,
        (-e[1] + cx[1] + s[1] * se) / 2,
        (-e[2] + cx[2] + s[2] * se) / 2]);
    }
    return [
      [cols[0][0], cols[1][0], cols[2][0]],
      [cols[0][1], cols[1][1], cols[2][1]],
      [cols[0][2], cols[1][2], cols[2][2]],
    ];
  }
  const transpose = (m) => [
    [m[0][0], m[1][0], m[2][0]],
    [m[0][1], m[1][1], m[2][1]],
    [m[0][2], m[1][2], m[2][2]],
  ];
  const applyMat = (m, p) => [
    m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2],
    m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2],
    m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2],
  ];
  const eq3 = (a, b) => a[0] === b[0] && a[1] === b[1] && a[2] === b[2];

  /* perm[to] = from: the sticker at `from` lands on `to`. Positions and
     normals rotate with the SAME matrix; a miss is a bug, not a guess. */
  function permForAxis(axis, mat) {
    const perm = new Array(36);
    /* layer tables are keyed by the uppercase vertex letter */
    const v = axis.toUpperCase();
    const layer = axis === v ? LAYER_UPPER[v] : LAYER_TIP[v];
    for (const from of layer) {
      const fl = FL[from];
      const p = applyMat(mat, fl.p9);
      const n = applyMat(mat, fl.normal);
      const face = FACES.find((f) => eq3(FACE_NORMAL[f], n));
      if (!face) throw new Error("rotated normal landed on no face for move " + axis);
      const to = LAND[face + "|" + p.join(",")];
      if (to === undefined) throw new Error("rotated sticker landed off the solid for move " + axis);
      perm[to] = from;
    }
    for (let i = 0; i < 36; i++) if (perm[i] === undefined) perm[i] = i;
    return perm;
  }

  /* move tokens: U L R B (layer turns) and u l r b (bare tips), each with
     a ' for counter-clockwise. NO "2" token exists: both are order 3. */
  const MOVES = [];
  for (const f of FACES) MOVES.push(f, f + "'", f.toLowerCase(), f.toLowerCase() + "'");

  const MOVE_PERM = {};
  for (const f of FACES) {
    const plus = plusMat(VERT[f]);
    MOVE_PERM[f] = permForAxis(f, transpose(plus));        /* clockwise */
    MOVE_PERM[f + "'"] = permForAxis(f, plus);             /* counter */
    const lo = f.toLowerCase();
    MOVE_PERM[lo] = permForAxis(lo, transpose(plus));
    MOVE_PERM[lo + "'"] = permForAxis(lo, plus);
  }

  function parseMove(tok) {
    const m = /^([ULRBulrb])(')?$/.exec(tok);
    if (!m) {
      throw new Error("bad pyraminx move token: " + tok
        + (tok.includes("2") ? " — every pyraminx move has order 3, so there is no \"2\"" : ""));
    }
    return m[0];  /* letter + optional prime: the prime picks the inverse perm */
  }

  function applyMove(state, tok) {
    const key = parseMove(tok);
    const perm = MOVE_PERM[key];
    const out = new Array(36);
    for (let i = 0; i < 36; i++) out[i] = state[perm[i]];
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
  for (let f = 0; f < 4; f++) for (let i = 0; i < 9; i++) SOLVED.push(FACES[f]);

  const DEFAULT_CONFIG = { seed: "pyraminx-lbl", scrambleLen: 15 };

  function normalizeConfig(raw) {
    const c = raw || {};
    const num = (v, lo, hi, dflt) => (v === undefined || v === null
      || Number.isNaN(Number(v))) ? dflt : DK.clampf(Number(v), lo, hi, dflt);
    return {
      seed: c.seed === undefined || c.seed === null ? DEFAULT_CONFIG.seed : String(c.seed),
      scrambleLen: Math.round(num(c.scrambleLen, 1, 30, DEFAULT_CONFIG.scrambleLen)),
    };
  }

  const pyraminxStamp = (cfg) => "pyraminx|" + cfg.seed + "|" + cfg.scrambleLen;

  function makeScramble(cfg) {
    const rng = DK.makeRng("pyraminx-scramble:" + cfg.seed);
    const toks = [];
    let last = "";
    while (toks.length < cfg.scrambleLen) {
      const m = MOVES[Math.floor(rng() * MOVES.length)];
      if (m[0].toLowerCase() === last) continue;  /* no same-axis consecutive */
      toks.push(m);
      last = m[0].toLowerCase();
    }
    return toks;
  }

  /* ---------- cubie view: pieces, tip alignment, the two laws ---------- */

  const colorsAt = (state, f) => f.map((i) => state[i]);
  const sortedAt = (state, f) => colorsAt(state, f).sort().join("");

  /* an edge is UNFLIPPED when, read across the slot's canonical (face-letter
     ascending) facelet order `g`, its two colors appear in ascending order —
     equivalently the piece's alphabetically smaller sticker sits on the
     slot's alphabetically first face. At home every edge reads 0. */
  function edgeFlip(state, edge) {
    const c0 = state[edge.g[0]], c1 = state[edge.g[1]];
    return c0 < c1 ? 0 : 1;
  }

  /* permutation of pieces over slots, as slot index -> piece index */
  function edgePerm(state) {
    return EDGE_SLOTS.map((s) => EDGE_IDS.indexOf(sortedAt(state, s.f)));
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

  /* tip v reads aligned when, on each face meeting v, the tip's sticker
     shows the same color as the axial center's sticker. Lowercase moves
     change this for one tip; layer turns never do (they carry tip and
     center together), so the solver can fix tips early and they stay. */
  function tipAligned(state, v) {
    return FACES_AT[v].every((f, k) =>
      state[TIP_SLOTS[v][k]] === state[CTR_SLOTS[v][k]]);
  }

  function cubieView(state) {
    return {
      tips: FACES.map((v) => sortedAt(state, TIP_SLOTS[v])),
      ctrs: FACES.map((v) => sortedAt(state, CTR_SLOTS[v])),
      ep: EDGE_SLOTS.map((s) => sortedAt(state, s.f)),
      eo: EDGE_SLOTS.map((s) => edgeFlip(state, s)),
      aligned: FACES.map((v) => tipAligned(state, v)),
    };
  }

  const isHome = (state, f) => f.every((i, j) => state[i] === SOLVED[f[j]]);

  /* ---------- validator: refuse impossible states, with reasons ---------- */

  /* Laws (each confirmed against the full reachable group by simulation —
     see the test suite):
       - color counts: 9 stickers of each of the 4 colors;
       - piece multiplicity: the 4 tip, 4 center and 6 edge color sets
         each appear exactly once;
       - edge permutation parity is EVEN: every layer turn is a 3-cycle;
       - flipped edges come in even number: every layer turn flips 0 or 2.
     Tips and centers have NO orientation law (all 3^4 x 3^4 twist states
     are reachable — the tips by lowercase turns, the centers because a
     layer turn twists its own center and nothing constrains the sum). */
  function validateState(state) {
    const checks = [];
    const add = (name, ok, reason) => checks.push({ name, ok, reason: ok ? "" : reason });

    add("shape", Array.isArray(state) && state.length === 36,
      "a pyraminx state is an array of 36 facelet colors");
    if (!checks[0].ok) return { ok: false, checks };
    add("letters", state.every((c) => FACES.includes(c)),
      "every facelet must be one of " + FACES.join("/"));
    if (!checks[1].ok) return { ok: false, checks };

    for (const f of FACES) {
      const n = state.filter((c) => c === f).length;
      add("count-9-" + f, n === 9,
        "color " + f + " appears " + n + " times, expected 9");
    }

    const tipSets = FACES.map((v) => sortedAt(state, TIP_SLOTS[v]));
    for (const id of TIP_IDS) {
      const n = tipSets.filter((t) => t === id).length;
      add("pieces-tip-" + id, n === 1,
        "tip color set " + id + " appears " + n + " times, expected exactly once");
    }
    const ctrSets = FACES.map((v) => sortedAt(state, CTR_SLOTS[v]));
    for (const id of CTR_IDS) {
      const n = ctrSets.filter((t) => t === id).length;
      add("pieces-ctr-" + id, n === 1,
        "center color set " + id + " appears " + n + " times, expected exactly once");
    }
    const edgeSets = EDGE_SLOTS.map((s) => sortedAt(state, s.f));
    for (const id of EDGE_IDS) {
      const n = edgeSets.filter((t) => t === id).length;
      add("pieces-edge-" + id, n === 1,
        "edge color set " + id + " appears " + n + " times, expected exactly once");
    }

    const perm = edgePerm(state);
    if (perm.every((p) => p >= 0)) {
      add("edge-permutation", permParity(perm) === 0,
        "edge permutation is odd — every layer turn is a 3-cycle, so only even permutations are reachable");
      const flips = EDGE_SLOTS.map((s) => edgeFlip(state, s)).reduce((a, b) => a + b, 0);
      add("edge-flip", flips % 2 === 0,
        flips + " edges are flipped — a layer turn flips 0 or 2 edges, so the count stays even");
    } else {
      add("edge-permutation", false,
        "an edge slot holds colors that name no edge piece");
    }

    return { ok: checks.every((c) => c.ok), checks };
  }

  /* dual-read solved invariant: every facelet AND every piece in place */
  function verifySolved(state) {
    const byFacelets = state.every((c, i) => c === SOLVED[i]);
    let byCubies = true;
    for (const v of FACES) {
      if (!isHome(state, TIP_SLOTS[v]) || !isHome(state, CTR_SLOTS[v])) byCubies = false;
    }
    for (const e of EDGE_SLOTS) if (!isHome(state, e.f)) byCubies = false;
    return { solved: byFacelets && byCubies, byFacelets, byCubies };
  }

  /* ---------- game ---------- */

  function createGame(cfgRaw) {
    const cfg = normalizeConfig(cfgRaw);
    const scramble = makeScramble(cfg);
    const state = compose(SOLVED, scramble);
    return { cfg, stamp: pyraminxStamp(cfg), scramble, state,
      solvedStart: verifySolved(state).solved };
  }

  globalThis.PyraminxCore = {
    FACES, VERT, FACE_NORMAL, FACE_BASE, FACE_OF, FACE_CORNERS, FACES_AT,
    SLOT_BARY, SLOT_DISP, FACELETS,
    TIP_SLOTS, CTR_SLOTS, TIP_IDS, CTR_IDS, EDGE_SLOTS, EDGE_IDS,
    LAYER_UPPER, LAYER_TIP, MOVES, MOVE_PERM, SOLVED,
    parseMove, applyMove, applyAlg, compose,
    tipAligned, edgeFlip, edgePerm, permParity, cubieView, isHome,
    validateState, verifySolved,
    createGame,
    DEFAULT_CONFIG, normalizeConfig, pyraminxStamp, makeScramble,
  };
})();
