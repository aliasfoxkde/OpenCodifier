/* Sparring Partner core suite — the deterministic contract the shell relies
   on. Import order matters: the SDK first, then fighter-core; read the
   globals.

   Run: node --test tests/fighter_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'fighter-core.js'));
const FC = globalThis.FighterCore;

test('config normalization clamps hostile input', () => {
  const c = FC.normalizeConfig({
    seed: 42, adaptivity: 9, aggression: -3, difficulty: 'no',
    rounds: 4, botPlayer: 'yes',
  });
  assert.equal(c.seed, '42');
  assert.equal(c.adaptivity, 0.95);
  assert.equal(c.aggression, 0);
  assert.equal(c.difficulty, 2, 'hostile difficulty falls back');
  assert.equal(c.rounds, 5, 'even round counts snap to the odd neighbour');
  assert.equal(c.botPlayer, false, 'only === true counts as bot');
});

test('the same seed and script replay identically; a new seed diverges', () => {
  const script = (i) => (i % 40 < 20 ? { walk: 1 } : i % 80 < 60 ? { jab: true } : { block: true });
  const run = (seed) => {
    const g = FC.createGame({ seed, rounds: 1, difficulty: 3 });
    while (!g.over && g.ticks < 900) FC.tickGame(g, script(g.ticks), null);
    return FC.runSummary(g);
  };
  assert.deepEqual(run('echo'), run('echo'), 'same seed, same fight');
  assert.notDeepEqual(run('echo'), run('foxtrot'), 'a new seed fights differently');
});

test('attacks run startup, active, recovery — mid-recovery inputs drop', () => {
  const g = FC.createGame({ seed: 'frames', difficulty: 5 });
  g.reaction = 9999;                                /* keep zed out of the way */
  const adv = (n, input) => { for (let i = 0; i < n; i++) FC.tickGame(g, input || null, null); };
  g.you.x = 55; g.zed.x = 68;                       /* inside jab range */
  FC.tickGame(g, { jab: true }, null);              /* commit tick: startup f0 */
  adv(4);
  assert.equal(g.you.state, 'active', '4 startup frames, then the swing is live');
  adv(3);
  assert.equal(g.you.state, 'recovery', '3 active frames, then recovery');
  FC.tickGame(g, { jab: true }, null);
  assert.equal(g.you.state, 'recovery', 'no re-commit during recovery');
  adv(4);
  assert.equal(g.you.state, 'recovery', '6 recovery frames run their course');
  FC.tickGame(g, { jab: true }, null);
  assert.equal(g.you.state, 'idle', 'recovery ends idle — the input was spent');
  FC.tickGame(g, { jab: true }, null);
  assert.equal(g.you.state, 'startup', 'a fresh jab only after idle');
});

test('a close jab lands for 6 and meters up; a far one whiffs', () => {
  const near = FC.createGame({ seed: 'poke', difficulty: 5 });
  near.you.x = 55; near.zed.x = 68;
  FC.tickGame(near, { jab: true }, null);
  for (let i = 0; i < 5; i++) FC.tickGame(near, null, null);
  assert.equal(near.zed.health, 94, 'jab pays 6');
  assert.equal(near.you.hits, 1);
  assert.ok(near.you.meter > 3 && near.zed.meter > 2, 'both meters fed');
  const far = FC.createGame({ seed: 'poke', difficulty: 5 });
  FC.tickGame(far, { jab: true }, null);
  for (let i = 0; i < 5; i++) FC.tickGame(far, null, null);
  assert.equal(far.zed.health, 100, 'out of range is a whiff');
  assert.equal(far.you.hits, 0);
  assert.equal(far.you.meter, 0, 'whiffs feed nobody');
});

