/* Traffic Simulator — core suite. Import order matters: the SDK first,
   then traffic-core; read the globals.

   Spec-pinned tests (DECISIONS-SDK-PLAN §5.11): no gridlock deadlock
   under seeded spawns; emergency preemption fires; determinism; lights
   respect minimum-phase.

   Run: node --test tests/traffic_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'traffic-core.js'));
const TC = globalThis.TrafficCore;

const recorder = (choice, outcome = 'accept') => {
  const reqs = [];
  return {
    reqs,
    choose: (req) => {
      reqs.push(req);
      if (outcome !== 'accept') return { outcome, answers: [] };
      return {
        outcome: 'accept',
        answers: [{ question_id: 'light-phase', type: 'choice', choice,
          confidence: 0.7, distribution: { entries: [] } }],
      };
    },
  };
};

const runFor = (cfg, seconds, engineChoose, perTick) => {
  const g = TC.createGame(cfg);
  for (let i = 0; i < seconds * TC.TICKS_PER_S; i++) {
    TC.tickGame(g, engineChoose);
    if (perTick) perTick(g, i);
  }
  return g;
};

/* park n ghost cars on a lane behind (x, y) heading dir — fills occ so
   carsOnArm/queueCounts see the queue without running the physics */
function parkQueue(g, x, y, dir, n) {
  const [dx, dy] = TC.DIRS[dir];
  for (let i = 0; i < n; i++) {
    const cx = x - dx * i, cy = y - dy * i;
    g.occ[cy * TC.GW + cx] += 1;
    g.cars.push({ id: 'ghost' + i, x: cx, y: cy, dir, exitDir: dir,
      plannedFor: -1, state: 'stop', waitT: 0, age: 0,
      waitLogged: true, emergency: false });
  }
}

test('config normalization clamps and pins the defaults', () => {
  assert.deepEqual(TC.normalizeConfig(null), TC.DEFAULT_CONFIG);
  const x = TC.normalizeConfig({ spawnRate: 999, congestion: 7, askBudget: -3, seed: 9 });
  assert.equal(x.spawnRate, 90);
  assert.equal(x.congestion, 1);
  assert.equal(x.askBudget, 0);
  assert.equal(x.seed, '9');
  const z = TC.normalizeConfig({ spawnRate: 0, askBudget: 0 });
  assert.equal(z.spawnRate, 0, '0 spawn is a real config (empty roads)');
  assert.equal(z.askBudget, 0, '0 budget is a real config (rules-only)');
  assert.equal(TC.normalizeConfig({ congestion: 0 }).congestion, 0);
});

test('SPEC: determinism — same seed, same city; new seed, new city', () => {
  const a = TC.runSummary(runFor(TC.DEFAULT_CONFIG, 45, null));
  const b = TC.runSummary(runFor(TC.DEFAULT_CONFIG, 45, null));
  assert.deepEqual(a, b);
  const c = TC.runSummary(runFor({ seed: 'det-2' }, 45, null));
  assert.notDeepEqual(a, c);
});

test('SPEC: no gridlock deadlock — seeded spawns keep completing trips', () => {
  let midCompleted = 0;
  const g = runFor(TC.DEFAULT_CONFIG, 120, null, (gg, i) => {
    if (i === 60 * TC.TICKS_PER_S) midCompleted = gg.completed;
  });
  const s = TC.runSummary(g);
  assert.ok(s.completed > 20, 'cars complete trips, got ' + s.completed);
  assert.ok(s.throughput > 5, 'throughput above 5 cars/min, got ' + s.throughput);
  assert.ok(s.completed > midCompleted,
    'the second minute still moved: ' + midCompleted + ' → ' + s.completed);
  assert.ok(s.moving > 0, 'cars are moving at the end: ' + s.moving);
  assert.equal(s.waiting < s.cars || s.cars === 0, true, 'not everyone is stuck');
});

