/* GridWars — core suite. Import order matters: the SDK first, then
   gridwars-core; read the globals.

   Spec-pinned: same seed = same spawn sequence and same match; tiers
   escalate on score and time and gate the roster; each enemy type's rule
   ladder picks the expected tag in a crafted situation; splitters leave
   children; weapon params change the sim; particle counts are exact; the
   multiplier is geoms; a bomb clears the field for nothing; the engine gets
   exactly one kind of question and falls back cleanly.

   Run: node --test tests/gridwars_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'gridwars-core.js'));
const GC = globalThis.GridWarsCore;

const TYPE = GC.ENEMY_TYPES;
const TAU = Math.PI * 2;

/* a quiet arena: no spawns, no shield, nothing on the field */
function quiet(cfg) {
  const g = GC.createGame(cfg);
  g.spawnIn = 1e9;                 /* wave composition stays, spawns stop */
  g.enemies.length = 0;
  g.player.invuln = 0;
  return g;
}

const place = (g, type, x, y) => GC.spawnEnemy(g, type, x, y);
const tagOf = (g, e) => GC.enemyDecide(g, e);

/* ---------- config ---------- */

test('config normalization clamps and pins the defaults', () => {
  assert.deepEqual(GC.normalizeConfig(null), GC.DEFAULT_CONFIG);
  const c = GC.normalizeConfig({
    seed: 42, difficulty: 'nope', spawnRate: 99, maxEnemies: 1,
    shipSpeed: 9999, inertia: -3, lives: 100, invuln: 99,
    warp: 5, density: 1, bombs: 99,
  });
  assert.equal(c.seed, '42');
  assert.equal(c.difficulty, 'standard');
  assert.equal(c.spawnRate, 2);
  assert.equal(c.maxEnemies, 8);
  assert.equal(c.shipSpeed, 460);
  assert.equal(c.inertia, 0);
  assert.equal(c.lives, 9);
  assert.equal(c.invuln, 6);
  assert.equal(c.warp, 1);
  assert.equal(c.density, 24);
  assert.equal(c.bombs, 6);
});

test('weapon mods clamp to the published limits', () => {
  const w = GC.normalizeWeapons({
    pulse: { cooldown: 999, dmg: 99 },
    scatter: { count: 99, spread: 99 },
    wave: { r1: 9999 },
  });
  assert.equal(w.pulse.cooldown, GC.WEAPON_LIMITS.pulse.cooldown[1]);
  assert.equal(w.pulse.dmg, GC.WEAPON_LIMITS.pulse.dmg[1]);
  assert.equal(w.scatter.count, GC.WEAPON_LIMITS.scatter.count[1]);
  assert.equal(w.scatter.spread, GC.WEAPON_LIMITS.scatter.spread[1]);
  assert.equal(w.wave.r1, GC.WEAPON_LIMITS.wave.r1[1]);
  assert.deepEqual(GC.normalizeWeapons(null), (() => {
    const out = {};
    for (const k of GC.WEAPON_ORDER) {
      out[k] = {};
      for (const key of Object.keys(GC.WEAPON_LIMITS[k])) out[k][key] = GC.WEAPONS[k][key];
    }
    return out;
  })());
});

/* ---------- determinism ---------- */

test('same seed, same match — spawn sequence, decisions, particles', () => {
  const run = (seed) => {
    const g = GC.createGame({ seed });
    const spawns = [];
    let prev = null;
    for (let i = 0; i < 1500; i++) {
      g.input.fire = i % 7 !== 0;
      g.input.right = (i % 120) < 40;
      g.input.up = (i % 120) >= 40 && (i % 120) < 80;
      GC.tickGame(g, null);
      if (g.lastEntered !== prev) { spawns.push(g.lastEntered); prev = g.lastEntered; }
    }
    return { spawns, summary: JSON.stringify(GC.runSummary(g)) };
  };
  const a = run('det-seed');
  const b = run('det-seed');
  const c = run('det-other');
  assert.deepEqual(a.spawns, b.spawns, 'same seed replays the same spawn order');
  assert.equal(a.summary, b.summary, 'same seed replays the same match');
  assert.notDeepEqual(a.spawns, c.spawns, 'a different seed spawns differently');
  assert.ok(a.spawns.length > 5, 'the sequence is long enough to be evidence');
});

