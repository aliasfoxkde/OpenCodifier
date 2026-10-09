/* Pong core suite — the deterministic contract the shell and the scoreboard
   rely on. Same shape as the maze core suite: import the SDK first (the
   core delegates rng/clamping to it), then pong-core.js, read the global.

   Run: node --test tests/pong_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'pong-core.js'));
const PC = globalThis.PongCore;

const W = { pursue: 3, bounce: 1, center: 1, taunt: 0, risk: 1 };
const TAUNT = { pursue: 0, bounce: 0, center: 0, taunt: 5, risk: 0 };

test('config and reward normalization clamps hostile input', () => {
  const c = PC.normalizeConfig({
    seed: 42, ballSpeed: 9999, paddleH: -3, paddleSpeed: 1e9,
    winScore: 0, reactEvery: 'nope', noise: NaN,
  });
  assert.equal(c.seed, '42');
  assert.equal(c.ballSpeed, 80);
  assert.equal(c.paddleH, 5);
  assert.equal(c.paddleSpeed, 60);
  assert.equal(c.winScore, 1);
  assert.equal(c.reactEvery, 6);
  assert.equal(c.noise, 0.12, 'NaN noise falls back to the default');

  const w = PC.normalizeRewards({ pursue: 99, bounce: -5, center: 'x', taunt: 2.50001 });
  assert.deepEqual(w.weights, {
    pursue: 5, bounce: 0, center: 1, taunt: 2.5, risk: 1,
  });
  /* the flat shape (no .weights wrapper) normalizes too */
  const flat = PC.normalizeRewards({ pursue: 4 });
  assert.equal(flat.weights.pursue, 4);
  assert.equal(flat.weights.bounce, 1);
});

test('the match stamp separates weights and seed — verdicts compare like with like', () => {
  const base = {};
  const a = PC.matchStamp(base, W, W);
  assert.equal(a, PC.matchStamp(base, W, W), 'identical setups stamp identically');
  assert.notEqual(a, PC.matchStamp(base, W, TAUNT), 'right weights are in the stamp');
  assert.notEqual(a, PC.matchStamp(base, TAUNT, W), 'left weights are in the stamp');
  assert.notEqual(a, PC.matchStamp({ seed: 'other' }, W, W), 'seed is in the stamp');
});

test('bot-vs-bot matches finish deterministically', () => {
  for (const seed of ['alpha', 'bravo', 'charlie', 'delta', 'echo']) {
    const cfg = { seed };
    const one = PC.simulateMatch(cfg, W, W);
    const two = PC.simulateMatch(cfg, W, W);
    assert.deepEqual(PC.matchSummary(one), PC.matchSummary(two), seed);
    assert.ok(one.over, seed + ' finishes');
    assert.ok(['left', 'right'].includes(one.winner), seed + ' has a winner');
    const winScore = one.cfg.winScore;
    const top = Math.max(one.paddles.left.score, one.paddles.right.score);
    assert.equal(top, winScore, seed + ' ends on the win score');
  }
});

test('rewards change behavior: a taunt paddle shadows its opponent', () => {
  /* same seed, same everything but the right paddle's weights: with
     taunt=5 the right paddle hugs the left paddle's y much more closely */
  function meanGap(weights) {
    const m = PC.createMatch({ seed: 'shadow' }, W, weights);
    PC.serve(m);
    let sum = 0, n = 0;
    while (!m.over && m.ticks < 5400) {
      PC.tickMatch(m, {});
      if (m.ticks % 10 === 0 && m.ticks > 120) {
        sum += Math.abs(m.paddles.right.y - m.paddles.left.y);
        n += 1;
      }
    }
    return sum / n;
  }
  const gapTaunt = meanGap(TAUNT);
  const gapPlay = meanGap(W);
  assert.ok(gapTaunt < gapPlay,
    `taunt gap ${gapTaunt.toFixed(2)} should beat play gap ${gapPlay.toFixed(2)}`);
});

test('the speed cap is real: an out-of-position paddle concedes the point', () => {
  /* no randomness at all: a straight ball at 60 u/s crosses the right
     plane 44 ticks after this manual serve; the right paddle starts at 30
     and is told to hold y=5 — it covers at most 13.2 units by then, so it
     cannot be on the ball's line. The point must go to left, and every
     paddle step must honor the speed cap on the way. */
  const m = PC.createMatch({ seed: 'cap', paddleSpeed: 18 }, W, W);
  m.ball = { x: 50, y: 30, vx: 60, vy: 0 };
  let steps = 0;
  let maxY = 0;
  let prev = m.paddles.right.y;
  while (m.paddles.left.score === 0 && m.ticks < 500) {
    PC.tickMatch(m, { left: 30, right: 5 });
    const dy = Math.abs(m.paddles.right.y - prev);
    maxY = Math.max(maxY, dy);
    prev = m.paddles.right.y;
    steps += 1;
  }
  assert.equal(m.paddles.left.score, 1, 'the unreachable ball scores');
  assert.equal(m.paddles.right.misses, 1, 'and it counts as a real miss');
  assert.ok(m.ticks < 200, `point lands this side of forever (${m.ticks} ticks)`);
  assert.ok(maxY <= 18 / 60 + 1e-9, `no paddle teleporting (max step ${maxY.toFixed(4)})`);
  assert.ok(steps === m.ticks);
});