test('SPEC: lights respect minimum phase — no early switches without preemption', () => {
  const g = runFor(TC.DEFAULT_CONFIG, 90, null);
  const lastSwitch = {};
  for (const entry of g.log) {
    if (entry.kind !== 'light') continue;
    const id = entry.note.split(' ')[0];
    const preempt = entry.note.includes('preempt');
    if (lastSwitch[id] !== undefined && !preempt) {
      const gap = entry.tick - lastSwitch[id];
      assert.ok(gap >= TC.MIN_PHASE - TC.TICKS_PER_S,
        id + ' switched after ' + gap + ' ticks (< min phase)');
    }
    lastSwitch[id] = entry.tick;
  }
  assert.ok(Object.keys(lastSwitch).length > 0, 'lights actually switched');
});

test('SPEC: emergency preemption fires, runs the light, and clears', () => {
  const g = runFor(TC.DEFAULT_CONFIG, 10, null);   // warm traffic
  const spawnTick = g.ticks;
  const id = TC.injectEmergency(g);
  assert.ok(id, 'the emergency spawned');
  for (let i = 0; i < 30 * TC.TICKS_PER_S; i++) TC.tickGame(g, null);
  const s = TC.runSummary(g);
  const preemptTicks = g.log.filter((l) => l.kind === 'light'
    && l.note.includes('preempt')).map((l) => l.tick);
  assert.ok(preemptTicks.length > 0,
    'a red-arm light preempted for the emergency, log = '
      + JSON.stringify(g.log.filter((l) => l.kind === 'light').slice(0, 5)));
  assert.ok(preemptTicks[0] <= spawnTick + 14 * TC.TICKS_PER_S,
    'the first preempt lands inside the approach window, tick '
      + preemptTicks[0] + ' vs spawn ' + spawnTick);
  assert.equal(s.emergency, null, 'the emergency vehicle completed and cleared');
});

test('RULE: only lights ask — candidates are keep/switch, never stop/go/wait/turn', () => {
  const eng = recorder('keep');
  const g = runFor({ spawnRate: 30 }, 40, eng.choose);
  assert.ok(eng.reqs.length > 0, 'the engine was asked about phases');
  for (const req of eng.reqs) {
    assert.equal(req.questions[0].id, 'light-phase');
    const ids = req.questions[0].candidates.map((c) => c.id);
    assert.deepEqual(ids.sort(), ['keep', 'switch'],
      'only keep/switch on the wire: ' + ids);
  }
  const s = TC.runSummary(g);
  assert.ok((s.tally.stop || 0) > 0, 'the stop rule actually fired');
  assert.ok((s.tally.go || 0) > 0, 'the go rule actually fired');
  assert.ok(Object.keys(s.tally).some((k) => k.startsWith('turn-')),
    'turns were chosen by argmax');
  assert.ok(s.rungs.rule > 0);
});

test('the escalated request carries the FULL wire contract', () => {
  const g = TC.createGame({ seed: 'wire' });
  const light = g.lights[0];
  light.t = TC.MIN_PHASE + TC.LIGHT_DECIDE;       // scored band, past min
  parkQueue(g, light.vx, light.hy - 1, 2, 3);     // 3 southbound on green
  parkQueue(g, light.vx + 4, light.hy + 1, 1, 2); // 2 eastbound on red, clear of the box
  const eng = recorder('switch');
  g.engineChoose = eng.choose;
  g.tokens = 5;
  const q = TC.queueCounts(g, light);
  assert.equal(q.inX, 0, 'no ghosts inside the box');
  assert.ok(Math.abs((300 + 40 * q.onGreen) - (300 + 40 * q.onRed)) <= TC.TIE_EPS,
    'the fixture is a genuine near-tie: ' + JSON.stringify(q));
  TC.decideLight(g, light);
  assert.equal(eng.reqs.length, 1, 'the near-tie went to the engine');
  const req = eng.reqs[0];
  assert.equal(typeof req.state.text, 'string');
  assert.ok(req.state.text.includes(light.id), 'the state names the light');
  assert.ok(req.state.text.includes('Phase NS'), 'the state names the phase');
  assert.deepEqual(req.state.facts, {}, 'facts is the tagged-enum empty object');
  assert.equal(req.questions[0].type, 'choice');
  assert.equal(req.questions[0].id, 'light-phase');
  assert.ok(req.questions[0].candidates.every((c) => c.description.length > 10));
  assert.deepEqual(req.policy,
    { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: 'low' });
  assert.equal(g.engineCalls, 1);
  assert.equal(light.phase, 'EW', 'the honored switch executed');
});

