/* Ant Colony — core suite. Import order matters: the SDK first, then
   antcolony-core; read the globals.

   Spec-pinned tests (DECISIONS-SDK-PLAN §5.12): a seeded run forms a trail
   (food-to-nest delivery rate rises); no ant ever ASKS about RETURN (or
   GATHER) — those are rules; the batching budget is respected; determinism.

   Run: node --test tests/antcolony_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'antcolony-core.js'));
const AC = globalThis.AntCore;

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
        answers: [{ question_id: 'ant-behavior', type: 'choice', choice,
          confidence: 0.7, distribution: { entries: [] } }],
      };
    },
  };
};

/* run for a number of SIM SECONDS with per-window gather probes */
const runFor = (cfg, seconds, engineChoose, probe) => {
  const g = AC.createGame(cfg);
  const ticks = seconds * AC.TICKS_PER_S;
  for (let i = 0; i < ticks; i++) {
    AC.tickGame(g, engineChoose);
    if (probe) probe(g, i);
  }
  return g;
};

test('config normalization clamps and pins the defaults', () => {
  const c = AC.normalizeConfig(null);
  assert.deepEqual(c, AC.DEFAULT_CONFIG);
  const x = AC.normalizeConfig({ ants: 9999, evaporation: 9, deposit: 99,
    foodPiles: 0, obstacles: 'nope', decideRate: 40, askBudget: -3, seed: 7 });
  assert.equal(x.ants, 1000, 'ants clamp to the 50–1000 spec band (floor 10)');
  assert.equal(x.evaporation, 0.95);
  assert.equal(x.deposit, 8);
  assert.equal(x.foodPiles, 3, '0 is falsy → default, then clamped');
  assert.equal(x.obstacles, 'none');
  assert.equal(x.decideRate, 4);
  assert.equal(x.askBudget, 0);
  assert.equal(x.seed, '7');
  assert.equal(AC.normalizeConfig({ ants: 0 }).ants, 120, '0 falls back to the default');
  assert.equal(AC.normalizeConfig({ diversion: false }).obstacles, 'none',
    'unknown keys are dropped, not smuggled in');
});

test('SPEC: determinism — same seed, same colony; new seed, new colony', () => {
  const a = AC.runSummary(runFor(AC.DEFAULT_CONFIG, 45, null));
  const b = AC.runSummary(runFor(AC.DEFAULT_CONFIG, 45, null));
  assert.deepEqual(a, b);
  const c = AC.runSummary(runFor({ seed: 'det-2' }, 45, null));
  assert.notDeepEqual(a, c);
});

test('SPEC: no ant ever asks about RETURN (or GATHER) — rules, not questions', () => {
  const eng = recorder('explore');
  const g = runFor(AC.DEFAULT_CONFIG, 50, eng.choose);
  assert.ok(eng.reqs.length > 0, 'the engine was asked about something');
  for (const req of eng.reqs) {
    const ids = req.questions[0].candidates.map((cd) => cd.id);
    assert.ok(!ids.includes('return'), 'RETURN is never a candidate: ' + ids);
    assert.ok(!ids.includes('gather'), 'GATHER is never a candidate: ' + ids);
    assert.ok(ids.every((id) => AC.BEHAVIORS.includes(id)),
      'escalated ids are exactly the contested behaviors');
  }
  /* and the rules actually fire as decisions */
  const summary = AC.runSummary(g);
  assert.ok(summary.rungs.rule > 0);
  assert.ok(g.ants.some((a) => a.behavior === 'return') || summary.delivered > 0
    || summary.gathers > 0, 'the RETURN/GATHER rules ran');
});

test('SPEC: the batching budget is respected — asks ≤ budget × sim seconds + 1', () => {
  const eng = recorder('explore');
  const g = runFor({ askBudget: 1, ants: 400, seed: 'budget' }, 60, eng.choose);
  const s = AC.runSummary(g);
  assert.ok(s.engineCalls <= 61,
    '400 ants at 1 Hz must not exceed ~1 ask/s, got ' + s.engineCalls);
  assert.ok(s.decisions > 400 * 45,
    'the colony still decided thousands of times, got ' + s.decisions);
  /* zero budget → zero asks, colony runs entirely on rules */
  const none = AC.runSummary(runFor({ askBudget: 0 }, 20, recorder('explore').choose));
  assert.equal(none.engineCalls, 0);
  assert.ok(none.rungs.engine === 0 && none.rungs.rule > 0);
});

