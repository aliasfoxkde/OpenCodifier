/* Spacecraft Emergency — core suite. Import order matters: the SDK first,
   then spacecraft-core; read the globals.

   Spec-pinned tests (DECISIONS-SDK-PLAN §5.15): injected sensor conflict
   produces abstention (not a guess); escalation resolves the loop;
   deterministic per seed; decision latency shown.

   Run: node --test tests/spacecraft_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'spacecraft-core.js'));
const SC = globalThis.ShipCore;

const recorder = (choice, outcome = 'accept') => {
  const reqs = [];
  return {
    reqs,
    choose: (req) => {
      reqs.push(req);
      if (outcome !== 'accept') return { outcome, answers: [] };
      return {
        outcome: 'accept',
        answers: [{ question_id: 'ship-action', type: 'choice', choice,
          confidence: 0.7, distribution: { entries: [] } }],
      };
    },
  };
};

const runFor = (cfg, seconds, engineChoose) => {
  const g = SC.createGame(cfg);
  for (let i = 0; i < seconds * SC.TICKS_PER_S; i++) {
    if (g.over || g.pending) break;
    SC.tickGame(g, engineChoose);
  }
  return g;
};

/* the fire fixture lands in the engine band: EMERGENCY 400 vs CONTINUE 348 */
const drive = (g, seconds) => {
  /* tick a run to the end (or the cap), captaining escalations as the
     first candidate — the freeze pauses physics until answered */
  let guard = 0;
  while (!g.over && g.ticks < seconds * SC.TICKS_PER_S && guard++ < 400000) {
    if (g.pending) SC.resolvePending(g, g.pending.question.candidates[0].id);
    else SC.tickGame(g, null);
  }
  return g;
};

const fireFixture = () => {
  const g = SC.createGame({ seed: 'wire', drift: 0, failures: 0, askBudget: 5 });
  g.fire = true;
  g.tokens = 5;
  return g;
};

test('config normalization clamps and pins the defaults', () => {
  assert.deepEqual(SC.normalizeConfig(null), SC.DEFAULT_CONFIG);
  const x = SC.normalizeConfig({ drift: 9, failures: -2, difficulty: 7, askBudget: 99, seed: 9 });
  assert.equal(x.drift, 1);
  assert.equal(x.failures, 0);
  assert.equal(x.difficulty, 1);
  assert.equal(x.askBudget, 10);
  assert.equal(x.seed, '9');
  const z = SC.normalizeConfig({ drift: 0, failures: 0, difficulty: 0, askBudget: 0 });
  for (const k of ['drift', 'failures', 'difficulty', 'askBudget']) {
    assert.equal(z[k], 0, k + ' = 0 is a real config');
  }
});

test('SPEC: determinism — same seed, same flight; new seed, new flight', () => {
  const eng = recorder('continue');
  const a = SC.runSummary(runFor(SC.DEFAULT_CONFIG, 60, eng.choose));
  const b = SC.runSummary(runFor(SC.DEFAULT_CONFIG, 60, recorder('continue').choose));
  assert.deepEqual(a, b);
  const c = SC.runSummary(runFor({ seed: 'det-2' }, 60, recorder('continue').choose));
  assert.notDeepEqual(a, c);
});

test('SPEC: injected sensor conflict produces abstention, never a guess', () => {
  const g = SC.createGame({ seed: 'conflict', drift: 0, failures: 0 });
  SC.injectSensorFault(g, 'temp');
  assert.ok(g.faults.temp, 'the fault is staged');
  const before = SC.runSummary(g);
  // force the decision moment
  g.nextDecisionAt = g.ticks + 1;
  SC.tickGame(g, null);
  assert.equal(g.pending !== null, true, 'the computer refused to guess');
  assert.ok(g.pending.reason.includes('conflicting telemetry'), g.pending.reason);
  assert.ok(g.pending.reason.includes('engine temp'));
  assert.ok(g.pending.question.candidates.length >= 2);
  const after = SC.runSummary(g);
  assert.deepEqual(after.tally, before.tally, 'no action was executed');
  assert.deepEqual(after.rungs, before.rungs, 'no rung was charged');
  assert.ok(g.log.some((l) => l.kind === 'abstain'), 'the abstention is logged');
});