test('duck slips under hook and special but not jab; block chips to 20%', () => {
  const strike = (setup, move) => {
    const g = FC.createGame({ seed: 'guard', difficulty: 5 });
    g.reaction = 9999;                              /* zed holds whatever we pin */
    g.you.x = 55; g.zed.x = 68;                     /* inside every attack range */
    setup(g);
    FC.tickGame(g, { [move]: true }, null);
    for (let i = 0; i < 25; i++) FC.tickGame(g, null, null);
    return g;
  };
  const pin = (state) => (g) => {
    FC.commit(g.zed, state);
    g.zed.holdUntil = g.ticks + 999;
  };
  const ducked = strike(pin('duck'), 'hook');
  assert.equal(ducked.zed.health, 100, 'high swings pass over a duck');
  const jabbed = strike(pin('duck'), 'jab');
  assert.equal(jabbed.zed.health, 94, 'jabs hit low');
  const blocked = strike(pin('block'), 'hook');
  assert.equal(blocked.zed.health, 97, 'block chips 14 to 3');
  assert.equal(blocked.you.dealt, 3);
  assert.ok(blocked.zed.meter > 0, 'even blocked damage feeds the meter');
});

test('special needs a full meter, drains it, and pays 24', () => {
  const g = FC.createGame({ seed: 'meter', difficulty: 5 });
  g.reaction = 9999;                                /* an idle zed eats it clean */
  g.you.x = 50; g.zed.x = 68;
  g.you.meter = 99;
  assert.equal(FC.commit(g.you, 'special'), false, 'short meter refuses');
  g.you.meter = 100;
  assert.equal(FC.commit(g.you, 'special'), true);
  assert.equal(g.you.meter, 0, 'the meter drains on commit');
  for (let i = 0; i < 22; i++) FC.tickGame(g, null, null);
  assert.equal(g.zed.health, 76, 'special pays 24');
});

test('a jab spammer is read and countered; a hook spammer is ducked', () => {
  const lesson = (move, input) => {
    const g = FC.createGame({ seed: 'lesson', difficulty: 5, rounds: 1 });
    g.you.x = 60; g.zed.x = 68;
    let lastIdle = null;      /* zed's reads BETWEEN the spammer's moves */
    let seen = 0;
    for (let i = 0; i < 2400 && !g.over; i++) {     /* hook cycles 25 ticks */
      if (g.zed.health < 40) g.zed.health = 100;    /* keep the lesson going */
      if (g.you.health < 40) g.you.health = 100;
      FC.tickGame(g, input, null);
      if (g.lastDecision && g.lastDecision.sit === 'close×idle' && g.ticks > seen) {
        seen = g.ticks;
        lastIdle = g.lastDecision;
      }
    }
    assert.ok(g.habits.observations > 60, move + ' spam was recorded: ' + g.habits.observations);
    const probs = FC.habitProbs(g, 'close×idle');
    for (const k of FC.ANSWERS) {
      if (k === move) continue;
      assert.ok(probs[move] > probs[k],
        move + ' predicted over ' + k + ` (${probs[move]} vs ${probs[k]})`);
    }
    assert.ok(lastIdle, 'zed read the idle situation along the way');
    assert.ok(lastIdle.predicted.startsWith(move),
      'the read is on the record: ' + lastIdle.predicted);
    return lastIdle.pick;
  };
  assert.equal(lesson('jab', { jab: true }), 'jab',
    'vs jab spam zed jabs back (trading) — duck would eat the pokes');
  assert.equal(lesson('hook', { hook: true }), 'duck',
    'vs hook spam zed slips under and punishes');
});

