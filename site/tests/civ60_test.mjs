/* AI Civilization in 60 Seconds — core suite. Import order matters: the
   SDK first, then civ60-core; read the globals.

   Spec-pinned tests (DECISIONS-SDK-PLAN §5.13): a shortage event shifts
   the decide distribution toward FARM/EAT; population recovers or
   collapses coherently; determinism; batching budget.

   Run: node --test tests/civ60_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'civ60-core.js'));
const CC = globalThis.CivCore;

const recorder = (choice, outcome = 'accept') => {
  const reqs = [];
  return {
    reqs,
    choose: (req) => {
      reqs.push(req);
      if (outcome !== 'accept') return { outcome, answers: [] };
      return {
        outcome: 'accept',
        answers: [{ question_id: 'citizen-behavior', type: 'choice', choice,
          confidence: 0.7, distribution: { entries: [] } }],
      };
    },
  };
};

const runTicks = (g, seconds, engineChoose, perTick) => {
  for (let i = 0; i < seconds * CC.TICKS_PER_S; i++) {
    if (g.over) break;
    CC.tickGame(g, engineChoose);
    if (perTick) perTick(g, i);
  }
  return g;
};

const tallyShare = (tally, keys) => {
  const tot = Object.values(tally).reduce((a, b) => a + b, 0);
  const hit = keys.reduce((a, k) => a + (tally[k] || 0), 0);
  return tot ? hit / tot : 0;
};

test('config normalization clamps and pins the defaults', () => {
  assert.deepEqual(CC.normalizeConfig(null), CC.DEFAULT_CONFIG);
  const x = CC.normalizeConfig({ citizens: 999, askBudget: 50, events: 0, seed: 9 });
  assert.equal(x.citizens, 60);
  assert.equal(x.askBudget, 10);
  assert.equal(x.events, false);
  assert.equal(x.seed, '9');
  assert.equal(CC.normalizeConfig({ citizens: 0 }).citizens, 12, '0 falls back');
});

test('SPEC: a shortage shifts the decide distribution toward FARM/EAT', () => {
  // the cascade is front-loaded: measure the 30 s window after t=10
  const window = (withShortage) => {
    const g = CC.createGame(CC.DEFAULT_CONFIG);
    runTicks(g, 10);
    const before = JSON.parse(JSON.stringify(g.tally));
    if (withShortage) CC.injectEvent(g, 'shortage');
    runTicks(g, 30);
    const delta = {};
    for (const k of Object.keys(g.tally)) delta[k] = g.tally[k] - (before[k] || 0);
    return delta;
  };
  const base = tallyShare(window(false), ['farm', 'eat']);
  const short = tallyShare(window(true), ['farm', 'eat']);
  assert.ok(short > base,
    'shortage window farm+eat share ' + short.toFixed(3) +
    ' must beat base ' + base.toFixed(3));
  assert.ok(short - base > 0.02, 'the shift is substantial, got ' +
    (short - base).toFixed(3));
  const logged = CC.createGame(CC.DEFAULT_CONFIG);
  runTicks(logged, 10);
  CC.injectEvent(logged, 'shortage');
  assert.ok(logged.log.some((l) => l.kind === 'shortage'), 'the event is logged');
});

test('SPEC: population is coherent — pop = start + births + migrants − deaths − fled', () => {
  const g = CC.createGame(CC.DEFAULT_CONFIG);
  runTicks(g, 90);
  // stir the pot: migrants in, a raid through, births along the way
  CC.injectEvent(g, 'migrants');
  runTicks(g, 20);
  CC.injectEvent(g, 'raid');
  runTicks(g, 30);
  const s = CC.runSummary(g);
  const expect = CC.DEFAULT_CONFIG.citizens + s.births + s.migrants
    - s.deaths - s.fled;
  assert.equal(s.pop, expect,
    `pop ${s.pop} != start ${CC.DEFAULT_CONFIG.citizens} + births ${s.births} + migrants ${s.migrants} − deaths ${s.deaths} − fled ${s.fled}`);
  assert.equal(s.coherent, true);
  assert.ok(s.births > 0, 'the city grew under surplus, births = ' + s.births);
  assert.ok(s.homes > CC.START.homes, 'homes were built along the way');
});

test('SPEC: determinism — same seed, same city; new seed, new city', () => {
  const a = CC.runSummary(runTicks(CC.createGame(CC.DEFAULT_CONFIG), 60));
  const b = CC.runSummary(runTicks(CC.createGame(CC.DEFAULT_CONFIG), 60));
  assert.deepEqual(a, b);
  const c = CC.runSummary(runTicks(CC.createGame({ seed: 'det-2' }), 60));
  assert.notDeepEqual(a, c);
});

test('SPEC: the batching budget is respected — asks ≤ budget × sim seconds + 2', () => {
  const eng = recorder('farm');
  const g = CC.createGame({ askBudget: 1, citizens: 40, seed: 'budget' });
  runTicks(g, 60, eng.choose);
  const s = CC.runSummary(g);
  assert.ok(s.engineCalls <= 62,
    '40 citizens at ~1.3 Hz must not exceed ~1 ask/s, got ' + s.engineCalls);
  assert.ok(s.decisions > 40 * 40,
    'the city still decided thousands of times, got ' + s.decisions);
  const none = CC.runSummary(runTicks(CC.createGame({ askBudget: 0 }), 20,
    recorder('farm').choose));
  assert.equal(none.engineCalls, 0);
  assert.equal(none.rungs.engine, 0);
  assert.ok(none.rungs.rule > 0);
});

test('RULE: sleep is never a candidate — energy drain lands it as a rule', () => {
  const eng = recorder('farm');
  const g = CC.createGame({ seed: 'sleep', citizens: 8 });
  runTicks(g, 80, eng.choose);
  assert.ok(eng.reqs.length > 0, 'the engine was asked about something');
  for (const req of eng.reqs) {
    const ids = req.questions[0].candidates.map((c) => c.id);
    assert.ok(!ids.includes('sleep'), 'sleep is never on the wire');
    assert.ok(!ids.includes('have-child'), 'have-child is never on the wire');
    assert.ok(ids.every((id) => CC.BEHAVIORS.includes(id)),
      'candidates are all real contested behaviors');
  }
  const s = CC.runSummary(g);
  assert.ok((s.tally.sleep || 0) > 0, 'the sleep rule actually fired');
  assert.ok(s.rungs.rule > 0);
});

test('RULE: urgent eat (hunger ≥ 80, food on hand) never reaches the wire', () => {
  const g = CC.createGame({ seed: 'eat', citizens: 6 });
  g.food = 30;
  const c = g.citizens[0];
  c.hunger = 95; c.energy = 90;
  g.engineChoose = recorder('work').choose;
  g.tokens = 5;
  const r = CC.decide(g, c);
  assert.equal(r.behavior, 'eat');
  assert.equal(r.rung, 'rule', 'urgent eat is a rule, not a question');
  assert.equal(g.food, 29, 'the ration was taken');
  assert.ok(c.hunger < 95, 'hunger dropped');
  assert.equal(g.engineCalls, 0);
});

test('rationing eat IS a candidate in the thin-granary band (40 ≤ hunger < 80)', () => {
  const eng = recorder('eat');
  const g = CC.createGame({ seed: 'ration', citizens: 6 });
  g.food = 25; g.wood = 8; g.homes = 2;          /* no build (8 < 12): farm 600 vs eat 500 */
  for (const c of g.citizens) c.hunger = 60;     /* bellies 0.6 → farm 600, eat 500 */
  const c = g.citizens[0];
  c.energy = 90;
  g.engineChoose = eng.choose;
  g.tokens = 5;
  const r = CC.decide(g, c);
  assert.equal(eng.reqs.length, 1, 'farm 600 vs work 550 is a genuine tie');
  const cand = eng.reqs[0].questions[0].candidates.map((x) => x.id);
  assert.ok(cand.includes('eat'), 'eat is on the wire at hunger 60: ' + cand.join(','));
  assert.equal(r.behavior, 'eat', 'the honored answer executed');
  assert.equal(g.food, 24, 'the ration was taken');
  assert.equal(c.hunger, 15, 'hunger dropped 45');
});