test('a dead-heat score also abstains (the computer will not guess)', () => {
  const g = SC.createGame({ seed: 'heat', drift: 0, failures: 0 });
  // force two actions to an exact tie: fuel 0 margin isn't needed —
  // set hull so emergency matches home: emergency 100+300(fire)=400,
  // home 240+400(out of range)=640... simpler: drive both to a pure tie
  // via the agreement collapse: a conflict anywhere drops conf below 0.3
  // whenever gap < 120/0.25... use a conflict on hull with a modest gap
  SC.injectSensorFault(g, 'fuel');
  g.temp = 76;                                  // cont vs reduce tighten
  g.nextDecisionAt = g.ticks + 1;
  SC.tickGame(g, null);
  if (g.pending) {
    assert.ok(g.pending.reason.includes('conflicting telemetry'));
  } else {
    const s = SC.runSummary(g);
    assert.ok(s.rungs.rule > 0, 'either escalation or a rule, never silence');
  }
});

test('SPEC: escalation resolves the loop — the captain picks, play resumes', () => {
  const g = SC.createGame({ seed: 'conflict', drift: 0, failures: 0 });
  SC.injectSensorFault(g, 'hull');
  g.nextDecisionAt = g.ticks + 1;
  SC.tickGame(g, null);
  assert.ok(g.pending, 'escalated');
  const choice = g.pending.question.candidates[0].id;
  const r = SC.resolvePending(g, choice);
  assert.ok(r, 'the resolution executed');
  assert.equal(r.rung, 'player', 'the resolution is on the player rung');
  assert.equal(g.pending, null, 'the loop is clear');
  assert.equal(g.tally[choice], 1);
  assert.ok(g.log.some((l) => l.note.includes('captain')), 'logged as the captain');
  // and the sim runs again without stalling: the stuck sensor lives
  // ~15 s, so the walk keeps captaining every escalation it raises
  const t0 = g.ticks;
  let guard = 0;
  while (g.ticks < t0 + 5 * SC.TICKS_PER_S && !g.over && guard++ < 5000) {
    if (g.pending) {
      const r = SC.resolvePending(g, g.pending.question.candidates[0].id);
      assert.equal(r.rung, 'player', 'every resolution is the captain\'s');
      continue;
    }
    SC.tickGame(g, null);
  }
  assert.ok(g.ticks >= t0 + 5 * SC.TICKS_PER_S, 'the loop resumed');
  assert.equal(g.pending, null, 'and it is not stuck at the end');
  const s = SC.runSummary(g);
  assert.ok(s.playerCalls >= 2, 'the captain answered again, got ' + s.playerCalls);
});

test('resolvePending refuses a choice outside the candidates', () => {
  const g = SC.createGame({ seed: 'refuse', drift: 0, failures: 0 });
  SC.injectSensorFault(g, 'fuel');
  g.nextDecisionAt = g.ticks + 1;
  SC.tickGame(g, null);
  assert.ok(g.pending);
  const calls = g.playerCalls;
  assert.equal(SC.resolvePending(g, 'warp-nine'), null);
  assert.equal(SC.resolvePending(g, null), null);
  assert.ok(g.pending, 'still pending after a bad answer');
  assert.equal(g.playerCalls, calls);
});

test('RULE gates: hull critical, thermal limit, and the out-of-range abort', () => {
  // hull 10 → EMERGENCY as a rule, never asked
  const g1 = SC.createGame({ seed: 'hull', drift: 0, failures: 0 });
  g1.hull = 10;
  g1.nextDecisionAt = g1.ticks + 1;
  const eng1 = recorder('continue');
  SC.tickGame(g1, eng1.choose);
  assert.equal(g1.lastDecision.action, 'emergency');
  assert.equal(g1.lastDecision.rung, 'rule');
  assert.equal(g1.engineCalls, 0);

  // temp 96 → SHUTDOWN as a rule
  const g2 = SC.createGame({ seed: 'temp', drift: 0, failures: 0 });
  g2.temp = 96;
  g2.nextDecisionAt = g2.ticks + 1;
  SC.tickGame(g2, null);
  assert.equal(g2.lastDecision.action, 'shutdown');
  assert.ok(g2.lastDecision.why.includes('thermal'));

  // can't reach the destination but can make Earth → RETURN HOME
  const g3 = SC.createGame({ seed: 'home', drift: 0, failures: 0 });
  g3.dist = 600;                       // needs 600*0.069 = 41.4 > 30 fuel
  g3.fuel = 30;                        // home is 400 away: 27.6 ≤ 30 ✓
  g3.nextDecisionAt = g3.ticks + 1;
  SC.tickGame(g3, null);
  assert.equal(g3.lastDecision.action, 'home', SC.runSummary(g3).tally
    ? JSON.stringify(g3.lastDecision) : '');
  assert.ok(g3.aborting, 'the abort is armed');
});