test('a near-tie escalates with the habit table in state; the answer is honored', () => {
  const g = FC.createGame({ seed: 'wire', difficulty: 5 });
  g.you.x = 60; g.zed.x = 68;
  /* find a habit count vector whose top two EVs land inside the near-tie
     window — the same primitives decideResponse aggregates */
  const evs = (counts) => {
    g.habits.sit['close×idle'] = counts;
    const probs = FC.habitProbs(g, 'close×idle');
    const d = FC.dist(g);
    const rows = [];
    for (const id of FC.ANSWERS) {
      if (id === 'special') continue;             /* zed's meter is empty */
      let ev = 0;
      for (const p of FC.ANSWERS) ev += probs[p] * FC.VALUE[id][p];
      if (id === 'jab' || id === 'hook') {
        ev += g.cfg.aggression * 200;             /* the flat attack bonus */
        if (d > FC.MOVES[id].range) ev -= 500;    /* the whiff risk */
      }
      rows.push([id, ev]);
    }
    rows.sort((a, b) => b[1] - a[1]);
    return rows;
  };
  let counts = null;
  outer:
  for (let a = 0; a <= 4; a++) for (let b = 0; b <= 4; b++)
  for (let c = 0; c <= 4; c++) for (let dd = 0; dd <= 4; dd++)
  for (let e = 0; e <= 4; e++) {
    const rows = evs({ jab: a, hook: b, block: c, duck: dd, dash: e, special: 0 });
    const gap = rows[0][1] - rows[1][1];
    if (gap > 0 && gap <= FC.TIE_EPS) { counts = { jab: a, hook: b, block: c, duck: dd, dash: e, special: 0 }; break outer; }
  }
  assert.ok(counts, 'an exact near-tie exists in the count grid');
  let req = null;
  let calls = 0;
  const res = FC.decideResponse(g, (r) => {
    calls += 1; req = r;
    return { outcome: 'accept', answers: [{ choice: r.questions[0].candidates[0].id, confidence: 0.8 }] };
  });
  assert.equal(calls, 1, 'one escalation');
  assert.equal(res.rung, 'engine', 'the engine answer is credited');
  assert.equal(g.rungs.engine, 1);
  assert.equal(res.pick, req.questions[0].candidates[0].id);
  /* the full wire contract — a bare {questions} request throws in the WASM */
  assert.match(req.state.text, /Habit read/);
  assert.match(req.state.text, /close×idle/, 'the situation names the habit row');
  assert.deepEqual(req.state.facts, {}, 'facts is the tagged-enum empty object');
  assert.equal(req.questions[0].type, 'choice');
  assert.equal(req.questions[0].id, 'fighter-response');
  assert.ok(req.questions[0].candidates.length >= 2);
  assert.ok(req.questions[0].candidates.every((c) => c.description.length > 5));
  assert.equal(req.policy.risk, 'low');
  assert.equal(g.lastDecision.rung, 'engine');
});

test('an abstain, unknown id, or null falls back to the habit argmax', () => {
  for (const answer of [
    { outcome: 'abstain', answers: [] },
    { outcome: 'accept', answers: [{ choice: 'flying-leg-scissors' }] },
    null,
  ]) {
    const g = FC.createGame({ seed: 'fallback', difficulty: 5 });
    g.you.x = 60; g.zed.x = 68;
    /* any counts work — with an engine that never answers, the top EV wins
       regardless of the near-tie window */
    g.habits.sit['close×idle'] = { jab: 2, hook: 0, block: 1, duck: 0, dash: 0, special: 0 };
    const before = FC.runSummary(g).rungs;
    const res = FC.decideResponse(g, () => answer);
    assert.equal(res.rung, 'habit', 'labelled habit, never engine: ' + JSON.stringify(answer));
    assert.equal(g.rungs.engine, 0);
    assert.ok(g.rungs.habit >= 1);
    assert.equal(g.rungs.engine, before.engine);
  }
});

test('rounds: KO takes a round, the match ends at the majority, the clock decides', () => {
  const g = FC.createGame({ seed: 'rounds', rounds: 3, difficulty: 5 });
  g.you.x = 60; g.zed.x = 68;
  g.zed.health = 1;
  /* capture at the KO itself — the interlude drains on and round increments */
  let saw = null;
  while (!g.over && !saw) {
    FC.tickGame(g, { jab: true }, null);
    if (g.roundResult) saw = { ...g.roundResult, interlude: g.interlude, wins: g.you.wins };
  }
  assert.deepEqual(saw, { winner: 'you', how: 'ko', interlude: FC.INTERLUDE_TICKS, wins: 1 });
  while (!g.over && g.round === 1) FC.tickGame(g, null, null);
  assert.equal(g.round, 2);
  assert.equal(g.zed.health, 100, 'fresh health each round');
  assert.equal(g.you.x, 32, 'positions reset');
  /* finish the match: two more KOs */
  for (let r = 2; r <= 3 && !g.over; r++) {
    while (g.round !== r && !g.over) FC.tickGame(g, null, null);
    g.zed.health = 1; g.you.x = 60; g.zed.x = 68;
    while (!g.over && g.round === r) FC.tickGame(g, { jab: true }, null);
  }
  assert.equal(g.over, true);
  assert.equal(g.result, 'won', 'two of three takes the match');
  /* the clock: whoever is ahead when time expires takes the round */
  const t = FC.createGame({ seed: 'clock', rounds: 1, difficulty: 5 });
  t.you.health = 90; t.zed.health = 60;
  t.roundTicks = FC.ROUND_TICKS - 1;
  FC.tickGame(t, null, null);
  assert.deepEqual(t.roundResult, { winner: 'you', how: 'time' });
  assert.equal(t.over, true, 'rounds 1: one round decides the match');
});

