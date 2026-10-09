/* Apex Line core suite — the deterministic contract the shell relies on.
   Import order matters: the SDK first, then racing-core; read the globals.

   Run: node --test tests/racing_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'racing-core.js'));
const RC = globalThis.RacingCore;

test('config normalization clamps hostile input', () => {
  const c = RC.normalizeConfig({
    seed: 42, lengthSegs: 99, curviness: 7, hilliness: -3,
    laps: 0, opponents: 99, skill: 'no', traffic: NaN,
    topSpeedPct: 9, grip: 0, botPlayer: 'yes',
  });
  assert.equal(c.seed, '42');
  assert.equal(c.lengthSegs, 200);
  assert.equal(c.curviness, 1);
  assert.equal(c.hilliness, 0);
  assert.equal(c.laps, 1);
  assert.equal(c.opponents, 6);
  assert.equal(c.skill, 3, 'hostile skill falls back');
  assert.equal(c.traffic, 0.3, 'NaN falls back');
  assert.equal(c.topSpeedPct, 1.5);
  assert.equal(c.grip, 1);
  assert.equal(c.botPlayer, false, 'only === true counts as bot');
});

test('the track builds deterministically and closes its elevation loop', () => {
  const a = RC.buildTrack({ seed: 'liney' });
  const b = RC.buildTrack({ seed: 'liney' });
  assert.equal(a.segs.length, b.segs.length);
  assert.deepEqual(a.segs, b.segs, 'same seed, same track');
  const c = RC.buildTrack({ seed: 'other' });
  assert.notDeepEqual(a.segs, c.segs, 'different seed, different track');
  assert.equal(a.totalLen, a.segs.length * RC.SEG_LEN);
  /* the loop closes: easing back ends at elevation 0 */
  assert.equal(a.segs[a.segs.length - 1].y, 0);
  let maxCurve = 0;
  for (const s of a.segs) maxCurve = Math.max(maxCurve, Math.abs(s.curve));
  assert.ok(maxCurve > 0.3, 'seed liney has bends');
});

test('the standing grid staggers racers ahead of the player', () => {
  const g = RC.createGame({ seed: 'grid', opponents: 4, traffic: 0.5 });
  assert.equal(g.player.z, 0);
  assert.equal(g.player.lap, 1);
  const racers = g.cars.filter((c) => c.kind === 'racer');
  assert.equal(racers.length, 4);
  for (const r of racers) {
    assert.ok(r.z > g.player.z, 'racer starts ahead');
    assert.ok(r.topSpeed <= g.maxSpeed, 'racers do not out-top the physics');
    assert.deepEqual(Object.keys(r.weights).sort(), ['apex', 'inside', 'outside', 'slipstream']);
  }
  /* no two cars share a spawn slot */
  const slots = g.cars.map((c) => Math.round(c.z) + ':' + c.x.toFixed(1));
  assert.equal(new Set(slots).size, slots.length, 'spawn slots are distinct');
});

test('hold-accel finishes the race with one lap time per lap', () => {
  const g = RC.createGame({ seed: 'timer', laps: 2, opponents: 2, traffic: 0 });
  while (!g.over && g.ticks < 60 * 60 * 20) RC.tickGame(g, { steer: 0, accel: true }, null);
  const s = RC.runSummary(g);
  assert.equal(s.result, 'finished');
  assert.equal(s.lapTimes.length, 2, 'one lap time per lap');
  assert.ok(s.lapTimes.every((t) => t > 10), 'laps take real time');
  assert.ok(s.bestLap <= Math.min(...s.lapTimes) + 1e-9);
  assert.equal(s.over, true);
  assert.ok(s.position >= 1, 'a finish position exists');
});

test('off-road is slow: put the car in the dirt and the speed caps', () => {
  const g = RC.createGame({ seed: 'dirt', opponents: 0, traffic: 0 });
  g.player.speed = g.maxSpeed;
  g.player.x = 1.4;
  for (let i = 0; i < 30; i++) RC.tickGame(g, { steer: 0, accel: true }, null);
  assert.ok(g.player.speed <= g.maxSpeed * 0.36,
    'off-road caps at ~35%: ' + g.player.speed.toFixed(0));
});

