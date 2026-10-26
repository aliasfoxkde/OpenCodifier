/* Rubik's Clock solver — core suite. Import order matters: the SDK first,
   then clock-core; read the globals.

   Spec-pinned tests: the derived move table equals the WCA scrambler's
   reference rows (tnoodle-lib ClockPuzzle.java) and satisfies the
   documented pin identities (4 dials around each up pin, reverse side
   sees only the up-pin corner dials); moves commute (Jaap: the clock's
   positions form an Abelian group); y2 is an involution that conjugates
   each move to its swapped-half image; the four corner-pair laws are
   preserved by every move and exactly characterize the reachable set;
   the solver's published-order plan lands every seeded scramble on
   all-zeros with every replayed step law-legal; invalid states are
   refused with named reasons.

   Run: node --test site/tests/clock_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'clock-core.js'));
const K = globalThis.ClockCore;
const DK = globalThis.DecisionsSDK;

const SOLVED = K.SOLVED;
const rand = DK.makeRng("clock-test");
const randomWalkState = (steps) => {
  /* random walk from solved: law-legal by construction */
  let s = SOLVED.slice();
  for (let i = 0; i < steps; i++) {
    const m = K.MOVES[Math.floor(rand() * K.MOVES.length)];
    s = K.applyMove(s, m + Math.floor(rand() * 12) + "+");
  }
  return s;
};
const randState = () => randomWalkState(8);

test('solved state verifies; a scrambled one does not; round-trip closes', () => {
  assert.equal(K.verifySolved(SOLVED.slice()).solved, true);
  const g = K.createGame({ seed: "spec" });
  assert.equal(K.verifySolved(g.state).solved, false);
  assert.equal(K.validateState(g.state).ok, true);
  const r = K.solve(g.state);
  assert.equal(K.verifySolved(r.final).solved, true);
  assert.deepEqual(K.applyAlg(g.state, r.moves), r.final, "replay drift");
});

test('derived table equals the WCA scrambler reference and the pin identities', () => {
  /* load already aborts on mismatch; pin the equality explicitly */
  for (const m of K.MOVES) {
    assert.deepEqual(K.MOVE_VEC[m], K.REFERENCE[m], m + " row drifted from tnoodle");
  }
  /* wiki: a dial turned next to its up pin turns "the 4 mini clocks
     around this pin" — the front set is the union of the around-pin
     sets, each dial counted once */
  for (const m of K.MOVES) {
    const want = new Set();
    for (const pin of K.PINS_UP[m]) for (const d of K.AROUND[pin]) want.add(d);
    for (let d = 0; d < 9; d++) {
      assert.equal(K.MOVE_VEC[m][d], want.has(d) ? 1 : 0, m + " front dial " + d);
    }
  }
  /* Jaap: turn a corner next to an up button and "none of the clocks on
     the reverse side move, except for the corner clocks" — exactly one
     back entry, at the same physical corner's back dial, opposite sign */
  for (const m of K.MOVES) {
    const backEntries = [];
    for (let d = 9; d < 18; d++) if (K.MOVE_VEC[m][d] !== 0) backEntries.push(d);
    const upCorners = K.PINS_UP[m].map((p) => K.CORNER_PAIRS[p][1]).sort((a, b) => a - b);
    assert.deepEqual(backEntries, upCorners, m + " reverse-side dials");
    for (const d of backEntries) assert.equal(K.MOVE_VEC[m][d], -1, m + " back dial " + d);
  }
});