test('SPEC: a seeded run forms a trail — food-to-nest delivery rate rises', () => {
  let first = 0;
  const g = runFor(AC.DEFAULT_CONFIG, 150, null, (gg, i) => {
    if (i === 50 * AC.TICKS_PER_S) first = gg.delivered;
  });
  const s = AC.runSummary(g);
  const early = first, late = s.delivered - first;
  assert.ok(s.delivered > 0, 'food actually reached the nest, got ' + s.delivered);
  assert.ok(s.trailMax > 0.5, 'a pheromone trail exists, max ' + s.trailMax);
  assert.ok(late > early,
    'the first 50 s is discovery, the trail-fed flow after beats it: '
    + early + ' then ' + late);
});

test('the escalated request carries the FULL wire contract (the racer rule)', () => {
  const g = AC.createGame({ seed: 'wire', ants: 10 });
  g.piles = [];                                   /* kill food sense → search = 300 */
  const ant = g.ants[0];
  ant.x = 200; ant.y = 200; ant.heading = 0; ant.carrying = false;
  /* pheromone 0.45 on both probe cells → follow-trail 360 vs explore 360 */
  g.pher[20 * AC.GW + 21] = 0.45;                 /* (21,20): ahead + right */
  g.pher[19 * AC.GW + 21] = 0.45;                 /* (21,19): left */
  const eng = recorder('explore');
  g.engineChoose = eng.choose;
  g.tokens = 1;
  const res = AC.decide(g, ant);
  assert.equal(eng.reqs.length, 1, 'the genuine tie went to the engine');
  assert.equal(res.rung, 'engine');
  assert.equal(res.behavior, 'explore', 'the honored answer is executed');
  const req = eng.reqs[0];
  assert.equal(typeof req.state.text, 'string');
  assert.ok(req.state.text.includes('Ant a0'), 'the state names the ant');
  assert.ok(req.state.text.includes('carrying nothing'));
  assert.deepEqual(req.state.facts, {}, 'facts is the tagged-enum empty object');
  assert.equal(req.questions.length, 1);
  assert.equal(req.questions[0].type, 'choice');
  assert.equal(req.questions[0].id, 'ant-behavior');
  const ids = req.questions[0].candidates.map((c) => c.id);
  assert.ok(ids.includes('follow-trail') && ids.includes('explore') && ids.includes('search'),
    'the full contested set is on the wire: ' + ids.join(','));
  assert.ok(req.questions[0].candidates.every((c) => c.description === AC.BEHAVIOR_DESC[c.id]));
  assert.deepEqual(req.policy,
    { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: 'low' });
  assert.equal(g.engineCalls, 1);
  assert.equal(g.rungs.engine, 1);
  assert.ok(res.evGap <= AC.TIE_EPS, 'contested gap inside TIE_EPS, got ' + res.evGap);
});

test('engine answers only count when they name a live candidate; abstain falls back', () => {
  const mk = () => {
    const g = AC.createGame({ seed: 'wire', ants: 10 });
    g.piles = [];
    const ant = g.ants[0];
    ant.x = 200; ant.y = 200; ant.heading = 0;
    g.pher[20 * AC.GW + 21] = 0.45;
    g.pher[19 * AC.GW + 21] = 0.45;
    g.engineChoose = recorder('dance').choose;     /* not a candidate */
    g.tokens = 1;
    return g;
  };
  const bad = mk();
  const r1 = AC.decide(bad, bad.ants[0]);
  assert.equal(r1.rung, 'rule', 'a non-candidate answer is ignored');
  assert.equal(bad.engineCalls, 1, 'the ask still happened');
  assert.equal(r1.behavior, 'follow-trail', 'argmax wins the stable tie');

  const abst = mk();
  abst.engineChoose = recorder(null, 'abstain').choose;
  const r2 = AC.decide(abst, abst.ants[0]);
  assert.equal(r2.rung, 'rule', 'abstention falls back to the argmax');
  assert.equal(r2.behavior, 'follow-trail');
});

test('no token, no ask: an exhausted budget decides by rule and says so', () => {
  const g = AC.createGame({ seed: 'wire', ants: 10 });
  g.piles = [];
  const ant = g.ants[0];
  ant.x = 200; ant.y = 200; ant.heading = 0;
  g.pher[20 * AC.GW + 21] = 0.45;
  g.pher[19 * AC.GW + 21] = 0.45;
  const eng = recorder('explore');
  g.engineChoose = eng.choose;
  g.tokens = 0;                                    /* budget spent */
  const res = AC.decide(g, ant);
  assert.equal(eng.reqs.length, 0, 'no ask without a token');
  assert.equal(res.rung, 'rule');
  assert.equal(res.behavior, 'follow-trail');
  assert.equal(g.engineCalls, 0);
});

