/* Pac-maze core suite — the deterministic contract the shell relies on.
   Import order matters: the SDK first, then maze-core (board + BFS), then
   pac-core; read the globals. The default board is the classic arcade 28x31;
   tests that exercise the generated-maze machinery pin board: 'generated'.

   Run: node --test tests/pacman_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'maze-core.js'));
await import(path.join(siteDir, '..', 'pac-core.js'));
const PC = globalThis.PacCore;

test('config normalization clamps hostile input', () => {
  const c = PC.normalizeConfig({
    seed: 42, cells: 999, braidPct: -5, ghosts: 99,
    ghostSpeed: 'nope', playerSpeed: NaN, frightS: 0.1,
    pelletFill: 3, lives: -2, botPlayer: 'yes',
  });
  assert.equal(c.seed, '42');
  assert.equal(c.cells, 29);
  assert.equal(c.braidPct, 0);
  assert.equal(c.ghosts, 6);
  assert.equal(c.ghostSpeed, 4.2, 'hostile ghost speed falls back');
  assert.equal(c.playerSpeed, 5.4, 'NaN falls back');
  assert.equal(c.frightS, 2);
  assert.equal(c.pelletFill, 1);
  assert.equal(c.lives, 1);
  assert.equal(c.botPlayer, false, 'only === true counts as bot');
  assert.equal(c.board, 'classic', 'the arcade board is the default');
  assert.equal(PC.normalizeConfig({ board: 'generated' }).board, 'generated');
  assert.equal(PC.normalizeConfig({ board: 'bogus' }).board, 'classic',
    'anything else is the arcade board');
});

test('the board builds deterministically with pac furniture in place', () => {
  const a = PC.buildBoard({ seed: 'boardy' });
  const b = PC.buildBoard({ seed: 'boardy' });
  assert.equal(a.w, a.h, 'square board');
  assert.deepEqual(Array.from(a.grid), Array.from(b.grid), 'same seed, same maze');
  assert.deepEqual([...a.pellets], [...b.pellets], 'same pellets');
  assert.deepEqual(a.powers, b.powers, 'same power tiles');
  assert.equal(a.powers.length, 4, 'exactly four power pellets');
  for (const key of a.powers) {
    assert.equal(a.pellets.has(key), false, 'power tiles are not plain pellets');
    assert.ok(PC.passable(a, key % a.w, (key / a.w) | 0), 'power pellet on floor');
  }
  assert.ok(PC.passable(a, a.start.x, a.start.y), 'player starts on floor');
  assert.ok(a.ghostTiles.length >= 1, 'at least one ghost spawn');
  /* the player starts FAR from the ghost home — adjacent spawn was a
     measured instant-death loop (seed 'tie', 3 lives in ~60 ticks) */
  const dHome = PC.distFrom(a, a.home.x, a.home.y);
  assert.ok(dHome[a.start.y * a.w + a.start.x] >= 8, 'spawn is not beside the house');
  for (const gt of a.ghostTiles) {
    assert.ok(PC.passable(a, gt.x, gt.y), 'ghost spawn on floor');
  }
});

test('movement is tile-locked and walls block the player', () => {
  const g = PC.createGame({ seed: 'movey', ghosts: 1, board: 'generated', cells: 11 });
  const b = g.board;
  /* no input: classic auto-run — the player keeps rolling in its facing
     direction (the shell queues intent persistently), never into a wall,
     never off the board */
  for (let i = 0; i < 120; i++) {
    PC.tickGame(g, null);
    const key = g.player.y * b.w + g.player.x;
    assert.equal(b.grid[key], 0, 'player never inside a wall');
    assert.ok(g.player.x >= 0 && g.player.x < b.w, 'x in bounds');
    assert.ok(g.player.y >= 0 && g.player.y < b.h, 'y in bounds');
  }
  /* explicit intent: same invariant while steering */
  for (let i = 0; i < 600; i++) {
    PC.tickGame(g, { want: g.player.dir });
    const key = g.player.y * b.w + g.player.x;
    assert.equal(b.grid[key], 0, 'player never inside a wall');
    assert.ok(g.player.x >= 0 && g.player.x < b.w, 'x in bounds');
    assert.ok(g.player.y >= 0 && g.player.y < b.h, 'y in bounds');
  }
});

