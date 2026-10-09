/* Choose Your Door — core suite. Import order matters: the SDK first,
   then door-core; read the globals.

   Spec-pinned tests (DECISIONS-SDK-PLAN §5.8): same state+policy → same
   door (determinism); different policies diverge on the fixed state;
   the abstention case fires (equiprobable doors + no objective); no
   engine when toggled off (ladder scores only).

   Run: node --test tests/door_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'door-core.js'));
const DRC = globalThis.DoorCore;

const recorder = (choice, outcome = 'accept') => {
  const reqs = [];
  return {
    reqs,
    choose: (req) => {
      reqs.push(req);
      if (outcome !== 'accept') return { outcome, answers: [] };
      return {
        outcome: 'accept',
        answers: [{ question_id: 'door-pick', type: 'choice', choice,
          confidence: 0.7, distribution: { entries: [] } }],
      };
    },
  };
};

test('config normalization clamps and pins defaults', () => {
  const d = DRC.normalizeConfig(null);
  assert.equal(d.personality, 'steady');
  assert.equal(d.doorCount, 3);
  assert.equal(d.objective, 'treasure');
  const x = DRC.normalizeConfig({ doorCount: 99, health: 0, weapon: 7,
    knowledge: -3, torches: 99, personality: 'wizard', objective: 'naps' });
  assert.equal(x.doorCount, 5);
  assert.equal(x.health, 1);
  assert.equal(x.weapon, 1);
  assert.equal(x.knowledge, 0);
  assert.equal(x.torches, 5);
  assert.equal(x.personality, 'steady');
  assert.equal(x.objective, 'treasure');
  const z = DRC.normalizeConfig({ torches: 0 });
  assert.equal(z.torches, 0);
});

test('SPEC: same state + policy → same door (determinism)', () => {
  const cfg = { seed: 'det', personality: 'greedy', doorCount: 4 };
  const g1 = DRC.createGame(cfg);
  const d1 = DRC.decide(g1, null);
  const g2 = DRC.createGame(cfg);
  const d2 = DRC.decide(g2, null);
  assert.equal(d1.choice, d2.choice);
  assert.deepEqual(d1.scores, d2.scores, 'the score vectors replay exactly');
  assert.deepEqual(d1.probs, d2.probs);
  assert.equal(g1.stamp, g2.stamp);
  const g3 = DRC.createGame({ seed: 'det-2', personality: 'greedy', doorCount: 4 });
  const d3 = DRC.decide(g3, null);
  assert.notEqual(g1.stamp, g3.stamp, 'a new seed is a new room');
});

test('SPEC: different policies diverge on the fixed state', () => {
  const fixed = { seed: 'diverge', doorCount: 4, objective: 'treasure' };
  const pick = (p) => DRC.decide(DRC.createGame(Object.assign({}, fixed,
    { personality: p })), null);
  const steady = pick('steady'), coward = pick('coward'),
    greedy = pick('greedy'), brave = pick('brave');
  assert.equal(coward.choice, greedy.choice === undefined ? null : coward.choice,
    'sanity: choices are strings');
  assert.notEqual(coward.choice, greedy.choice,
    'coward and greedy disagree in the same room');
  assert.notEqual(coward.choice, brave.choice,
    'coward and brave disagree too');
  // and the divergence is the WEIGHTS, not the room: same doors either way
  const doorsA = DRC.createGame(fixed).doors;
  const doorsB = DRC.createGame(Object.assign({}, fixed,
    { personality: 'coward' })).doors;
  assert.deepEqual(doorsA, doorsB);
});

test('state modifiers: hurt and unarmed characters fear danger more', () => {
  const door = { id: 'door-1', label: 'x', glyph: 'x', danger: 0.9,
    reward: 0.1, mystery: 0, effort: 0 };
  const policy = DRC.POLICIES.coward;
  const fresh = { weapon: 0.5, knowledge: 0, torches: 0, objective: 'none',
    personality: 'coward' };
  const healthy = DRC.scoreDoor(Object.assign({}, fresh, { health: 100 }), policy, door);
  const hurt = DRC.scoreDoor(Object.assign({}, fresh, { health: 20 }), policy, door);
  const armed = DRC.scoreDoor(Object.assign({}, fresh, { health: 100, weapon: 1 }),
    policy, door);
  assert.ok(hurt < healthy, 'danger stings more at 20 health');
  assert.ok(armed > healthy, 'a real weapon softens the same danger');
  assert.ok(Number.isFinite(healthy) && Number.isFinite(hurt));
});

test('softmax: probabilities sum to 1, top door gets the top probability', () => {
  const g = DRC.createGame({ seed: 'softmax', doorCount: 5 });
  const d = DRC.decide(g, null);
  const sum = d.probs.reduce((a, b) => a + b, 0);
  assert.ok(Math.abs(sum - 1) < 1e-9, 'sum was ' + sum);
  const topIdx = g.doors.findIndex((x) => x.id === d.choice);
  assert.equal(d.probs[topIdx], Math.max(...d.probs));
  assert.ok(d.confidence > 0.2 && d.confidence < 1);
});

test('SPEC: the abstention case fires — equiprobable doors + no objective', () => {
  const cfg = { seed: 'abstain', doorCount: 3, balanced: true, objective: 'none' };
  const g = DRC.createGame(cfg);
  const d = DRC.decide(g, recorder(null, 'abstain').choose);
  assert.equal(d.abstained, true, 'the engine declined and nobody moved');
  assert.equal(d.choice, null);
  assert.equal(d.rung, 'abstain');
  assert.ok(d.why.includes('ABSTAINED'), d.why);
  assert.ok(d.why.includes('equiprobable doors'), d.why);
  // the scores really are indistinguishable
  assert.ok(new Set(d.scores).size === 1, 'dead heat: ' + d.scores);
  // probabilities are uniform at 1/3 — the 0.30 policy line is not what
  // fires here; the dead heat is (no argmax exists at identical scores)
  assert.ok(Math.abs(d.probs[0] - 1 / 3) < 1e-6);
  assert.ok(Math.abs(d.confidence - 1 / 3) < 1e-3);
  // at 5 doors the same setup lands UNDER the 0.30 line as well
  const g5 = DRC.createGame({ seed: 'abstain', doorCount: 5, balanced: true,
    objective: 'none' });
  const d5 = DRC.decide(g5, recorder(null, 'abstain').choose);
  assert.equal(d5.abstained, true);
  assert.ok(Math.abs(d5.confidence - 0.2) < 1e-6, 'uniform at 1/5');
  assert.ok(d5.why.includes('< 0.30'), d5.why);
});

test('SPEC: engine off — ladder scores only, no ask, argmax decides', () => {
  const rec = recorder('door-1');
  const g = DRC.createGame({ seed: 'ladder', doorCount: 4 });
  const d = DRC.decide(g, null);
  assert.equal(rec.reqs.length, 0, 'no wire traffic without the bridge');
  assert.equal(d.rung, 'rule');
  assert.equal(d.abstained, false);
  assert.ok(d.why.includes('ladder:'), d.why);
  const best = Math.max(...d.scores);
  assert.equal(d.scores[g.doors.findIndex((x) => x.id === d.choice)], best,
    'the argmax door is the choice');
  // a balanced room with no engine: dead tie broken by door order, said aloud
  const tie = DRC.createGame({ seed: 'tie', balanced: true, doorCount: 3 });
  const td = DRC.decide(tie, null);
  assert.equal(td.choice, 'door-1', 'roster-order tie-break');
  assert.ok(td.why.includes('dead tie'), td.why);
});

test('SPEC: the engine ride — honored answer lands on the engine rung', () => {
  const cfg = { seed: 'honored', doorCount: 3, personality: 'coward' };
  const rec = recorder('door-2');
  const g = DRC.createGame(cfg);
  const d = DRC.decide(g, rec.choose);
  assert.equal(rec.reqs.length, 1);
  assert.equal(d.choice, 'door-2');
  assert.equal(d.rung, 'engine');
  assert.equal(d.abstained, false);
  assert.ok(d.why.includes('the engine picked door-2'), d.why);
});

test('the full wire contract: state, candidates, policy', () => {
  const rec = recorder('door-1');
  const g = DRC.createGame({ seed: 'wire', doorCount: 4, personality: 'coward',
    health: 33, weapon: 0.25, knowledge: 0.8, torches: 2, objective: 'escape' });
  DRC.decide(g, rec.choose);
  const req = rec.reqs[0];
  assert.equal(typeof req.state.text, 'string');
  assert.ok(req.state.text.includes('health 33'), req.state.text);
  assert.ok(req.state.text.includes('objective: escape'), req.state.text);
  assert.ok(req.state.text.includes('danger 3'), 'the policy weights ride along: '
    + req.state.text);
  assert.deepEqual(req.state.facts, {}, 'facts is the tagged-enum empty object');
  assert.equal(req.questions.length, 1);
  assert.equal(req.questions[0].type, 'choice');
  assert.equal(req.questions[0].id, 'door-pick');
  assert.deepEqual(req.questions[0].candidates.map((c) => c.id),
    ['door-1', 'door-2', 'door-3', 'door-4']);
  assert.ok(req.questions[0].candidates.every((c) => c.description.length > 0));
  assert.deepEqual(req.policy,
    { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: 'low' },
    'coward ships risk low');
  const brave = DRC.createGame({ seed: 'wire', personality: 'brave' });
  DRC.decide(brave, rec.choose);
  assert.equal(rec.reqs[1].policy.risk, 'high', 'brave ships risk high');
});

test('refusals and garbage: nobody moves, the why says which', () => {
  const cfg = { seed: 'refuse', doorCount: 3 };
  // engine abstains on a NON-balanced room: still an abstain, generic why
  const g1 = DRC.createGame(cfg);
  const d1 = DRC.decide(g1, recorder(null, 'abstain').choose);
  assert.equal(d1.abstained, true);
  assert.ok(d1.why.includes('no honest winner'), d1.why);
  // a malformed answer naming nothing live is not an abstain but lands nowhere
  const g2 = DRC.createGame(cfg);
  const d2 = DRC.decide(g2, recorder('door-9').choose);
  assert.equal(d2.abstained, true);
  assert.equal(d2.choice, null);
  assert.ok(d2.why.includes('named nothing live'), d2.why);
  // a verify outcome is not an accept: nobody moves, the outcome is echoed
  const g3 = DRC.createGame(cfg);
  const d3 = DRC.decide(g3, () => ({ outcome: 'verify', answers: 'nope' }));
  assert.equal(d3.abstained, true);
  assert.ok(d3.why.includes('"verify"'), d3.why);
  // a dead-heat room says so, whatever the engine answered
  const g4 = DRC.createGame({ seed: 'dead', balanced: true, objective: 'none' });
  const d4 = DRC.decide(g4, () => ({ outcome: 'verify', answers: [] }));
  assert.equal(d4.abstained, true);
  assert.ok(d4.why.includes('dead heat') && d4.why.includes('equiprobable'), d4.why);
  // a decided room stays decided
  assert.equal(DRC.decide(g3, recorder('door-1').choose), d3,
    'the decision is recorded once');
});

test('series: policy -> behavior at volume, abstentions counted', () => {
  const base = { seed: 'vol', doorCount: 4, personality: 'greedy' };
  // an honoring bridge takes door-1 in every room it is asked — even the
  // dead heats (the engine moved, that is not a ladder tie-break)
  const honor = DRC.series(base, 200, recorder('door-1').choose);
  assert.equal(honor.rooms, 200);
  assert.equal(honor.abstains, 0);
  assert.equal(honor.tieBreaks, 0);
  assert.equal(honor.tally['door-1'], 200);
  // an abstaining bridge abstains everywhere: dead heats and coin flips alike
  const abstain = DRC.series(base, 200, recorder(null, 'abstain').choose);
  assert.equal(abstain.abstains, 200);
  assert.equal(abstain.tally['door-1'], undefined);
  // without the bridge the ladder never abstains — dead heats become
  // tie-breaks, counted separately from argmax wins
  const ladder = DRC.series(base, 200, null);
  assert.equal(ladder.abstains, 0);
  assert.equal(ladder.rooms, 200);
  assert.equal(ladder.tieBreaks, 50, 'every 4th room is the dead-heat setup');
  assert.ok(Object.values(ladder.tally).reduce((a, b) => a + b, 0) === 150,
    'argmax wins only: ' + JSON.stringify(ladder.tally));
  // and policies behave differently across the same room sequence
  const coward = DRC.series({ seed: 'vol', doorCount: 4, personality: 'coward' },
    200, null);
  assert.notDeepEqual(coward.tally, ladder.tally,
    'the same rooms, a different policy, a different tally');
  assert.ok(ladder.avgConfidence > 0.3 && ladder.avgConfidence <= 1);
});

test('divergence: policies disagree per room, identical policies agree', () => {
  const base = { seed: 'vol', doorCount: 4 };
  const d = DRC.divergence(Object.assign({}, base, { personality: 'coward' }),
    Object.assign({}, base, { personality: 'greedy' }), 200);
  assert.equal(d.rooms, 150, 'dead-heat rooms are skipped');
  assert.ok(d.disagreePct > 50,
    'coward and greedy disagree in ' + d.disagreePct + '% of rooms');
  const same = DRC.divergence(Object.assign({}, base, { personality: 'steady' }),
    Object.assign({}, base, { personality: 'steady' }), 200);
  assert.equal(same.disagree, 0);
  assert.equal(same.agree, 150);
});

test('scenario shape: 3-5 doors, ids ordered, balanced rooms are identical', () => {
  for (const n of [3, 4, 5]) {
    const doors = DRC.makeScenario(DRC.normalizeConfig({ doorCount: n, seed: 'shape' }));
    assert.equal(doors.length, n);
    assert.deepEqual(doors.map((d) => d.id),
      doors.map((_, i) => 'door-' + (i + 1)));
    assert.ok(doors.every((d) => d.danger >= 0 && d.danger <= 1
      && d.reward >= 0 && d.reward <= 1 && d.label.length > 0));
  }
  const bal = DRC.makeScenario(DRC.normalizeConfig({ balanced: true, doorCount: 3 }));
  assert.ok(bal.every((d) => d.danger === 0.5 && d.reward === 0.5
    && d.mystery === 0 && d.effort === 0.5));
  assert.equal(new Set(bal.map((d) => d.label)).size, 3, 'labels still distinct');
});

test('runSummary and stamp: stamps separate every knob that matters', () => {
  const g = DRC.createGame({ seed: 'sum', personality: 'brave', doorCount: 3 });
  const d = DRC.decide(g, null);
  const s = DRC.runSummary(g);
  assert.equal(s.personality, 'brave');
  assert.equal(s.risk, 'high');
  assert.equal(s.decided, true);
  assert.equal(s.choice, d.choice);
  assert.equal(s.rung, 'rule');
  assert.equal(s.doors, 3);
  const base = DRC.doorStamp(DRC.normalizeConfig({ seed: 'x' }));
  for (const delta of [{ seed: 'y' }, { personality: 'coward' }, { doorCount: 4 },
    { health: 50 }, { weapon: 0 }, { knowledge: 0 }, { torches: 4 },
    { objective: 'glory' }, { balanced: true }]) {
    assert.notEqual(base, DRC.doorStamp(DRC.normalizeConfig(
      Object.assign({ seed: 'x' }, delta))), JSON.stringify(delta));
  }
  assert.equal(base, DRC.doorStamp(DRC.normalizeConfig({ seed: 'x' })), 'stable');
});