test('commutativity: wheel moves are additive and commute (Jaap: Abelian)', () => {
  for (let i = 0; i < 60; i++) {
    const s = randState();
    const a = K.MOVES[Math.floor(rand() * 9)] + Math.floor(rand() * 12) + "+";
    const b = K.MOVES[Math.floor(rand() * 9)] + Math.floor(rand() * 12) + "+";
    assert.deepEqual(K.applyMove(K.applyMove(s, a), b),
      K.applyMove(K.applyMove(s, b), a), a + " vs " + b);
  }
  /* and longer sequences: any permutation of the same WHEEL tokens
     agrees — y2 is a puzzle flip, not a translation, so it stays out */
  const s = randState();
  const toks = ["UR3+", "D2-", "ALL5+", "UL4-", "R6-", "L1+"];
  for (let i = 0; i < 12; i++) {
    const shuffled = toks.slice();
    for (let j = shuffled.length - 1; j > 0; j--) {
      const p = Math.floor(rand() * (j + 1));
      [shuffled[j], shuffled[p]] = [shuffled[p], shuffled[j]];
    }
    assert.deepEqual(K.applyAlg(s, shuffled.join(" ")), K.applyAlg(s, toks.join(" ")));
  }
});

test('y2 is an involution and conjugates each move to its swapped-half image', () => {
  const s = randState();
  assert.deepEqual(K.applyAlg(s, "y2 y2"), s, "y2 must be its own inverse");
  for (const m of K.MOVES) {
    const tok = m + "3+";
    /* moving the flipped puzzle = flipping the puzzle moved by the
       swapped-half vector: T_g . S = S . T_{S(g)} */
    const lhs = K.applyMove(K.swapHalves(s), tok);
    const rhs = K.swapHalves(K.applyVec(s, K.swapVec(K.MOVE_VEC[m]), 3));
    assert.deepEqual(lhs, rhs, "conjugation " + m);
  }
});

test('corner-pair laws: every move preserves them from any law-legal state', () => {
  const pairSum = (s, k) => (((s[K.CORNER_PAIRS[k][0]] + s[K.CORNER_PAIRS[k][1]]) % 12) + 12) % 12;
  for (let i = 0; i < 40; i++) {
    const s = randState();
    for (const k of K.PAIR_LAWS) assert.equal(pairSum(s, k), 0, "walk left " + k + " unsolved");
    for (const m of K.MOVES) {
      const t = K.applyMove(s, m + Math.floor(rand() * 12) + "+");
      for (const k of K.PAIR_LAWS) assert.equal(pairSum(t, k), 0, m + " broke " + k);
    }
    const flipped = K.swapHalves(s);
    for (const k of K.PAIR_LAWS) assert.equal(pairSum(flipped, k), 0, "y2 broke " + k);
  }
});

test('solver sweep: 520 seeded scrambles all close, verified and law-legal', () => {
  let totalWheel = 0, maxWheel = 0;
  const RUNS = 520;
  for (let i = 0; i < RUNS; i++) {
    const g = K.createGame({ seed: "sweep-" + i, scrambleLen: 6 });
    const r = K.solve(g.state);
    assert.equal(r.verified.solved, true, "seed sweep-" + i + " not solved");
    assert.equal(r.legal, true, "seed sweep-" + i + " illegal");
    assert.deepEqual(K.applyAlg(g.state, r.moves), r.final, "seed sweep-" + i + " replay drift");
    assert.ok(r.wheelMoves <= 14, "seed sweep-" + i + " used " + r.wheelMoves + " wheel moves");
    for (const st of r.steps) {
      assert.equal(K.validateState(st.stateAfter).ok, true,
        "seed sweep-" + i + " illegal intermediate");
    }
    totalWheel += r.wheelMoves;
    if (r.wheelMoves > maxWheel) maxWheel = r.wheelMoves;
  }
  const mean = totalWheel / RUNS;
  console.log(`solve sweep: ${RUNS} scrambles, mean ${mean.toFixed(3)} wheel moves, max ${maxWheel}`);
  assert.ok(mean < 15, "mean wheel moves " + mean + " regressed");
  /* random-walk states (not just scrambles) close too */
  for (let i = 0; i < 60; i++) {
    const s = randomWalkState(1 + Math.floor(rand() * 25));
    const r = K.solve(s);
    assert.equal(K.verifySolved(r.final).solved, true, "walk " + i + " not solved");
  }
});