test('a pure pursuer closes distance; a pure retreater opens it', () => {
  /* drive the player to a fixed tile, then watch one ghost decide */
  const mk = (weights) => {
    const g = PC.createGame({ seed: 'hunt', ghosts: 1, board: 'generated', cells: 11 });
    const gh = g.ghosts[0];
    gh.weights = weights;
    return g;
  };
  const chase = mk({ pursue: 5, intercept: 0, patrol: 0, retreat: 0 });
  const flee = mk({ pursue: 0, intercept: 0, patrol: 0, retreat: 5 });
  const d0 = (g) => PC.distFrom(g.board, g.ghosts[0].x, g.ghosts[0].y)[
    g.player.y * g.board.w + g.player.x];
  /* let the ghost make many decisions while the player holds still */
  for (let i = 0; i < 900; i++) PC.tickGame(chase, null);
  for (let i = 0; i < 900; i++) PC.tickGame(flee, null);
  /* the ghosts may have eaten the player (that ends the run) — only compare
     live runs; a dead run with lives left resets positions, which is fine */
  const dChase = d0(chase);
  const dFlee = d0(flee);
  assert.ok(dChase < dFlee,
    `pursuer closer (${dChase}) than retreater (${dFlee})`);
});

test('frightened retreat is a rule, not a choice', () => {
  const g = PC.createGame({ seed: 'fright', ghosts: 2, board: 'generated', cells: 11 });
  /* force a power pellet into the player's mouth */
  const p = g.player;
  const key = p.y * g.board.w + p.x;
  g.board.powers.push(key);
  PC.tickGame(g, null);
  assert.ok(g.t < g.frightUntil, 'fright window opened');
  assert.equal(g.powersEaten, 1);
  /* during fright, decisions are labelled fright and never ask the engine */
  let decisions = 0;
  const t0 = g.t;
  while (g.t < g.frightUntil && g.t - t0 < 600) {
    PC.tickGame(g, null);
    if (g.lastDecision && g.lastDecision.rung === 'fright') decisions += 1;
  }
  assert.ok(decisions > 0, 'fright rules were applied and labelled');
  assert.equal(g.engineCalls, 0, 'no engine calls during fright');
});

test('ghost ties escalate to the engine and the answer is honored', () => {
  /* all-zero weights tie every candidate at every decision */
  let calls = 0;
  let seenCalls = 0;
  let sawEngineRung = false;
  let lastReq = null;
  const g = PC.createGame({ seed: 'ties', ghosts: 1, board: 'generated', cells: 11 });
  g.ghosts[0].weights = { pursue: 0, intercept: 0, patrol: 0, retreat: 0 };
  const engineChoose = (req) => {
    calls += 1;
    lastReq = req;
    return { outcome: 'accept', answers: [{ type: 'choice', question_id: 'ghost-dir', choice: req.questions[0].candidates[0].id, confidence: 0.8 }] };
  };
  while (g.ticks < 1200 && !g.over) {
    PC.tickGame(g, null, engineChoose);
    /* check the trace when the call is fresh — later fright/eyes decisions
       legitimately overwrite it before the run ends */
    if (calls > seenCalls) {
      seenCalls = calls;
      sawEngineRung = sawEngineRung || (g.lastDecision && g.lastDecision.rung === 'engine');
    }
  }
  assert.ok(calls > 0, 'all-zero weights must escalate');
  assert.equal(g.rungs.engine, calls, 'every answered tie counts as the engine rung');
  const q = lastReq.questions[0];
  assert.equal(q.type, 'choice');
  assert.equal(q.id, 'ghost-dir');
  assert.ok(q.candidates.length >= 2, 'a tie carries at least two candidates');
  assert.ok(q.candidates.every((c) => ['up', 'down', 'left', 'right'].includes(c.id)));
  assert.ok(q.candidates.every((c) => c.description.length > 0));
  assert.ok(sawEngineRung, 'the trace names the engine when it answers');
});