test('RULE: carrying → RETURN without asking; food underfoot → GATHER without asking', () => {
  const g = AC.createGame({ seed: 'rules', ants: 5 });
  g.engineChoose = recorder('explore').choose;
  g.tokens = 5;
  const ant = g.ants[0];

  ant.carrying = true;
  const r1 = AC.decide(g, ant);
  assert.equal(r1.behavior, 'return');
  assert.equal(r1.rung, 'rule');

  ant.carrying = false;
  ant.x = g.piles[0].x; ant.y = g.piles[0].y;      /* stand on the pile */
  const r2 = AC.decide(g, ant);
  assert.equal(r2.behavior, 'gather');
  assert.equal(r2.rung, 'rule');
  assert.equal(ant.carrying, true, 'the food is picked up');
  assert.equal(g.gathers, 1);
  assert.ok(ant.heading !== undefined, 'the ant turned toward home');
  assert.equal(g.rungs.rule, 2);
  assert.equal(g.engineCalls, 0, 'neither rule needed the engine');
  assert.equal(g.rungs.engine, 0);
});

test('RULE: a depleted pile stops yielding', () => {
  const g = AC.createGame({ seed: 'deplet', ants: 4 });
  const pile = g.piles[0];
  pile.amount = 1;
  const ant = g.ants[0];
  ant.x = pile.x; ant.y = pile.y;
  AC.decide(g, ant);
  assert.equal(ant.carrying, true);
  assert.equal(pile.amount, 0);
  const ant2 = g.ants[1];
  ant2.x = pile.x; ant2.y = pile.y; ant2.carrying = false;
  const r = AC.decide(g, ant2);
  assert.notEqual(r.behavior, 'gather', 'an empty pile is not food');
  assert.equal(g.gathers, 1);
});

test('DEFEND: the spider nearby puts defend on the wire; a quorum makes it flee', () => {
  const g = AC.createGame({ seed: 'spider', ants: 30 });
  g.piles = [];
  const ant = g.ants[0];
  ant.x = AC.NEST.x; ant.y = AC.NEST.y - 20;       /* near nest → defend scores */
  ant.heading = 0;
  g.spider.x = ant.x + 30; g.spider.y = ant.y;     /* inside THREAT_RADIUS */
  const b1 = g.ants[1], b2 = g.ants[2];
  for (const b of [b1, b2]) { b.x = g.spider.x + 10; b.y = g.spider.y; b.carrying = false; }
  const eng = recorder('defend');
  g.engineChoose = eng.choose;
  g.tokens = 5;
  const res = AC.decide(g, ant);
  assert.ok(eng.reqs.length === 1 || res.behavior === 'defend',
    'defend was either asked about or chosen');
  if (eng.reqs.length) {
    const ids = eng.reqs[0].questions[0].candidates.map((c) => c.id);
    assert.ok(ids.includes('defend'), 'defend is on the wire under threat: ' + ids.join(','));
  }
  /* quorum: three defenders inside the ring → the spider bolts */
  g.ants.forEach((a, i) => {
    a.behavior = i < 3 ? 'defend' : 'search';
    a.x = g.spider.x + (i % 3) * 5; a.y = g.spider.y;
  });
  g.spider.fleeUntil = 0;
  AC.tickGame(g, null);
  assert.ok(g.spider.fleeUntil > g.ticks, 'the spider fled the quorum');
  assert.equal(g.spiderFled, 1);
});

test('pheromone physics: deposit raises, evaporation decays, diffusion spreads', () => {
  const g = AC.createGame({ seed: 'pher', ants: 4 });
  const ant = { x: 205, y: 205, behavior: 'return' };
  AC.deposit(g, ant, 1);                           /* one second of laying */
  const [cx, cy] = [20, 20];
  const i = cy * AC.GW + cx;
  const laid = g.pher[i];
  assert.ok(laid >= 1.0, 'deposit = rate × dt, got ' + laid);

  g.pher[i] = 2;
  AC.ageField(g, 1);                               /* one second of aging */
  assert.ok(Math.abs(g.pher[i] - 1) < 0.2, 'evaporation 0.5/s halves it, got ' + g.pher[i]);

  g.pher.fill(0);
  g.pher[i] = 1;
  g.diffuseIn = 0;                                 /* force the diffusion pass */
  AC.ageField(g, 0.01);
  const right = g.pher[cy * AC.GW + cx + 1];
  const down = g.pher[(cy + 1) * AC.GW + cx];
  assert.ok(right > 0, 'diffusion reaches the right neighbour, got ' + right);
  assert.ok(down > 0, 'diffusion reaches the neighbour below, got ' + down);
  assert.ok(g.pher[i] < 1, 'the source cell gave some up');
});