test('pickTarget follows the weights and detects genuine ties', () => {
  const state = {
    ticks: 1000,
    paddles: {
      left: { y: 30, h: 10, lastOwnHit: 900 },
      right: { y: 30, h: 10, lastOwnHit: 900 },
    },
    ball: { x: 50, y: 30, vx: -40, vy: 10 },
  };
  const cands = PC.policyCandidates(state, 'left');
  assert.equal(cands.length, 4);

  /* all weight on pursuit → the intercept candidate wins */
  const chase = PC.pickTarget(cands, { pursue: 5, bounce: 0, center: 0, taunt: 0, risk: 0 });
  assert.equal(chase.best.name, 'intercept');
  assert.equal(chase.tie, null);

  /* every weight zero → all scores equal → a genuine tie to escalate */
  const zero = PC.pickTarget(cands, { pursue: 0, bounce: 0, center: 0, taunt: 0, risk: 0 });
  assert.ok(zero.tie, 'all-zero weights must produce a tie');
  assert.equal(zero.tie.length, 2);

  /* determinism: same inputs, same argmax */
  const again = PC.pickTarget(cands, { pursue: 5, bounce: 0, center: 0, taunt: 0, risk: 0 });
  assert.equal(again.best.name, chase.best.name);
});

test('the engine tie path is honored: the choice answer picks the target', () => {
  const m = PC.createMatch(
    { seed: 'ties', reactEvery: 1, noise: 0 },
    { pursue: 0, bounce: 0, center: 0, taunt: 0, risk: 0 },
    { pursue: 0, bounce: 0, center: 0, taunt: 0, risk: 0 },
  );
  PC.serve(m);
  let calls = 0;
  let lastReq = null;
  const engineChoose = (req) => {
    calls += 1;
    lastReq = req;
    /* the measured real-engine shape: answers[0].choice + confidence,
       outcome accept (see wasm probe, FINDINGS.md) */
    return {
      outcome: 'accept',
      answers: [{ type: 'choice', question_id: 'paddle-target', choice: 'mirror', confidence: 0.8 }],
    };
  };
  while (!m.over && m.ticks < 2000) PC.tickMatch(m, { engineChoose });
  assert.ok(calls > 0, 'all-zero weights + reactEvery 1 must escalate');
  assert.ok(m.engineCalls === calls, 'the match counts engine calls');
  const q = lastReq.questions[0];
  assert.equal(q.type, 'choice', 'ties are choice questions');
  /* all-zero weights tie everything; name-order puts center and edge on top */
  assert.deepEqual(q.candidates.map((c) => c.id), ['center', 'edge']);
  assert.equal(q.candidates[0].description.length > 0, true, 'candidates carry descriptions');
});

test('an engine abstain or unknown id leaves the decision to the local formula', () => {
  for (const answer of [
    { outcome: 'abstain', answers: [] },
    { outcome: 'accept', answers: [{ type: 'choice', choice: 'nope' }] },
    null, /* bridge failure */
  ]) {
    const m = PC.createMatch(
      { seed: 'abstain', reactEvery: 1, noise: 0 },
      { pursue: 0, bounce: 0, center: 0, taunt: 0, risk: 0 },
      { pursue: 0, bounce: 0, center: 0, taunt: 0, risk: 0 },
    );
    PC.serve(m);
    let calls = 0;
    const engineChoose = () => { calls += 1; return answer; };
    while (!m.over && m.ticks < 2000) PC.tickMatch(m, { engineChoose });
    assert.ok(calls > 0, 'escalation still happens');
    assert.ok(m.over, 'match still finishes on the local formula');
  }
});

test('plane-cross hit detection: no tunneling at high speed', () => {
  /* ball screams across the paddle plane with a huge per-tick step; the
     interpolated y decides — a paddle squarely on that y MUST catch it */
  const paddles = {
    left: { y: 30, h: 10, accel: 1.0 },
    right: { y: 30, h: 10, accel: 1.0 },
  };
  const fast = { x: 50, y: 30.4, vx: -400, vy: 0 };   /* 6.7 units per tick */
  let hit = null;
  for (let i = 0; i < 20 && hit === null; i++) hit = PC.stepBall(fast, paddles);
  assert.equal(hit, 'left', 'a 400 u/s ball cannot tunnel through the paddle');

  /* just outside the face: same speed, same everything, must NOT hit */
  const paddles2 = {
    left: { y: 45, h: 10, accel: 1.0 },
    right: { y: 30, h: 10, accel: 1.0 },
  };
  const wide = { x: 50, y: 10, vx: -400, vy: 0 };
  let hit2 = null;
  for (let i = 0; i < 20 && hit2 === null; i++) hit2 = PC.stepBall(wide, paddles2);
  assert.equal(hit2, null, 'a ball outside the face is not a hit');
});

test('wall reflection keeps the ball in the field', () => {
  const b = { x: 50, y: 0.2, vx: 10, vy: -100 };
  PC.reflectWalls(b);
  assert.ok(b.y > 0 && b.y < PC.FIELD_H);
  assert.ok(b.vy > 0, 'reflection flips vy');
  const b2 = { x: 50, y: PC.FIELD_H - 0.2, vx: 10, vy: 100 };
  PC.reflectWalls(b2);
  assert.ok(b2.y > 0 && b2.y < PC.FIELD_H);
  assert.ok(b2.vy < 0);
});
