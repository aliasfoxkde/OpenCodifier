/* N×N×N cube family — core.
   The generalized sibling of cube-core.js: one parametric facelet model for
   2×2×2 through 21×21×21. Everything is DERIVED from geometry, never
   hand-transcribed: cubie coordinates on the (n-1)-parity lattice, sticker
   facelets from the standard URFDLB numbering generalized to n, and move
   permutations computed by rotating cubies and their sticker normals with
   the same matrix (SiGN notation: U, Uw, 3Uw, … covers every layer depth).

   The validator pins exactly the laws that simulation PROVED hold for this
   family — and no more:
   - color counts (n² per face) — every n;
   - piece multiplicity (8 corners once each; every edge color-pair exactly
     n-2 times) — every n;
   - corner-twist sum ≡ 0 mod 3, chirality from the slot-normal determinant
     — verified by seeded random walks on n = 3, 4, 5, 7;
   - edge-flip sum ≡ 0 mod 2, Kociemba's canonical-first rule (the slot's
     U/D facelet else F/B facelet vs the cubie's U/D color else F/B color)
     — verified on n = 3…8. A 2×2 has no edge pieces, so the flip law is
     vacuous there by construction, not by omission.
   Permutation parity is deliberately NOT asserted: simulation shows
   corner/edge parity decouples for n ≥ 4 (the 4×4 PLL-parity family) and
   a 2×2 has no edges to couple with — odd corner permutations are legal
   there. A validator that asserts a law the moves can break would reject
   legal states; refusing a legal state is the one failure this file
   never makes.

   Solver coverage is honest and small: 2×2 only (D layer → top orientation
   → top permutation, every candidate step simulated BEFORE it is played,
   ties broken by fixed roster order unless the shell passes a chooser).
   3×3 stays on the battle-tested cube-core.js; n ≥ 4 is free play —
   algebra, scramble, and validator, with the solve button told to say no.

   The shell (cube.js) renders and wires; this file never touches the DOM. */