test('the escalated request carries the FULL wire contract (the racer rule)', () => {
  const g = CC.createGame({ seed: 'wire', citizens: 6 });
  // flat ground: farm 450 vs work 475 — a 25-point gap inside TIE_EPS.
  // food 16 ≤ pop×3 keeps the have-child rule dormant; hunger 30 keeps
  // both urgent-eat and rationing-eat off the wire.
  for (const c of g.citizens) c.hunger = 30;
  const c = g.citizens[0];
  c.energy = 90; c.fear = 0;
  g.food = 16; g.wood = 10; g.homes = 6;         /* no homeless → no build */
  const eng = recorder('work');
  g.engineChoose = eng.choose;
  g.tokens = 1;
  const res = CC.decide(g, c);
  assert.equal(eng.reqs.length, 1, 'the near-tie went to the engine');
  const req = eng.reqs[0];
  assert.equal(typeof req.state.text, 'string');
  assert.ok(req.state.text.includes('Citizen c0'), 'the state names the citizen');
  assert.ok(req.state.text.includes('Hunger 30'));
  assert.deepEqual(req.state.facts, {}, 'facts is the tagged-enum empty object');
  assert.equal(req.questions.length, 1);
  assert.equal(req.questions[0].type, 'choice');
  assert.equal(req.questions[0].id, 'citizen-behavior');
  const ids = req.questions[0].candidates.map((x) => x.id);
  assert.ok(ids.length >= 2, 'a real choice, not a single option');
  assert.ok(req.questions[0].candidates.every((x) => x.description === CC.BEHAVIOR_DESC[x.id]));
  assert.deepEqual(req.policy,
    { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: 'low' });
  assert.equal(g.engineCalls, 1);
  if (ids.includes(res.behavior) && res.rung === 'engine') {
    assert.equal(res.behavior, 'work', 'the honored answer is executed');
  }
});