test('the ring has walls and the fighters never overlap or swap sides', () => {
  const g = FC.createGame({ seed: 'ring', difficulty: 5 });
  g.you.x = 62; g.zed.x = 65;                    /* closer than MIN_GAP */
  for (let i = 0; i < 30; i++) FC.tickGame(g, { walk: 1 }, null);
  assert.ok(g.zed.x - g.you.x >= FC.MIN_GAP - 1e-9, 'pushed apart, never merged');
  assert.ok(g.you.x < g.zed.x, 'sides never swap');
  const walled = FC.createGame({ seed: 'ring', difficulty: 5 });
  for (let i = 0; i < 120; i++) FC.tickGame(walled, { walk: -1 }, null);
  assert.ok(walled.you.x >= 4 - 1e-9, 'the left wall holds');
});

test('resetLearning erases the table the button shows', () => {
  const g = FC.createGame({ seed: 'audit', difficulty: 5 });
  g.you.x = 60; g.zed.x = 68;
  for (let i = 0; i < 200 && !g.over; i++) {
    if (g.zed.health < 30) g.zed.health = 100;
    FC.tickGame(g, { jab: true }, null);
  }
  assert.ok(FC.habitTable(g).length > 0, 'the table has rows to expose');
  assert.ok(g.habits.observations > 0);
  FC.resetLearning(g);
  assert.deepEqual(FC.habitTable(g), [], 'the reset wipes every row');
  assert.equal(g.habits.observations, 0);
  const probs = FC.habitProbs(g, 'close×idle');
  const vals = FC.ANSWERS.map((k) => probs[k]);
  assert.ok(vals.every((v) => v === vals[0]), 'a wiped table predicts uniform again');
});

test('bot-only matches are deterministic, finish, and stay sane', () => {
  for (const seed of ['alpha', 'bravo']) {
    const cfg = { seed, rounds: 3, difficulty: 3 };
    const one = FC.simulateMatch({ ...cfg });
    const two = FC.simulateMatch({ ...cfg });
    assert.deepEqual(FC.runSummary(one), FC.runSummary(two), seed);
    const s = FC.runSummary(one);
    assert.equal(s.over, true, seed + ' finishes inside the guard');
    assert.ok(['won', 'lost'].includes(s.result), seed + ' has a winner');
    assert.ok(s.observations > 20, seed + ' learned something: ' + s.observations);
    assert.ok(s.rungs.habit + s.rungs.engine > 5, seed + ' made real decisions');
    assert.ok(s.wins.you + s.wins.zed >= 2, seed + ' won rounds add up');
  }
});

test('the stamp separates seed, adaptivity, style, delay, and length', () => {
  const base = { seed: 's', adaptivity: 0.75, aggression: 1, difficulty: 2, rounds: 3 };
  const a = FC.fighterStamp(base);
  assert.equal(a, FC.fighterStamp({ ...base }));
  assert.notEqual(a, FC.fighterStamp({ ...base, seed: 't' }));
  assert.notEqual(a, FC.fighterStamp({ ...base, adaptivity: 0.9 }));
  assert.notEqual(a, FC.fighterStamp({ ...base, aggression: 2 }));
  assert.notEqual(a, FC.fighterStamp({ ...base, difficulty: 4 }));
  assert.notEqual(a, FC.fighterStamp({ ...base, rounds: 5 }));
});