test('seeded wave composition: sized, tier-valid, seed-stable', () => {
  const a = GC.waveComposition(3, 2, 'seed-x');
  const b = GC.waveComposition(3, 2, 'seed-x');
  const c = GC.waveComposition(3, 2, 'seed-y');
  assert.deepEqual(a, b);
  assert.notDeepEqual(a, c);
  assert.equal(a.length, 6 + 2 * 3);
  assert.ok(a.every((t) => GC.typesForTier(2).includes(t)),
    'tier 2 waves never draw tier 3 or 4 geometry');
  assert.equal(GC.waveComposition(40, 4, 's').length, 60, 'composition is capped');
});

/* ---------- tiers ---------- */

test('tiers escalate on score and time, and gate the roster', () => {
  assert.equal(GC.tierFor(0, 0, 'standard'), 1);
  assert.equal(GC.tierFor(2000, 0, 'standard'), 2);
  assert.equal(GC.tierFor(1999, 0, 'standard'), 1);
  assert.equal(GC.tierFor(6000, 0, 'standard'), 3);
  assert.equal(GC.tierFor(12000, 0, 'standard'), 4);
  /* time buys tier too: 40 s of survival is 1000 effective points */
  assert.equal(GC.tierFor(1000, 40, 'standard'), 2);
  assert.equal(GC.tierFor(0, 80, 'standard'), 2);
  /* chill pushes the thresholds out, overdrive pulls them in */
  assert.equal(GC.tierFor(2000, 0, 'chill'), 1, 'chill needs 3200 for tier 2');
  assert.equal(GC.tierFor(3200, 0, 'chill'), 2);
  assert.equal(GC.tierFor(1200, 0, 'overdrive'), 2, 'overdrive reaches tier 2 early');
  const monotone = [];
  for (let s = 0; s <= 20000; s += 250) monotone.push(GC.tierFor(s, 0, 'standard'));
  for (let i = 1; i < monotone.length; i++) {
    assert.ok(monotone[i] >= monotone[i - 1], 'tier never goes backwards');
  }
  assert.deepEqual(GC.unlocksForTier(1), ['grunt', 'drifter']);
  assert.deepEqual(GC.unlocksForTier(3), ['snake']);
  assert.deepEqual(GC.unlocksForTier(4), ['weaver', 'well']);
  const t4 = GC.typesForTier(4);
  assert.ok(t4.includes('weaver') && t4.includes('well') && t4.includes('grunt'));
  assert.ok(!t4.includes('cubelet'), 'children are never wave geometry');
});

/* ---------- the enemy decision ladder ---------- */

test('grunt: chases, and jinks off a closing fire line', () => {
  const g = quiet({ seed: 'grunt' });
  const e = place(g, 'grunt', 200, 300);
  assert.equal(tagOf(g, e), 'chase');
  assert.equal(e.tag, 'chase');
  /* a bullet coming straight at it */
  g.bullets.push({ x: 100, y: 300, vx: 12, vy: 0, dmg: 1, life: 60, color: '#fff' });
  assert.equal(GC.inboundBullet(g, e), g.bullets[0]);
  assert.equal(tagOf(g, e), 'jink');
  /* a bullet moving away is not a threat */
  g.bullets.length = 0;
  g.bullets.push({ x: 260, y: 300, vx: 12, vy: 0, dmg: 1, life: 60, color: '#fff' });
  assert.equal(GC.inboundBullet(g, e), null);
  assert.equal(tagOf(g, e), 'chase');
});

test('drifter wanders when far and only leans in when close', () => {
  const g = quiet({ seed: 'drifter' });
  const far = place(g, 'drifter', 900, 560);
  assert.equal(tagOf(g, far), 'wander');
  const near = place(g, 'drifter', g.player.x + 60, g.player.y);
  assert.equal(tagOf(g, near), 'chase');
});

test('cube wanders, and flees a closing bullet instead of jinking', () => {
  const g = quiet({ seed: 'cube' });
  const e = place(g, 'cube', 200, 200);
  assert.equal(tagOf(g, e), 'wander');
  g.bullets.push({ x: 100, y: 200, vx: 12, vy: 0, dmg: 1, life: 60, color: '#fff' });
  assert.equal(tagOf(g, e), 'flee');
});