test('engine answers only count when they name a live candidate; abstain falls back', () => {
  const mk = () => {
    const g = CC.createGame({ seed: 'wire', citizens: 6 });
    for (const c of g.citizens) c.hunger = 30;
    const c = g.citizens[0];
    c.energy = 90;
    g.food = 16; g.wood = 10; g.homes = 6;
    g.tokens = 1;
    return g;
  };
  const bad = mk();
  bad.engineChoose = recorder('prayer').choose;
  const r1 = CC.decide(bad, bad.citizens[0]);
  assert.equal(r1.rung, 'rule', 'a non-candidate answer is ignored');
  assert.equal(bad.engineCalls, 1, 'the ask still happened');

  const abst = mk();
  abst.engineChoose = recorder(null, 'abstain').choose;
  const r2 = CC.decide(abst, abst.citizens[0]);
  assert.equal(r2.rung, 'rule', 'abstention falls back to the argmax');
  assert.equal(abst.engineCalls, 1);
});

test('RAID: fight rallies the guard line and repels; the raid is logged', () => {
  const g = CC.createGame({ seed: 'raid', citizens: 10 });
  CC.injectEvent(g, 'raid');
  assert.ok(g.raid, 'the raid started');
  runTicks(g, 25);
  const s = CC.runSummary(g);
  assert.equal(s.repelled, 1, 'the guard line repelled the raid, repelled = ' + s.repelled);
  assert.ok(g.log.some((l) => l.kind === 'raid' && l.note.includes('repelled')));
  assert.ok(g.citizens.every((c) => c.fear < 40), 'fear decayed after the raid');
});

test('a second raid while one runs is refused; events are logged with ticks', () => {
  const g = CC.createGame({ seed: 'raid2', citizens: 10 });
  assert.equal(CC.injectEvent(g, 'raid'), 'raid');
  assert.equal(CC.injectEvent(g, 'raid'), null, 'no double-raid');
  assert.equal(CC.injectEvent(g, 'volcano'), null, 'unknown events refused');
  runTicks(g, 1);
  const g2 = CC.createGame({ seed: 'log', citizens: 8 });
  runTicks(g2, 5);
  CC.injectEvent(g2, 'harvest');
  assert.ok(g2.log.length >= 1);
  assert.equal(g2.log[g2.log.length - 1].tick, 5 * CC.TICKS_PER_S);
  assert.ok(g2.log[g2.log.length - 1].note.includes('food'));
});

test('build → homes: construction raises homes while wood flows', () => {
  const g = CC.createGame({ seed: 'build', citizens: 12 });
  g.homes = 2;
  g.wood = 40;
  runTicks(g, 30);
  const s = CC.runSummary(g);
  assert.ok(s.homes > 2, 'homes were built, got ' + s.homes);
  assert.ok((s.tally.build || 0) > 0, 'build landed on the tally');
  assert.ok(g.wood >= 0 && s.coherent, 'the economy never went negative');
});