test('obstacles: the wall blocks movement and ants bounce off it', () => {
  const g = AC.createGame({ seed: 'wall', obstacles: 'wall', ants: 20 });
  let hits = 0;
  for (let cy = 0; cy < AC.GH; cy++) {
    if (g.blocked[cy * AC.GW + Math.floor(AC.GW / 2)]) hits++;
    if (g.blocked[cy * AC.GW + Math.floor(AC.GW / 2) + 1]) hits++;
  }
  assert.ok(hits > 30, 'the wall is a real column of blocked cells, got ' + hits);
  const gap = [];
  for (let cy = 0; cy < AC.GH; cy++) {
    if (!g.blocked[cy * AC.GW + Math.floor(AC.GW / 2)]) gap.push(cy);
  }
  assert.equal(gap.length, AC.GH - hits / 2, 'exactly one gap row band');

  /* march an ant straight into the wall: it never ends a tick inside it */
  const ant = g.ants[0];
  ant.x = (AC.GW / 2 - 2) * AC.CELL; ant.y = 10 * AC.CELL;
  ant.heading = 0; ant.behavior = 'explore';
  const dt = 1 / AC.TICKS_PER_S;
  for (let i = 0; i < 200; i++) AC.moveAnt(g, ant, dt);
  const [cx, cy] = [Math.floor(ant.x / AC.CELL), Math.floor(ant.y / AC.CELL)];
  assert.equal(g.blocked[cy * AC.GW + cx], 0, 'the ant is never inside the wall');
});

test('decisions accumulate at volume; perSecond ≈ ants × decideRate', () => {
  const eng = recorder('explore');
  const g = runFor({ ants: 120, decideRate: 2, seed: 'volume' }, 30, eng.choose);
  const s = AC.runSummary(g);
  assert.ok(s.decisions > 120 * 2 * 25,
    '120 ants at 2 Hz for 30 s decide thousands of times, got ' + s.decisions);
  const expect = 120 * 2;
  assert.ok(Math.abs(s.perSecond - expect) / expect < 0.15,
    'perSecond tracks ants × rate, got ' + s.perSecond);
  assert.equal(s.rungs.rule + s.rungs.engine, s.decisions,
    'every decision lands on a rung');
  assert.ok(s.engineCalls <= 30 * 2 + 1, 'default budget holds at 2/s');
});

test('rungs account for honors, not asks; summary fields are honest', () => {
  const eng = recorder('nonexistent-behavior');
  const g = runFor({ seed: 'honest', ants: 40 }, 25, eng.choose);
  const s = AC.runSummary(g);
  assert.ok(s.engineCalls > 0, 'asks happened');
  assert.equal(s.rungs.engine, 0, 'a garbage answer never becomes an engine rung');
  assert.equal(s.rungs.rule, s.decisions);
  assert.equal(s.ticks, 25 * AC.TICKS_PER_S);
  assert.equal(s.simSeconds, 25);
  assert.ok(typeof s.trailMax === 'number' && s.trailMax >= 0);
});

test('config and stamp: stamps separate every knob that changes the colony', () => {
  const base = AC.colonyStamp(AC.normalizeConfig({ seed: 'x' }));
  assert.notEqual(base, AC.colonyStamp(AC.normalizeConfig({ seed: 'y' })));
  assert.notEqual(base, AC.colonyStamp(AC.normalizeConfig({ seed: 'x', ants: 300 })));
  assert.notEqual(base, AC.colonyStamp(AC.normalizeConfig({ seed: 'x', evaporation: 0.7 })));
  assert.notEqual(base, AC.colonyStamp(AC.normalizeConfig({ seed: 'x', deposit: 4 })));
  assert.notEqual(base, AC.colonyStamp(AC.normalizeConfig({ seed: 'x', foodPiles: 5 })));
  assert.notEqual(base, AC.colonyStamp(AC.normalizeConfig({ seed: 'x', obstacles: 'wall' })));
  assert.notEqual(base, AC.colonyStamp(AC.normalizeConfig({ seed: 'x', decideRate: 2 })));
  assert.notEqual(base, AC.colonyStamp(AC.normalizeConfig({ seed: 'x', askBudget: 4 })));
  assert.equal(base, AC.colonyStamp(AC.normalizeConfig({ seed: 'x' })), 'stable');
});

test('a no-bridge run works on rules alone; food piles are seeded and finite', () => {
  const s = AC.runSummary(runFor(AC.DEFAULT_CONFIG, 40, null));
  assert.equal(s.engineCalls, 0);
  assert.ok(s.rungs.rule > 0);
  assert.ok(s.decisions > 0);

  const piles = AC.createGame({ seed: 'same' }).piles;
  const again = AC.createGame({ seed: 'same' }).piles;
  assert.deepEqual(piles, again, 'pile placement is seeded');
  assert.ok(piles.length >= 1 && piles.length <= 8);
  assert.ok(piles.every((p) => p.amount > 0 && p.amount <= 400));
  assert.ok(piles.every((p) => p.cy <= 36),
    'piles spawn north of the nest band');
});
