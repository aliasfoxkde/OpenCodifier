/* Rubik's Cube Solver — core.
   A 3x3x3 cube as 54 facelets with every move permutation DERIVED from
   geometry (cubie coordinates + sticker normals rotated by the same
   matrix), never hand-transcribed tables. Facelet layout follows the
   standard URFDLB numbering. On top of the moves: a validator that
   refuses impossible states WITH the named reason (color counts,
   piece multiplicity, corner-twist sum, edge-flip sum, permutation
   parity), a deterministic layer-by-layer solver whose every step is
   named and human-followable, equal-cost tie enumeration so the
   ENGINE (never the solver itself) breaks genuine ties, seeded
   scrambles, and a redundant solved-invariant verifier.

   The engine never invents moves: it only ranks among variants the
   deterministic layer already proved legal and equal-cost; a refusal
   falls to the code's roster-order default, on the rung.

   The shell (cube.js) renders the CSS 3D cube; this file never
   touches the DOM. */

(function () {
  "use strict";
  const DK = globalThis.DecisionsSDK;

  /* ---------- geometry: faces, coordinates, facelet layout ---------- */

  const FACES = ["U", "R", "F", "D", "L", "B"];
  /* axis vector of each face's outward normal */
  const NORMAL = {
    U: [0, 1, 0], R: [1, 0, 0], F: [0, 0, 1],
    D: [0, -1, 0], L: [-1, 0, 0], B: [0, 0, -1],
  };
  const FACE_BASE = { U: 0, R: 9, F: 18, D: 27, L: 36, B: 45 };

  /* facelet index for the sticker of face f on the cubie at (x,y,z),
     standard URFDLB numbering (U rows back->front, D rows front->back,
     R cols front->back, L cols back->front, B cols right->left) */
  function faceletAt(face, x, y, z) {
    const b = FACE_BASE[face];
    switch (face) {
      case "U": return b + (z + 1) * 3 + (x + 1);
      case "R": return b + (1 - y) * 3 + (1 - z);
      case "F": return b + (1 - y) * 3 + (x + 1);
      case "D": return b + (1 - z) * 3 + (x + 1);
      case "L": return b + (1 - y) * 3 + (z + 1);
      case "B": return b + (1 - y) * 3 + (1 - x);
      default: throw new Error("unknown face " + face);
    }
  }

  /* rotate (x,y,z) for a clockwise turn of `face`, viewed from that face.
     Clockwise viewed from the face's own side = negative right-hand
     rotation about that face's normal (U, D, F, B already followed the
     rule; R and L originally carried the +90 form — inverted). */
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

  /* the 8 corner slots and 12 edge slots with their facelet triples/pairs */
  const CORNER_SLOTS = [];
  const EDGE_SLOTS = [];
  (function () {
    const coords = [];
    for (const x of [-1, 0, 1]) for (const y of [-1, 0, 1]) for (const z of [-1, 0, 1]) {
      if (x === 0 && y === 0 && z === 0) continue;
      coords.push([x, y, z]);
    }
    for (const [x, y, z] of coords) {
      /* faces in sorted order so slot.id reads the same left-to-right as
         slot.f is ordered: slot "UL" has f = [U facelet, L facelet] */
      const faces = FACES.filter((f) => {
        const [nx, ny, nz] = NORMAL[f];
        return nx * x + ny * y + nz * z === 1;
      }).sort();
      const mk = () => ({
        id: faces.join(""), x, y, z,
        f: faces.map((f) => faceletAt(f, x, y, z)),
      });
      if (faces.length === 3) CORNER_SLOTS.push(mk());
      else if (faces.length === 2) EDGE_SLOTS.push(mk());
    }
    CORNER_SLOTS.sort((a, b) => (a.id < b.id ? -1 : 1));
    EDGE_SLOTS.sort((a, b) => (a.id < b.id ? -1 : 1));
  })();

  /* ---------- move permutations, derived ---------- */

  function permForFace(face) {
    const perm = new Array(54).fill(0).map((_, i) => i);
    const moved = [];
    const layer = (p) => {
      const [nx, ny, nz] = NORMAL[face];
      return nx * p[0] + ny * p[1] + nz * p[2] === 1;
    };
    const cubies = CORNER_SLOTS.map((c) => [c.x, c.y, c.z])
      .concat(EDGE_SLOTS.map((e) => [e.x, e.y, e.z]));
    for (const [x, y, z] of cubies) {
      if (!layer([x, y, z])) continue;
      const np = rotateFor(face, [x, y, z]);
      for (const f of FACES) {
        const [nx, ny, nz] = NORMAL[f];
        if (nx * x + ny * y + nz * z !== 1) continue;
        const from = faceletAt(f, x, y, z);
        const nrm = rotateFor(face, [nx, ny, nz]);
        const nf = FACES.find((g) => {
          const [mx, my, mz] = NORMAL[g];
          return mx === nrm[0] && my === nrm[1] && mz === nrm[2];
        });
        const to = faceletAt(nf, np[0], np[1], np[2]);
        perm[to] = from;   /* the sticker at `from` lands on `to` */
        if (!moved.includes(to)) moved.push(to);
      }
    }
    return { perm, moved };
  }

  const MOVE_PERM = {};
  for (const f of FACES) MOVE_PERM[f] = permForFace(f);

  function applyPerm(state, perm) {
    const out = new Array(54);
    for (let i = 0; i < 54; i++) out[i] = state[perm.perm[i]];
    return out;
  }

  /* move tokens: "U", "U'", "U2" — 18 in all */
  const MOVES = [];
  for (const f of FACES) { MOVES.push(f, f + "'", f + "2"); }

  function parseMove(tok) {
    const m = /^([URFDLB])('2|2|')?$/.exec(tok);
    if (!m) throw new Error("bad move token: " + tok);
    const face = m[1];
    const q = tok.includes("2") ? 2 : (tok.includes("'") ? 3 : 1);
    return { face, q };
  }

  function applyMove(state, tok) {
    const { face, q } = parseMove(tok);
    let s = state;
    for (let i = 0; i < q; i++) s = applyPerm(s, MOVE_PERM[face]);
    return s;
  }

  function applyAlg(state, alg) {
    let s = state;
    for (const tok of alg.trim().split(/\s+/).filter(Boolean)) {
      s = applyMove(s, tok);
    }
    return s;
  }

  /* ---------- solved state, config, scrambles ---------- */

  const SOLVED = [];
  for (let f = 0; f < 6; f++) for (let i = 0; i < 9; i++) SOLVED.push(FACES[f]);
  const FACE_OF = (i) => FACES[Math.floor(i / 9)];

  const DEFAULT_CONFIG = {
    seed: "cube-lbl", scrambleLen: 20, method: "lbl",
    coachDetail: "full", engineTies: true,
  };

  function normalizeConfig(raw) {
    const c = raw || {};
    const num = (v, lo, hi, dflt) => (v === undefined || v === null
      || Number.isNaN(Number(v))) ? dflt : DK.clampf(Number(v), lo, hi, dflt);
    return {
      seed: c.seed === undefined || c.seed === null ? DEFAULT_CONFIG.seed : String(c.seed),
      scrambleLen: Math.round(num(c.scrambleLen, 1, 40, DEFAULT_CONFIG.scrambleLen)),
      method: c.method === "lbl" ? "lbl" : DEFAULT_CONFIG.method,
      coachDetail: ["none", "names", "full"].includes(c.coachDetail)
        ? c.coachDetail : DEFAULT_CONFIG.coachDetail,
      engineTies: c.engineTies === undefined ? DEFAULT_CONFIG.engineTies : !!c.engineTies,
      /* shape mods: same 3x3 facelet algebra, same solver — geometry only */
      cubeType: ["classic", "mirror", "ghost"].includes(c.cubeType) ? c.cubeType : "classic",
    };
  }

  const cubeStamp = (cfg) => "cube|" + cfg.seed + "|" + cfg.scrambleLen
    + "|" + cfg.method + "|" + cfg.coachDetail + "|" + (cfg.engineTies ? 1 : 0);

  function makeScramble(cfg) {
    const rng = DK.makeRng("cube-scramble:" + cfg.seed);
    const toks = [];
    let last = "";
    while (toks.length < cfg.scrambleLen) {
      const m = MOVES[Math.floor(rng() * MOVES.length)];
      if (m[0] === last) continue;           /* no same-face consecutive */
      toks.push(m);
      last = m[0];
    }
    return toks;
  }

  /* ---------- cubie view: piece-level reading of a facelet state ---------- */

  /* A cubie is identified by its color SET, whose sorted form equals the
     id of the slot it occupies when solved (cubie FRU belongs in slot
     FRU). That makes slot ids the cubie namespace. */

  /* chirality of each corner slot's sorted face order: (n1 x n2) . n3.
     Corner twists need a consistent handedness or the twist SUM is not
     conserved by moves (4 slots are right-handed, 4 left-handed). */
  const CHIRALITY = {};
  for (const slot of CORNER_SLOTS) {
    const [a, b, c] = slot.f.map((fi) => NORMAL[FACE_OF(fi)]);
    const cross = [
      a[1] * b[2] - a[2] * b[1],
      a[2] * b[0] - a[0] * b[2],
      a[0] * b[1] - a[1] * b[0],
    ];
    CHIRALITY[slot.id] = cross[0] * c[0] + cross[1] * c[1] + cross[2] * c[2];
  }

  /* corner twist: chirality-weighted index of the U/D-colored sticker
     within the slot's sorted facelets, relative to the slot's HOME
     orientation (a home corner twists 0). Sum over the 8 corners is
     0 mod 3 for every legal state. */
  const HOME_TWIST = {};
  for (const slot of CORNER_SLOTS) {
    const homeIdx = slot.f.findIndex((fi) => {
      const f = FACE_OF(fi);
      return f === "U" || f === "D";
    });
    HOME_TWIST[slot.id] = ((CHIRALITY[slot.id] * homeIdx) % 3 + 3) % 3;
  }
  function cornerTwist(state, slot) {
    const idx = state[slot.f[0]] === "U" || state[slot.f[0]] === "D" ? 0
      : state[slot.f[1]] === "U" || state[slot.f[1]] === "D" ? 1 : 2;
    const raw = ((CHIRALITY[slot.id] * idx) % 3 + 3) % 3;
    return ((raw - HOME_TWIST[slot.id]) % 3 + 3) % 3;
  }

  /* edge flip, Kociemba's position rule: each slot has a canonical-first
     facelet (its U/D face when it has one, else its F/B face) and each
     cubie a canonical-first color (its U/D color, else its F/B color).
     The edge is unflipped iff that color sits on that facelet. The
     membership test ("F/B color on an F/B face") is NOT equivalent — an
     F turn moves band cubies between F/B faces and would under-count. */
  function canonicalFirstIdx(slot) {
    for (let i = 0; i < slot.f.length; i++) {
      const f = FACE_OF(slot.f[i]);
      if (f === "U" || f === "D") return i;
    }
    return 0; /* sorted faces put B/F before L/R, so index 0 is the F/B face */
  }
  function canonicalFirstColor(colors) {
    const ud = colors.find((c) => c === "U" || c === "D");
    if (ud) return ud;
    return colors.find((c) => c === "F" || c === "B");
  }
  function edgeFlip(state, slot) {
    const want = canonicalFirstColor(slot.f.map((i) => state[i]));
    return state[slot.f[canonicalFirstIdx(slot)]] === want ? 0 : 1;
  }

  /* permutation parity via transposition count */
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

  function cubieView(state) {
    const cp = CORNER_SLOTS.map((s) => s.f.map((i) => state[i]).sort().join(""));
    const ep = EDGE_SLOTS.map((s) => s.f.map((i) => state[i]).sort().join(""));
    return {
      cp,
      co: CORNER_SLOTS.map((s) => cornerTwist(state, s)),
      ep,
      eo: EDGE_SLOTS.map((s) => edgeFlip(state, s)),
    };
  }

  const CORNER_IDS = CORNER_SLOTS.map((s) => s.id);
  const EDGE_IDS = EDGE_SLOTS.map((s) => s.id);

  /* ---------- validator: refuse impossible states, with reasons ---------- */

  /* Every check names itself; failures carry a human-followable reason.
     Order: cheap structural checks first, then the three orientation /
     parity laws that no sequence of legal moves can violate. */
  function validateState(state) {
    const checks = [];
    const add = (name, ok, reason) => checks.push({ name, ok, reason: ok ? "" : reason });

    if (!Array.isArray(state) || state.length !== 54
      || !state.every((c) => FACES.includes(c))) {
      add("shape", false, "state must be 54 stickers, each U R F D L or B");
      return { ok: false, checks };
    }
    add("shape", true, "");

    for (const f of FACES) {
      const n = state.filter((c) => c === f).length;
      add("count-" + f, n === 9,
        "color " + f + " has " + n + " stickers — every face needs exactly 9");
    }

    /* piece identity: each slot must hold a REAL cubie, and each real
       cubie must appear exactly once */
    const view = cubieView(state);
    const cornerSeen = {};
    for (let i = 0; i < 8; i++) {
      const set = view.cp[i];
      if (!CORNER_IDS.includes(set)) {
        add("corner-piece-" + CORNER_SLOTS[i].id, false,
          "corner slot " + CORNER_SLOTS[i].id + " shows colors " + set
          + " — no real corner cubie has that combination (one of U/D, one of R/L, one of F/B)");
      }
      cornerSeen[set] = (cornerSeen[set] || 0) + 1;
    }
    for (const id of CORNER_IDS) {
      if (cornerSeen[id] === undefined) {
        add("corner-piece-" + id, false, "no " + id + " corner cubie anywhere on the cube");
      } else if (cornerSeen[id] > 1) {
        add("corner-piece-" + id, false,
          cornerSeen[id] + " " + id + " corner cubies found — each real cubie exists once");
      }
    }

    const edgeSeen = {};
    for (let i = 0; i < 12; i++) {
      const set = view.ep[i];
      if (!EDGE_IDS.includes(set)) {
        add("edge-piece-" + EDGE_SLOTS[i].id, false,
          "edge slot " + EDGE_SLOTS[i].id + " shows colors " + set
          + " — no real edge cubie has that combination (no U/D pairs with F/B, no R/L pairs)");
      }
      edgeSeen[set] = (edgeSeen[set] || 0) + 1;
    }
    for (const id of EDGE_IDS) {
      if (edgeSeen[id] === undefined) {
        add("edge-piece-" + id, false, "no " + id + " edge cubie anywhere on the cube");
      } else if (edgeSeen[id] > 1) {
        add("edge-piece-" + id, false,
          edgeSeen[id] + " " + id + " edge cubies found — each real cubie exists once");
      }
    }

    /* the three laws */
    const twistSum = view.co.reduce((a, b) => a + b, 0) % 3;
    add("corner-twist", twistSum === 0,
      "corner twists sum to " + ((twistSum % 3) + 3) % 3 + " mod 3 — legal cubes always sum to 0"
      + " (a lone twisted corner cannot be reached by turning faces)");

    const flipSum = view.eo.reduce((a, b) => a + b, 0) % 2;
    add("edge-flip", flipSum === 0,
      "edge flips sum to " + (flipSum % 2) + " mod 2 — legal cubes always sum to 0"
      + " (a lone flipped edge cannot be reached by turning faces)");

    const cpar = permParity(view.cp.map((id) => CORNER_IDS.indexOf(id)));
    const epar = permParity(view.ep.map((id) => EDGE_IDS.indexOf(id)));
    add("permutation-parity", cpar === epar,
      "corner permutation is " + (cpar ? "odd" : "even") + " but edge permutation is "
      + (epar ? "odd" : "even") + " — a single swap (corners without edges or the reverse)"
      + " cannot be reached by turning faces");

    return { ok: checks.every((c) => c.ok), checks };
  }

  /* ---------- redundant solved verifier ---------- */

  /* Two independent reads of "solved": the facelet string, and the cubie
     view (every cubie home, no twist, no flip). Both must agree. */
  function verifySolved(state) {
    const byFacelets = state.every((c, i) => c === SOLVED[i]);
    const view = cubieView(state);
    const byCubies = view.cp.every((id, i) => id === CORNER_IDS[i] && view.co[i] === 0)
      && view.ep.every((id, i) => id === EDGE_IDS[i] && view.eo[i] === 0);
    return { solved: byFacelets && byCubies, byFacelets, byCubies };
  }

  /* ---------- derived case algorithms ---------- */

  /* Every insert/eject/LL algorithm below is a CONSTANT that the tests
     pin by simulation: each must solve its named case state and fix the
     protected facelet positions pointwise, so it behaves identically in
     ANY state where its precondition holds. ySub rotates an alg (and its
     precondition) around the U axis: F->R->B->L, U and D fixed. */

  const YMAP = { F: "R", R: "B", B: "L", L: "F", U: "U", D: "D" };
  function ySub(toks, times) {
    const t = ((times % 4) + 4) % 4;
    return toks.map((tok) => {
      let f = tok[0];
      for (let i = 0; i < t; i++) f = YMAP[f];
      return f + tok.slice(1);
    });
  }
  /* the band slot and D-corner slot a y^k rotation maps the canonical
     ones to (ids re-sorted after face substitution) */
  function ySlot(id, times) {
    const t = ((times % 4) + 4) % 4;
    return id.split("").map((f) => {
      let g = f;
      for (let i = 0; i < t; i++) g = YMAP[g];
      return g;
    }).sort().join("");
  }

  const CORNER_RIGHT = ["R", "U", "R'"];        /* D sticker facing R */
  const CORNER_LEFT = ["F'", "U'", "F"];        /* D sticker facing F */
  const CORNER_UP = ["R", "U2", "R'", "U'", "R", "U", "R'"];
  const FLIP_INSERT = ["R", "U'", "R'", "F"]; /* flipped cross edge (fixes
     every other D-edge position pointwise; corner-neutrality is NOT
     required — the cross phase runs before corners are placed) */
  const MID_RIGHT = ["U", "R", "U'", "R'", "U'", "F'", "U", "F"];
  const MID_LEFT = ["U'", "L'", "U", "L", "U", "F", "U'", "F'"];
  const LL = {
    eo: ["F", "R", "U", "R'", "U'", "F'"],
    /* inverse of eo: takes the L-shape (2 adjacent) straight to the cross */
    eoL: ["F", "U", "R", "U'", "R'", "F'"],
    sune: ["R", "U", "R'", "U", "R", "U2", "R'"],
    antiSune: ["R", "U2", "R'", "U'", "R", "U'", "R'"],
    aPerm: ["R'", "F", "R'", "B2", "R", "F'", "R'", "B2", "R2"],
    /* pure edge 3-cycles, machine-verified (corners + F2L fixed) */
    uPerm: ["R", "U'", "R", "U", "R", "U", "R", "U'", "R'", "U'", "R2"],
    /* T-perm: swaps two adjacent corners + two edges (odd pair — the only
       way to answer a bare transposition); Y-perm: the diagonal version */
    tPerm: ["R", "U", "R'", "U'", "R'", "F", "R2", "U'", "R'", "U'", "R", "U", "R'", "F'"],
    yPerm: ["F", "R", "U'", "R'", "U'", "R", "U", "R'", "F'", "R", "U", "R'", "U'", "R'", "F", "R", "F'"],
  };
  const U_PERM_INV = LL.uPerm.slice().reverse().map((t) =>
    t.endsWith("2") ? t : t.endsWith("'") ? t[0] : t[0] + "'");
  /* opposite-pair edge double swap (H); adjacent double swaps are two
     conjugated U-perms composed — the greedy reaches them in two rounds */
  const H_PERM = ["R2", "U2", "R", "U2", "R2", "U2", "R2", "U2", "R", "U2", "R2"];

  /* corner-orientation distance table. Readout twist (cornerTwist) is
     intrinsic twist PLUS a per-(piece,slot) offset kappa, so the table is
     keyed on the intrinsic coordinate: for each piece, which U slot it
     sits in + its intrinsic twist — the full 648-config space. kappa and
     the generator deltas are read off simulation at load; BFS from solved
     over {sune, anti-sune} x 4 U alignments gives exact distances, and
     the solver walks strictly downhill on them (raw count-greedy provably
     stalls on 312 of 648 configs). */
  const CO_TOP = CORNER_SLOTS.filter((c) => c.y === 1);
  const coSlotOfPiece = (state, home) => CO_TOP.find((s) =>
    s.f.map((i) => state[i]).sort().join("") === home.id);
  /* kappa[piece][slot]: readout of an untwisted piece away from home */
  const CO_KAPPA = (() => {
    const conj = [];
    for (let a = 0; a < 4; a++) for (let b = 0; b < 4; b++) {
      const pre = Array(a).fill("U"), post = Array(b).fill("U");
      for (const base of [LL.aPerm,
        LL.aPerm.slice().reverse().map((t) => t.endsWith("2") ? t : t.endsWith("'") ? t[0] : t[0] + "'"),
        LL.tPerm, LL.yPerm]) {
        conj.push(pre.concat(base, post).join(" "));
      }
    }
    const kap = CO_TOP.map(() => ({}));
    for (const seq of conj) {
      const st = applyAlg(SOLVED.slice(), seq);
      for (const piece of CO_TOP) {
        const slot = coSlotOfPiece(st, piece);
        if (!slot) continue;
        const val = cornerTwist(st, slot);
        if (kap[CO_TOP.indexOf(piece)][CO_TOP.indexOf(slot)] === undefined) {
          kap[CO_TOP.indexOf(piece)][CO_TOP.indexOf(slot)] = val;
        } else if (kap[CO_TOP.indexOf(piece)][CO_TOP.indexOf(slot)] !== val) {
          throw new Error("kappa inconsistency for " + piece.id + " at " + slot.id);
        }
      }
    }
    for (let i = 0; i < 4; i++) {
      for (let j = 0; j < 4; j++) if (kap[i][j] === undefined) {
        throw new Error("kappa missing for piece " + CO_TOP[i].id + " at " + CO_TOP[j].id);
      }
    }
    return kap;
  })();
  /* intrinsic twist vector + slot assignment of the four U pieces */
  const coConfigOf = (state) => {
    let assign = "", w = "";
    for (const piece of CO_TOP) {
      const slot = coSlotOfPiece(state, piece);
      if (!slot) return null;
      assign += CO_TOP.indexOf(slot);
      w += ((cornerTwist(state, slot) - CO_KAPPA[CO_TOP.indexOf(piece)][CO_TOP.indexOf(slot)]) % 3 + 3) % 3;
    }
    return assign + "|" + w;
  };
  /* generator maps in intrinsic coordinates: piece at slot s moves to
     dst with intrinsic twist + delta */
  const coMapOf = (algStr) => {
    const after = applyAlg(SOLVED.slice(), algStr);
    return CO_TOP.map((from) => {
      const dst = CO_TOP.find((x) =>
        x.f.map((i) => after[i]).sort().join("") === from.id);
      const delta = ((cornerTwist(after, dst) - CO_KAPPA[CO_TOP.indexOf(from)][CO_TOP.indexOf(dst)]) % 3 + 3) % 3;
      return { pos: CO_TOP.indexOf(dst), delta };
    });
  };
  const CO_U = coMapOf("U").map((m) => m.pos);
  const CO_MAPS = [coMapOf(LL.sune.join(" ")), coMapOf(LL.antiSune.join(" "))];
  const CO_DIST = (() => {
    /* orientation-solved = every readout zero: for each slot assignment
       there is exactly one twist vector that reads zero, so there are 24
       targets, not one. The generator graph splits into components, so
       distances are BFS'd from ALL targets simultaneously. */
    const targets = [];
    const permute = (arr, pre) => {
      if (!pre) pre = [];
      if (!arr.length) {
        let w = "";
        for (let p = 0; p < 4; p++) w += ((3 - CO_KAPPA[p][pre[p]]) % 3 + 3) % 3;
        targets.push(pre.join("") + "|" + w);
        return;
      }
      for (let i = 0; i < arr.length; i++) {
        permute([...arr.slice(0, i), ...arr.slice(i + 1)], pre.concat(arr[i]));
      }
    };
    permute([0, 1, 2, 3], null);
    const dist = new Map(targets.map((t) => [t, 0]));
    let layer = targets.slice();
    while (layer.length) {
      const next = [];
      for (const key of layer) {
        const [as, ws] = key.split("|");
        const assign = as.split("").map(Number);
        const w = ws.split("").map(Number);
        const d0 = dist.get(key);
        for (let j = 0; j < 4; j++) {
          /* both arrays are piece-indexed: assign[p] = slot of piece p.
             U turns send each piece's slot through CO_U and never twist. */
          let a2 = assign.slice();
          for (let k = 0; k < j; k++) a2 = a2.map((s) => CO_U[s]);
          for (let b = 0; b < 4; b++) {
            const a4 = (() => { let r = a2.slice();
              for (let k = 0; k < b; k++) r = r.map((s) => CO_U[s]); return r; })();
            for (const M of CO_MAPS) {
              const a3 = [0, 0, 0, 0], w3 = [0, 0, 0, 0];
              for (let p = 0; p < 4; p++) {
                a3[p] = M[a4[p]].pos;
                w3[p] = (w[p] + M[a4[p]].delta) % 3;
              }
              const k2 = a3.join("") + "|" + w3.join("");
              if (!dist.has(k2)) { dist.set(k2, d0 + 1); next.push(k2); }
            }
          }
        }
      }
      layer = next;
    }
    return dist;
  })();
  if (typeof process !== "undefined" && process.env.CUBE_DEBUG) {
    let dia = 0;
    for (const v of CO_DIST.values()) if (v > dia) dia = v;
    console.error("CO table: " + CO_DIST.size + " configs, diameter " + dia);
  }
  const coKeyOf = (state) => coConfigOf(state);
  /* greedy consumes one distance per round, so the round cap is the
     table diameter plus headroom — a fixed 8 exits mid-descent */
  const CO_CAP = Math.max(...CO_DIST.values()) + 2;

  /* ---------- solver ---------- */

  const CROSS_IDS = ["DF", "DR", "BD", "DL"];
  const CORNER_ORDER = ["DFR", "DFL", "BDL", "BDR"]; /* sorted slot ids */
  const BAND_ORDER = ["FR", "BR", "BL", "FL"];
  /* y^k image of the canonical DFR corner-right case, for slot lookup */
  const slotOf = (id) => CORNER_SLOTS.concat(EDGE_SLOTS).find((s) => s.id === id);
  const U_SLOT_OF = { DF: "FU", DR: "RU", BD: "BU", DL: "LU" };
  const U_CORNER_OF = { DFR: "FRU", DFL: "FLU", BDL: "BLU", BDR: "BRU" };
  /* y-rotation chain index for each slot: ySub^k of the canonical alg
     targets ySlot(canonical id, k) — NOT list order (DFR -> DRB -> DBL
     -> DFL, while FR -> BR -> BL -> FL happens to match list order). */
  const YK = {};
  for (let k = 0; k < 4; k++) {
    YK[ySlot("DFR", k)] = k;
    YK[ySlot("FR", k)] = k;
    YK[ySlot("DF", k)] = k;
  }
  const ykOf = (id) => {
    if (YK[id] !== undefined) return YK[id];
    throw new Error("no y-chain index for " + id);
  };

  const colorsAt = (state, slot) => slot.f.map((i) => state[i]);
  const findCubie = (state, id, slots) =>
    slots.find((s) => colorsAt(state, s).slice().sort().join("") === id);
  const isHome = (state, slot) => slot.f.every((i, j) => state[i] === SOLVED[slot.f[j]]);

  function compose(state, toks) {
    let s = state;
    for (const t of toks) s = applyMove(s, t);
    return s;
  }

  /* Solve a scrambled state layer by layer. `choose`, when given,
     receives {label, options, detail} at genuine equal-cost ties and
     must return an index (null/undefined = roster order, on the rung).
     Returns {moves, steps, ties, final, verified}. */
  let CO_CALL = 0;
  function solveLBL(state, choose) {
    const callId = ++CO_CALL;
    const allMoves = [];
    const steps = [];
    const ties = [];
    let cur = state.slice();
    let guard = 0;

    const push = (phase, title, detail, toks, tie) => {
      if (!toks.length && !detail) return;
      cur = compose(cur, toks);
      for (const t of toks) allMoves.push(t);
      steps.push({ phase, title, detail: detail || "", moves: toks.slice(),
        tie: tie || null, stateAfter: cur.slice() });
    };
    /* try candidate (alignment, alg) pairs; first verified success wins;
       equal-success alternatives are a tie for the engine */
    const tryAll = (phase, title, cands, win) => {
      const good = [];
      for (const c of cands) {
        const after = compose(cur, c.moves);
        if (win(after) && validateState(after).ok) good.push(c);
      }
      if (!good.length) return false;
      let picked = good[0];
      if (good.length > 1) {
        const tie = {
          phase, title,
          options: good.map((g) => g.label),
          chosen: 0, source: "roster",
        };
        if (choose) {
          const idx = choose({ label: tie.title, options: tie.options, phase });
          if (idx !== null && idx !== undefined && idx >= 0 && idx < good.length) {
            tie.chosen = idx;
            tie.source = "engine";
          } else {
            tie.source = "refused";
          }
        }
        ties.push(tie);
        picked = good[tie.chosen];
      }
      push(phase, title, picked.detail || good[0].detail, picked.moves,
        good.length > 1
          ? { options: good.map((g) => g.label), chosenIndex: picked === good[0] ? 0 : good.indexOf(picked) }
          : null);
      return true;
    };
    /* pick among equal-cost candidates: ask the engine at genuine ties,
       recording engine / refused / roster exactly like tryAll */
    const pickTied = (phase, title, good) => {
      const tie = { phase, title, options: good.map((g) => g.label),
        chosen: 0, source: good.length > 1 ? "roster" : "unique" };
      if (good.length > 1 && choose) {
        const idx = choose({ label: title, options: tie.options, phase });
        if (idx !== null && idx !== undefined && idx >= 0 && idx < good.length) {
          tie.chosen = idx;
          tie.source = "engine";
        } else {
          tie.source = "refused";
        }
      }
      ties.push(tie);
      good[tie.chosen].tie = { options: tie.options, chosenIndex: tie.chosen };
      return good[tie.chosen];
    };

    /* --- phase 1: the cross (four D edges) --- */
    for (const id of CROSS_IDS) {
      const slot = slotOf(id);
      if (isHome(cur, slot)) {
        push("cross", id + " edge already home", "", []);
        continue;
      }
      /* lift to the U layer */
      let hops = 0;
      let pos = findCubie(cur, id, EDGE_SLOTS);
      while (pos.y !== 1 && hops < 3) {
        hops++;
        let eject = null;
        if (pos.y === -1) {
          /* cross layer: the side face's double turn lifts it */
          const side = pos.id.replace("D", "");
          eject = { moves: [side + "2"], label: side + "2 lifts it out" };
        } else {
          /* band slot: a corner-insert trigger on an adjacent face ejects it */
          const k = ykOf(pos.id);
          eject = { moves: ySub(CORNER_RIGHT, k), label: "trigger lifts it out of " + pos.id };
        }
        push("cross", "free the " + id + " edge",
          pos.y === -1 ? "it sits in the bottom row wrong — a half turn of its side face lifts it up"
            : eject.label, eject.moves, null);
        pos = findCubie(cur, id, EDGE_SLOTS);
      }
      /* align above its slot and insert (D up: double turn; flipped: insert alg) */
      const g = id[1] === "D" ? id[0] : id[1]; /* the side face of this cross edge */
      let done = false;
      for (let j = 0; j < 4 && !done; j++) {
        const aligned = compose(cur, ["U".repeat(0) + "U"].slice(0, 0).concat(Array(j).fill("U")));
        const pos2 = findCubie(aligned, id, EDGE_SLOTS);
        if (pos2.id !== U_SLOT_OF[id]) continue;
        const colors = colorsAt(aligned, pos2);
        const dOnTop = colors[U_SLOT_OF[id].indexOf("U")] === "D";
        const k = ykOf(id);
        const cands = dOnTop
          ? [{ moves: [g + "2"], label: g + "2 drops it in", detail: "the D sticker faces up — a half turn drops it straight down" }]
          : [{ moves: ySub(FLIP_INSERT, k),
               label: "flipped-edge insert", detail: "the D sticker faces sideways — this six-move insert flips it into place" }];
        const seq = Array(j).fill("U").concat(cands[0].moves);
        const after = compose(cur, seq);
        if (isHome(after, slot) && validateState(after).ok) {
          push("cross", "place the " + id + " edge",
            (j ? "turn U " + j + " quarter" + (j > 1 ? "s" : "") + " to line it up. " : "")
            + cands[0].detail, seq, null);
          done = true;
        }
      }
      if (!done) throw new Error("cross insert failed for " + id);
    }

    /* --- phase 2: first-layer corners --- */
    for (const id of CORNER_ORDER) {
      const slot = slotOf(id);
      if (isHome(cur, slot)) {
        push("corners", id + " corner already home", "", []);
        continue;
      }
      let pos = findCubie(cur, id, CORNER_SLOTS);
      if (pos.y === -1) {
        /* stuck in the D layer (wrong slot or twisted): that slot's own
           insert trigger swaps it with whatever sits above */
        const k = ykOf(pos.id);
        const ej = ySub(CORNER_RIGHT, k);
        push("corners", "eject the " + id + " corner",
          "it sits in the bottom layer wrong — the trigger over " + pos.id
          + " swaps it with the corner above", ej, null);
        pos = findCubie(cur, id, CORNER_SLOTS);
      }
      const uSlot = U_CORNER_OF[id];
      const k = ykOf(id);
      let done = false;
      for (let j = 0; j < 4 && !done; j++) {
        const seqPre = Array(j).fill("U");
        const aligned = compose(cur, seqPre);
        const pos2 = findCubie(aligned, id, CORNER_SLOTS);
        if (pos2.id !== uSlot) continue;
        const colors = colorsAt(aligned, pos2);
        const dFace = colors.indexOf("D") >= 0
          ? FACE_OF(pos2.f[colors.indexOf("D")]) : null;
        const cands = [];
        if (dFace === "U") {
          cands.push({ moves: ySub(CORNER_UP, k), label: "D-up insert",
            detail: "the D sticker faces up — this seven-move sequence seats it" });
        } else {
          cands.push({ moves: ySub(CORNER_RIGHT, k), label: "right trigger",
            detail: "the D sticker faces sideways — a three-move trigger twists it down" });
          cands.push({ moves: ySub(CORNER_LEFT, k), label: "left trigger",
            detail: "the D sticker faces the other side — the mirrored trigger seats it" });
        }
        for (const c of cands) {
          const seq = seqPre.concat(c.moves);
          const after = compose(cur, seq);
          if (isHome(after, slot) && validateState(after).ok) {
            push("corners", "seat the " + id + " corner",
              (j ? "turn U " + j + " quarter" + (j > 1 ? "s" : "") + " to bring it over its slot. " : "")
              + c.detail, seq, null);
            done = true;
            break;
          }
        }
      }
      if (!done) throw new Error("corner insert failed for " + id);
    }

    /* --- phase 3: middle-layer edges --- */
    for (const id of BAND_ORDER) {
      const slot = slotOf(id);
      if (isHome(cur, slot)) {
        push("middle", id + " edge already home", "", []);
        continue;
      }
      let pos = findCubie(cur, id, EDGE_SLOTS);
      if (pos.y === 0) {
        /* stuck in the band: any U-edge insert into that slot ejects it */
        const k = BAND_ORDER.indexOf(pos.id);
        const ej = ySub(MID_RIGHT, k);
        push("middle", "eject the " + id + " edge",
          "it sits in the middle ring wrong — running that slot's insert once spits it up top", ej, null);
        pos = findCubie(cur, id, EDGE_SLOTS);
      }
      /* both orientations of the cubie are insertable: MID_RIGHT@k for
         the slot its F/B color faces, MID_LEFT@k for the mirrored one —
         collect every (alg, alignment) that simulates to home */
      const cands = [];
      for (let kk = 0; kk < 4; kk++) {
        if (ySlot("FR", kk) === id) cands.push({ alg: ySub(MID_RIGHT, kk), label: "right insert" });
        if (ySlot("FL", kk) === id) cands.push({ alg: ySub(MID_LEFT, kk), label: "left insert" });
      }
      let done = false;
      for (let j = 0; j < 4 && !done; j++) {
        const seqPre = Array(j).fill("U");
        const aligned = compose(cur, seqPre);
        const pos2 = findCubie(aligned, id, EDGE_SLOTS);
        if (pos2.y !== 1) continue;
        for (const c of cands) {
          const seq = seqPre.concat(c.alg);
          const after = compose(cur, seq);
          if (isHome(after, slot) && validateState(after).ok) {
            push("middle", "insert the " + id + " edge",
              (j ? "turn U " + j + " quarter" + (j > 1 ? "s" : "") + " to line the edge up. " : "")
              + (c.label === "right insert"
                ? "the top sticker points right — the right-hand eight-move insert slots it"
                : "the top sticker points left — the mirrored left-hand insert slots it"),
              seq, null);
            done = true;
            break;
          }
        }
      }
      if (!done) throw new Error("middle insert failed for " + id);
    }

    /* --- phase 4: last layer --- */
    const uEdges = EDGE_SLOTS.filter((e) => e.y === 1);
    const uCorners = CORNER_SLOTS.filter((c) => c.y === 1);
    const orientedEdges = (s) => uEdges.filter((e) => {
      const ui = e.f.find((i) => FACE_OF(i) === "U");
      return s[ui] === "U";
    }).length;
    const orientedCorners = (s) => uCorners.filter((c) => {
      const ui = c.f.find((i) => FACE_OF(i) === "U");
      return s[ui] === "U";
    }).length;
    const cornersPlaced = (s) => uCorners.filter((c) =>
      colorsAt(s, c).slice().sort().join("") === c.id && isHome(s, c)).length;
    const edgesPlaced = (s) => uEdges.filter((e) =>
      colorsAt(s, e).slice().sort().join("") === e.id && isHome(s, e)).length;

    /* edge orientation (dot / line / L / solved) */
    for (let round = 0; round < 4 && orientedEdges(cur) < 4; round++) {
      const cands = [];
      for (let j = 0; j < 4; j++) {
        for (const [nm, alg] of [["eo", LL.eo], ["eoL", LL.eoL]]) {
          const seq = Array(j).fill("U").concat(alg);
          const after = compose(cur, seq);
          cands.push({ moves: seq, after,
            label: "U" + (j ? "^" + j : "") + " then " + (nm === "eo" ? "F R U R' U' F'" : "F U R U' R' F'"),
            j, score: orientedEdges(after) });
        }
      }
      const best = Math.max(...cands.map((c) => c.score));
      if (best <= orientedEdges(cur)) throw new Error("edge orientation stuck");
      const good = cands.filter((c) => c.score === best);
      const pick = pickTied("oll-edges", "line-up for the edge flip", good);
      push("oll-edges", "orient the top edges",
        orientedEdges(pick.after) === 4 ? "" : pick.label.split(" then ")[1] + " flips the top edges toward solved", pick.moves, pick.tie || null);
    }

    /* corner orientation (sune family) — greedy on the CO distance table */
    for (let round = 0; round < CO_CAP && CO_DIST.get(coKeyOf(cur)) > 0; round++) {
      const d0 = CO_DIST.get(coKeyOf(cur));
      if (d0 === undefined) throw new Error("corner orientation state missing from distance table");
      const cands = [];
      for (let a = 0; a < 4; a++) {
        for (let b = 0; b < 4; b++) {
          for (const [name, alg] of [["sune", LL.sune], ["anti-sune", LL.antiSune]]) {
            const seq = Array(a).fill("U").concat(alg, Array(b).fill("U"));
            const after = compose(cur, seq);
            const d = CO_DIST.get(coKeyOf(after));
            cands.push({ moves: seq, after,
              label: (a ? "U^" + a + " + " : "") + name + (b ? " + U^" + b : ""),
              d: d === undefined ? Infinity : d });
          }
        }
      }
      const best = Math.min(...cands.map((c) => c.d));
      if (best >= d0) {
        if (typeof process !== "undefined" && process.env.CUBE_DEBUG) {
          console.error("CO[c" + callId + "] stuck: key", coKeyOf(cur), "d0", d0,
            "cands", cands.map((c) => c.label + "=" + c.d).join(", "),
            "readouts", CO_TOP.map((c) => cornerTwist(cur, c)).join(""));
        }
        throw new Error("corner orientation stuck (distance " + d0 + ")");
      }
      const good = cands.filter((c) => c.d === best);
      const pick = pickTied("oll-corners", "corner twist setup", good);
      if (typeof process !== "undefined" && process.env.CUBE_DEBUG) {
        const k = coKeyOf(pick.after);
        console.error("CO[c" + callId + "] round", round, "picked", pick.label, "-> dist", CO_DIST.get(k), "key", k,
          "readouts", CO_TOP.map((c) => cornerTwist(pick.after, c)).join(""));
      }
      push("oll-corners", "orient the top corners",
        CO_DIST.get(coKeyOf(pick.after)) === 0 ? "" : "each sune twists three corners — repeated with the right setup it walks the twist to zero", pick.moves, pick.tie || null);
    }
    if (typeof process !== "undefined" && process.env.CUBE_DEBUG) {
      const k = coKeyOf(cur);
      console.error("CO[c" + callId + "] exit dist", CO_DIST.get(k), "key", k, "readouts", CO_TOP.map((c) => cornerTwist(cur, c)).join(""));
    }

    /* corner permutation — 3-cycles (A both directions) for even cases,
       T/Y transpositions for the odd pair-swap cases. Candidates take
       U^a before AND U^b after: bare pre-alignment composes rather than
       conjugates and misses most of the 12-case space (enumerated). */
    const aPermInv = LL.aPerm.slice().reverse().map((t) =>
      t.endsWith("2") ? t : t.endsWith("'") ? t[0] : t[0] + "'");
    for (let round = 0; round < 4 && cornersPlaced(cur) < 4; round++) {
      const cands = [];
      for (let a = 0; a < 4; a++) {
        for (let b = 0; b < 4; b++) {
          for (const [name, alg] of [["A-perm", LL.aPerm], ["A-perm inverse", aPermInv],
            ["T-perm", LL.tPerm], ["Y-perm", LL.yPerm]]) {
            const seq = Array(a).fill("U").concat(alg, Array(b).fill("U"));
            const after = compose(cur, seq);
            cands.push({ moves: seq,
              label: (a ? "U^" + a + " + " : "") + name + (b ? " + U^" + b : ""),
              score: cornersPlaced(after) });
          }
        }
      }
      const best = Math.max(...cands.map((c) => c.score));
      const base = cornersPlaced(cur);
      if (best <= base) {
        const det = uCorners.map((c) => c.id + "<" + colorsAt(cur, c).join("")
          + "/tw" + ((cornerTwist(cur, c) - CO_KAPPA[CO_TOP.indexOf(CO_TOP.find((p) =>
              p.f.map((i) => cur[i]).sort().join("") === c.id))][CO_TOP.indexOf(c)]) % 3 + 3) % 3);
        throw new Error("corner permutation stuck (round " + round + ", base " + base + "); " + det.join(" "));
      }
      const good = cands.filter((c) => c.score === best);
      const pick = pickTied("pll-corners", "corner cycle setup", good);
      push("pll-corners", "cycle the top corners", "", pick.moves, pick.tie || null);
    }
    if (cornersPlaced(cur) < 4) {
      const det = uCorners.map((c) => c.id + "<" + colorsAt(cur, c).join("")
        + (CO_TOP.includes(c) ? "/tw" + cornerTwist(cur, c) : ""));
      throw new Error("corner permutation stuck; " + det.join(" "));
    }

    /* edge permutation (U-perm family) */
    for (let round = 0; round < 4 && edgesPlaced(cur) < 4; round++) {
      const cands = [];
      for (let j = 0; j < 4; j++) {
        for (const [name, alg] of [["Ua", LL.uPerm], ["Ub", U_PERM_INV],
          ["H", H_PERM]]) {
          /* two-sided conjugate: the trailing U undoes the alignment so the
             corners (already home) stay home — a bare U^j prefix rotates
             them away and no edge-only alg brings them back */
          const back = (4 - j) % 4;
          const seq = Array(j).fill("U").concat(alg, Array(back).fill("U"));
          const after = compose(cur, seq);
          cands.push({ moves: seq, after,
            label: "U" + (j ? "^" + j : "") + " + " + name + "-perm" + (back ? " + U^" + back : ""),
            j, name, score: edgesPlaced(after) });
        }
      }
      const best = Math.max(...cands.map((c) => c.score));
      if (best <= edgesPlaced(cur)) {
        const displaced = EDGE_SLOTS.filter((e) => !isHome(cur, e)).map((e) => e.id);
        throw new Error("edge permutation stuck; displaced: " + displaced.join(","));
      }
      const good = cands.filter((c) => c.score === best);
      const pick = pickTied("pll-edges", "edge cycle choice", good);
      push("pll-edges", "cycle the top edges", "", pick.moves, pick.tie || null);
    }

    /* final AUF: the one U turn that lines the top face with the sides */
    for (let j = 0; j < 4; j++) {
      const seq = Array(j).fill("U");
      const after = compose(cur, seq);
      if (verifySolved(after).solved) {
        push("auf", "final U turn", j ? "one last quarter turn lines the top with the sides" : "", seq, null);
        break;
      }
    }
    if (!verifySolved(cur).solved) {
      throw new Error("last-layer alignment failed; the PLL phases left "
        + cornersPlaced(cur) + "/4 corners and " + edgesPlaced(cur) + "/4 edges home");
    }

    const verified = verifySolved(cur);
    return { moves: allMoves, steps, ties, final: cur, verified,
      legal: validateState(cur).ok };
  }

  /* ---------- game ---------- */

  function createGame(cfgRaw) {
    const cfg = normalizeConfig(cfgRaw);
    const scramble = makeScramble(cfg);
    const state = compose(SOLVED, scramble);
    return { cfg, stamp: cubeStamp(cfg), scramble, state,
      solvedStart: verifySolved(state).solved };
  }

  globalThis.CubeCore = {
    FACES, NORMAL, FACE_BASE, MOVES, SOLVED, FACE_OF,
    CORNER_SLOTS, EDGE_SLOTS, MOVE_PERM, CORNER_IDS, EDGE_IDS,
    CHIRALITY,
    faceletAt, rotateFor, applyMove, applyAlg, parseMove,
    cornerTwist, edgeFlip, permParity, cubieView,
    validateState, verifySolved,
    ySub, ySlot,
    CORNER_RIGHT, CORNER_LEFT, CORNER_UP, FLIP_INSERT, MID_RIGHT, MID_LEFT, LL,
    U_PERM_INV, H_PERM,
    solveLBL, createGame,
    DEFAULT_CONFIG, normalizeConfig, cubeStamp, makeScramble,
  };
})();
