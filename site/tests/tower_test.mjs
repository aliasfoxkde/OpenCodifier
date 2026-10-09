/* Tower Defense — core suite. Import order matters: the SDK first, then
   tower-core; read the globals.

   Spec-pinned tests (DECISIONS-SDK-PLAN §5.14): seeded waves are clearable
   by the default loadout; tower policy changes change outcomes; enemies
   divert only when the alternate route exists; determinism.

   Run: node --test tests/tower_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'tower-core.js'));
const TC = globalThis.TowerCore;

/* a scripted engine: records every request, answers `choice` */
const recorder = (choice, outcome = 'accept') => {
  const reqs = [];
  return {
    reqs,
    choose: (req) => {
      reqs.push(req);
      if (outcome !== 'accept') return { outcome, answers: [] };
      return {
        outcome: 'accept',
        answers: [{ question_id: 'tower-policy', type: 'choice', choice,
          confidence: 0.7, distribution: { entries: [] } }],
      };
    },
  };
};

/* full run with an optional mutation applied before ticking */
const run = (cfg, engineChoose, mutate) => {
  const g = TC.createGame(cfg);
  if (mutate) mutate(g);
  let guard = TC.TICKS_PER_S * 60 * 20;
  while (!g.over && guard-- > 0) TC.tickGame(g, engineChoose);
  assert.ok(guard > 0, 'the run finished inside the guard (no livelock)');
  return g;
};

/* a crafted tower + two IDENTICAL wounded runners at the same spot near
   the base: every target policy scores exactly the same (kill bonus puts
   them above save-special) → gap 0 ≤ TIE_EPS → the engine is asked */
const tiedSnapshot = () => {
  const g = TC.createGame({ seed: 'tie', waves: 1 });
  g.towers = [];
  const spot = TC.posOn(TC.MAIN, 1200);
  const tw = {
    id: 't0', x: spot.x, y: spot.y, range: 300, dmg: 12, rate: 30,
    special: true, charge: 50, cooldown: 0, hp: 100, disabledUntil: 0,
    policy: 'auto', forced: 0, shots: 0, kills: 0,
  };
  g.towers.push(tw);
  const mk = (id) => ({
    id, type: 'runner', hp: 10, maxHp: 18, speed: 85, bounty: 4, leaks: 1,
    saboteur: false, route: 'main', t: 1200, state: 'advance', stateUntil: 0,
    decideIn: 30, target: null, diverted: false,
  });
  g.enemies.push(mk('a'), mk('b'));
  return { g, tw };
};

test('config normalization clamps waves/speed and pins the defaults', () => {
  assert.deepEqual(TC.normalizeConfig(null), TC.DEFAULT_CONFIG);
  const c = TC.normalizeConfig({ waves: 99, speed: 100, loadout: 'nope', diversion: 0, seed: 7 });
  assert.equal(c.waves, 20);
  assert.equal(c.speed, 8);
  assert.equal(c.loadout, 'standard');
  assert.equal(c.diversion, false);
  assert.equal(c.seed, '7');
});

test('route geometry: the alternate route is longer (diverting has a cost)', () => {
  assert.ok(TC.ALT.total > TC.MAIN.total, 'ALT longer than MAIN');
  const start = TC.posOn(TC.MAIN, 0);
  const end = TC.posOn(TC.MAIN, TC.MAIN.total);
  assert.ok(start.x < end.x, 'the path crosses the field left to right');
  /* the alt route shares both endpoints with the split/join points */
  const a0 = TC.posOn(TC.ALT, 0);
  const split = TC.posOn(TC.MAIN, TC.SPLIT_T);
  assert.deepEqual([a0.x, a0.y], [split.x, split.y]);
  const aEnd = TC.posOn(TC.ALT, TC.ALT.total);
  const join = TC.posOn(TC.MAIN, TC.JOIN_T);
  assert.deepEqual([aEnd.x, aEnd.y], [join.x, join.y]);
});

test('wave composition is seeded, sized 6+2·wave, and only uses real types', () => {
  const a = TC.waveComposition(3, 'seed-x');
  const b = TC.waveComposition(3, 'seed-x');
  const c = TC.waveComposition(3, 'seed-y');
  assert.deepEqual(a, b);
  assert.notDeepEqual(a, c);
  assert.equal(a.length, 6 + 2 * 3);
  assert.ok(a.every((t) => t in TC.ENEMY_TYPES));
  assert.equal(TC.waveComposition(1, 's').length, 8);
});