test('rocket: aims, charges when aligned, brakes when the charge is spent', () => {
  const g = quiet({ seed: 'rocket' });
  const e = place(g, 'rocket', 200, 300);
  assert.equal(tagOf(g, e), 'aim');
  /* point it straight at the player: the aim rung releases the charge */
  e.h = Math.atan2(g.player.y - e.y, g.player.x - e.x);
  GC.enemyStep(g, e);
  assert.equal(e.mode, 'charge');
  assert.equal(e.tag, 'charge');
  /* spend the charge: it brakes */
  e.chargeT = GC.ROCKET_CHARGE_MAX + 1;
  GC.enemyStep(g, e);
  assert.equal(e.mode, 'brake');
  assert.equal(e.tag, 'brake');
  /* bleeding speed off puts it back on the aim rung */
  for (let i = 0; i < 200; i++) GC.enemyStep(g, e);
  assert.equal(e.mode, 'aim');
});

test('snake follows the trail it can smell, wanders without one', () => {
  const g = quiet({ seed: 'snake' });
  const e = place(g, 'snake', 60, 60);
  assert.equal(tagOf(g, e), 'wander');
  g.trail.push({ x: 80, y: 80 });
  assert.ok(GC.nearestTrail(g, e), 'the trail is inside its sense radius');
  assert.equal(tagOf(g, e), 'trail');
});

test('well pulls until it has eaten its fill, then bursts', () => {
  const g = quiet({ seed: 'well' });
  const e = place(g, 'well', 700, 300);
  assert.equal(tagOf(g, e), 'pull');
  e.feed = GC.WELL_BURST - 1;
  assert.equal(tagOf(g, e), 'pull');
  GC.damageEnemy(g, e, 1, e.x, e.y);      /* fire feeds a well, never hurts it */
  assert.equal(e.dead, true, 'the eighth swallow bursts it');
  assert.ok(g.geoms.length >= TYPE.well.geoms, 'the burst scatters geoms');
});

test('every tag in the vocabulary is reachable and counted', () => {
  const g = quiet({ seed: 'tags' });
  assert.deepEqual(GC.TAGS, ['chase', 'wander', 'charge', 'brake', 'aim', 'flee',
    'jink', 'trail', 'split', 'pull', 'burst']);
  const cases = [
    ['grunt', (e) => { e.x = 200; e.y = 300; }, 'chase'],
    ['drifter', (e) => { e.x = 900; e.y = 560; }, 'wander'],
    ['cube', (e) => { e.x = 200; e.y = 200; g.bullets.push(
      { x: 100, y: 200, vx: 12, vy: 0, dmg: 1, life: 60, color: '#fff' }); }, 'flee'],
    ['rocket', (e) => { e.mode = 'charge'; }, 'charge'],
    ['rocket', (e) => { e.mode = 'brake'; }, 'brake'],
    ['snake', (e) => { e.x = 60; e.y = 60; g.trail.push({ x: 70, y: 70 }); }, 'trail'],
    ['well', (e) => { e.feed = GC.WELL_BURST; }, 'burst'],
  ];
  for (const [type, setup, want] of cases) {
    g.enemies.length = 0;
    g.bullets.length = 0;
    g.trail.length = 0;
    const e = place(g, type, 300, 300);
    setup(e);
    assert.equal(tagOf(g, e), want, type + ' → ' + want);
    assert.ok(g.decisions.tags[want] > 0, want + ' is counted');
  }
});

/* ---------- splitting ---------- */

test('a shot cube leaves three cubelets, a weaver four weavlings', () => {
  const g = quiet({ seed: 'split' });
  const cube = place(g, 'cube', 700, 300);
  GC.damageEnemy(g, cube, 99, 700, 300);
  assert.equal(g.enemies.filter((e) => e.type === 'cubelet').length, GC.SPLIT_CHILDREN.cube.n);
  assert.ok(g.decisions.tags.split >= 1, 'splitting is a counted decision');

  g.enemies.length = 0;
  const weaver = place(g, 'weaver', 700, 300);
  GC.damageEnemy(g, weaver, 99, 700, 300);
  assert.equal(g.enemies.filter((e) => e.type === 'weavling').length, GC.SPLIT_CHILDREN.weaver.n);
});

test('children do not split again, and the spawn cap holds', () => {
  const g = quiet({ seed: 'nocascade' });
  const kid = place(g, 'cubelet', 700, 300);
  GC.damageEnemy(g, kid, 99, 700, 300);
  assert.equal(g.enemies.filter((e) => !e.dead).length, 0, 'no grandchildren');

  const capped = quiet({ seed: 'cap', maxEnemies: 4 });
  const parent = place(capped, 'cube', 700, 300);
  GC.damageEnemy(capped, parent, 99, 700, 300);
  assert.ok(capped.enemies.filter((e) => !e.dead).length <= 4, 'the cap is a hard ceiling');
});

/* ---------- weapons ---------- */