test('engine answers only count when they name a live candidate; abstain falls back', () => {
  const mk = () => {
    const g = TC.createGame({ seed: 'wire' });
    const light = g.lights[0];
    light.t = TC.MIN_PHASE + TC.LIGHT_DECIDE;
    parkQueue(g, light.vx, light.hy - 1, 2, 3);
    parkQueue(g, light.vx + 4, light.hy + 1, 1, 2);
    g.tokens = 5;
    return { g, light };
  };
  const bad = mk();
  bad.g.engineChoose = recorder('flash-yellow').choose;   // not a candidate
  TC.decideLight(bad.g, bad.light);
  assert.equal(bad.g.engineCalls, 1, 'the ask still happened');
  assert.equal(bad.g.rungs.engine, 0, 'a non-candidate answer is ignored');
  assert.equal(bad.light.phase, 'NS', 'the argmax (keep, 420 vs 380) held');

  const abst = mk();
  abst.g.engineChoose = recorder(null, 'abstain').choose;
  TC.decideLight(abst.g, abst.light);
  assert.equal(abst.g.engineCalls, 1);
  assert.equal(abst.g.rungs.engine, 0, 'abstention falls back to the argmax');
  assert.equal(abst.light.phase, 'NS');
});

test('SPEC: the batching budget is respected — asks ≤ budget × sim seconds + 2', () => {
  const eng = recorder('keep');
  const g = runFor({ askBudget: 1, spawnRate: 30, seed: 'budget' }, 60, eng.choose);
  const s = TC.runSummary(g);
  assert.ok(s.engineCalls <= 62,
    '12 lights at 1 Hz must not exceed ~1 ask/s, got ' + s.engineCalls);
  assert.ok(s.decisions > 600, 'lights and cars still decided, got ' + s.decisions);
  const none = TC.runSummary(runFor({ askBudget: 0 }, 20, recorder('keep').choose));
  assert.equal(none.engineCalls, 0);
  assert.equal(none.rungs.engine, 0);
  assert.ok(none.rungs.rule > 0);
});

test('REROUTE: the exit argmax reads the street ahead, and gridlock reroutes', () => {
  const mkCar = () => ({ id: 'cX', x: TC.VX[0], y: TC.HY[0] - 1, dir: 2,
    exitDir: 2, plannedFor: -1, state: 'cruise', waitT: 0, age: 0,
    waitLogged: false, emergency: false });
  const jam = (g, n) => {   // downstream of every exit: straight, left(E), right(W)
    parkQueue(g, TC.VX[0], TC.HY[0] + 1 + n, 2, n);
    parkQueue(g, TC.VX[0] + 1 + n, TC.HY[0] + 1, 1, n);
    parkQueue(g, TC.VX[0] - n, TC.HY[0], 3, n);
  };
  const li = TC.lightIndexAt(TC.VX[0], TC.HY[0]);

  // gridlock everywhere: all arms tie at -60, straight wins the order
  // tiebreak, sees its own queue >= REROUTE_QUEUE, flips to a side street
  const grid = TC.createGame({ seed: 'reroute' });
  jam(grid, TC.REROUTE_QUEUE + 1);
  const car = mkCar();
  TC.decideTurn(grid, car, li);
  assert.equal(car.exitDir, 1, 'the reroute picked a side street');
  assert.equal(grid.tally.reroute, 1, 'the reroute was recorded');

  // a jammed straight with open sides is an ordinary turn, not a reroute:
  // the argmax already refuses the jammed arm
  const jammed = TC.createGame({ seed: 'reroute-jam' });
  parkQueue(jammed, TC.VX[0], TC.HY[0] + 1 + 6, 2, 6);
  const car2 = mkCar();
  TC.decideTurn(jammed, car2, li);
  assert.notEqual(car2.exitDir, 2, 'the car refused the jammed straight arm');
  assert.equal(jammed.tally.reroute, undefined);
  assert.equal(jammed.tally['turn-left'], 1);

  // open grid: straight wins on the tiebreak-free argmax
  const clear = TC.createGame({ seed: 'reroute2' });
  const car3 = mkCar();
  TC.decideTurn(clear, car3, li);
  assert.equal(car3.exitDir, 2, 'an open straight arm wins normally');
  assert.equal(clear.tally['turn-straight'], 1);
});