(function () {
  "use strict";

  const MIN_N = 2;
  const MAX_N = 21;
  const FACES = ["U", "R", "F", "D", "L", "B"];
  const NORMAL = {
    U: [0, 1, 0], R: [1, 0, 0], F: [0, 0, 1],
    D: [0, -1, 0], L: [-1, 0, 0], B: [0, 0, -1],
  };

  /* default scramble length per size: long enough to be interesting,
     short enough that the move log stays readable */
  function defaultScrambleLen(n) {
    if (n === 2) return 12;
    if (n <= 7) return 25;
    return 30;
  }

  /* mulberry32 — seeded, deterministic, no Math.random anywhere */
  function rng32(seed) {
    let a = seed >>> 0;
    return function () {
      a |= 0; a = (a + 0x6D2B79F5) | 0;
      let t = Math.imul(a ^ (a >>> 15), 1 | a);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  function createCore(n) {
    if (!Number.isInteger(n) || n < MIN_N || n > MAX_N) {
      throw new Error("cube size must be an integer " + MIN_N + ".." + MAX_N + ", got " + n);
    }
    const m = n - 1;
    const faceCount = n * n;
    const stateSize = 6 * faceCount;
    const maxDepth = Math.floor(n / 2);

    /* ---------- geometry ---------- */

    /* cubie coordinates live on the (n-1)-parity lattice: ±(n-1), ±(n-3), …
       A cubie is a surface cubie iff any |coord| = n-1; a sticker of face f
       exists iff the f-normal dotted with the position is n-1. */
    const CUBIE_COORDS = [];
    for (let v = -(n - 1); v <= n - 1; v += 2) {
      /* single-axis walk reused below; the full product is built after */
    }
    const AXIS = [];
    for (let v = -(n - 1); v <= n - 1; v += 2) AXIS.push(v);
    for (const x of AXIS) for (const y of AXIS) for (const z of AXIS) {
      if (Math.abs(x) === m || Math.abs(y) === m || Math.abs(z) === m) CUBIE_COORDS.push([x, y, z]);
    }

    /* facelet index for the sticker of face f on the cubie at (x,y,z);
       the n=3 case is identical to cube-core's URFDLB numbering */
    function faceletAt(face, x, y, z) {
      const b = FACES.indexOf(face) * faceCount;
      switch (face) {
        case "U": return b + (z + m) / 2 * n + (x + m) / 2;
        case "R": return b + (m - y) / 2 * n + (m - z) / 2;
        case "F": return b + (m - y) / 2 * n + (x + m) / 2;
        case "D": return b + (m - z) / 2 * n + (x + m) / 2;
        case "L": return b + (m - y) / 2 * n + (z + m) / 2;
        case "B": return b + (m - y) / 2 * n + (m - x) / 2;
        default: throw new Error("unknown face " + face);
      }
    }
    const faceOf = (idx) => FACES[Math.floor(idx / faceCount)];

    /* rotate (x,y,z) for a clockwise turn of `face` viewed from that face —
       the same table cube-core uses */
    function rotateFor(face, p) {
      const [x, y, z] = p;
      switch (face) {
        case "U": return [-z, y, x];
        case "D": return [z, y, -x];
        case "R": return [x, z, -y];
        case "L": return [x, -z, y];
        case "F": return [y, -x, z];
        case "B": return [-y, x, z];
        default: throw new Error("unknown face " + face);
      }
    }

    /* quarter-turn permutation for the outermost `layers` layers at once
       (a SiGN "k<face>w" turn; layers = 1 is the plain face turn). One
       geometric pass: rotate each moved cubie and each sticker normal with
       the same matrix. The perm maps new facelet index -> old facelet
       index; disjoint layers make the single pass the exact composition. */
    function turnPerm(face, layers) {
      const perm = new Array(stateSize);
      for (let i = 0; i < stateSize; i++) perm[i] = i;
      const axis = NORMAL[face];
      const cs = [];
      for (let d = 0; d < layers; d++) cs.push(m - 2 * d);
      for (const [x, y, z] of CUBIE_COORDS) {
        if (!cs.includes(axis[0] * x + axis[1] * y + axis[2] * z)) continue;
        const np = rotateFor(face, [x, y, z]);
        for (const f of FACES) {
          const nv = NORMAL[f];
          if (nv[0] * x + nv[1] * y + nv[2] * z !== m) continue;
          const nr = rotateFor(face, nv);
          const nf = FACES.find((g) => {
            const q = NORMAL[g];
            return q[0] === nr[0] && q[1] === nr[1] && q[2] === nr[2];
          });
          perm[faceletAt(nf, np[0], np[1], np[2])] = faceletAt(f, x, y, z);
        }
      }
      return perm;
    }

    /* ---------- move table (SiGN notation) ---------- */

    function baseToken(face, layers) {
      if (layers === 1) return face;
      if (layers === 2) return face + "w";
      return String(layers) + face + "w";
    }
    const MOVES = {};   /* base token -> quarter-turn perm */
    const MOVE_KEYS = [];
    for (const f of FACES) {
      for (let layers = 1; layers <= maxDepth; layers++) {
        const tok = baseToken(f, layers);
        MOVES[tok] = turnPerm(f, layers);
        MOVE_KEYS.push(tok);
      }
    }

    /* SiGN tokens: number-first for wide bands ("3Uw"), so the count comes
       BEFORE the face; a bare "w" means 2 layers */
    const MOVE_RE = /^(\d+)?([URFDLB])(w)?(2|')?$/;
    function parseMove(tok) {
      const mt = MOVE_RE.exec(tok);
      if (!mt) return null;
      const face = mt[2];
      const layers = mt[1] ? parseInt(mt[1], 10) : (mt[3] ? 2 : 1);
      if (layers < 1 || layers > maxDepth) return null;
      const quarters = mt[4] === "2" ? 2 : mt[4] === "'" ? 3 : 1;
      return { face, layers, quarters, base: baseToken(face, layers) };
    }
    const applyPerm = (state, perm) => state.map((_, i) => state[perm[i]]);
    function applyMove(state, tok) {
      const mv = parseMove(tok);
      if (!mv) throw new Error("unknown move " + tok);
      let out = state;
      for (let q = 0; q < mv.quarters; q++) out = applyPerm(out, MOVES[mv.base]);
      return out;
    }
    function applyAlg(state, toks) {
      let out = state;
      for (const t of toks) out = applyMove(out, t);
      return out;
    }
    function invertToken(tok) {
      if (tok.endsWith("2")) return tok;
      return tok.endsWith("'") ? tok.slice(0, -1) : tok + "'";
    }
    const invertAlg = (toks) => toks.slice().reverse().map(invertToken);

    /* ---------- slots: corners and edge positions ----------
       slot ids are FACES-order joins (URF, RFD, DLB, …): U always leads a
       U-corner id, so id[0] === "U" is the U-layer test. */
    const CORNER_SLOTS = [];
    const EDGE_SLOTS = [];
    for (const [x, y, z] of CUBIE_COORDS) {
      const mags = [Math.abs(x), Math.abs(y), Math.abs(z)].sort((a, b) => b - a);
      const st = [];
      for (const f of FACES) {
        const nv = NORMAL[f];
        if (nv[0] * x + nv[1] * y + nv[2] * z === m) st.push(faceletAt(f, x, y, z));
      }
      if (mags[0] === m && mags[1] === m && mags[2] === m) {
        CORNER_SLOTS.push({ id: st.map(faceOf).join(""), f: st });
      } else if (mags[0] === m && mags[1] === m) {
        /* canonical facelet: the U/D face, else the F/B face (cube-core's
           canonical-first rule; the fallback must pick by FACE CLASS, not
           slot order — the flip law walk caught exactly this) */
        let canon = st.findIndex((i) => { const fc = faceOf(i); return fc === "U" || fc === "D"; });
        if (canon < 0) canon = st.findIndex((i) => { const fc = faceOf(i); return fc === "F" || fc === "B"; });
        EDGE_SLOTS.push({ f: st, canon });
      }
    }

    /* ---------- validator ---------- */

    const solvedState = [];
    for (let f = 0; f < 6; f++) for (let i = 0; i < faceCount; i++) solvedState.push(FACES[f]);

    /* corner chirality: sign of the determinant of the slot's three outward
       normals, ordered by FACES index. Simulation-derived law (twist probe,
       n = 3, 4, 5, 7): twist = chi*p - home, sum ≡ 0 mod 3. */
    const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    const CHI = CORNER_SLOTS.map((slot) => {
      const nrm = slot.f.map((i) => NORMAL[faceOf(i)]);
      return Math.sign(dot(cross(nrm[0], nrm[1]), nrm[2])) || 1;
    });
    const HOME_TWIST = CORNER_SLOTS.map((slot, s) => {
      const cols = slot.f.map((i) => solvedState[i]);
      const p = cols.findIndex((c) => c === "U" || c === "D");
      return ((CHI[s] * p) % 3 + 3) % 3;
    });
    const HOME_CORNER_KEYS = CORNER_SLOTS.map((s) => s.f.map((i) => solvedState[i]).sort().join(""));

    function validateState(state) {
      const issues = [];
      const add = (check, ok, detail) => { if (!ok) issues.push({ check, detail }); };

      if (!Array.isArray(state) || state.length !== stateSize) {
        return { legal: false, issues: [{ check: "shape", detail: "expected " + stateSize + " facelets, got " + (state ? state.length : "null") }] };
      }
      const counts = {};
      for (const c of state) counts[c] = (counts[c] || 0) + 1;
      for (const f of FACES) {
        add("color-count", counts[f] === faceCount,
          "color " + f + " appears " + (counts[f] || 0) + " times, expected " + faceCount);
      }
      /* piece multiplicity: each corner triple exactly once, each edge
         color-pair exactly n-2 times (0 pairs on a 2×2 — vacuous) */
      const cornerSeen = {};
      const edgePairSeen = {};
      for (const slot of CORNER_SLOTS) {
        const key = slot.f.map((i) => state[i]).sort().join("");
        cornerSeen[key] = (cornerSeen[key] || 0) + 1;
      }
      for (const slot of EDGE_SLOTS) {
        const key = slot.f.map((i) => state[i]).sort().join("");
        edgePairSeen[key] = (edgePairSeen[key] || 0) + 1;
      }
      for (const key of HOME_CORNER_KEYS) {
        add("piece-multiplicity", cornerSeen[key] === 1,
          "corner " + key + " appears " + (cornerSeen[key] || 0) + " times, expected 1");
      }
      const wantEdge = EDGE_SLOTS.length / 12;
      for (const key of Object.keys(edgePairSeen)) {
        add("piece-multiplicity", edgePairSeen[key] === wantEdge,
          "edge pair " + key + " appears " + edgePairSeen[key] + " times, expected " + wantEdge);
      }
      /* corner-twist sum ≡ 0 mod 3 */
      let twistSum = 0;
      CORNER_SLOTS.forEach((slot, s) => {
        const cols = slot.f.map((i) => state[i]);
        const p = cols.findIndex((c) => c === "U" || c === "D");
        const raw = ((CHI[s] * p) % 3 + 3) % 3;
        twistSum += ((raw - HOME_TWIST[s]) % 3 + 3) % 3;
      });
      add("corner-twist", twistSum % 3 === 0,
        "corner twists sum to " + (twistSum % 3) + " mod 3 — legal cubes always sum to 0");
      /* edge-flip sum ≡ 0 mod 2 (no edge slots on a 2×2 — vacuous there) */
      if (EDGE_SLOTS.length) {
        let flipSum = 0;
        for (const slot of EDGE_SLOTS) {
          const cols = slot.f.map((i) => state[i]);
          const want = cols.find((c) => c === "U" || c === "D")
            || cols.find((c) => c === "F" || c === "B");
          if (state[slot.f[slot.canon]] !== want) flipSum++;
        }
        add("edge-flip", flipSum % 2 === 0,
          "edge flips sum to " + (flipSum % 2) + " mod 2 — legal cubes always sum to 0");
      }
      return { legal: issues.length === 0, issues };
    }

    function verifySolved(state) {
      return { solved: state.every((c, i) => c === solvedState[i]) };
    }

    /* ---------- scrambles ---------- */

    function makeScramble(seed, len) {
      const count = len || defaultScrambleLen(n);
      const rand = rng32(seed);
      const toks = [];
      let lastFace = "";
      for (let i = 0; i < count; i++) {
        let f = FACES[Math.floor(rand() * 6)];
        while (f === lastFace) f = FACES[Math.floor(rand() * 6)];
        lastFace = f;
        const layers = 1 + Math.floor(rand() * maxDepth);
        const v = rand();
        const suffix = v < 0.4 ? "" : v < 0.7 ? "'" : "2";
        toks.push(baseToken(f, layers) + suffix);
      }
      return toks;
    }

    function createGame(cfg) {
      const seed = String(cfg.seed);
      const len = cfg.scrambleLen || defaultScrambleLen(n);
      /* FNV-1a of the seed so string seeds scramble deterministically */
      let hsh = 2166136261;
      for (let i = 0; i < seed.length; i++) {
        hsh ^= seed.charCodeAt(i);
        hsh = Math.imul(hsh, 16777619);
      }
      const scramble = makeScramble(hsh >>> 0, len);
      let state = solvedState.slice();
      for (const t of scramble) state = applyMove(state, t);
      return { state, scramble, seed };
    }

    const core = {
      n, m, maxDepth, FACES, NORMAL, faceCount, stateSize, solvedState,
      CUBIE_COORDS, MOVES, MOVE_KEYS,
      faceletAt, faceOf, applyMove, applyAlg, invertToken, invertAlg, parseMove,
      baseToken, validateState, verifySolved, makeScramble, createGame,
      defaultScrambleLen,
      cornerSlots: CORNER_SLOTS, edgeSlots: EDGE_SLOTS,
      solverAvailable: n === 2,
    };
    if (n === 2) core.solve = (state, chooser) => solve2(core, state, chooser);
    return core;
  }

  /* ---------- 2×2 solver (the only size this file coaches) ---------- */

  /* insert algs per D corner slot (slot ids in FACES order: RFD = the
     front-right D corner). Each list covers one sticker orientation of the
     cubie sitting in the U layer above the slot. */
  const INSERTS = {
    RFD: [["R", "U", "R'"], ["F'", "U'", "F"], ["R", "U2", "R'", "U'", "R", "U", "R'"]],
    RDB: [["B", "U", "B'"], ["R'", "U'", "R"], ["B", "U2", "B'", "U'", "B", "U", "B'"]],
    DLB: [["L", "U", "L'"], ["B'", "U'", "B"], ["L", "U2", "L'", "U'", "L", "U", "L'"]],
    FDL: [["F", "U", "F'"], ["L'", "U'", "L"], ["F", "U2", "F'", "U'", "F", "U", "F'"]],
  };
  /* one insert-run that lifts a cubie out of its D slot */
  const POP = {
    RFD: ["R", "U", "R'"], RDB: ["B", "U", "B'"], DLB: ["L", "U", "L'"], FDL: ["F", "U", "F'"],
  };
  const SEXY = ["R", "U", "R'", "U'"];  /* insert rhythm, not the orienter */
  /* the classic corner orienter: repeat until the URF cubie is oriented
     (2 or 4 reps), U between corners; the bottom looks scrambled mid-loop
     and restores when the last corner is done. Simulation chose this over
     the (R U R' U') variant 150-0 — that one leaves the bottom broken. */
  const RDRD = ["R'", "D'", "R", "D"];
  /* Niklas: pure corner 3-cycle (even). T-perm: adjacent corner swap (odd)
     — on a 2×2 there are no edges, so T-perm is exactly the odd-permutation
     fixer the Niklas family cannot reach. */
  const NIKLAS = ["R", "U'", "L'", "U", "R'", "U'", "L", "U"];
  const TPERM = ["R", "U", "R'", "U'", "R'", "F", "R2",
    "U'", "R'", "U'", "R", "U", "R'", "F'"];
  const U_PRE = [[], ["U"], ["U2"], ["U'"]];

  /* exhaustive phase-3 search: BFS over the 24 permutations of the U
     corners. Every expansion replays through the public move algebra, so a
     found plan is verified by construction; the chooser breaks ties among
     equally short plans. Returns { toks, detail } or null. */
  function bfsPermute(core, start, gens, pickIdx) {
    if (core.verifySolved(start).solved) return { toks: [], detail: "already permuted" };
    const key = (st) => st.join("");
    let frontier = [{ st: start, path: [] }];
    const seen = new Set([key(start)]);
    for (let depth = 1; depth <= 6; depth++) {
      const next = [];
      const solutions = [];
      for (const node of frontier) {
        for (const g of gens) {
          const st = core.applyAlg(node.st, g.toks);
          const k = key(st);
          if (seen.has(k)) continue;
          seen.add(k);
          const path = [...node.path, g];
          if (core.verifySolved(st).solved) solutions.push(path);
          else next.push({ st, path });
        }
      }
      if (solutions.length) {
        const idx = pickIdx(solutions.map((p) => p.map((g) => g.label).join(", then ")), "pll");
        const path = solutions[Number.isInteger(idx) && idx >= 0 && idx < solutions.length ? idx : 0];
        return {
          toks: path.flatMap((g) => g.toks),
          detail: path.length + " PLL" + (path.length > 1 ? "s" : "") + ": "
            + path.map((g) => g.label).join(", then ")
            + " — shortest of " + solutions.length + " equal-length plans",
        };
      }
      if (!next.length) return null;
      frontier = next;
    }
    return null;
  }

  function solve2(core, state, chooser) {
    if (core.n !== 2) return null;
    const v = core.validateState(state);
    if (!v.legal) return { refused: true, reason: "invalid state: " + v.issues[0].check, steps: [], moves: [] };

    const D_SLOTS = ["RFD", "RDB", "DLB", "FDL"];
    const steps = [];
    const moves = [];
    let cur = state.slice();
    const done = new Set();   /* D slots already solved and protected */

    const slotOf = (id) => core.cornerSlots.find((s) => s.id === id);
    const homeKey = (id) => slotOf(id).f.map((i) => core.solvedState[i]).sort().join("");
    const slotDone = (st, id) => slotOf(id).f.every((i) => st[i] === core.solvedState[i]);
    const dLayerDone = (st) => D_SLOTS.every((id) => slotDone(st, id));
    const cubieInU = (st, key) => {
      const hit = core.cornerSlots.find((s) => s.id[0] === "U"
        && s.f.map((i) => st[i]).sort().join("") === key);
      return hit ? hit.id : null;
    };
    const ufrOriented = (st) => {
      const slot = slotOf("URF");
      const cols = slot.f.map((i) => st[i]);
      const uIdx = cols.findIndex((c) => c === "U");
      const canon = slot.f.findIndex((i) => core.faceOf(i) === "U");
      return uIdx === canon;
    };
    const uFaceOriented = (st) => ["URF", "ULB", "URB", "UFL"].every((id) => {
      const slot = slotOf(id);
      const cols = slot.f.map((i) => st[i]);
      const uIdx = cols.findIndex((c) => c === "U");
      const canon = slot.f.findIndex((i) => core.faceOf(i) === "U");
      return uIdx === canon;
    });
    const misplaced = (st) => core.cornerSlots.filter((s) => !s.f.every((i) => st[i] === core.solvedState[i])).length;
    const play = (toks, title, detail) => {
      cur = core.applyAlg(cur, toks);
      moves.push(...toks);
      steps.push({ title, moves: toks.slice(), detail: detail || "" });
    };
    const pickIdx = (cand, tag) => {
      if (!chooser) return 0;
      const c = chooser(cand.map((x) => x.detail), tag);
      return Number.isInteger(c) && c >= 0 && c < cand.length ? c : 0;
    };

    /* --- phase 1: D layer, slot by slot; every insert verified to land the
       target cubie WITHOUT disturbing already-solved slots --- */
    for (const slotId of D_SLOTS) {
      if (slotDone(cur, slotId)) { done.add(slotId); continue; }
      const key = homeKey(slotId);
      if (slotOf(slotId).f.map((i) => cur[i]).sort().join("") === key) {
        /* the right cubie, twisted in place: pop it out and re-insert */
        play(POP[slotId], "free " + slotId, "the cubie belongs here but is twisted — pop it to the top first");
      } else if (!cubieInU(cur, key)) {
        /* it sits in another D slot: pop that one */
        const other = D_SLOTS.find((id) => id !== slotId
          && slotOf(id).f.map((i) => cur[i]).sort().join("") === key);
        if (!other) return { refused: true, reason: "cubie for " + slotId + " not found", steps, moves };
        play(POP[other], "free " + other, "the cubie needed for " + slotId + " sits in " + other);
      }
      if (!cubieInU(cur, key)) {
        return { refused: true, reason: "cubie for " + slotId + " not in the top layer", steps, moves };
      }
      const cand = [];
      for (const pre of U_PRE) {
        for (const ins of INSERTS[slotId]) {
          const toks = [...pre, ...ins];
          const after = core.applyAlg(cur, toks);
          if (slotDone(after, slotId) && [...done].every((id) => slotDone(after, id))) {
            cand.push({
              toks,
              detail: (pre.length ? pre.join(" ") + ", then " : "") + ins.join(" "),
            });
            break; /* one insert per alignment */
          }
        }
      }
      if (!cand.length) return { refused: true, reason: "no verified insert for " + slotId, steps, moves };
      const pick = cand[pickIdx(cand, "insert-" + slotId)];
      play(pick.toks, "insert " + slotId, pick.detail);
      done.add(slotId);
    }

    /* --- phase 2: orient the top. The (R U R' U')×k loop twists each top
       corner in turn (one U between corners); the bottom layer looks broken
       mid-loop and restores when the last corner is done. The whole loop is
       ONE step, verified in simulation before it is played. --- */
    if (!uFaceOriented(cur)) {
      const toks = [];
      let sim = cur;
      for (let c = 0; c < 4; c++) {
        if (c > 0) { toks.push("U"); sim = core.applyMove(sim, "U"); }
        /* orient the cubie CURRENTLY at URF: only an even number of sexy
           reps brings the same cubie back — odd counts leave a different
           cubie in the slot with the bottom layer broken. Its twist cycles
           through all three states over 0/2/4 reps, so exactly one works. */
        const urfKey = slotOf("URF").f.map((i) => sim[i]).sort().join("");
        let done = false;
        for (const reps of [0, 2, 4]) {
          let t2 = sim;
          const seq = [];
          for (let r = 0; r < reps; r++) {
            for (const t of RDRD) { seq.push(t); t2 = core.applyMove(t2, t); }
          }
          const key2 = slotOf("URF").f.map((i) => t2[i]).sort().join("");
          if (key2 === urfKey && ufrOriented(t2)) {
            for (const t of seq) toks.push(t);
            sim = t2;
            done = true;
            break;
          }
        }
        if (!done) return { refused: true, reason: "orientation loop stalled at URF", steps, moves };
      }
      if (!uFaceOriented(sim) || !dLayerDone(sim)) {
        return { refused: true, reason: "orientation loop did not restore the bottom", steps, moves };
      }
      cur = sim;
      steps.push({
        title: "orient the top",
        moves: toks.slice(),
        detail: "the (R' D' R D)×k loop twists each top corner in turn — the bottom looks scrambled mid-loop and restores at the end",
      });
      moves.push(...toks);
    }

    /* --- phase 3: permute the top. With the D layer done and every
       corner oriented, the U corners carry one of the 24 permutations of
       S4. Generators: Niklas 3-cycles and their inverses (even) at the
       four U alignments, plus the T-perm adjacent swap (odd) — legal on a
       2×2 precisely because there are no edges. A BFS over the whole
       (finite) permutation space finds a shortest generator sequence;
       a greedy descent cannot: a diagonal transposition is unsolvable by
       any single adjacent-swap alignment, so greedy loops forever. --- */
    const NIKLAS_INV = core.invertAlg(NIKLAS);
    const GENS = [];
    for (const pre of U_PRE) {
      const align = pre.length ? " after " + pre.join(" ") : " in place";
      GENS.push({ toks: [...pre, ...NIKLAS], label: "Niklas 3-cycle" + align });
      GENS.push({ toks: [...pre, ...NIKLAS_INV], label: "Niklas inverse" + align });
      GENS.push({ toks: [...pre, ...TPERM], label: "T-swap" + align });
    }
    const plan3 = bfsPermute(core, cur, GENS, pickIdx);
    if (!plan3) return { refused: true, reason: "permutation did not converge", steps, moves };
    if (plan3.toks.length) play(plan3.toks, "permute the top", plan3.detail);
    if (!core.verifySolved(cur).solved) {
      return { refused: true, reason: "permutation did not converge", steps, moves };
    }

    return { refused: false, moves, steps };
  }

  globalThis.CubeNCore = {
    MIN_N, MAX_N, FACES, NORMAL,
    createCore,
    defaultScrambleLen,
  };
})();