test('solver order follows the published plan phases', () => {
  const g = K.createGame({ seed: "phases", scrambleLen: 9 });
  const r = K.solve(g.state);
  const order = K.SOLVE_PLAN.map((p) => p.phase);
  let seen = new Set();
  let lastIdx = -1;
  for (const st of r.steps) {
    if (st.phase.endsWith("-flip")) continue;
    const pi = order.indexOf(st.phase);
    assert.ok(pi >= 0, "unknown phase " + st.phase);
    assert.ok(pi >= lastIdx, "phase regression at " + st.phase);
    if (pi > lastIdx) seen.add(st.phase);
    lastIdx = pi;
  }
  const used = r.wheelMoves > 0;
  assert.equal(used, true);
});

test('validator rejections: out-of-range, non-integer, wrong shape, broken pairs', () => {
  const fails = (state, name, frag) => {
    const v = K.validateState(state);
    assert.equal(v.ok, false, "expected rejection: " + name);
    const hit = v.checks.find((c) => !c.ok && c.reason.includes(frag));
    assert.ok(hit, "no check named for " + name + " mentioning " + frag);
    return v;
  };
  const bad13 = SOLVED.slice(); bad13[4] = 13;
  fails(bad13, "dial 13", "reads 13");
  const badNeg = SOLVED.slice(); badNeg[9] = -1;
  fails(badNeg, "dial -1", "reads -1");
  const badFloat = SOLVED.slice(); badFloat[3] = 2.5;
  fails(badFloat, "float dial", "2.5");
  const badStr = SOLVED.slice(); badStr[7] = "3";
  fails(badStr, "string dial", "reads 3");
  fails([0, 0, 0], "short state", "18 dial readings");
  fails(SOLVED.slice().concat([0]), "long state", "18 dial readings");
  fails("nope", "non-array", "18 dial readings");
  /* break one corner pair in range: front UR +1 alone */
  const badPair = randomWalkState(6).slice();
  badPair[K.CORNER_PAIRS.UR[0]] = (badPair[K.CORNER_PAIRS.UR[0]] + 1) % 12;
  const v = fails(badPair, "broken UR pair", "corner UR");
  assert.equal(v.checks.find((c) => c.name === "corner-pair-UR").ok, false);
});

test('move algebra: parse forms, mod-12 equivalence, refusals', () => {
  assert.deepEqual(K.parseMove("UR3+"), { name: "UR", amount: 3 });
  assert.deepEqual(K.parseMove("UR2-"), { name: "UR", amount: 10 });
  assert.deepEqual(K.parseMove("DL+"), { name: "DL", amount: 1 });
  assert.deepEqual(K.parseMove("DL-"), { name: "DL", amount: 11 });
  assert.deepEqual(K.parseMove("ALL8-"), { name: "ALL", amount: 4 });
  assert.deepEqual(K.parseMove("y2"), { name: "y2", amount: 0 });
  /* full turns vanish; complement spellings agree */
  const s = randState();
  assert.deepEqual(K.applyMove(s, "UR12+"), s);
  assert.deepEqual(K.applyMove(s, "UR0+"), s);
  assert.deepEqual(K.applyMove(s, "UR4-"), K.applyMove(s, "UR8+"));
  assert.deepEqual(K.applyMove(s, "ALL6+"), K.applyMove(s, "ALL6-"));
  /* whitespace-separated sequences */
  assert.deepEqual(K.applyAlg(s, "  y2   UL4-  "), K.applyMove(K.applyMove(s, "y2"), "UL4-"));
  for (const bad of ["", "XX3+", "ur3+", "UR", "UR3", "UR3*", "y", "y3", "UR-3", "UR++"]) {
    assert.throws(() => K.parseMove(bad), undefined, "token " + JSON.stringify(bad) + " should refuse");
  }
  /* UR-3 style is deliberately not in the grammar; the sign trails */
  assert.deepEqual(K.applyMove(s, "UR9-"), K.applyMove(s, "UR3+"));
});

