/* Tests for cube-n-core.js — the generalized N×N×N family.
   The validator's laws are simulation-derived (see the core header); these
   tests keep them pinned: if a law ever fails on a seeded walk, the move
   algebra or a slot definition regressed. Run: node --test tests/cube-n-core.test.mjs */
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const src = readFileSync(path.join(here, "..", "cube-n-core.js"), "utf8");
/* the core is a plain IIFE attaching to globalThis — evaluate it in this
   context the same way a browser does */
new Function(src)();
const CNC = globalThis.CubeNCore;

test("createCore bounds and shape", () => {
  assert.throws(() => CNC.createCore(1));
  assert.throws(() => CNC.createCore(22));
  assert.throws(() => CNC.createCore(3.5));
  const c2 = CNC.createCore(2);
  assert.equal(c2.stateSize, 24);
  assert.equal(c2.faceCount, 4);
  assert.equal(c2.edgeSlots.length, 0);      /* a 2×2 has no edge pieces */
  assert.equal(c2.cornerSlots.length, 8);
  const c4 = CNC.createCore(4);
  assert.equal(c4.stateSize, 96);
  assert.equal(c4.edgeSlots.length, 24);     /* 12 edge lines × 2 wings */
  assert.equal(c4.maxDepth, 2);
  const c5 = CNC.createCore(5);
  assert.equal(c5.edgeSlots.length, 36);     /* 12 × 3 (two wings + center edge) */
  const c21 = CNC.createCore(21);
  assert.equal(c21.stateSize, 6 * 441);
  assert.equal(c21.edgeSlots.length, 12 * 19);
});

test("facelet numbering matches cube-core at n=3 (URFDLB)", () => {
  /* lattice coords for n=3 are {-2, 0, 2}; anchors are cube-core's own
     values (probed, not guessed): U(2,2,2)=8, R=9, F=20, D(2,-2,-2)=35,
     L(-2,2,-2)=36, B(-2,2,-2)=47 */
  const c = CNC.createCore(3);
  assert.equal(c.faceletAt("U", 2, 2, 2), 8);
  assert.equal(c.faceletAt("R", 2, 2, 2), 9);
  assert.equal(c.faceletAt("F", 2, 2, 2), 20);
  assert.equal(c.faceletAt("D", 2, -2, -2), 35);
  assert.equal(c.faceletAt("L", -2, 2, -2), 36);
  assert.equal(c.faceletAt("B", -2, 2, -2), 47);
  /* the whole facelet space is a bijection: 54 distinct indices */
  const seen = new Set();
  for (const [x, y, z] of c.CUBIE_COORDS) {
    for (const f of CNC.FACES) {
      const nv = CNC.NORMAL[f];
      if (nv[0] * x + nv[1] * y + nv[2] * z !== 2) continue;
      const idx = c.faceletAt(f, x, y, z);
      assert.ok(idx >= 0 && idx < 54, "idx in range");
      assert.ok(!seen.has(idx), "duplicate facelet " + idx);
      seen.add(idx);
    }
  }
  assert.equal(seen.size, 54);
});

test("solved state validates and verifies", () => {
  for (const n of [2, 3, 4, 5, 7, 21]) {
    const c = CNC.createCore(n);
    const v = c.validateState(c.solvedState);
    assert.deepEqual(v.issues, [], "n=" + n);
    assert.equal(v.legal, true);
    assert.equal(c.verifySolved(c.solvedState).solved, true);
  }
});

test("seeded random walks keep every validator law (n = 2..8)", () => {
  /* the simulation-derived laws: color counts, piece multiplicity,
     corner-twist sum, edge-flip sum. Any violation here means the move
     algebra or slot definitions regressed. */
  for (const n of [2, 3, 4, 5, 6, 7, 8]) {
    const c = CNC.createCore(n);
    let state = c.solvedState.slice();
    for (let i = 0; i < 250; i++) {
      const toks = c.makeScramble(1000 + i, 1);
      state = c.applyAlg(state, toks);
      const v = c.validateState(state);
      assert.deepEqual(v.issues, [], "n=" + n + " step " + i + " after " + toks.join(" ")
        + ": " + v.issues.map((x) => x.check).join(","));
    }
  }
});

test("moves compose and invert back to solved", () => {
  for (const n of [2, 4, 5, 11]) {
    const c = CNC.createCore(n);
    for (const tok of c.MOVE_KEYS) {
      const s1 = c.applyMove(c.solvedState, tok);
      if (c.verifySolved(s1).solved) continue; /* a full-cube token cannot exist, but stay safe */
      const back = c.applyMove(s1, c.invertToken(tok));
      assert.equal(c.verifySolved(back).solved, true, "n=" + n + " token " + tok);
    }
  }
});