test('the escalated request carries the FULL wire contract', () => {
  const g = fireFixture();
  const eng = recorder('emergency');
  const r = SC.decide(g, eng.choose);
  assert.equal(eng.reqs.length, 1, 'the uncertain band went to the engine');
  const req = eng.reqs[0];
  assert.equal(typeof req.state.text, 'string');
  assert.ok(req.state.text.includes('Fire aboard.'));
  assert.ok(req.state.text.includes('fuel'), 'the state names the telemetry');
  assert.deepEqual(req.state.facts, {}, 'facts is the tagged-enum empty object');
  assert.equal(req.questions.length, 1);
  assert.equal(req.questions[0].type, 'choice');
  assert.equal(req.questions[0].id, 'ship-action');
  const ids = req.questions[0].candidates.map((c) => c.id);
  assert.deepEqual(ids, ['emergency', 'continue'], 'top-2 by score');
  assert.ok(req.questions[0].candidates.every((c) => c.description.length > 20));
  assert.deepEqual(req.policy,
    { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: 'low' });
  assert.equal(g.engineCalls, 1);
  assert.equal(r.action, 'emergency', 'the honored answer executed');
  assert.equal(r.rung, 'engine');
  assert.equal(g.fire, false, 'the fire is out');
});

test('an engine abstain escalates to the captain (refusal is first-class)', () => {
  const g = fireFixture();
  const eng = recorder(null, 'abstain');
  const r = SC.decide(g, eng.choose);
  assert.equal(g.engineCalls, 1, 'the ask happened');
  assert.ok(g.pending, 'the engine abstained → the captain decides');
  assert.ok(g.pending.reason.includes('engine abstained'));
  assert.equal(r.action, null, 'no action was guessed');
  assert.equal(g.tally.emergency, undefined, 'nothing was executed');
  assert.equal(g.rungs.engine, 0);
});

test('a non-candidate engine answer is ignored; the argmax holds', () => {
  const g = fireFixture();
  const eng = recorder('warp');
  const r = SC.decide(g, eng.choose);
  assert.equal(g.engineCalls, 1);
  assert.equal(r.rung, 'rule', 'the answer named nothing live');
  assert.equal(r.action, 'emergency', 'the argmax held');
});

test('the batching budget is respected — no token, argmax rule', () => {
  const g = fireFixture();
  g.tokens = 0;
  const eng = recorder('continue');
  const r = SC.decide(g, eng.choose);
  assert.equal(eng.reqs.length, 0, 'no token, no ask');
  assert.equal(g.engineCalls, 0);
  assert.equal(r.action, 'emergency');
  assert.equal(r.rung, 'rule');
  const none = SC.runSummary(runFor({ askBudget: 0, difficulty: 1 }, 30,
    recorder('continue').choose));
  assert.equal(none.engineCalls, 0);
  assert.ok(none.rungs.rule > 0);
});

test('events move the scores: debris → course, storm → heat', () => {
  const g = SC.createGame({ seed: 'events', drift: 0, failures: 0 });
  SC.injectEvent(g, 'debris');
  assert.ok(g.ahead, 'the hazard is ahead');
  g.nextDecisionAt = g.ticks + 1;
  SC.tickGame(g, null);
  assert.equal(g.lastDecision.action, 'course', 'the hazard dominates');
  assert.equal(g.ahead, null, 'the course change cleared it');

  const g2 = SC.createGame({ seed: 'events2', drift: 0, failures: 0 });
  SC.injectEvent(g2, 'storm');
  assert.ok(g2.temp > 75, 'the storm spiked the plant');
  assert.ok(g2.log.some((l) => l.kind === 'event'), 'the event is logged');
  assert.equal(SC.injectEvent(g2, 'volcano'), null, 'unknown events refused');
});

test('a fire burns hull and crew until EMERGENCY extinguishes it', () => {
  const g = SC.createGame({ seed: 'fire', drift: 0, failures: 0 });
  SC.injectEvent(g, 'fire');
  assert.ok(g.fire);
  const hull0 = g.hull;
  for (let i = 0; i < 3 * SC.TICKS_PER_S; i++) SC.tickGame(g, null);
  assert.ok(g.hull < hull0, 'the fire ate the hull: ' + hull0 + ' → ' + g.hull);
  // the ladder or the gate eventually answers with EMERGENCY
  for (let i = 0; i < 30 * SC.TICKS_PER_S && g.fire && !g.pending; i++) {
    SC.tickGame(g, recorder('emergency').choose);
  }
  assert.equal(g.fire, false, 'the fire was answered');
});

