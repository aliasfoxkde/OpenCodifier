/* Rubik's Cube solver — core suite. Import order matters: the SDK first,
   then cube-core; read the globals.

   Spec-pinned tests (playground plan §5.16): the LBL solver solves every
   seeded scramble to a verified solved state with legal moves; the pinned
   LL constants do by simulation exactly what their comments claim (pure
   edge 3-cycles, pure double swaps, corner transpositions); genuine ties
   reach the choose callback and a refusal falls back to roster order;
   same seed → same moves.

   Run: node --test tests/cube_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'cube-core.js'));
const C = globalThis.CubeCore;

const apply = (state, toks) => {
  let s = state;
  for (const t of toks) s = C.applyMove(s, t);
  return s;
};
const inv = (toks) => toks.slice().reverse().map((t) =>
  t.endsWith("2") ? t : t.endsWith("'") ? t[0] : t[0] + "'");

/* slot helpers: a U-layer slot is home iff every facelet matches SOLVED */
const U_CORNER_IDS = ["FRU", "FLU", "BLU", "BRU"];
const U_EDGE_IDS = ["FU", "RU", "BU", "LU"];
const slotHome = (s, id) => {
  const sl = [...C.CORNER_SLOTS, ...C.EDGE_SLOTS].find((x) => x.id === id);
  return sl.f.every((i, j) => s[i] === C.SOLVED[sl.f[j]]);
};
const uCornersHome = (s) => U_CORNER_IDS.filter((id) => slotHome(s, id)).length;
const uEdgesHome = (s) => U_EDGE_IDS.filter((id) => slotHome(s, id)).length;
/* F2L = everything below the last layer is home */
const f2lHome = (s) => [...C.CORNER_SLOTS, ...C.EDGE_SLOTS]
  .filter((sl) => !sl.id.includes("U"))
  .every((sl) => sl.f.every((i, j) => s[i] === C.SOLVED[sl.f[j]]));
const fromSolved = (toks) => apply(C.SOLVED.slice(), toks);

test('solved state verifies; a scrambled one does not', () => {
  assert.equal(C.verifySolved(C.SOLVED.slice()).solved, true);
  const g = C.createGame({ seed: "spec", scrambleLen: 20 });
  assert.equal(C.verifySolved(g.state).solved, false);
  assert.equal(C.validateState(g.state).ok, true);
});

test('pinned LL constants: edge algs are F2L- and corner-pure', () => {
  /* Ua/Ub are pure edge 3-cycles; H a pure double swap — the corners AND
     the first two layers must come back untouched */
  for (const [name, alg] of [["Ua", C.LL.uPerm], ["Ub", C.U_PERM_INV],
    ["H", C.H_PERM]]) {
    const s = fromSolved(alg);
    assert.equal(f2lHome(s), true, name + " disturbs F2L");
    assert.equal(uCornersHome(s), 4, name + " moves corners");
    assert.ok(uEdgesHome(s) < 4, name + " should move edges");
  }
  /* the pair composes to identity (Ub is Ua's inverse) */
  assert.equal(C.verifySolved(fromSolved([...C.LL.uPerm, ...C.U_PERM_INV])).solved, true);
});

test('pinned LL constants: T/Y perms swap a corner pair (and an edge pair)', () => {
  for (const [name, alg] of [["T", C.LL.tPerm], ["Y", C.LL.yPerm]]) {
    const s = fromSolved(alg);
    assert.equal(f2lHome(s), true, name + " disturbs F2L");
    assert.equal(uCornersHome(s), 2, name + " should transpose two corners");
    assert.equal(uEdgesHome(s), 2, name + " should transpose two edges");
  }
});

test('pinned LL constants: A-perm cycles corners, leaves edges', () => {
  const s = fromSolved(C.LL.aPerm);
  assert.equal(f2lHome(s), true);
  assert.equal(uEdgesHome(s), 4, "A-perm must leave edges alone");
  assert.equal(uCornersHome(s), 1, "A-perm is a corner 3-cycle");
  /* the inverse really is the inverse */
  assert.equal(C.verifySolved(fromSolved([...C.LL.aPerm, ...inv(C.LL.aPerm)])).solved, true);
});

test('pinned LL constants: sune/anti-sune are inverses, EO algs are inverses', () => {
  assert.equal(C.verifySolved(fromSolved([...C.LL.sune, ...C.LL.antiSune])).solved, true);
  assert.equal(C.verifySolved(fromSolved([...C.LL.eo, ...C.LL.eoL])).solved, true,
    "eoL is eo's inverse (line->cross and L->cross)");
});