test("wide tokens turn their full band (3Uw on 7×7 reaches depth 3)", () => {
  /* n=7: maxDepth = 3, so 3Uw is legal here (it was not on 5×5). Layers
     count inward from U: depth 1 = y=6, depth 2 = y=4, depth 3 = y=2;
     the fixed center (y=0) never moves. An R-face sticker at y=2 sits in
     band 3: Uw (band 1-2) must leave it, 3Uw must move it. */
  const c = CNC.createCore(7);
  const wing = c.faceletAt("R", 6, 2, 0);
  const center = c.faceletAt("U", 0, 6, 0);
  const afterUw = c.applyMove(c.solvedState, "Uw");
  assert.equal(afterUw[wing], "R", "Uw must not reach a depth-3 sticker");
  assert.equal(afterUw[center], "U", "Uw must not move the fixed center");
  const after3 = c.applyMove(c.solvedState, "3Uw");
  assert.notEqual(after3[wing], "R", "3Uw must turn the depth-3 band");
  assert.equal(after3[center], "U", "no U token may move the fixed center");
  assert.throws(() => c.applyMove(c.solvedState, "4Uw"), /unknown move/,
    "band deeper than maxDepth is refused");
});

test("scrambles are deterministic per seed and produce legal states", () => {
  for (const n of [2, 4, 5, 9]) {
    const c = CNC.createCore(n);
    const a = c.makeScramble(77, 20);
    const b = c.makeScramble(77, 20);
    const d = c.makeScramble(78, 20);
    assert.deepEqual(a, b);
    assert.notDeepEqual(a, d);
    const g = c.createGame({ seed: "abc", scrambleLen: 20 });
    assert.equal(c.validateState(g.state).legal, true);
    assert.equal(c.verifySolved(g.state).solved, false);
    assert.equal(g.scramble.length, 20);
  }
});

test("createGame is reproducible for the same string seed", () => {
  const c = CNC.createCore(4);
  const g1 = c.createGame({ seed: "42", scrambleLen: 10 });
  const g2 = c.createGame({ seed: "42", scrambleLen: 10 });
  assert.deepEqual(g1.state, g2.state);
  assert.deepEqual(g1.scramble, g2.scramble);
});

test("illegal states are refused with named reasons", () => {
  const c = CNC.createCore(4);
  /* wrong color count: swap one facelet's color */
  const bad = c.solvedState.slice();
  bad[0] = "L";
  const v = c.validateState(bad);
  assert.equal(v.legal, false);
  assert.ok(v.issues.some((i) => i.check === "color-count"));
  /* wrong shape */
  assert.equal(c.validateState([1, 2, 3]).legal, false);
  /* a single swapped corner pair breaks piece multiplicity on a solved
     board? no — swapping two corners keeps counts; but it breaks the twist
     law only if twisted… a pure corner swap on a solved cube keeps every
     law we pin (parity is deliberately unpinned). So instead: give one
     corner slot a duplicate of another corner's colors. */
  const dup = c.solvedState.slice();
  const a = c.cornerSlots[0].f, b = c.cornerSlots[1].f;
  for (let k = 0; k < 3; k++) dup[a[k]] = c.solvedState[b[k]];
  const vd = c.validateState(dup);
  assert.equal(vd.legal, false);
  assert.ok(vd.issues.some((i) => i.check === "piece-multiplicity" || i.check === "color-count"));
});

test("2×2 solver: 1000 seeded scrambles all solve, every step pre-verified", () => {
  const c = CNC.createCore(2);
  let solved = 0;
  let maxSteps = 0;
  let maxMoves = 0;
  for (let seed = 1; seed <= 1000; seed++) {
    const g = c.createGame({ seed: String(seed), scrambleLen: 12 });
    const plan = c.solve(g.state);
    assert.ok(plan, "seed " + seed + " returned null");
    assert.equal(plan.refused, false, "seed " + seed + " refused: " + (plan.reason || ""));
    /* replay the plan independently and confirm it lands on solved */
    let s = g.state;
    for (const step of plan.steps) {
      for (const t of step.moves) s = c.applyMove(s, t);
    }
    assert.equal(c.verifySolved(s).solved, true, "seed " + seed + " plan did not solve");
    assert.equal(c.validateState(s).legal, true, "seed " + seed + " plan left an illegal state");
    solved++;
    maxSteps = Math.max(maxSteps, plan.steps.length);
    maxMoves = Math.max(maxMoves, plan.moves.length);
  }
  assert.equal(solved, 1000);
  assert.ok(maxSteps <= 12, "step count exploded: " + maxSteps);
  assert.ok(maxMoves <= 160, "move count exploded: " + maxMoves);
});

test("2×2 solver: accepts an engine chooser for genuine ties", () => {
  const c = CNC.createCore(2);
  const g = c.createGame({ seed: "7", scrambleLen: 12 });
  let calls = 0;
  const plan = c.solve(g.state, () => { calls++; return 0; });
  assert.equal(plan.refused, false);
  /* ties exist on most runs; the chooser must have been consulted at least
     once across the phases */
  assert.ok(calls >= 1, "chooser never consulted");
});

test("2×2 solver refuses an illegal state", () => {
  const c = CNC.createCore(2);
  const bad = c.solvedState.slice();
  bad[0] = "L";
  const plan = c.solve(bad);
  assert.equal(plan.refused, true);
  assert.match(plan.reason, /invalid state/);
});

test("no solver hook is exposed for sizes the core does not coach", () => {
  for (const n of [3, 4, 5, 21]) {
    const c = CNC.createCore(n);
    assert.equal(c.solverAvailable, false);
    assert.equal(c.solve, undefined);
  }
});
