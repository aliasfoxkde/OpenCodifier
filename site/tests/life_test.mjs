/* Life core suite — the deterministic contract the sandbox relies on.
   Same shape as the pong suite: import the SDK first (the core delegates
   rng/clamping/hashing to it), then life-core.js, read the global.

   Run: node --test tests/life_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'life-core.js'));
const LC = globalThis.LifeCore;

const hist = (n) => Array.from({ length: 13 }, () => n);

function run(cfg, pat, gens) {
  const w = LC.createWorld(cfg);
  if (pat) {
    LC.stampPattern(w, LC.PATTERNS.find((p) => p.name === pat),
      cfg.cols >> 2, cfg.rows >> 2);
  }
  for (let i = 0; i < gens; i++) LC.step(w);
  return w;
}

test('rule strings parse, dedupe, and fall back to Life on hostile input', () => {
  assert.equal(LC.normalizeRule('B3/S23').label, 'B3/S23');
  assert.equal(LC.normalizeRule('3/23').label, 'B3/S23');
  assert.equal(LC.normalizeRule('b36/s23').label, 'B36/S23');
  assert.equal(LC.normalizeRule('B2/S').label, 'B2/S');
  assert.equal(LC.normalizeRule('S/B3').label, 'B3/S23', 'swapped sides is not a rule');
  assert.equal(LC.normalizeRule('').label, 'B3/S23');
  assert.equal(LC.normalizeRule('garbage').label, 'B3/S23');
  assert.equal(LC.normalizeRule(null).label, 'B3/S23');
  assert.equal(LC.normalizeRule('B99/S23').label, 'B3/S23', '9 is not a neighbor count');
  assert.equal(LC.normalizeRule('B33/S3').label, 'B3/S3', 'duplicate digits dedupe');
  /* presets carry canonical strings the shell can feed back in */
  const values = Object.values(LC.PRESETS)
    .map((p) => 'B' + p.born.join('') + '/S' + p.survive.join(''));
  for (const v of values) assert.equal(LC.normalizeRule(v).label, v);
});

test('the classic patterns classify as themselves', () => {
  const block = run({ cols: 20, rows: 20 }, 'block', 4);
  assert.deepEqual(LC.classify(LC.features(block, hist(block.pop))),
    { label: 'still', conf: 0.97 });

  const blinker = run({ cols: 20, rows: 20, wrap: true }, 'blinker', 6);
  const fb = LC.features(blinker, hist(blinker.pop));
  assert.equal(fb.period, 2);
  assert.equal(LC.classify(fb).label, 'oscillator');

  const pulsar = run({ cols: 30, rows: 30 }, 'pulsar', 8);
  const fp = LC.features(pulsar, hist(pulsar.pop));
  assert.equal(fp.period, 3, 'the shipped pulsar art must actually pulse');
  assert.equal(LC.classify(fp).label, 'oscillator');
});

test('a glider is a spaceship on a torus and ash against a wall', () => {
  const flown = run({ cols: 40, rows: 40, wrap: true }, 'glider', 12);
  const ff = LC.features(flown, hist(flown.pop));
  assert.equal(ff.period, 4, 'glider period');
  assert.equal(LC.classify(ff).label, 'spaceship');

  const crashed = run({ cols: 16, rows: 16 }, 'glider', 60);
  const fc = LC.features(crashed, hist(crashed.pop));
  assert.equal(LC.classify(fc).label, 'still', 'the ash re-reads as still');
  assert.equal(crashed.pop, 4, 'a block of ash is left');
});

test('wrap and bounded boards genuinely diverge once a wall is involved', () => {
  const a = run({ cols: 16, rows: 16, wrap: true }, 'glider', 60);
  const b = run({ cols: 16, rows: 16, wrap: false }, 'glider', 60);
  assert.notDeepEqual(Array.from(a.cells), Array.from(b.cells));
});

test('empty boards read extinct; trend branches fire on the features', () => {
  const e = run({ cols: 20, rows: 20 }, null, 4);
  assert.equal(LC.classify(LC.features(e, hist(0))).label, 'extinct');

  /* classify speaks features directly — hand-build them for the branches
     a fixed pattern run can't reach on demand */
  const base = { gen: 30, pop: 60, births: 9, deaths: 1, activity: 20,
    trend: 3, period: 0, drift: 0, density: 0.1 };
  assert.equal(LC.classify({ ...base }).label, 'exploding', 'trend > 1.6');
  assert.equal(LC.classify({ ...base, pop: 6, trend: 0.3 }).label, 'dying', 'trend < 0.5');
  assert.equal(LC.classify({ ...base, pop: 40, trend: 1 }).label, 'mixed');
  /* a period-2 cycle WITH drift is a ship, without drift an oscillator */
  assert.equal(LC.classify({ ...base, pop: 4, period: 4, drift: 1.41 }).label, 'spaceship');
  assert.equal(LC.classify({ ...base, pop: 4, period: 4, drift: 0 }).label, 'oscillator');
});