test('SPEC: seeded waves are clearable by the default loadout (dead engine)', () => {
  const g = run(TC.DEFAULT_CONFIG, null);
  const s = TC.runSummary(g);
  assert.equal(s.victory, true, 'default loadout clears the default 5 waves');
  assert.equal(s.wave, TC.DEFAULT_CONFIG.waves);
  assert.ok(s.lives > 0, 'lives survive the default run');
  assert.ok(s.kills > 0, 'towers actually kill');
  assert.ok(s.gold > TC.START_GOLD, 'bounties pay');
});

test('SPEC: tower policy changes change outcomes', () => {
  const withPolicy = (policy) => {
    const s = TC.runSummary(run({ seed: 'pol', waves: 4 }, null,
      (g) => g.towers.forEach((t) => { t.policy = policy; })));
    return s;
  };
  const auto = TC.runSummary(run({ seed: 'pol', waves: 4 }, null));
  const weakest = withPolicy('weakest');
  const strongest = withPolicy('strongest');
  const identical = (a, b) => a.kills === b.kills && a.ticks === b.ticks && a.lives === b.lives;
  assert.ok(!(identical(auto, weakest) && identical(auto, strongest) && identical(weakest, strongest)),
    'at least one forced policy plays differently from auto');
  /* forced overrides are recorded as forced, not as ladder rungs */
  const g = run({ seed: 'pol', waves: 4 }, null, (gg) => gg.towers.forEach((t) => { t.policy = 'fastest'; }));
  assert.ok(g.towers.every((t) => t.forced > 0 || t.shots === 0));
});

test('SPEC: enemies divert only when the alternate route exists', () => {
  const no = TC.runSummary(run({ seed: 'div', waves: 6, diversion: false }, null));
  assert.equal(no.diverted, 0, 'no diversions without the route');
  assert.equal(no.diversionUnlocked, false);
  const yes = TC.runSummary(run({ seed: 'div', waves: 6, diversion: true }, null));
  assert.ok(yes.diverted > 0, 'under pressure, some enemies take the alternate route');
});

test('SPEC: determinism — same seed, same run; new seed, new run', () => {
  const a = TC.runSummary(run({ seed: 'det', waves: 5, loadout: 'screen' }, null));
  const b = TC.runSummary(run({ seed: 'det', waves: 5, loadout: 'screen' }, null));
  assert.deepEqual(a, b);
  const c = TC.runSummary(run({ seed: 'det-2', waves: 5, loadout: 'screen' }, null));
  assert.notDeepEqual(a, c);
});

test('genuine near-ties escalate: shoot-now vs hold-special within TIE_EPS', () => {
  const { g, tw } = tiedSnapshot();
  const eng = recorder('nearest');
  const res = TC.decideTower(g, tw, eng.choose);
  assert.equal(eng.reqs.length, 1, 'the near-tie went to the engine');
  assert.equal(res.rung, 'engine');
  assert.equal(res.pick, 'nearest', 'the engine answer is honored');
  assert.ok(res.evGap <= TC.TIE_EPS, 'the contested gap is inside TIE_EPS, got ' + res.evGap);
  assert.equal(g.enemies[0].hp, 10 - 12, 'the honored policy fired on its target');
  assert.equal(g.kills, 1, 'the wounded runner died');
  assert.equal(g.engineCalls, 1);
  assert.equal(g.rungs.engine, 1);
});

test('the escalated request carries the FULL wire contract (the racer rule)', () => {
  const { g, tw } = tiedSnapshot();
  const eng = recorder('nearest');
  TC.decideTower(g, tw, eng.choose);
  const req = eng.reqs[0];
  assert.equal(typeof req.state.text, 'string');
  assert.ok(req.state.text.includes('Tower t0 snapshot'), 'the snapshot names the tower');
  assert.ok(req.state.text.includes('2 enemies in range'), 'the snapshot counts threats');
  assert.deepEqual(req.state.facts, {}, 'facts is the tagged-enum empty object');
  assert.equal(req.questions.length, 1);
  assert.equal(req.questions[0].type, 'choice');
  assert.equal(req.questions[0].id, 'tower-policy');
  /* live candidates: same-target policies are deduped to one action, so
     the set is a non-empty subset of POLICIES, not always all six */
  const ids = req.questions[0].candidates.map((c) => c.id);
  assert.ok(ids.length >= 2, 'a real choice, not a single option');
  assert.ok(ids.every((id) => TC.POLICIES.indexOf(id) !== -1), 'ids are policies');
  assert.deepEqual(
    new Set(ids).size, ids.length, 'no duplicate candidates');
  assert.ok(req.questions[0].candidates.every((c) => c.description === TC.POLICY_DESC[c.id]));
  assert.deepEqual(req.policy, { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: 'low' });
});