test('starvation kills coherently; the last death ends the run', () => {
  /* the whole floor-minimum city (4), no food, no energy: the sleep rule
     (checked first) keeps everyone from farming, and the starve clocks run
     out mid-sleep before the first farm decision lands (45 ticks > 30) */
  const g = CC.createGame({ seed: 'starve', citizens: 4 });
  g.food = 0;
  for (const c of g.citizens) {
    c.hunger = 100;
    c.energy = 0;
    c.starve = CC.STARVE_TICKS - 30;
  }
  runTicks(g, 3);
  const s = CC.runSummary(g);
  assert.equal(s.deaths, 4, 'starvation killed, deaths = ' + s.deaths);
  assert.equal(s.coherent, true, 'deaths stay in the identity');
  assert.equal(s.over, true, 'the last death ends the run');
  assert.ok(g.log.some((l) => l.kind === 'collapse'));
});

test('festival lifts the city; migrants arrive as migrants, not births', () => {
  const g = CC.createGame({ seed: 'fest', citizens: 8 });
  const c0 = g.citizens[0];
  c0.fear = 60; c0.energy = 20; c0.hunger = 50;
  CC.injectEvent(g, 'festival');
  assert.equal(c0.fear, 0, 'fear cleared');
  assert.equal(c0.energy, 40, 'energy lifted');
  assert.equal(c0.hunger, 40, 'hunger eased');
  assert.ok(g.festival > 0, 'the festival runs for a while');

  const before = CC.runSummary(g).pop;
  CC.injectEvent(g, 'migrants');
  const s = CC.runSummary(g);
  assert.equal(s.pop, before + CC.MIGRANTS);
  assert.equal(s.migrants, CC.MIGRANTS, 'migrants are their own term');
  assert.equal(s.births, 0, 'not laundered as births');
  assert.equal(s.coherent, true);
});

test('decisions accumulate at volume; perSecond ≈ citizens × decide-rate', () => {
  const eng = recorder('farm');
  const g = CC.createGame({ citizens: 30, seed: 'volume' });
  runTicks(g, 30, eng.choose);
  const s = CC.runSummary(g);
  assert.ok(s.decisions > 30 * 1.33 * 20,
    '30 citizens at ~1.33 Hz decide thousands of times, got ' + s.decisions);
  const expect = 30 * (CC.TICKS_PER_S / CC.DECIDE_EVERY);
  assert.ok(Math.abs(s.perSecond - expect) / expect < 0.15,
    'perSecond tracks citizens × rate, got ' + s.perSecond);
  const tot = Object.values(s.tally).reduce((a, b) => a + b, 0);
  assert.equal(tot, s.decisions, 'the tally and the decision count agree');
  assert.equal(s.rungs.rule + s.rungs.engine, s.decisions,
    'every decision lands on a rung');
});

test('history samples every 5 sim-seconds for the chart', () => {
  const g = CC.createGame({ seed: 'hist', citizens: 10 });
  runTicks(g, 32);
  assert.ok(g.history.length >= 6, 'samples at t=0,5,…,30, got ' + g.history.length);
  assert.deepEqual(g.history[0], { t: 5, food: g.history[0].food,
    wood: g.history[0].wood, homes: g.history[0].homes, pop: g.history[0].pop });
  assert.ok(g.history.every((h, i) => i === 0 || h.t > g.history[i - 1].t),
    'the series is time-ordered');
  assert.ok(g.history.every((h) => Number.isFinite(h.food) && Number.isFinite(h.wood)
    && h.pop > 0));
});

test('config and stamp: stamps separate every knob that changes the city', () => {
  const base = CC.civStamp(CC.normalizeConfig({ seed: 'x' }));
  assert.notEqual(base, CC.civStamp(CC.normalizeConfig({ seed: 'y' })));
  assert.notEqual(base, CC.civStamp(CC.normalizeConfig({ seed: 'x', citizens: 30 })));
  assert.notEqual(base, CC.civStamp(CC.normalizeConfig({ seed: 'x', askBudget: 4 })));
  assert.notEqual(base, CC.civStamp(CC.normalizeConfig({ seed: 'x', events: false })));
  assert.equal(base, CC.civStamp(CC.normalizeConfig({ seed: 'x' })), 'stable');
});

test('a no-bridge run works on rules alone and stays coherent', () => {
  const g = CC.createGame(CC.DEFAULT_CONFIG);
  runTicks(g, 45);
  const s = CC.runSummary(g);
  assert.equal(s.engineCalls, 0);
  assert.ok(s.rungs.rule > 0);
  assert.equal(s.coherent, true);
});