test('an engine abstain or unknown id falls back to the weights argmax', () => {
  for (const answer of [
    { outcome: 'abstain', answers: [] },
    { outcome: 'accept', answers: [{ choice: 'sideways' }] },
    null,
  ]) {
    const g = PC.createGame({ seed: 'abstain', ghosts: 1, board: 'generated', cells: 11 });
    g.ghosts[0].weights = { pursue: 0, intercept: 0, patrol: 0, retreat: 0 };
    let calls = 0;
    while (g.ticks < 600 && !g.over) {
      PC.tickGame(g, null, () => { calls += 1; return answer; });
    }
    assert.ok(calls > 0, 'escalation still happens');
    assert.ok(g.rungs.weights > 0, 'local formula answered, labelled weights');
    assert.equal(g.rungs.engine, 0, 'nothing credited to the engine');
  }
});

test('contact with a hunter costs a life; the last one ends the run', () => {
  const g = PC.createGame({ seed: 'doom', ghosts: 1, board: 'generated', cells: 11, lives: 2 });
  /* two kills spend both lives; the loop stops there because the run is
     over and tickGame no-ops once it is */
  for (let i = 0; i < 2; i++) {
    const gh = g.ghosts[0];
    if (gh.state === 'house') { gh.state = 'active'; gh.releaseAt = 0; }
    gh.x = g.player.x; gh.y = g.player.y;
    const before = g.lives;
    PC.tickGame(g, null);
    assert.equal(g.lives, before - 1, 'contact kills');
  }
  assert.equal(g.over, true, 'out of lives');
  assert.equal(PC.runSummary(g).lives, 0);
});

test('eating a frightened ghost scores a chain and sends eyes home', () => {
  const g = PC.createGame({ seed: 'munch', ghosts: 1, board: 'generated', cells: 11 });
  const gh = g.ghosts[0];
  if (gh.state === 'house') { gh.state = 'active'; gh.releaseAt = 0; }
  /* player on a power pellet, ghost adjacent: fright, then step in */
  const b = g.board;
  const key = g.player.y * b.w + g.player.x;
  b.powers.push(key);
  PC.tickGame(g, null);
  assert.ok(g.t < g.frightUntil, 'fright active');
  gh.x = g.player.x; gh.y = g.player.y; gh.px = gh.x; gh.py = gh.y;
  const scoreBefore = g.score;
  PC.tickGame(g, null);
  assert.equal(g.ghostsEaten, 1, 'the frightened ghost was eaten');
  assert.equal(g.score - scoreBefore, 200, 'chain 1 pays 200');
  assert.equal(gh.state, 'eyes', 'the ghost is homing');
  /* eyes path home — it must eventually arrive (or the run ends first) */
  let guarded = 0;
  while (gh.state === 'eyes' && guarded < 3600) {
    PC.tickGame(g, null);
    guarded += 1;
  }
  assert.equal(gh.x, b.home.x, 'eyes arrived home');
  assert.equal(gh.y, b.home.y);
  assert.equal(gh.state, 'house', 'then respawns from the house');
  assert.ok(g.rungs.eyes > 0, 'eyes moves labelled as the rule they are');
});

test('clearing the board advances the level with fresh furniture', () => {
  const g = PC.createGame({ seed: 'clear', ghosts: 1 });
  g.readyUntil = 0;                    /* skip the READY gate: test the clear */
  const level0 = g.level;
  /* eat everything by hand, then tick: the core should roll the level */
  g.board.pellets.clear();
  for (const k of g.board.powers) g.eatenPowers.add(k);
  PC.tickGame(g, null);
  assert.equal(g.level, level0 + 1);
  assert.ok(g.board.pellets.size > 0, 'fresh pellets down');
  assert.equal(g.eatenPowers.size, 0, 'power pellets reset');
});

test('bot-only runs are deterministic and make real progress', () => {
  for (const seed of ['alpha', 'bravo']) {
    const cfg = { board: 'generated', seed, botPlayer: true, ghosts: 3, maxTicks: 3600 };
    const one = PC.simulateGame(cfg);
    const two = PC.simulateGame(cfg);
    assert.deepEqual(PC.runSummary(one), PC.runSummary(two), seed);
    const s = PC.runSummary(one);
    assert.ok(s.pelletsEaten > 20, seed + ' eats pellets');
    assert.ok(s.ticks > 0, seed + ' ticks');
  }
});

