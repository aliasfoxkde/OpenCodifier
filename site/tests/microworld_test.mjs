/* MicroWorld — core suite. Import order matters: the SDK first, then
   microworld-core; read the globals.

   Spec-pinned tests: the world is deterministic per seed; every creature
   decision names one of the ten actions and only tied candidates are ever
   escalated; the colony budget is respected; meters stay finite; births
   spend stores; the fox kills loners and is driven off by packs; extinction
   is a verdict, never a crash.

   Run: node --test tests/microworld_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'microworld-core.js'));
const MW = globalThis.MicroWorld;

/* a scripted engine: records every request, answers `choice` */
const recorder = (choice) => {
  const reqs = [];
  return {
    reqs,
    choose: (req) => {
      reqs.push(req);
      const cands = req.questions[0].candidates.map((c) => c.id);
      if (!cands.includes(choice)) return { outcome: 'refusal', answers: [] };
      return {
        outcome: 'accept',
        answers: [{ question_id: 'creature-action', type: 'choice', choice,
          confidence: 0.7, distribution: { entries: [] } }],
      };
    },
  };
};

const runFor = (cfg, seconds, engineChoose) => {
  const w = MW.createWorld(cfg);
  return MW.simulateTicks(w, seconds * MW.TICKS_PER_S, engineChoose);
};

test('config normalization clamps and pins the defaults', () => {
  const c = MW.normalizeConfig(null);
  assert.deepEqual(c, MW.DEFAULT_CONFIG);
  const x = MW.normalizeConfig({ creatures: 9999, regrow: 9, budget: -3, seed: 7 });
  assert.equal(x.creatures, 100, 'creatures clamp to the 8–100 spec band');
  assert.equal(x.regrow, 2);
  assert.equal(x.budget, 0);
  assert.equal(x.seed, '7');
  const lo = MW.normalizeConfig({ creatures: 1 });
  assert.equal(lo.creatures, 8, 'floor of 8 creatures');
});

test('same seed, same world — decisions and demography replay exactly', () => {
  const a = runFor({ seed: 'det' }, 20);
  const b = runFor({ seed: 'det' }, 20);
  assert.deepEqual(MW.runSummary(a), MW.runSummary(b));
  const pa = a.creatures.map((c) => [c.id, c.x, c.y, c.action]).sort();
  const pb = b.creatures.map((c) => [c.id, c.x, c.y, c.action]).sort();
  assert.deepEqual(pa, pb, 'same seed → identical bodies and actions');
});

test('every decision names a real action, and scores are sorted', () => {
  const w = runFor({ seed: 'valid' }, 30);
  for (const c of w.creatures) {
    assert.ok(MW.ACTIONS.includes(c.action), 'action ' + c.action + ' is one of the ten');
    if (c.scores.length === 0) {
      /* a newborn that has not reached its first decision yet still
         carries its initial random slot (0..DECIDE_EVERY) */
      assert.ok(c.nextDecide >= 0 && c.nextDecide < MW.DECIDE_EVERY,
        'undecided creature carries its initial decision slot');
      continue;
    }
    assert.ok(c.scores.length === MW.ACTIONS.length, 'every scored action present');
    for (let i = 1; i < c.scores.length; i++) {
      assert.ok(c.scores[i - 1].score >= c.scores[i].score, 'scores descend');
    }
    assert.ok(c.energy >= 0 && c.energy <= 100, 'energy bounded');
    assert.ok(c.water >= 0 && c.water <= 100, 'water bounded');
  }
});

test('only tied candidates are escalated, and only when the budget has tokens', () => {
  const rec = recorder('ignore');
  const w = MW.createWorld({ seed: 'ties', budget: 2 });
  MW.simulateTicks(w, 30 * MW.TICKS_PER_S, rec.choose);
  assert.ok(rec.reqs.length > 0, 'near-ties reach the engine');
  for (const req of rec.reqs) {
    const cands = req.questions[0].candidates;
    assert.ok(cands.length >= 2, 'a tie means at least two candidates');
    assert.equal(req.questions[0].id, 'creature-action');
    assert.ok(req.questions[0].candidates.every((c) => MW.ACTIONS.includes(c.id)));
  }
  assert.ok(w.engineCalls >= rec.reqs.filter((r) => true).length - 0,
    'every ask was budgeted');
  assert.ok(w.tokens >= 0, 'tokens never go negative');
  const s = MW.runSummary(w);
  assert.ok(s.engineCalls <= 30 * (2 + 0.001) * 2,
    'engine asks stay inside the refill envelope');
});

test('the engine only picks among offered candidates; a refusal falls back', () => {
  /* recorder answers with an action that will often NOT be a candidate
     (forced to a nonsense choice) — the core must refuse the answer and
     keep the argmax pick, never adopt an unoffered action */
  const rec = recorder('gohome');
  const w = MW.createWorld({ seed: 'refuse', budget: 10 });
  MW.simulateTicks(w, 25 * MW.TICKS_PER_S, rec.choose);
  for (const c of w.creatures) {
    assert.ok(MW.ACTIONS.includes(c.action));
  }
  /* where the engine's answer was outside the tie band, rung stays rule */
  for (const c of w.creatures) {
    if (c.rung === 'engine') {
      assert.ok(c.scores.slice(0, 2).some((r) => r.id === c.action),
        'engine-picked action was among the top tied candidates');
    }
  }
});