test('running up the back of a car costs you its speed', () => {
  const g = RC.createGame({ seed: 'bump', opponents: 1, traffic: 0 });
  const other = g.cars.find((c) => c.kind === 'racer');
  other.x = g.player.x;
  other.z = g.player.z + RC.SEG_LEN;    /* one segment ahead, same line */
  other.speed = g.maxSpeed * 0.3;
  g.player.speed = g.maxSpeed;
  RC.tickGame(g, { steer: 0, accel: true }, null);
  assert.ok(g.player.speed <= other.speed * 0.95,
    'the bump sheds the difference: ' + g.player.speed.toFixed(0));
});

test('all-zero weights tie every stretch; the engine answer is honored', () => {
  let calls = 0;
  let lastReq = null;
  const g = RC.createGame({ seed: 'ties', laps: 1, opponents: 2, skill: 2 });
  for (const c of g.cars) if (c.kind === 'racer') c.weights = { apex: 0, inside: 0, outside: 0, slipstream: 0 };
  g.cfg.botPlayer = true;
  while (!g.over && g.ticks < 60 * 120) {
    RC.tickGame(g, null, (req) => {
      calls += 1;
      lastReq = req;
      return { outcome: 'accept', answers: [{ choice: req.questions[0].candidates[0].id, confidence: 0.8 }] };
    });
  }
  assert.ok(calls > 0, 'all-zero weights must escalate');
  const q = lastReq.questions[0];
  assert.equal(q.type, 'choice');
  assert.ok(['racer-line', 'player-line'].includes(q.id));
  assert.equal(q.candidates.length, 4);
  assert.ok(q.candidates.every((c) => c.description.length > 0));
  assert.ok(g.rungs.engine > 0, 'the engine rung is credited: ' + JSON.stringify(g.rungs));
  assert.ok(g.lastDecision, 'the trace has a line to show');
  /* the full wire contract — a bare {questions} request throws inside the
     WASM decide() and burns the bridge's fail limit mid-race */
  assert.ok(lastReq.state && typeof lastReq.state.text === 'string' && lastReq.state.text.length > 10,
    'state.text carries the tie story');
  assert.deepEqual(lastReq.state.facts, {}, 'facts is the tagged-enum empty object');
  assert.ok(lastReq.questions[0].text && lastReq.questions[0].text.length > 5,
    'the question has text');
  assert.equal(lastReq.policy.risk, 'low');
});

test('an abstain or unknown id falls back to the weights argmax', () => {
  for (const answer of [
    { outcome: 'abstain', answers: [] },
    { outcome: 'accept', answers: [{ choice: 'sideways' }] },
    null,
  ]) {
    const g = RC.createGame({ seed: 'abstain', laps: 1, opponents: 2 });
    for (const c of g.cars) if (c.kind === 'racer') c.weights = { apex: 0, inside: 0, outside: 0, slipstream: 0 };
    g.cfg.botPlayer = true;
    let calls = 0;
    while (!g.over && g.ticks < 60 * 60) {
      RC.tickGame(g, null, () => { calls += 1; return answer; });
    }
    assert.ok(calls > 0, 'escalation still happens');
    assert.ok(g.rungs.line > 0, 'the local argmax answered, labelled line');
    assert.equal(g.rungs.engine, 0, 'nothing credited to the engine');
  }
});