test('same-target policies collapse: one enemy in range makes ONE target candidate', () => {
  const g = TC.createGame({ seed: 'one', waves: 1 });
  g.towers = [];
  const spot = TC.posOn(TC.MAIN, 1200);
  const tw = {
    id: 't0', x: spot.x, y: spot.y, range: 300, dmg: 12, rate: 30,
    special: false, charge: 0, cooldown: 0, hp: 100, disabledUntil: 0,
    policy: 'auto', forced: 0, shots: 0, kills: 0,
  };
  g.towers.push(tw);
  g.enemies.push({
    id: 'solo', type: 'runner', hp: 10, maxHp: 18, speed: 85, bounty: 4,
    leaks: 1, saboteur: false, route: 'main', t: 1200, state: 'advance',
    stateUntil: 0, decideIn: 30, target: null, diverted: false,
  });
  const eng = recorder('nearest');
  const r = TC.decideTower(g, tw, eng.choose);
  /* one action only → gap is the full distance to nothing → no ask */
  assert.equal(eng.reqs.length, 0, 'no fake question about a single action');
  assert.equal(r.pick, 'nearest');
  assert.equal(r.rung, 'rule');
});

test('the engine answer only counts when it names a real candidate; abstain falls back', () => {
  const bad = tiedSnapshot();
  const r1 = TC.decideTower(bad.g, bad.tw, recorder('teleport-enemies').choose);
  assert.equal(r1.rung, 'rule', 'a non-candidate choice is ignored');
  assert.equal(r1.pick, 'nearest', 'argmax wins (stable order on a full tie)');
  const abst = tiedSnapshot();
  const r2 = TC.decideTower(abst.g, abst.tw, recorder(null, 'abstain').choose);
  assert.equal(r2.rung, 'rule', 'an abstention falls back to the argmax');
  assert.equal(abst.g.engineCalls, 1, 'the ask still happened');
});

test('the armed special fires as a blast and resets; unarmed, it holds', () => {
  const armed = tiedSnapshot();
  armed.tw.charge = TC.SPECIAL_CHARGE;
  const r = TC.decideTower(armed.g, armed.tw, recorder('nearest').choose);
  assert.equal(r.pick, 'use-special', '2 enemies in the blast beats any single shot');
  assert.equal(armed.tw.charge, 0);
  assert.equal(armed.g.blasts.length, 1);
  assert.ok(armed.g.enemies[0].hp <= 0, 'the blast kills the first runner');
  assert.ok(armed.g.enemies[1].hp <= 0, '…and the second');
  assert.equal(armed.g.kills, 2);

  const unarmed = tiedSnapshot();
  unarmed.tw.charge = 30;
  const r2 = TC.decideTower(unarmed.g, unarmed.tw, recorder('nearest').choose);
  assert.notEqual(r2.pick, 'use-special', 'an unarmed special is never picked');
  assert.equal(unarmed.g.blasts.length, 0);
  assert.ok(unarmed.tw.charge > 0, 'charge was not wasted');
});

test('saboteurs TARGET TOWER: the tower goes down for DISABLE_TICKS, then repairs', () => {
  const g = TC.createGame({ seed: 'sab', waves: 9 });
  TC.startWave(g);
  g.spawnQueue = [];
  const e = TC.spawnEnemy(g, 'saboteur');
  e.t = 270;                                        /* 60 px off tower t0 */
  TC.decideEnemy(g, e);
  assert.equal(e.state, 'target-tower', 'a healthy saboteur next to a tower attacks it');
  assert.equal(e.target, g.towers[0]);

  const tw = g.towers[0];
  let guard = 4000;
  while (tw.disabledUntil === 0 && guard-- > 0) TC.tickGame(g, null);
  assert.ok(tw.disabledUntil > 0, 'the sabotage broke the tower');
  const downAt = tw.disabledUntil;
  guard = 4000;
  while (g.ticks < downAt && guard-- > 0) TC.tickGame(g, null);
  assert.ok(g.ticks >= downAt);
  assert.equal(tw.disabledUntil, downAt, 'still the same outage');
  assert.equal(tw.hp, 100, 'the tower is repaired when the outage ends');
});