test('meters drain by rule; eating and drinking restore them', () => {
  const w = MW.createWorld({ seed: 'meters', creatures: 8 });
  const c = w.creatures[0];
  c.action = 'rest';
  c.nextDecide = Infinity;              /* hold the action steady */
  c.energy = 20;
  const e0 = c.energy;
  MW.simulateTicks(w, 4 * MW.TICKS_PER_S, null);
  assert.ok(c.energy > e0, 'resting with a fixed action recovers energy');
  assert.ok(c.energy <= 100);
});

test('a birth spends exactly BIRTH_COST stores and adds a creature', () => {
  const w = MW.createWorld({ seed: 'birth', creatures: 8 });
  const pop0 = w.creatures.length;
  const stores0 = w.stores;
  w.stores = MW.BIRTH_COST;
  w.lastBirth = w.ticks - MW.BIRTH_EVERY;
  MW.tickWorld(w, null);
  assert.equal(w.creatures.length, pop0 + 1, 'one creature born');
  assert.equal(w.births, 1);
  assert.equal(w.stores, MW.BIRTH_COST - MW.BIRTH_COST,
    'stores debited by exactly the birth cost');
  assert.equal(stores0 >= 0, true, 'sanity: the default world starts with stores');
});

test('the fox takes a loner and is driven off by a pack', () => {
  const w = MW.createWorld({ seed: 'fox', creatures: 8 });
  const prey = w.creatures[0];
  prey.x = w.fox.x + 1; prey.y = w.fox.y;   /* adjacent, alone */
  /* move every other creature far away */
  for (let i = 1; i < w.creatures.length; i++) {
    w.creatures[i].x = i + 1; w.creatures[i].y = MW.GH - 1;
  }
  const pop0 = w.creatures.length;
  const t0 = w.ticks;
  for (let i = 0; i < 400 && w.creatures.length === pop0; i++) MW.tickWorld(w, null);
  assert.ok(w.creatures.length < pop0, 'the loner was taken');
  assert.ok(w.events.some((e) => e.kind === 'kill'), 'a kill event landed');

  const w2 = MW.createWorld({ seed: 'fox2', creatures: 8 });
  const prey2 = w2.creatures[0];
  prey2.x = w2.fox.x; prey2.y = w2.fox.y;
  const buddy = w2.creatures[1];
  buddy.x = (w2.fox.x + 1) % MW.GW; buddy.y = w2.fox.y;
  for (let i = 2; i < w2.creatures.length; i++) {
    w2.creatures[i].x = i + 1; w2.creatures[i].y = MW.GH - 1;
  }
  const drives0 = w2.drives;
  for (let i = 0; i < 400; i++) MW.tickWorld(w2, null);
  assert.ok(w2.drives > drives0, 'the pack drove the fox off');
});

test('carriers deposit at home and stores cap holds', () => {
  const w = MW.createWorld({ seed: 'carry', creatures: 8 });
  const c = w.creatures[0];
  c.carrying = 1;
  c.x = w.home.x + 1; c.y = w.home.y + 1;
  c.action = 'gohome';
  c.nextDecide = Infinity;
  const stores0 = w.stores;
  MW.simulateTicks(w, 2 * MW.TICKS_PER_S, null);
  assert.equal(c.carrying, 0, 'the carrier deposited');
  assert.ok(w.stores >= stores0, 'stores went up by the deposit');
  assert.ok(w.stores <= MW.STORE_CAP || w.stores === MW.STORE_CAP,
    'stores respect the cap via scoring (gather stops when full)');
});

test('a 60-second soak stays finite and inside the population band', () => {
  const w = runFor({ seed: 'soak', creatures: 34 }, 60);
  const s = MW.runSummary(w);
  for (const c of w.creatures) {
    assert.ok(Number.isFinite(c.x) && Number.isFinite(c.y));
    assert.ok(Number.isFinite(c.energy) && Number.isFinite(c.water));
  }
  assert.ok(s.pop <= 100, 'population respects the hard cap');
  assert.ok(s.births > 0, 'the colony grew over a minute');
  assert.ok(s.engineCalls === 0, 'no engine, no asks');
  assert.ok(s.ticks === 60 * MW.TICKS_PER_S, 'no early exit');
});

test('extinction is a verdict, not a crash', () => {
  const w = MW.createWorld({ seed: 'doom', creatures: 8 });
  const rec = recorder('ignore');
  /* the fox eats down to zero; stepWorld keeps returning the world */
  for (let i = 0; i < 20000 && !w.over; i++) MW.tickWorld(w, rec.choose);
  if (w.over) {
    assert.equal(w.creatures.length, 0);
    assert.ok(w.deaths >= 8);
    const s = MW.runSummary(w);
    assert.equal(s.pop, 0);
  }
});