test('seeded solves: every solve ends verified, legal, and phase-ordered', () => {
  const PHASES = ["cross", "corners", "middle", "oll-edges", "oll-corners",
    "pll-corners", "pll-edges", "auf"];
  for (let k = 0; k < 30; k++) {
    const g = C.createGame({ seed: "spec-" + k, scrambleLen: 20 });
    const r = C.solveLBL(g.state, undefined);
    assert.equal(r.verified.solved, true, "seed " + k + " not solved");
    assert.equal(r.legal, true, "seed " + k + " illegal state");
    assert.deepEqual(apply(g.state, r.moves), r.final, "seed " + k + " replay drift");
    /* phases appear in method order (skippable, never out of order) */
    let hi = 0;
    for (const st of r.steps) {
      const pi = PHASES.indexOf(st.phase);
      assert.ok(pi >= hi, "seed " + k + " phase regression at " + st.phase);
      hi = pi;
    }
  }
});

test('choose: engine picks the last option; refusal falls back to roster', () => {
  const g = C.createGame({ seed: "spec-ties", scrambleLen: 20 });
  const asked = [];
  const last = C.solveLBL(g.state, (q) => { asked.push(q.phase); return q.options.length - 1; });
  assert.equal(last.verified.solved, true);
  assert.ok(asked.length > 0, "no ties reached the engine");
  for (const t of last.ties) {
    if (t.source === "asked") assert.equal(t.chosen, t.options.length - 1);
  }
  const ref = C.solveLBL(g.state, () => null);
  assert.equal(ref.verified.solved, true, "refusal must not derail the solve");
  assert.ok(ref.ties.every((t) => t.source !== "asked"));
  /* determinism across callback styles: same moves when nobody chooses */
  const plain = C.solveLBL(g.state, undefined);
  assert.deepEqual(plain.moves, ref.moves);
});

test('determinism: same seed, same moves; different seed, different scramble', () => {
  const a = C.solveLBL(C.createGame({ seed: "det", scrambleLen: 20 }).state, undefined);
  const b = C.solveLBL(C.createGame({ seed: "det", scrambleLen: 20 }).state, undefined);
  assert.deepEqual(a.moves, b.moves);
  const c = C.createGame({ seed: "det-2", scrambleLen: 20 });
  const d = C.createGame({ seed: "det-3", scrambleLen: 20 });
  assert.notDeepEqual(c.scramble, d.scramble);
});

test('config: scrambleLen clamps to 1..40; a solved input solves in zero moves', () => {
  assert.equal(C.normalizeConfig({ scrambleLen: 999 }).scrambleLen, 40);
  assert.equal(C.normalizeConfig({ scrambleLen: 0 }).scrambleLen, 1);
  const g = C.createGame({ seed: "spec-short", scrambleLen: 1 });
  const r = C.solveLBL(g.state, undefined);
  assert.equal(r.verified.solved, true);
  const r0 = C.solveLBL(C.SOLVED.slice(), undefined);
  assert.equal(r0.moves.length, 0);
  assert.equal(C.cubeStamp(C.normalizeConfig({})), C.cubeStamp(C.normalizeConfig({})));
});

test('cubeType shape mods: accepted values pin, algebra and stamp unchanged', () => {
  const base = { seed: 'mod', scrambleLen: 5 };
  assert.equal(C.normalizeConfig({ ...base, cubeType: 'mirror' }).cubeType, 'mirror');
  assert.equal(C.normalizeConfig({ ...base, cubeType: 'ghost' }).cubeType, 'ghost');
  assert.equal(C.normalizeConfig({ ...base, cubeType: 'bogus' }).cubeType, 'classic');
  assert.equal(C.normalizeConfig(base).cubeType, 'classic');
  // a shape mod is geometry only: same stamp, same scramble, same solve plan
  const g1 = C.createGame(C.normalizeConfig(base));
  const g2 = C.createGame(C.normalizeConfig({ ...base, cubeType: 'mirror' }));
  assert.equal(C.cubeStamp(C.normalizeConfig(base)), C.cubeStamp(C.normalizeConfig({ ...base, cubeType: 'ghost' })));
  assert.deepEqual(g1.scramble, g2.scramble, 'same seed scrambles identically under any mod');
});