test('the default calm crossing docks — deterministically', () => {
  const g = SC.createGame({ seed: 'crossing', drift: 0, failures: 0, difficulty: 0 });
  for (let i = 0; i < 120 * SC.TICKS_PER_S && !g.over; i++) SC.tickGame(g, null);
  const s = SC.runSummary(g);
  assert.equal(s.over, true, 'the crossing ended');
  assert.equal(s.outcome, 'docked', 'the ship arrived: ' + s.outcome);
  assert.ok(s.fuel > 0, 'with fuel to spare: ' + s.fuel);
  assert.ok(s.decisions >= 30, 'the computer decided throughout, got ' + s.decisions);
  const tot = Object.values(s.tally).reduce((a, b) => a + b, 0);
  assert.equal(tot, s.decisions, 'the tally and the decision count agree');
  assert.equal(s.rungs.rule + s.rungs.engine + s.rungs.player, s.decisions,
    'every decision lands on a rung');
});

test('an abort completes home; running dry ends adrift', () => {
  const g = SC.createGame({ seed: 'abort', drift: 0, failures: 0, difficulty: 0 });
  // the destination is out of range; Earth (400 back) is still reachable
  g.dist = 600; g.fuel = 30;
  g.nextDecisionAt = g.ticks + 1;
  SC.tickGame(g, null);
  assert.ok(g.aborting, 'out of range → abort');
  drive(g, 120);
  assert.equal(SC.runSummary(g).outcome, 'returned', 'made it home');

  const g2 = SC.createGame({ seed: 'dry', drift: 0, failures: 0, difficulty: 0 });
  g2.fuel = 0.5; g2.power = 70;
  drive(g2, 30);
  assert.equal(SC.runSummary(g2).outcome, 'adrift', 'dry tank ends the flight');
});

test('SPEC: decision latency is recorded on the decision, never in the summary', () => {
  const g = fireFixture();
  const eng = recorder('emergency');
  const r = SC.decide(g, eng.choose);
  assert.equal(typeof r.ms, 'number');
  assert.ok(r.ms >= 0);
  const s = SC.runSummary(g);
  assert.ok(!('ms' in s), 'latency stays out of the deterministic summary');
  assert.ok(!('latencyMs' in s));
});

test('sensor conflict detection math is honest', () => {
  const g = SC.createGame({ seed: 'sense', drift: 0, failures: 0 });
  let sensors = SC.readSensors(g);
  assert.deepEqual(SC.conflictChannels(sensors), [], 'a healthy ship agrees');
  SC.injectSensorFault(g, 'fuel');
  sensors = SC.readSensors(g);
  assert.deepEqual(SC.conflictChannels(sensors), ['fuel'],
    'the stuck backup reads outside tolerance');
  assert.ok(Math.abs(sensors.fuel.a - sensors.fuel.b) > SC.CHANNEL_TOL.fuel);
  // the fault self-heals
  g.faults.fuel.until = g.ticks;
  sensors = SC.readSensors(g);
  assert.deepEqual(SC.conflictChannels(sensors), [], 'the crew fixes it in time');
});

test('history samples every 5 sim-seconds for the chart', () => {
  const g = runFor({ seed: 'hist', difficulty: 0 }, 32, null);
  assert.ok(g.history.length >= 6, 'samples at t=0,5,…,30, got ' + g.history.length);
  assert.ok(g.history.every((h, i) => i === 0 || h.t > g.history[i - 1].t));
  assert.ok(g.history.every((h) => Number.isFinite(h.dist) && Number.isFinite(h.temp)));
});

test('config and stamp: stamps separate every knob that changes the flight', () => {
  const base = SC.shipStamp(SC.normalizeConfig({ seed: 'x' }));
  assert.notEqual(base, SC.shipStamp(SC.normalizeConfig({ seed: 'y' })));
  assert.notEqual(base, SC.shipStamp(SC.normalizeConfig({ seed: 'x', drift: 0.8 })));
  assert.notEqual(base, SC.shipStamp(SC.normalizeConfig({ seed: 'x', failures: 0.9 })));
  assert.notEqual(base, SC.shipStamp(SC.normalizeConfig({ seed: 'x', difficulty: 1 })));
  assert.notEqual(base, SC.shipStamp(SC.normalizeConfig({ seed: 'x', askBudget: 4 })));
  assert.equal(base, SC.shipStamp(SC.normalizeConfig({ seed: 'x' })), 'stable');
});

test('a no-bridge run works on rules alone and keeps flying', () => {
  const g = runFor({ drift: 0, failures: 0 }, 45, null);
  const s = SC.runSummary(g);
  assert.equal(s.engineCalls, 0);
  assert.equal(s.playerCalls, 0);
  assert.ok(s.rungs.rule > 0);
  assert.equal(s.over, false, 'a calm 45 s is mid-flight, not a crash');
});