test('cooldown gates the fire rate', () => {
  const mk = (cd) => {
    const g = quiet({ seed: 'rate', weaponMods: { pulse: { cooldown: cd } } });
    g.input.fire = true;
    g.input.aim = 0;
    return g;
  };
  const fast = mk(3);
  const slow = mk(30);
  for (let i = 0; i < 600; i++) {
    fast.enemies.length = 0;
    slow.enemies.length = 0;
    GC.tickGame(fast, null);
    GC.tickGame(slow, null);
  }
  assert.ok(fast.stats.shots > slow.stats.shots * 3,
    'fast ' + fast.stats.shots + ' vs slow ' + slow.stats.shots);
});

test('damage changes how fast geometry dies', () => {
  const mk = (dmg) => {
    const g = quiet({ seed: 'dmg', weaponMods: { pulse: { dmg } } });
    g.input.fire = true;
    g.input.aim = 0;
    place(g, 'cube', g.player.x + 130, g.player.y);   /* hp 2: one light shot cannot finish it */
    return g;
  };
  const light = mk(1);
  const heavy = mk(4);
  for (let i = 0; i < 10; i++) { GC.tickGame(light, null); GC.tickGame(heavy, null); }
  assert.equal(heavy.stats.kills, 1, 'one heavy shot kills outright');
  assert.equal(light.stats.kills, 0, 'one light shot leaves it alive');
  assert.equal(light.enemies.filter(e => !e.dead)[0].hp, TYPE.cube.hp - 1,
    'the light shot still connects');
});

test('laser pierces everything on the beam, scatter sprays, wave rings a group', () => {
  /* laser: two grunts on one ray both take the beam */
  const g = quiet({ seed: 'laser' });
  g.player.weapon = 'laser';
  g.input.fire = true;
  g.input.aim = 0;
  place(g, 'grunt', g.player.x + 140, g.player.y);
  place(g, 'grunt', g.player.x + 320, g.player.y);
  place(g, 'grunt', g.player.x + 100, g.player.y + 160);   /* off the beam */
  GC.tickGame(g, null);
  assert.equal(g.stats.kills, 2, 'the beam kills along its length only');

  /* scatter: one trigger, three bullets at the tuned spread */
  const s = quiet({ seed: 'scatter', weaponMods: { scatter: { count: 5, spread: 0.5 } } });
  s.player.weapon = 'scatter';
  s.player.cd = 0;
  s.input.aim = 0;
  GC.fireWeapon(s);
  assert.equal(s.bullets.length, 5);
  const angles = s.bullets.map((b) => Math.atan2(b.vy, b.vx));
  assert.ok(new Set(angles.map((a) => a.toFixed(3))).size === 5, 'the spray fans out');

  /* wave: one ring, each enemy in the radius hit exactly once */
  const w = quiet({ seed: 'wave' });
  w.player.weapon = 'wave';
  w.player.cd = 0;
  w.input.fire = true;
  place(w, 'grunt', w.player.x + 40, w.player.y);
  place(w, 'grunt', w.player.x, w.player.y + 55);
  GC.fireWeapon(w);
  for (let i = 0; i < 40; i++) { w.enemies = w.enemies.filter((e) => !e.dead); GC.tickGame(w, null); }
  assert.equal(w.stats.kills, 2, 'the ring reached both');
});

test('snake armour: the body shrugs, the head dies', () => {
  const g = quiet({ seed: 'snake-armour' });
  const e = place(g, 'snake', 700, 300);
  GC.damageEnemy(g, e, 99, e.x + 60, e.y);        /* a body hit */
  assert.equal(e.hp, TYPE.snake.hp, 'the body took nothing');
  GC.damageEnemy(g, e, 1, e.x, e.y);              /* the head */
  assert.equal(e.hp, TYPE.snake.hp - 1);
  GC.damageEnemy(g, e, 99, e.x, e.y);
  assert.equal(e.dead, true);
});

/* ---------- particles ---------- */

test('a death spawns its exact debris count, and the pool caps', () => {
  const g = quiet({ seed: 'debris' });
  const e = place(g, 'grunt', 700, 300);
  GC.damageEnemy(g, e, 9, 700, 300);
  assert.equal(g.particles.length, TYPE.grunt.debris);

  const big = quiet({ seed: 'debris-cap' });
  for (let i = 0; i < 60; i++) place(big, 'grunt', 100 + (i % 30) * 20, 100 + Math.floor(i / 30) * 60);
  big.particles.length = 0;
  GC.useBomb(big);
  assert.equal(big.particles.length, GC.PARTICLE_CAP, 'the pool is a hard cap');
});