test('the stamp separates seed, size, ghosts, speeds, and fright', () => {
  const base = { board: 'generated', seed: 's', cells: 15, ghosts: 4, ghostSpeed: 4.2, frightS: 7 };
  const a = PC.pacStamp(base);
  assert.equal(a, PC.pacStamp({ ...base }));
  assert.notEqual(a, PC.pacStamp({ ...base, seed: 't' }));
  assert.notEqual(a, PC.pacStamp({ ...base, cells: 21 }));
  assert.notEqual(a, PC.pacStamp({ ...base, ghosts: 5 }));
  assert.notEqual(a, PC.pacStamp({ ...base, ghostSpeed: 5.1 }));
  assert.notEqual(a, PC.pacStamp({ ...base, frightS: 9 }));
  /* the classic stamp carries the arcade marker, drops generator knobs,
     and still separates what a classic run can vary */
  const cb = { seed: 's', ghosts: 4, ghostSpeed: 4.2, frightS: 7 };
  const ca = PC.pacStamp(cb);
  assert.ok(ca.includes('arcade'), 'the classic stamp names the board');
  assert.equal(ca, PC.pacStamp({ ...cb, cells: 25, braidPct: 30, pelletFill: 0.5 }),
    'generator knobs do not leak into the classic stamp');
  assert.notEqual(ca, PC.pacStamp({ ...cb, seed: 't' }));
  assert.notEqual(ca, PC.pacStamp({ ...cb, ghosts: 5 }));
  assert.notEqual(ca, PC.pacStamp({ ...cb, ghostSpeed: 5.1 }));
  assert.notEqual(ca, PC.pacStamp({ ...cb, playerSpeed: 6.4 }));
  assert.notEqual(ca, PC.pacStamp({ ...cb, frightS: 9 }));
  assert.notEqual(ca, PC.pacStamp({ ...cb, lives: 4 }));
});

test('ghosts never reverse when another direction is open', () => {
  const g = PC.createGame({ seed: 'norev', ghosts: 1, cells: 11 });
  const gh = g.ghosts[0];
  const b = g.board;
  const ctx = {
    playerDist: PC.distFrom(b, g.player.x, g.player.y),
    interceptDist: PC.distFrom(b, g.player.x, g.player.y),
  };
  /* walk the maze: every tile where the reverse direction and some other
     direction are both open must offer no reverse candidate; a dead end
     must offer exactly the reverse */
  let junctions = 0;
  let deadEnds = 0;
  for (let y = 1; y < b.h - 1; y++) {
    for (let x = 1; x < b.w - 1; x++) {
      if (!PC.passable(b, x, y)) continue;
      gh.x = x; gh.y = y;
      for (const comeFrom of PC.DIR_ORDER) {
        gh.dir = comeFrom;
        const back = PC.REVERSE_OF[comeFrom];
        const others = PC.DIR_ORDER.filter((n) => n !== back &&
          PC.passable(b, x + PC.DIRS[n].dx, y + PC.DIRS[n].dy));
        const { cands } = PC.ghostCandidates(b, gh, ctx);
        const dirs = cands.map((c) => c.dir);
        if (others.length > 0 && PC.passable(b, x + PC.DIRS[back].dx, y + PC.DIRS[back].dy)) {
          junctions += 1;
          assert.equal(dirs.includes(back), false,
            'reverse offered at (' + x + ',' + y + ') with ' + others.join('/') + ' open');
        }
        if (others.length === 0) {
          deadEnds += 1;
          assert.deepEqual(dirs, [back], 'dead end at (' + x + ',' + y + ') must reverse');
        }
      }
    }
  }
  assert.ok(junctions > 5, 'the maze has junctions to test');
  assert.ok(deadEnds >= 0);
});

test('ghost decisions over a live run are plentiful and labelled', () => {
  const g = PC.createGame({ seed: 'labels', ghosts: 3, board: 'generated', cells: 11 });
  while (g.ticks < 900 && !g.over) PC.tickGame(g, null);
  const s = PC.runSummary(g);
  const total = s.rungs.weights + s.rungs.engine + s.rungs.fright + s.rungs.eyes;
  assert.ok(total > 20, 'decisions happened: ' + JSON.stringify(s.rungs));
  assert.ok(s.lastDecision, 'the trace line has something to show');
  assert.ok(['weights', 'engine', 'fright', 'eyes'].includes(s.lastDecision.rung));
});