test('leaks cost lives (brutes cost 2) and defeat ends the run', () => {
  const g = TC.createGame({ seed: 'leak', waves: 1 });
  TC.startWave(g);
  g.enemies.length = 0;
  g.spawnQueue = [];
  const brute = TC.spawnEnemy(g, 'brute');
  brute.t = TC.MAIN.total;                          /* already at the base */
  const before = g.lives;
  TC.tickGame(g, null);
  assert.equal(g.lives, before - 2, 'a brute leak costs 2 lives');
  assert.ok(brute.done);

  const doomed = run({ seed: 'doom', waves: 9, loadout: 'solo' }, null, (gg) => { gg.lives = 1; });
  const s = TC.runSummary(doomed);
  assert.equal(s.over, true);
  assert.equal(s.victory, false, 'a solo loadout with 1 life loses eventually');
});

test('decisions accumulate at volume and perSecond is measured in sim time', () => {
  const eng = recorder('nearest');
  const g = run(TC.DEFAULT_CONFIG, eng.choose);
  const s = TC.runSummary(g);
  assert.ok(s.decisions > 400, 'a 5-wave run is a decision at volume, got ' + s.decisions);
  assert.ok(eng.reqs.length > 0, 'the engine was asked at least once in a real run');
  assert.equal(s.engineCalls, eng.reqs.length, 'asks are counted exactly');
  assert.ok(s.rungs.engine <= s.engineCalls, 'honored answers are a subset of asks');
  assert.equal(s.rungs.rule + s.rungs.engine, s.decisions, 'every decision lands on a rung');
  const expect = +(s.decisions / (s.ticks / TC.TICKS_PER_S)).toFixed(2);
  assert.ok(Math.abs(s.perSecond - expect) < 0.02, 'perSecond = decisions / sim seconds');
});

test('config and stamp: stamps separate seed, waves, diversion and loadout', () => {
  const a = TC.towerStamp(TC.normalizeConfig({ seed: 'x' }));
  const b = TC.towerStamp(TC.normalizeConfig({ seed: 'y' }));
  const c = TC.towerStamp(TC.normalizeConfig({ seed: 'x', loadout: 'artillery' }));
  const d = TC.towerStamp(TC.normalizeConfig({ seed: 'x', diversion: false }));
  const e = TC.towerStamp(TC.normalizeConfig({ seed: 'x', waves: 9 }));
  assert.notEqual(a, b);
  assert.notEqual(a, c);
  assert.notEqual(a, d);
  assert.notEqual(a, e);
  assert.equal(a, TC.towerStamp(TC.normalizeConfig({ seed: 'x' })), 'stable');
});

test('loadouts are real presets with distinct shapes', () => {
  assert.equal(TC.LOADOUTS.standard.slots.length, 5);
  assert.equal(TC.LOADOUTS.artillery.dmg > TC.LOADOUTS.screen.dmg, true, 'artillery hits harder');
  assert.equal(TC.LOADOUTS.screen.rate < TC.LOADOUTS.artillery.rate, true, 'screen fires faster');
  assert.equal(TC.LOADOUTS.screen.specialSlot, -1, 'screen has no special');
  /* every preset slot index is a real tower slot and a real tower builds */
  for (const [name, spec] of Object.entries(TC.LOADOUTS)) {
    for (const i of spec.slots) assert.ok(i >= 0 && i < TC.TOWER_SLOTS.length, name + ' slot ' + i);
    const g = TC.createGame({ loadout: name });
    assert.equal(g.towers.length, spec.slots.length);
    const sp = g.towers.find((t) => t.special);
    assert.equal(!!sp, spec.specialSlot !== -1, name + ' special wiring');
  }
});

test('a no-bridge run never asks; a broken bridge asks and degrades to rules', () => {
  const none = TC.runSummary(run(TC.DEFAULT_CONFIG, null));
  assert.equal(none.engineCalls, 0, 'null bridge → no asks at all');
  assert.equal(none.victory, true, 'rules alone clear the default waves');

  const broken = run(TC.DEFAULT_CONFIG, () => null);
  const s = TC.runSummary(broken);
  assert.ok(s.engineCalls > 0, 'a bridge that answers null is still asked');
  assert.equal(s.rungs.engine, 0, 'null answers are never honored');
  assert.equal(s.victory, true, 'the run completes on rules');
});