test('minimum phase rule: a light under MIN never switches, even at 0 gap', () => {
  const g = TC.createGame({ seed: 'minphase' });
  const light = g.lights[0];
  light.t = 0;                                    // fresh phase
  parkQueue(g, light.vx, light.hy - 1, 2, 4);     // big queues either way
  parkQueue(g, light.vx + 2, light.hy + 1, 1, 4);
  const eng = recorder('switch');
  g.engineChoose = eng.choose;
  g.tokens = 5;
  TC.decideLight(g, light);
  assert.equal(eng.reqs.length, 0, 'a fresh phase is never a question');
  assert.equal(light.phase, 'NS', 'the phase held through minimum');
  assert.equal(g.tally.keep, 1);
});

test('decisions accumulate at volume; the ledger stays consistent', () => {
  const g = runFor({ spawnRate: 40, seed: 'volume' }, 45, null);
  const s = TC.runSummary(g);
  assert.ok(s.decisions > 300, '45 busy seconds decide plenty, got ' + s.decisions);
  const tot = Object.values(s.tally).reduce((a, b) => a + b, 0);
  assert.equal(tot, s.decisions, 'the tally and the decision count agree');
  assert.equal(s.rungs.rule + s.rungs.engine, s.decisions,
    'every decision lands on a rung');
  assert.ok(s.waitP90 >= s.waitP50, 'p90 ≥ p50: ' + s.waitP50 + '/' + s.waitP90);
  assert.ok(s.avgTripS > 0, 'trips take time, got ' + s.avgTripS);
});

test('history samples every 5 sim-seconds for the chart', () => {
  const g = runFor({ seed: 'hist' }, 32, null);
  assert.ok(g.history.length >= 6, 'samples at t=0,5,…,30, got ' + g.history.length);
  assert.deepEqual(g.history[0], { t: 5, cars: g.history[0].cars,
    completed: g.history[0].completed, moving: g.history[0].moving,
    p50: g.history[0].p50 });
  assert.ok(g.history.every((h, i) => i === 0 || h.t > g.history[i - 1].t));
});

test('congestion slows the cruise cadence measurably', () => {
  assert.equal(TC.moveEvery(TC.normalizeConfig({ congestion: 0 })), 10);
  assert.equal(TC.moveEvery(TC.normalizeConfig({ congestion: 1 })), 18);
  const free = TC.runSummary(runFor({ spawnRate: 6, congestion: 0, seed: 'c0' }, 60, null));
  const jam = TC.runSummary(runFor({ spawnRate: 6, congestion: 1, seed: 'c0' }, 60, null));
  assert.ok(jam.avgTripS > free.avgTripS,
    'the same seed crawls under congestion: ' + free.avgTripS + 's → ' + jam.avgTripS + 's');
});

test('config and stamp: stamps separate every knob that changes the city', () => {
  const base = TC.trafficStamp(TC.normalizeConfig({ seed: 'x' }));
  assert.notEqual(base, TC.trafficStamp(TC.normalizeConfig({ seed: 'y' })));
  assert.notEqual(base, TC.trafficStamp(TC.normalizeConfig({ seed: 'x', spawnRate: 30 })));
  assert.notEqual(base, TC.trafficStamp(TC.normalizeConfig({ seed: 'x', congestion: 0.8 })));
  assert.notEqual(base, TC.trafficStamp(TC.normalizeConfig({ seed: 'x', askBudget: 4 })));
  assert.equal(base, TC.trafficStamp(TC.normalizeConfig({ seed: 'x' })), 'stable');
});

test('a no-bridge run works on rules alone and keeps flowing', () => {
  const s = TC.runSummary(runFor(TC.DEFAULT_CONFIG, 45, null));
  assert.equal(s.engineCalls, 0);
  assert.ok(s.rungs.rule > 0);
  assert.ok(s.completed > 0, 'rules-only traffic still completes trips');
});