test('soups are deterministic per seed and statistics are sane', () => {
  const a = LC.createWorld({ cols: 40, rows: 30, seed: 'soup-a' });
  const b = LC.createWorld({ cols: 40, rows: 30, seed: 'soup-a' });
  const c = LC.createWorld({ cols: 40, rows: 30, seed: 'soup-b' });
  LC.randomSoup(a, 0.3);
  LC.randomSoup(b, 0.3);
  LC.randomSoup(c, 0.3);
  assert.deepEqual(Array.from(a.cells), Array.from(b.cells), 'same seed, same soup');
  assert.notDeepEqual(Array.from(a.cells), Array.from(c.cells), 'seeds differ');
  assert.ok(a.pop > 100 && a.pop < 40 * 30 * 0.5, 'density ~0.3 landed');
});

test('seed proposals are deterministic, scored, and tie-breakable', () => {
  const cfg = { cols: 48, rows: 32, seed: 'prop' };
  const one = LC.seedCandidates(cfg, 3, 'salt');
  const two = LC.seedCandidates(cfg, 3, 'salt');
  assert.deepEqual(one.map((s) => [s.id, s.interest]), two.map((s) => [s.id, s.interest]));
  assert.equal(one.length, 3);
  for (const s of one) {
    assert.ok(s.interest >= 0 && s.interest <= 1, 'interest is a unit score');
    assert.ok(s.cells.length > 0, 'a proposal has cells');
    assert.ok(s.profile.gens > 0, 'the probe actually ran');
  }
  assert.equal(one[0].interest >= one[2].interest, true, 'sorted by interest');

  /* pickSeed: clear winner vs genuine tie */
  const win = LC.pickSeed([{ id: 'a', interest: 0.9 }, { id: 'b', interest: 0.4 }], 0.02);
  assert.equal(win.best.id, 'a');
  assert.equal(win.tie, null);
  const tie = LC.pickSeed([{ id: 'a', interest: 0.5 }, { id: 'b', interest: 0.5 }], 0.02);
  assert.ok(tie.tie, 'equal scores are a genuine tie');
  assert.deepEqual(tie.tie.map((s) => s.id), ['a', 'b']);
});

test('engine verdicts are honored, and abstention falls back cleanly', () => {
  assert.deepEqual(
    LC.applyEngineAnswer({ outcome: 'accept', answers: [{ choice: 'spaceship', confidence: 0.8 }] }),
    { label: 'spaceship', conf: 0.8 });
  assert.equal(LC.applyEngineAnswer({ outcome: 'abstain', answers: [] }), null);
  assert.equal(LC.applyEngineAnswer({ outcome: 'accept', answers: [{ choice: 'nope' }] }), null,
    'unknown labels are refused');
  assert.equal(LC.applyEngineAnswer(null), null);

  const req = LC.engineRequest({
    gen: 100, pop: 5, births: 1, deaths: 1, activity: 4,
    trend: 1, period: 4, drift: 1.41, density: 0.01,
  });
  assert.equal(req.questions[0].type, 'choice');
  assert.ok(req.questions[0].candidates.some((c) => c.id === 'spaceship'));
  assert.ok(req.questions[0].candidates.every((c) => c.description.length > 0));
});

test('experiments stamp like-for-like: dims, wrap, rule, seed', () => {
  const base = { cols: 96, rows: 56, wrap: false, rule: 'B3/S23', seed: 'x' };
  const a = LC.experimentStamp(base);
  assert.equal(a, LC.experimentStamp({ ...base }));
  assert.notEqual(a, LC.experimentStamp({ ...base, wrap: true }));
  assert.notEqual(a, LC.experimentStamp({ ...base, rule: 'B36/S23' }));
  assert.notEqual(a, LC.experimentStamp({ ...base, seed: 'y' }));
  assert.notEqual(a, LC.experimentStamp({ ...base, cols: 64, rows: 40 }));
});

test('resetCycle forgets a detected cycle, which re-locks in two steps', () => {
  const w = run({ cols: 20, rows: 20, wrap: true }, 'blinker', 6);
  assert.equal(w.period, 2);
  LC.resetCycle(w);
  assert.equal(w.period, 0);
  LC.step(w);
  assert.equal(w.period, 0, 'nothing to compare against on the first step back');
  LC.step(w);
  assert.equal(w.period, 0, 'the other phase is also unknown after the wipe');
  LC.step(w);
  assert.equal(w.period, 2, 'the cycle re-locks on the third');
});

test('the pattern library is sane: cells stamp, pop matches', () => {
  for (const p of LC.PATTERNS) {
    const w = LC.createWorld({ cols: 64, rows: 48, seed: 'lib' });
    LC.stampPattern(w, p, 8, 8);
    assert.equal(w.pop, p.cells.length, p.name + ' stamps all its cells');
    assert.ok(p.cells.length >= 3, p.name + ' has a pattern');
  }
});