test('scrambles: WCA shape, sign ranges, deterministic per seed', () => {
  const g = K.createGame({ seed: "shape", scrambleLen: 6 });
  const y = g.scramble.indexOf("y2");
  assert.equal(y, 6, "y2 must separate the two sections");
  assert.equal(g.scramble.length, 13, "6 + y2 + 6 tokens");
  const front = g.scramble.slice(0, y), back = g.scramble.slice(y + 1);
  for (const section of [front, back]) {
    const names = section.map((t) => K.parseMove(t).name);
    assert.equal(new Set(names).size, section.length, "no repeated move type in a section");
    for (const t of section) {
      const { amount } = K.parseMove(t);
      const shown = Number(t.match(/(\d+)/)[1]);
      assert.ok(shown >= 0 && shown <= 5, "amount " + t + " outside the WCA -5..+5 range");
      assert.equal(amount, t.endsWith("+") ? shown : (12 - shown) % 12);
    }
  }
  const h = K.createGame({ seed: "shape", scrambleLen: 6 });
  assert.deepEqual(g.scramble, h.scramble, "same seed must replay the same scramble");
  const i = K.createGame({ seed: "shape-2", scrambleLen: 6 });
  assert.notDeepEqual(g.scramble, i.scramble);
  assert.equal(K.normalizeConfig({}).scrambleLen, 6);
  assert.equal(K.normalizeConfig({ scrambleLen: 99 }).scrambleLen, 9);
  assert.equal(K.normalizeConfig({ scrambleLen: 0 }).scrambleLen, 1);
  assert.equal(K.clockStamp(K.normalizeConfig({})), K.clockStamp(K.normalizeConfig({})));
  assert.notEqual(K.clockStamp(K.normalizeConfig({ seed: "a" })),
    K.clockStamp(K.normalizeConfig({ seed: "b" })));
  assert.equal(K.createGame({ seed: "s" }).solvedStart, false);
});

test('solver refuses unreachable states with a named error; solved input needs nothing', () => {
  const broken = randomWalkState(5).slice();
  broken[K.CORNER_PAIRS.DL[0]] = (broken[K.CORNER_PAIRS.DL[0]] + 5) % 12;
  assert.throws(() => K.solve(broken), /validation|corner-pair/,
    "pair-broken state must refuse");
  assert.throws(() => K.solve([1, 2, 3]), /validation|18 dial/,
    "malformed state must refuse");
  const r = K.solve(SOLVED.slice());
  assert.deepEqual(r.moves, []);
  assert.deepEqual(r.final, SOLVED.slice());
  /* a single wheel move closes in one token */
  const r1 = K.solve(K.applyMove(SOLVED.slice(), "UR3+"));
  assert.equal(r1.verified.solved, true);
  assert.deepEqual(r1.moves, ["UR3-"]);
});

test('exact amounts: the raw algebra solution closes the state directly', () => {
  const flat = K.SOLVE_PLAN.flatMap((p) => p.names);
  for (let i = 0; i < 25; i++) {
    const s = randState();
    const amounts = K.solveAmounts(s);
    let t = s.slice();
    for (let j = 0; j < amounts.length; j++) {
      /* back-section entries count in the flipped frame; folding the y2
         pair in and out makes the raw amounts replayable in one frame */
      const name = flat[j];
      const flipped = j >= 5 && j < 10;
      if (flipped) t = K.applyMove(t, "y2");
      t = K.applyVec(t, K.MOVE_VEC[name], amounts[j]);
      if (flipped) t = K.applyMove(t, "y2");
    }
    assert.equal(K.verifySolved(t).solved, true, "raw amounts did not close walk " + i);
  }
});