test('personalities pick different lines on the same track', () => {
  const g = RC.createGame({ seed: 'curvy', laps: 3, opponents: 0, traffic: 0, curviness: 1 });
  const hauler = RC.createGame({ seed: 'curvy', laps: 3, opponents: 0, traffic: 0, curviness: 1 });
  /* drive two games: one where the solo follower thinks like hauler, one like wall */
  const tally = { apex: 0, inside: 0, outside: 0, slipstream: 0 };
  const tally2 = { apex: 0, inside: 0, outside: 0, slipstream: 0 };
  g.cars.push({ ...g.player, kind: 'racer', name: 'hauler', color: '#ff4d4d',
    weights: { apex: 5, inside: 1, outside: 0, slipstream: 0 }, z: g.player.z, x: 0.3,
    speed: 0, topSpeed: g.maxSpeed * 0.9, lap: 1, lapStart: 0, lapTimes: [], bestLap: null,
    line: 'inside', lineAt: 0, finished: null, position: null });
  hauler.cars.push({ ...hauler.player, kind: 'racer', name: 'wall', color: '#4de3ff',
    weights: { apex: 1, inside: 0, outside: 5, slipstream: 1 }, z: hauler.player.z, x: -0.3,
    speed: 0, topSpeed: hauler.maxSpeed * 0.9, lap: 1, lapStart: 0, lapTimes: [], bestLap: null,
    line: 'inside', lineAt: 0, finished: null, position: null });
  const watch = (game, name, t) => {
    const car = game.cars.find((c) => c.name === name);
    while (!game.over && game.ticks < 60 * 60 * 6) {
      RC.tickGame(game, null, null);
      if (car.lineAt === game.ticks) t[car.line] += 1;
    }
  };
  watch(g, 'hauler', tally);
  watch(hauler, 'wall', tally2);
  const sum = (t) => t.apex + t.inside + t.outside + t.slipstream;
  assert.ok(sum(tally) > 20 && sum(tally2) > 20, 'both watchers saw decisions');
  assert.ok(tally.apex > tally2.apex, `hauler apexes more (${tally.apex} vs ${tally2.apex})`);
  assert.ok(tally2.outside > tally.outside, `wall goes wide more (${tally2.outside} vs ${tally.outside})`);
});

test('traffic circulates forever and never finishes', () => {
  const g = RC.simulateGame({ seed: 'commuters', laps: 1, opponents: 1, traffic: 1 });
  const traffic = g.cars.filter((c) => c.kind === 'traffic');
  assert.ok(traffic.length >= 8, 'traffic 1.0 spawns a real field');
  for (const t of traffic) {
    assert.equal(t.finished, null, 'traffic never finishes');
    assert.equal(t.weights, null, 'traffic never decides');
  }
  const s = RC.runSummary(g);
  assert.ok(s.finishers.every((n) => n !== 'traffic'), 'no traffic in the finishers');
  assert.ok(s.finishers.includes('you'));
});

test('bot-only races are deterministic and beatable fields stay sane', () => {
  for (const seed of ['alpha', 'bravo']) {
    const cfg = { seed, laps: 2, opponents: 4, skill: 3, traffic: 0.3 };
    const one = RC.simulateGame({ ...cfg });
    const two = RC.simulateGame({ ...cfg });
    assert.deepEqual(RC.runSummary(one), RC.runSummary(two), seed);
    const s = RC.runSummary(one);
    assert.equal(s.result, 'finished', seed + ' finishes');
    assert.ok(s.position >= 1 && s.position <= 5, seed + ' finishes inside the field');
    assert.ok(s.lapTimes.length === 2, seed + ' logs both laps');
  }
});

test('natural ties reach the engine without any weight tampering', () => {
  let calls = 0;
  const g = RC.simulateGame({ seed: 'ties', laps: 2, opponents: 3, skill: 3 }, () => {
    calls += 1;
    return { outcome: 'abstain', answers: [] };
  });
  assert.ok(calls > 0, 'genuine ties happen over two laps: ' + calls);
});

test('the stamp separates seed, shape, laps, field, and difficulty', () => {
  const base = { seed: 's', lengthSegs: 500, curviness: 0.5, laps: 2, opponents: 3, skill: 3, grip: 3 };
  const a = RC.racingStamp(base);
  assert.equal(a, RC.racingStamp({ ...base }));
  assert.notEqual(a, RC.racingStamp({ ...base, seed: 't' }));
  assert.notEqual(a, RC.racingStamp({ ...base, lengthSegs: 600 }));
  assert.notEqual(a, RC.racingStamp({ ...base, laps: 3 }));
  assert.notEqual(a, RC.racingStamp({ ...base, opponents: 4 }));
  assert.notEqual(a, RC.racingStamp({ ...base, grip: 5 }));
});