/* ---------- the classic arcade board ---------- */

test('the arcade board is 28x31, deterministic, and furnished like the original', () => {
  const a = PC.buildClassicBoard({ seed: 'arcade' });
  const b = PC.buildClassicBoard({ seed: 'other' });
  assert.equal(a.w, 28);
  assert.equal(a.h, 31);
  assert.equal(a.classic, true);
  /* the board is the original: every seed builds the identical maze */
  assert.equal(a.pellets.size, b.pellets.size);
  assert.deepEqual([...a.pellets].sort((p, q) => p - q), [...b.pellets].sort((p, q) => p - q));
  assert.deepEqual(a.powers, b.powers);
  assert.equal(a.powers.length, 4, 'four power pellets, one per corner region');
  assert.ok(a.pellets.size > 200, 'a full pellet field: ' + a.pellets.size);
  for (const key of a.powers) {
    assert.equal(a.pellets.has(key), false, 'powers are not plain pellets');
    const x = key % a.w, y = (key / a.w) | 0;
    assert.ok(PC.passableC(a, x, y), 'power on open floor');
  }
  /* the player spawns between the two dots below the house */
  assert.deepEqual(a.start, { x: 13, y: 23 });
  assert.ok(PC.passableC(a, a.start.x, a.start.y), 'spawn on open floor');
  /* the ghost house is sealed except the door, and the door blocks players */
  for (const [dx, dy] of [[13, 12], [14, 12]]) {
    assert.equal(a.grid[dy * a.w + dx], 2, 'door tile at ' + dx + ',' + dy);
    assert.equal(PC.passable(a, dx, dy), false, 'players cannot use the door');
  }
  assert.ok(PC.passableC(a, a.home.x, a.home.y), 'the door-out tile is open corridor');
});

test('the row-14 tunnel wraps: walk left off the edge, arrive on the right', () => {
  const g = PC.createGame({ seed: 'tunnel', ghosts: 1 });
  g.readyUntil = 0; g.t = 600; g.phaseT = 600;
  g.player.x = 2; g.player.y = 14; g.player.dir = 'left'; g.player.want = 'left';
  let wrapped = false;
  for (let i = 0; i < 600 && !wrapped; i++) {
    PC.tickGame(g, { want: 'left' });
    if (g.player.x > 20) wrapped = true;
    assert.ok(g.player.x >= 0 && g.player.x < 28, 'wrapped x stays in bounds');
    assert.equal(g.player.y, 14, 'the walk stays in the tunnel');
  }
  assert.ok(wrapped, 'the player crossed the wrap');
});

test('ghosts leave the house on the classic stagger and never through walls', () => {
  const g = PC.createGame({ seed: 'house', ghosts: 4 });
  const [, pinky, inky, clyde] = g.ghosts;
  assert.equal(g.ghosts[0].phase, 'active', 'blinky starts outside');
  assert.equal(pinky.phase, 'house');
  while (g.t < 59) PC.tickGame(g, { want: null });
  assert.equal(pinky.phase, 'house', 'pinky still housed at ~1s');
  while (g.t < 600 && clyde.phase !== 'leaving') PC.tickGame(g, { want: null });
  assert.ok(pinky.phase === 'active' || pinky.phase === 'leaving', 'pinky is out or leaving');
  assert.equal(clyde.phase, 'leaving', 'clyde left on schedule (~7s)');
  while (g.t < 1800 && clyde.phase === 'leaving') PC.tickGame(g, { want: null });
  assert.notEqual(clyde.phase, 'leaving', 'the exit script completes');
  assert.equal(g.ghosts.every((gh) => gh.y >= 0 && gh.y < 31 && gh.x >= 0 && gh.x < 28),
    true, 'all ghosts in bounds');
});

test('the scatter/chase clock alternates on the arcade schedule', () => {
  const g = PC.createGame({ seed: 'clock', ghosts: 1 });
  g.readyUntil = 0;
  g.phaseT = 100;                                   /* inside the first 7s */
  assert.equal(PC.isScatter(g), true, 'opens scattered');
  g.phaseT = 7 * 60 + 100;                          /* inside the first 20s chase */
  assert.equal(PC.isScatter(g), false, 'then chases');
  g.phaseT = (7 + 20) * 60 + 100;                   /* second scatter */
  assert.equal(PC.isScatter(g), true, 'then scatters again');
  g.phaseT = 90 * 60;                               /* schedule long spent */
  assert.equal(PC.isScatter(g), false, 'ends in permanent chase');
  /* level 2 shortens the phases */
  g.level = 2; g.phaseT = 6 * 60;
  assert.equal(PC.isScatter(g), false, 'level 2 scatters only 5s');
});