test('particles fade out on their own', () => {
  const g = quiet({ seed: 'fade' });
  const e = place(g, 'grunt', 700, 300);
  GC.damageEnemy(g, e, 9, 700, 300);
  const n = g.particles.length;
  assert.ok(n > 0);
  for (let i = 0; i < 400; i++) GC.tickGame(g, null);
  assert.equal(g.particles.length, 0, 'every particle decays');
});

/* ---------- score, multiplier, geoms ---------- */

test('kills pay points times the multiplier, geoms raise the multiplier', () => {
  const g = quiet({ seed: 'score' });
  assert.equal(g.mult, 1);
  place(g, 'grunt', 700, 300);
  GC.damageEnemy(g, g.enemies[0], 9, 700, 300);
  assert.equal(g.score, TYPE.grunt.points);
  assert.equal(g.geoms.length, TYPE.grunt.geoms, 'the dead drop geoms');

  /* fly through GEOM_STEP of them: multiplier up */
  for (let i = 0; i < GC.GEOM_STEP; i++) g.geoms.push({ x: g.player.x, y: g.player.y, vx: 0, vy: 0 });
  GC.tickGame(g, null);
  assert.equal(g.stats.geomsTaken, GC.GEOM_STEP);
  assert.equal(g.mult, 2);
  assert.equal(g.stats.multPeak, 2);

  place(g, 'grunt', 700, 300);
  GC.damageEnemy(g, g.enemies[0], 9, 700, 300);
  assert.equal(g.score, TYPE.grunt.points + TYPE.grunt.points * 2, 'x2 pays twice');
});

test('dying resets the multiplier and costs a life; the last life ends the run', () => {
  const g = quiet({ seed: 'death' });
  g.mult = 7;
  g.taken = 3;
  g.player.invuln = 0;
  place(g, 'grunt', g.player.x, g.player.y);
  const lives = g.lives;
  GC.tickGame(g, null);
  assert.equal(g.lives, lives - 1);
  assert.equal(g.mult, 1);
  assert.equal(g.taken, 0);

  g.lives = 0;
  g.player.invuln = 0;
  place(g, 'grunt', g.player.x, g.player.y);
  GC.tickGame(g, null);
  assert.equal(g.over, true, 'no lives, no run');
  assert.equal(g.player.alive, false);
});

/* ---------- bombs ---------- */

test('a bomb clears the field for nothing: no score, no geoms', () => {
  const g = quiet({ seed: 'bomb' });
  g.player.invuln = 0;
  for (let i = 0; i < 8; i++) place(g, 'grunt', 120 + i * 60, 150 + (i % 2) * 200);
  const before = { score: g.score, bombs: g.bombs, enemies: g.enemies.length };
  const ok = GC.useBomb(g);
  assert.equal(ok, true);
  assert.equal(g.bombs, before.bombs - 1);
  assert.equal(g.score, before.score, 'a bomb scores nothing');
  assert.equal(g.geoms.length, 0, 'a bomb drops no geoms');
  assert.equal(g.enemies.filter((e) => !e.dead).length, 0, 'the field is clear');
  assert.equal(GC.useBomb(g), g.bombs > 0, 'bombs are spent one at a time');
});

/* ---------- the engine rung ---------- */

test('the engine names the wave lead when it answers with a real candidate', () => {
  const g = GC.createGame({ seed: 'engine' });
  const reqs = [];
  const ask = (req) => {
    reqs.push(req);
    return { outcome: 'accept', answers: [{ question_id: 'gridwars-wave-lead',
      type: 'choice', choice: 'drifter', confidence: 0.8 }] };
  };
  GC.nextWave(g, ask);
  assert.equal(reqs.length, 1, 'one ask per wave');
  const q = reqs[0].questions[0];
  assert.equal(q.type, 'choice');
  assert.equal(q.id, 'gridwars-wave-lead');
  assert.deepEqual(q.candidates.map((c) => c.id), GC.typesForTier(g.tier));
  assert.equal(g.lastEngine.rung, 'engine');
  assert.equal(g.lastEngine.pick, 'drifter');
  assert.equal(g.rungs.engine, 1);
  assert.equal(g.queue[0], 'drifter', 'the pick leads the wave');
});