test('classic fright is labelled, and eating two ghosts doubles the chain', () => {
  const g = PC.createGame({ seed: 'combo', ghosts: 2 });
  g.readyUntil = 0;
  const key = g.player.y * g.board.w + g.player.x;
  g.board.powers.push(key);
  PC.tickGame(g, { want: null });
  assert.ok(g.t < g.frightUntil, 'fright window opened');
  /* during fright the ghosts flee under the rule label, never the engine */
  const tFright = g.t;
  let frightDecisions = 0;
  while (g.t < tFright + 180 && !g.over) {
    PC.tickGame(g, { want: null });
    if (g.lastDecision && g.lastDecision.rung === 'fright') frightDecisions += 1;
  }
  assert.ok(frightDecisions > 0, 'fright moves carry the fright label');
  assert.equal(g.engineCalls, 0, 'no engine calls during fright');
  /* both hunters (one still housed — promote it) land on the player:
     200 then 400 in the same tick */
  for (const gh of g.ghosts) {
    if (gh.phase === 'house' || gh.phase === 'leaving') { gh.phase = 'active'; gh.releaseAt = 0; }
    gh.x = g.player.x; gh.y = g.player.y; gh.px = gh.x; gh.py = gh.y;
    gh.prog = 0;                     /* no stray movement remainder: they hold */
  }
  const before = g.score;
  PC.tickGame(g, { want: null });
  assert.equal(g.ghostsEaten, 2, 'both frightened ghosts eaten');
  assert.equal(g.score - before, 200 + 400, 'the classic doubling chain pays out');
  assert.ok(g.ghosts.every((gh) => gh.phase === 'eyes'), 'both are homing');
});

test('classic distance ties escalate to the engine and the answer is honored', () => {
  /* blinky at the ring midpoint with the player straight below: left and
     right are exactly equidistant — the arcade's own genuine tie */
  let calls = 0;
  let lastReq = null;
  const g = PC.createGame({ seed: 'ctie', ghosts: 1 });
  g.readyUntil = 0;
  g.phaseT = 3600;                    /* past the schedule: permanent chase */
  g.player.x = 13; g.player.y = 23; g.player.dir = 'up'; g.player.want = null;
  const gh = g.ghosts[0];
  gh.x = 13; gh.y = 17; gh.dir = 'up'; gh.px = 13; gh.py = 17;
  const engineChoose = (req) => {
    calls += 1;
    lastReq = req;
    return { outcome: 'accept', answers: [{ choice: req.questions[0].candidates[0].id }] };
  };
  /* tile-locked: the decision happens when the ghost arrives at its tile */
  while (calls === 0 && g.ticks < 300 && !g.over) PC.tickGame(g, { want: null }, engineChoose);
  assert.equal(calls, 1, 'the tie reached the engine exactly once');
  const q = lastReq.questions[0];
  assert.equal(q.id, 'ghost-dir');
  assert.deepEqual(q.candidates.map((c) => c.id).sort(), ['left', 'right'],
    'the equidistant pair');
  assert.equal(gh.x, 12, 'the engine answer was honored');
  assert.equal(g.rungs.engine, 1);
  assert.equal(g.lastDecision.rung, 'engine');
});

test('classic bot runs are deterministic and make real progress', () => {
  for (const seed of [4242, 90210]) {
    const cfg = { seed, botPlayer: true, ghosts: 4, maxTicks: 3600 };
    const one = PC.simulateGame(cfg);
    const two = PC.simulateGame(cfg);
    assert.deepEqual(PC.runSummary(one), PC.runSummary(two), String(seed));
    const s = PC.runSummary(one);
    assert.ok(s.pelletsEaten > 20, seed + ' eats pellets: ' + s.pelletsEaten);
    assert.ok(s.rungs.weights > 0, seed + ' records decisions');
  }
});