test('an abstain, a refusal, or a made-up candidate falls back to the seed', () => {
  for (const answer of [null, { outcome: 'abstain', answers: [] },
    { outcome: 'accept', answers: [{ choice: 'weaver' }] }]) {
    const g = GC.createGame({ seed: 'fallback' });
    const before = g.rungs.rule;
    GC.nextWave(g, () => answer);
    assert.equal(g.lastEngine.rung, 'rule', JSON.stringify(answer) + ' → rule');
    assert.equal(g.rungs.engine, 0);
    assert.equal(g.rungs.rule, before + 1);
    assert.ok(GC.typesForTier(g.tier).includes(g.lastEngine.pick),
      'the fallback is still a real candidate');
  }
});

test('no ask without the engine, and no ask when there is nothing to choose', () => {
  const g = GC.createGame({ seed: 'noask' });
  let asks = 0;
  const ask = () => { asks += 1; return null; };
  for (let i = 0; i < 400; i++) GC.tickGame(g, ask);
  assert.equal(asks, 0, 'tickGame with a null ask never calls out');
  assert.equal(g.engineCalls, 0);

  const single = GC.createGame({ seed: 'noask2' });
  GC.waveLead(single, () => { throw new Error('should not ask'); }, 'test', ['grunt']);
  assert.equal(single.lastEngine.rung, 'rule');
});

test('a tier-up asks which NEW geometry enters first', () => {
  const g = GC.createGame({ seed: 'tierup' });
  const reqs = [];
  GC.tickGame(g, (req) => { reqs.push(req); return { outcome: 'accept',
    answers: [{ choice: 'rocket', confidence: 0.9 }] }; });
  assert.equal(reqs.length, 0, 'tier 1 has nothing to unlock');

  g.score = GC.TIERS[1].score;      /* cross into tier 2 */
  GC.tickGame(g, (req) => { reqs.push(req); return { outcome: 'accept',
    answers: [{ choice: 'rocket', confidence: 0.9 }] }; });
  assert.equal(g.tier, 2);
  assert.equal(reqs.length, 1, 'the unlock is a real question');
  assert.deepEqual(reqs[0].questions[0].candidates.map((c) => c.id), GC.unlocksForTier(2));
  assert.equal(g.lastEntered, 'rocket', 'the engine pick enters first');
});

/* ---------- the grid ---------- */

test('the grid warps under a push and settles back on its springs', () => {
  const g = quiet({ seed: 'grid', warp: 1 });
  const node = g.grid.nodes[Math.floor(g.grid.nodes.length / 2)];
  GC.pushGrid(g, node.x - 40, node.y, 240, 12);
  GC.tickGame(g, null);
  const out = Math.hypot(node.x - node.ox, node.y - node.oy);
  assert.ok(out > 0.5, 'the push displaced the mesh, got ' + out);
  for (let i = 0; i < 600; i++) GC.tickGame(g, null);
  const rest = Math.hypot(node.x - node.ox, node.y - node.oy);
  assert.ok(rest < 0.75, 'the springs settled, got ' + rest);
});

test('zero warp still takes the hit and comes back', () => {
  const g = quiet({ seed: 'grid0', warp: 0 });
  const node = g.grid.nodes[0];
  GC.pushGrid(g, node.x + 40, node.y, 200, 10);
  GC.tickGame(g, null);
  assert.ok(Math.hypot(node.x - node.ox, node.y - node.oy) > 0.1, 'warp 0 is stiff, not frozen');
  for (let i = 0; i < 400; i++) GC.tickGame(g, null);
  assert.ok(Math.hypot(node.x - node.ox, node.y - node.oy) < 0.5);
});

/* ---------- run summary ---------- */

test('runSummary reports the fields the panels show', () => {
  const g = GC.createGame({ seed: 'summary' });
  g.player.invuln = 1e9;
  for (let i = 0; i < 240; i++) GC.tickGame(g, null);
  const s = GC.runSummary(g);
  for (const key of ['score', 'wave', 'tier', 'mult', 'lives', 'bombs', 'kills',
    'deaths', 'shots', 'geoms', 'particles', 'enemies', 'ticks', 'timeS',
    'decisions', 'rungs', 'engineCalls', 'over']) {
    assert.ok(key in s, 'summary has ' + key);
  }
  assert.equal(s.particles, g.particles.length);
  assert.equal(s.enemies, g.enemies.length);
  assert.ok(s.decisions.total > 0, 'decisions were made and counted');
  assert.ok(Object.keys(s.decisions.tags).every((k) => GC.TAGS.includes(k)),
    'every recorded tag is in the vocabulary');
});
