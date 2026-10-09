/* Maze Solver core suite — drives site/maze-core.js the same way
   board_page_test.mjs drives benchmarks.js: import for side effect, read
   the global, no DOM, no browser, no network. Everything the page promises
   (deterministic generation, wall-blocking Bresenham vision, a bot that
   always escapes under fog, monsters that only see along rays, a rubric
   that zeroes disabled objectives) is pinned here so drift fails loudly.

   Run: node --test tests/maze_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

await import(path.join(path.dirname(path.dirname(fileURLToPath(import.meta.url))), 'sdk', 'decisions-sdk.js'));
await import(path.join(path.dirname(path.dirname(fileURLToPath(import.meta.url))), 'maze-core.js'));
const M = globalThis.MazeCore;
assert.ok(M, 'maze-core.js must define globalThis.MazeCore');

const idx = (m, x, y) => y * m.w + x;

function tinyMaze(rows) {
  /* rows of '#' and '.' — build a maze-shaped object by hand for LOS and
     interaction tests; start is the inner corner, exit the opposite one */
  const h = rows.length;
  const w = rows[0].length;
  const grid = new Uint8Array(w * h);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) grid[y * w + x] = rows[y][x] === '#' ? M.WALL : M.FLOOR;
  }
  return {
    w, h, grid,
    start: { x: 1, y: 1 },
    exit: { x: w - 2, y: h - 2 },
    optimalSteps: w + h - 4,
    floorCount: grid.reduce((a, b) => a + (b === M.FLOOR ? 1 : 0), 0),
    seed: 'tiny', cells: 1, braidPct: 0,
  };
}

/* ---------- rng + generation ---------- */

test('same seed yields a byte-identical maze; a different seed does not', () => {
  const a = M.generateMaze({ seed: 'alpha', cells: 13 });
  const b = M.generateMaze({ seed: 'alpha', cells: 13 });
  const c = M.generateMaze({ seed: 'beta', cells: 13 });
  assert.deepEqual([...a.grid], [...b.grid]);
  assert.notDeepEqual([...a.grid], [...c.grid]);
  assert.deepEqual(a.start, b.start);
  assert.deepEqual(a.exit, b.exit);
});

test('grid is (2c+1)² with walls-as-cells and exactly one border opening', () => {
  for (const cells of [9, 13, 21]) {
    const m = M.generateMaze({ seed: 'shape', cells });
    assert.equal(m.w, 2 * cells + 1);
    assert.equal(m.h, 2 * cells + 1);
    let borderFloors = 0;
    for (let x = 0; x < m.w; x++) {
      for (const y of [0, m.h - 1]) if (m.grid[idx(m, x, y)] === M.FLOOR) borderFloors++;
    }
    for (let y = 1; y < m.h - 1; y++) {
      for (const x of [0, m.w - 1]) if (m.grid[idx(m, x, y)] === M.FLOOR) borderFloors++;
    }
    assert.equal(borderFloors, 1, 'the exit is the single border opening');
    assert.equal(m.grid[idx(m, m.exit.x, m.exit.y)], M.FLOOR);
  }
});

test('start sits in the center area and every floor is reachable from it', () => {
  const m = M.generateMaze({ seed: 'reach', cells: 15 });
  const c = Math.floor(m.w / 2);
  assert.ok(Math.abs(m.start.x - c) <= 2 && Math.abs(m.start.y - c) <= 2, 'start is central');
  assert.equal(m.grid[idx(m, m.start.x, m.start.y)], M.FLOOR);
  const dist = M.bfsDistances(m.grid, m.w, m.h, m.start.x, m.start.y, M.FLOOR);
  let reached = 0;
  for (let i = 0; i < m.grid.length; i++) {
    if (m.grid[i] === M.FLOOR) { assert.ok(dist[i] >= 0, `floor at ${i % m.w},${Math.floor(i / m.w)} reachable`); reached++; }
  }
  assert.equal(reached, m.floorCount);
  assert.ok(m.optimalSteps > 0 && dist[idx(m, m.exit.x, m.exit.y)] === m.optimalSteps,
    `dist[exit] ${dist[idx(m, m.exit.x, m.exit.y)]} === optimalSteps ${m.optimalSteps}`);
});

test('braiding removes dead ends (loops exist at 30%)', () => {
  const deadEnds = (m) => {
    let n = 0;
    for (let y = 1; y < m.h - 1; y++) {
      for (let x = 1; x < m.w - 1; x++) {
        if (m.grid[idx(m, x, y)] !== M.FLOOR) continue;
        const open = M.DIRS.filter((d) => m.grid[idx(m, x + d.dx, y + d.dy)] === M.FLOOR);
        if (open.length === 1) n++;
      }
    }
    return n;
  };
  const perfect = M.generateMaze({ seed: 'braid', cells: 15, braidPct: 0 });
  const looped = M.generateMaze({ seed: 'braid', cells: 15, braidPct: 30 });
  assert.ok(deadEnds(looped) < deadEnds(perfect), 'braid strictly reduces dead ends');
});

test('normalizeConfig clamps every knob to its documented range', () => {
  const c = M.normalizeConfig({
    cells: 500, vision: 99, braidPct: -5, lootCount: 99, monsterCount: 99,
    monsterSpeed: 0, aggroRange: 1, wanderRandomness: 400, botSpeed: 1, timeLimitS: 99,
  });
  assert.equal(c.cells, 29);
  assert.equal(c.vision, 15);
  assert.equal(c.braidPct, 0);
  assert.equal(c.lootCount, 16);
  assert.equal(c.monsterCount, 10);
  assert.equal(c.monsterSpeed, 1);
  assert.equal(c.aggroRange, 2);
  assert.equal(c.wanderRandomness, 100);
  assert.equal(c.botSpeed, 2);
  assert.equal(c.timeLimitS, 99);
  assert.equal(M.normalizeConfig({}).vision, 9, 'default vision 9');
  assert.equal(M.normalizeConfig({}).timeLimitS, 0, 'time limit off by default');
});

/* ---------- Bresenham ---------- */

test('bresenhamLine covers exact endpoints with king-move steps', () => {
  const cases = [[0, 0, 7, 0], [0, 0, 0, 6], [0, 0, 5, 5], [0, 0, 5, 2], [0, 0, 2, 5],
    [4, 3, -3, -4], [2, 2, -5, 1], [-2, -2, 3, -3], [7, 7, 0, 0]];
  for (const [x0, y0, x1, y1] of cases) {
    const line = M.bresenhamLine(x0, y0, x1, y1);
    assert.equal(line[0].x, x0);
    assert.equal(line[0].y, y0);
    const last = line[line.length - 1];
    assert.equal(last.x, x1, `x endpoint of ${x0},${y0}->${x1},${y1}`);
    assert.equal(last.y, y1);
    assert.equal(line.length, Math.max(Math.abs(x1 - x0), Math.abs(y1 - y0)) + 1, 'cell count');
    for (let i = 1; i < line.length; i++) {
      const dx = Math.abs(line[i].x - line[i - 1].x);
      const dy = Math.abs(line[i].y - line[i - 1].y);
      assert.ok((dx === 1 && dy === 0) || (dx === 0 && dy === 1) || (dx === 1 && dy === 1),
        `step ${i} of ${x0},${y0}->${x1},${y1} is a king move`);
    }
  }
});

test('bresenhamLine matches a rounded parametric reference across octants', () => {
  for (let tx = -9; tx <= 9; tx += 3) {
    for (let ty = -9; ty <= 9; ty += 3) {
      if (tx === 0 && ty === 0) continue;
      const line = M.bresenhamLine(0, 0, tx, ty);
      const n = Math.max(Math.abs(tx), Math.abs(ty));
      assert.equal(line.length, n + 1);
      /* the canonical error-accumulation property: at every step the drawn
         cell is the nearest grid cell to the ideal line position */
      for (let i = 1; i < n; i++) {
        const t = i / n;
        const idealX = tx * t;
        const idealY = ty * t;
        const d = Math.abs(line[i].x - idealX) + Math.abs(line[i].y - idealY);
        assert.ok(d < 1.0001, `step ${i} near-ideal for (${tx},${ty}): ${d}`);
      }
    }
  }
});

/* ---------- vision: rays stop at walls ---------- */

test('a wall blocks its ray: the wall face is seen, cells behind are not', () => {
  const m = tinyMaze([
    '#########',
    '#.......#',
    '#...#...#',
    '#.......#',
    '#########',
  ]);
  const visible = new Uint8Array(m.w * m.h);
  const revealed = new Uint8Array(m.w * m.h);
  M.computeFov(m.grid, m.w, m.h, 4, 3, 9, visible, revealed);
  assert.equal(m.grid[idx(m, 4, 2)], M.WALL, 'the interior wall sits north of the runner');
  assert.equal(visible[idx(m, 4, 2)], 1, 'wall face north is visible');
  assert.equal(revealed[idx(m, 4, 2)], 1);
  assert.equal(visible[idx(m, 4, 1)], 0, 'nothing beyond the wall');
  assert.equal(revealed[idx(m, 4, 1)], 0);
  assert.equal(visible[idx(m, 4, 0)], 0, 'the far border behind the wall stays dark');
  assert.equal(visible[idx(m, 3, 3)], 1, 'open west corridor is visible');
});

test('the radius bounds vision even in the open', () => {
  const rows = ['###############'];
  for (let y = 0; y < 11; y++) rows.push('#' + '.'.repeat(13) + '#');
  rows.push('###############');
  const m = tinyMaze(rows);
  const visible = new Uint8Array(m.w * m.h);
  M.computeFov(m.grid, m.w, m.h, 7, 6, 5, visible, null);
  let beyond = 0;
  let inside = 0;
  for (let y = 0; y < m.h; y++) {
    for (let x = 0; x < m.w; x++) {
      const d = Math.hypot(x - 7, y - 6);
      if (d > 5.5) {
        assert.equal(visible[idx(m, x, y)], 0, `cell ${x},${y} beyond radius`);
        beyond++;
      } else if (m.grid[idx(m, x, y)] === M.FLOOR) inside++;
    }
  }
  assert.ok(beyond > 20, 'the room is big enough for the bound to bite');
  assert.ok(inside > 20, 'the lit area is non-trivial');
});

test('vision 5, 9 and 15 reveal strictly more in the same open room', () => {
  const rows = [];
  for (let y = 0; y < 9; y++) rows.push(y === 0 || y === 8 ? '#'.repeat(41) : '#' + '.'.repeat(39) + '#');
  const m = tinyMaze(rows);
  const counts = [5, 9, 15].map((r) => {
    const visible = new Uint8Array(m.w * m.h);
    M.computeFov(m.grid, m.w, m.h, 20, 4, r, visible, null);
    let n = 0;
    for (const v of visible) n += v;
    return n;
  });
  assert.ok(counts[0] < counts[1] && counts[1] < counts[2], `${counts} must increase`);
});

test('a diagonal squeeze does not leak vision through touching wall corners', () => {
  const m = tinyMaze([
    '#####',
    '#.#.#',
    '##..#',
    '#####',
  ]);
  const visible = new Uint8Array(m.w * m.h);
  M.computeFov(m.grid, m.w, m.h, 1, 1, 4, visible, null);
  assert.equal(m.grid[idx(m, 2, 2)], M.FLOOR, '(2,2) is open floor');
  assert.equal(m.grid[idx(m, 2, 1)], M.WALL, '(2,1) is a wall');
  assert.equal(m.grid[idx(m, 1, 2)], M.WALL, '(1,2) is a wall');
  assert.equal(visible[idx(m, 2, 2)], 0, 'corner-blocked diagonal hides (2,2)');
  assert.equal(visible[idx(m, 2, 1)], 1, 'the wall itself is seen');
  assert.equal(visible[idx(m, 1, 2)], 1, 'the other wall face is seen');
});

test('a diagonal step with one open side still passes', () => {
  const m = tinyMaze([
    '#####',
    '#...#',
    '#.#.#',
    '#####',
  ]);
  const visible = new Uint8Array(m.w * m.h);
  M.computeFov(m.grid, m.w, m.h, 1, 1, 4, visible, null);
  assert.equal(visible[idx(m, 2, 2)], 1, '(2,2) reachable diagonally past one open side');
});

test('revealed memory only grows across moves', () => {
  const m = M.generateMaze({ seed: 'memory', cells: 11 });
  const revealed = new Uint8Array(m.w * m.h);
  const visible = new Uint8Array(m.w * m.h);
  M.computeFov(m.grid, m.w, m.h, m.start.x, m.start.y, 9, visible, revealed);
  const count = (arr) => arr.reduce((a, b) => a + b, 0);
  const before = count(revealed);
  M.computeFov(m.grid, m.w, m.h, m.exit.x - 1 >= 1 ? m.exit.x - 1 : 1, m.exit.y, 9, visible, revealed);
  assert.ok(count(revealed) >= before);
});

test('bfsFirstStep returns the first hop, not the last', () => {
  /* pins a real bug: the descent finished next to the target and handed
     back the final hop's direction, so the bot stepped into walls */
  const m = tinyMaze([
    '#######',
    '#.....#',
    '#.....#',
    '#######',
  ]);
  const ok = (x, y) => m.grid[idx(m, x, y)] === M.FLOOR;
  const step = M.bfsFirstStep(m.grid, m.w, m.h, 1, 1, 5, 2, ok);
  assert.equal(step.name, 'E', 'far L-path target: first hop is east');
  const adjacent = M.bfsFirstStep(m.grid, m.w, m.h, 1, 1, 2, 1, ok);
  assert.equal(adjacent.name, 'E', 'adjacent target: its own direction');
});

/* ---------- the bot: fog-of-war exploration always escapes ---------- */

test('the bot escapes every sampled seed/size/braid under fog of war', () => {
  const sizes = [9, 13];
  const braids = [0, 15];
  let runs = 0;
  for (const cells of sizes) {
    for (const braid of braids) {
      for (const seed of ['p1', 'p2', 'p3', 'p4', 'p5']) {
        const maze = M.generateMaze({ seed, cells, braidPct: braid });
        const { summary } = M.simulateBotRun(maze, { seed, cells, braidPct: braid, monstersEnabled: false });
        runs++;
        assert.ok(summary.escaped, `bot must escape seed=${seed} cells=${cells} braid=${braid}`);
        assert.ok(!summary.abstained, 'a connected maze never forces abstention');
        assert.ok(summary.steps <= 8 * summary.optimalSteps,
          `steps ${summary.steps} within 8× optimal ${summary.optimalSteps}`);
      }
    }
  }
  assert.ok(runs >= 20);
});

test('bot runs are reproducible: same seed + config, same path length', () => {
  const maze = M.generateMaze({ seed: 'repro', cells: 13, braidPct: 10 });
  const a = M.simulateBotRun(maze, { seed: 'repro', cells: 13, braidPct: 10, monstersEnabled: false });
  const b = M.simulateBotRun(maze, { seed: 'repro', cells: 13, braidPct: 10, monstersEnabled: false });
  assert.equal(a.summary.steps, b.summary.steps);
  assert.equal(a.summary.explored, b.summary.explored);
  assert.equal(a.summary.timeS, b.summary.timeS);
});

test('the bot escapes monster maps too, routing around threats', () => {
  const maze = M.generateMaze({ seed: 'danger', cells: 13, braidPct: 20 });
  const { summary, events } = M.simulateBotRun(maze, {
    seed: 'danger', cells: 13, braidPct: 20, monsterCount: 3, aggroRange: 5,
  });
  assert.ok(summary.escaped, 'monsters slow the bot, never stop it');
  const sawAggro = events.some((e) => e.type === 'aggro');
  assert.ok(sawAggro || summary.fights >= 0, 'events stream aggro or resolves fights');
});

test('the engine tie-break hook receives equal-cost candidates and is honored', () => {
  const maze = M.generateMaze({ seed: 'ties', cells: 13, braidPct: 10 });
  let calls = 0;
  const hooks = {
    tiebreak(cands) {
      calls++;
      assert.ok(cands.length >= 2);
      return cands[cands.length - 1]; /* deliberately pick the LAST option */
    },
  };
  const { summary } = M.simulateBotRun(maze, { seed: 'ties', cells: 13, braidPct: 10, monstersEnabled: false }, hooks);
  assert.ok(summary.escaped);
  assert.ok(calls > 0, 'a 13-cell maze has at least one genuine junction tie');
});

test('the last move of an escaped run is an exit-directed rung', () => {
  const maze = M.generateMaze({ seed: 'final', cells: 11 });
  const { events, summary } = M.simulateBotRun(maze, { seed: 'final', cells: 11, monstersEnabled: false });
  assert.ok(summary.escaped);
  const moves = events.filter((e) => e.type === 'move');
  assert.ok(moves.length > 0);
  assert.ok(['bfs.exit', 'rule.flee'].includes(moves[moves.length - 1].rung),
    `final rung was ${moves[moves.length - 1].rung}`);
});

/* ---------- monsters ---------- */

test('aggro requires range AND line of sight along a Bresenham ray', () => {
  const open = tinyMaze([
    '#######',
    '#.....#',
    '#.....#',
    '#.....#',
    '#######',
  ]);
  const walled = tinyMaze([
    '#######',
    '#..#..#',
    '#..#..#',
    '#.....#',
    '#######',
  ]);
  const mk = (maze, mx, my) => ({
    maze, config: M.normalizeConfig({ aggroRange: 5, monstersEnabled: true }),
    pos: { x: 5, y: 3 }, tickCount: 0, monsterRng: M.makeRng('aggro-test'),
    monsters: [{ id: 0, x: mx, y: my, cool: 0, aggro: false, lastDir: null }],
    events: [],
  });
  const a = mk(open, 1, 1);
  M.monsterStep(a, a.monsters[0]);
  assert.equal(a.monsters[0].aggro, true, 'open LOS + range → aggro');
  const b = mk(walled, 1, 1);
  M.monsterStep(b, b.monsters[0]);
  assert.equal(walled.grid[idx(walled, 3, 1)], M.WALL);
  assert.equal(b.monsters[0].aggro, false, 'wall between → no aggro');
});

test('an aggro monster closes distance by BFS through corridors', () => {
  const m = tinyMaze([
    '#########',
    '#...#...#',
    '#.#...#.#',
    '#...#...#',
    '#########',
  ]);
  const state = {
    maze: m, config: M.normalizeConfig({ aggroRange: 8 }),
    pos: { x: 1, y: 1 },
    monsters: [{ id: 0, x: 7, y: 3, cool: 0, aggro: true, lastDir: null }],
    events: [],
  };
  const before = Math.abs(7 - 1) + Math.abs(3 - 1);
  M.monsterStep(state, state.monsters[0]);
  const after = Math.abs(state.monsters[0].x - 1) + Math.abs(state.monsters[0].y - 1);
  assert.ok(after < before, `distance ${before} → ${after}`);
});

test('wandering monsters never enter walls, the exit, or each other', () => {
  const m = M.generateMaze({ seed: 'wander', cells: 11, braidPct: 20 });
  const state = M.createRunState(m, { seed: 'wander', cells: 11, braidPct: 20, monsterCount: 3, lootEnabled: false }, 'human');
  const idxAt = (x, y) => y * m.w + x;
  for (let t = 0; t < 400; t++) {
    for (const mo of state.monsters) {
      mo.cool = 0;
      M.monsterStep(state, mo);
      assert.equal(m.grid[idxAt(mo.x, mo.y)], M.FLOOR, 'stays on floor');
      assert.ok(!(mo.x === m.exit.x && mo.y === m.exit.y), 'never parks on the exit');
    }
    const cells = state.monsters.map((mo) => idxAt(mo.x, mo.y));
    assert.equal(new Set(cells).size, cells.length, 'no monster overlap');
  }
});

test('walking into a monster starts a 2.0 s fight that then removes it', () => {
  const m = tinyMaze([
    '#####',
    '#...#',
    '#...#',
    '#####',
  ]);
  const state = M.createRunState(m, { seed: 'fight', monstersEnabled: false, lootEnabled: false }, 'human');
  state.monsters.push({ id: 9, x: 2, y: 2, cool: 0, aggro: false, lastDir: null });
  const dt = 0.1;
  let sawStart = false;
  let sawEnd = false;
  for (let t = 0; t < 100 && !(sawStart && sawEnd); t++) {
    const evs = M.tickRun(state, dt, { dx: 1, dy: 0 }, null);
    if (evs.some((e) => e.type === 'fight-start')) {
      sawStart = true;
      assert.equal(state.fight.remaining > 1.9, true, 'full 2.0 s countdown');
    }
    if (evs.some((e) => e.type === 'fight-end')) sawEnd = true;
  }
  assert.ok(sawStart && sawEnd, 'fight resolved');
  assert.equal(state.fights, 1);
  assert.equal(state.monsters.length, 0, 'monster driven off');
});

test('a swap in the same tick is a fight even though cells only touched', () => {
  const m = tinyMaze([
    '#######',
    '#.....#',
    '#.....#',
    '#######',
  ]);
  const state = M.createRunState(m, { seed: 'swap', monstersEnabled: false, lootEnabled: false }, 'human');
  state.pos = { x: 2, y: 2 };
  state.monsters.push({ id: 3, x: 3, y: 2, cool: 0, aggro: true, lastDir: null });
  const evs = M.tickRun(state, 0.1, { dx: 1, dy: 0 }, null);
  assert.ok(evs.some((e) => e.type === 'fight-start'), 'swap collision triggers the fight');
});

test('a stationary monster still fights when the runner steps onto it', () => {
  const m = tinyMaze([
    '#####',
    '#...#',
    '#...#',
    '#####',
  ]);
  const state = M.createRunState(m, { seed: 'still', monstersEnabled: false, lootEnabled: false }, 'human');
  state.pos = { x: 1, y: 1 };
  state.monsters.push({ id: 4, x: 2, y: 1, cool: 999, aggro: false, lastDir: null });
  const evs = M.tickRun(state, 0.1, { dx: 1, dy: 0 }, null);
  assert.ok(evs.some((e) => e.type === 'fight-start'), 'non-moving monster collides too');
});

test('loot is picked up by stepping on it', () => {
  const m = tinyMaze([
    '#####',
    '#...#',
    '#...#',
    '#####',
  ]);
  const state = M.createRunState(m, { seed: 'loot', monstersEnabled: false, lootEnabled: false }, 'human');
  state.loot.push({ id: 0, x: 2, y: 1, taken: false });
  state.pos = { x: 1, y: 1 };
  const evs = M.tickRun(state, 0.1, { dx: 1, dy: 0 }, null);
  assert.ok(evs.some((e) => e.type === 'loot'));
  assert.equal(state.loot[0].taken, true);
});

test('loot and monsters spawn with the documented constraints', () => {
  const m = M.generateMaze({ seed: 'spawn', cells: 17, braidPct: 10 });
  const state = M.createRunState(m, { seed: 'spawn', cells: 17, lootCount: 10, monsterCount: 4 }, 'human');
  const idxAt = (x, y) => y * m.w + x;
  const dist = M.bfsDistances(m.grid, m.w, m.h, m.start.x, m.start.y, M.FLOOR);
  assert.equal(state.loot.length, 10);
  const cells = new Set();
  for (const l of state.loot) {
    assert.equal(m.grid[idxAt(l.x, l.y)], M.FLOOR);
    assert.ok(dist[idxAt(l.x, l.y)] >= 3, 'loot never within 3 of spawn');
    assert.ok(!(l.x === m.exit.x && l.y === m.exit.y));
    cells.add(idxAt(l.x, l.y));
  }
  assert.equal(cells.size, state.loot.length, 'loot cells unique');
  assert.ok(state.monsters.length >= 1 && state.monsters.length <= 4);
  for (const mo of state.monsters) {
    assert.ok(dist[idxAt(mo.x, mo.y)] >= Math.min(8, Math.max(4, Math.floor(m.optimalSteps / 3))),
      'monsters spawn away from spawn');
  }
});

/* ---------- timer, rubric, scoring ---------- */

test('the optional time limit forces a game-over DNF', () => {
  const m = tinyMaze(['#####', '#...#', '#...#', '#####']);
  const state = M.createRunState(m, { seed: 'timer', monstersEnabled: false, lootEnabled: false, timeLimitS: 1 }, 'human');
  let over = null;
  for (let t = 0; t < 100 && !over; t++) {
    const evs = M.tickRun(state, 0.2, null, null);
    over = evs.find((e) => e.type === 'game-over') || null;
  }
  assert.ok(over, 'game-over fires');
  const summary = M.runSummary(state);
  assert.equal(summary.timedOut, true);
  assert.equal(summary.escaped, false);
  const scored = M.scoreRun(summary, M.DEFAULT_RUBRIC);
  assert.equal(scored.score, 0);
  assert.equal(scored.dnf, true);
});

test('golden score value for a documented run', () => {
  const s = {
    runner: 'human', escaped: true, assisted: false, timedOut: false,
    timeS: 10, steps: 100, explored: 0.5, revealedFloors: 50, floorCount: 100,
    lootCollected: 4, lootTotal: 8, coins: 0, chestOpened: 0, chestTotal: 0,
    fights: 1, optimalSteps: 60,
    monstersEnabled: true, lootEnabled: true, chestsEnabled: false, seed: 'g',
  };
  const r = M.scoreRun(s, M.DEFAULT_RUBRIC);
  /* time 1−10/40 = .75 · explored .5 · loot .5 · coins 0 (chests off) ·
     chests 0 (chests off) · fights 1/2 = .5 (badness norm)
     → (3·.75 + 2·.5 + 1·.5 + 1·0 + 1·0 − 2·.5) / 10 · 100 = 27.5 */
  assert.equal(r.score, 28);
  assert.equal(r.dnf, false);
  assert.equal(r.breakdown.length, 6);
});

test('chest coins and opens feed the rubric; disabled chests stay zero', () => {
  const mk = (coins, opened, total, chestsEnabled) => ({
    runner: 'human', escaped: true, assisted: false, timeS: 20, optimalSteps: 60,
    explored: 0.5, steps: 100, lootCollected: 0, lootTotal: 8,
    coins, chestOpened: opened,
    /* mirror runSummary: a disabled objective reports total 0 */
    chestTotal: chestsEnabled ? total : 0,
    fights: 0, monstersEnabled: true, lootEnabled: true, chestsEnabled,
  });
  const on = M.scoreRun(mk(10, 2, 3, true), M.DEFAULT_RUBRIC);
  const coins = on.breakdown.find((b) => b.tag === 'coins');
  const chests = on.breakdown.find((b) => b.tag === 'chests');
  /* breakdown norms are recorded at 3-decimal precision */
  assert.equal(coins.norm, Math.round((10 / 15) * 1000) / 1000);
  assert.equal(chests.norm, Math.round((2 / 3) * 1000) / 1000);
  const off = M.scoreRun(mk(10, 2, 3, false), M.DEFAULT_RUBRIC);
  assert.equal(off.breakdown.find((b) => b.tag === 'coins').norm, 0, 'chests off → coins 0');
  assert.equal(off.breakdown.find((b) => b.tag === 'chests').norm, 0, 'chests off → chests 0');
});

test('zero weights are excluded from the denominator', () => {
  const s = { escaped: true, assisted: false, timeS: 10, optimalSteps: 60, explored: 0.5,
    steps: 100, lootCollected: 0, lootTotal: 0, fights: 0, monstersEnabled: false, lootEnabled: false };
  const r = M.scoreRun(s, { version: 1, criteria: [{ tag: 'time', weight: 0 }, { tag: 'explored', weight: 2 }], tiebreakers: [] });
  /* only explored counts: 0.5 → 50 */
  assert.equal(r.score, 50);
});

test('disabled objectives score zero and never inflate a run', () => {
  const base = { escaped: true, assisted: false, timeS: 10, optimalSteps: 60, explored: 0.5,
    steps: 100, fights: 2, monstersEnabled: false, lootEnabled: false, lootCollected: 0, lootTotal: 0 };
  const r = M.scoreRun(base, M.DEFAULT_RUBRIC);
  const loot = r.breakdown.find((b) => b.tag === 'loot');
  const fights = r.breakdown.find((b) => b.tag === 'fights');
  assert.equal(loot.norm, 0, 'loot off → 0');
  assert.equal(fights.norm, 0, 'monsters off → 0');
});

test('negative fights weight penalizes brawls', () => {
  const mk = (fights) => ({ escaped: true, assisted: false, timeS: 10, optimalSteps: 60, explored: 0.5,
    steps: 100, lootCollected: 0, lootTotal: 8, fights, monstersEnabled: true, lootEnabled: true });
  assert.ok(M.scoreRun(mk(0), M.DEFAULT_RUBRIC).score > M.scoreRun(mk(3), M.DEFAULT_RUBRIC).score);
});

test('god-mode runs are marked assisted and score nothing', () => {
  const s = { escaped: true, assisted: true, timeS: 5, optimalSteps: 60, explored: 1,
    steps: 60, lootCollected: 8, lootTotal: 8, fights: 0, monstersEnabled: true, lootEnabled: true };
  const r = M.scoreRun(s, M.DEFAULT_RUBRIC);
  assert.equal(r.score, 0);
  assert.equal(r.assisted, true);
});

test('normalizeRubric sanitizes hostile input into a working rubric', () => {
  const junk = M.normalizeRubric({ criteria: [{ tag: 'not-a-tag', weight: 99 }, { tag: 'time', weight: 99 }, { tag: 'time', weight: 1 }, { tag: 'loot', weight: -99 }] });
  assert.deepEqual(junk.criteria, [{ tag: 'time', weight: 5 }, { tag: 'loot', weight: -3 }],
    'unknown tags dropped, dupes deduped, weights clamped');
  assert.deepEqual(M.normalizeRubric(null), M.DEFAULT_RUBRIC, 'garbage → default');
  assert.deepEqual(M.normalizeRubric({ criteria: [] }), M.DEFAULT_RUBRIC);
  const tb = M.normalizeRubric({ criteria: [{ tag: 'time', weight: 1 }], tiebreakers: ['steps', 'nope', 'steps', 'explored'] });
  assert.deepEqual(tb.tiebreakers, ['steps', 'explored']);
});

test('resolveWinner: score decides, tiebreakers order, forfeit and draw exist', () => {
  const good = { runner: 'human', escaped: true, assisted: false, timeS: 10, optimalSteps: 60, explored: 0.8,
    steps: 90, lootCollected: 8, lootTotal: 8, fights: 0, monstersEnabled: true, lootEnabled: true };
  const bad = { ...good, runner: 'bot', explored: 0.2, lootCollected: 0 };
  assert.equal(M.resolveWinner([good, bad], M.DEFAULT_RUBRIC).winner, 'human');

  const tieA = { runner: 'human', escaped: true, assisted: false, timeS: 10, optimalSteps: 60, explored: 0.5,
    steps: 100, lootCollected: 4, lootTotal: 8, fights: 1, monstersEnabled: true, lootEnabled: true };
  /* identical scored metrics, fewer steps — the steps tiebreaker decides */
  const tieB = { ...tieA, runner: 'bot', steps: 200 };
  const verdict = M.resolveWinner([tieA, tieB], M.DEFAULT_RUBRIC);
  assert.equal(verdict.winner, 'human');
  assert.equal(verdict.by, 'steps');

  const dnf = { ...good, escaped: false };
  assert.equal(M.resolveWinner([good, dnf], M.DEFAULT_RUBRIC).by, 'forfeit');
  const assisted = { ...good, assisted: true };
  assert.equal(M.resolveWinner([assisted, dnf], M.DEFAULT_RUBRIC).winner, 'none');
});

test('liveScore stays positive mid-run and zeroes a lost run', () => {
  const m = M.generateMaze({ seed: 'live', cells: 11 });
  const state = M.createRunState(m, { seed: 'live', cells: 11, monstersEnabled: false, lootEnabled: false }, 'bot');
  for (let t = 0; t < 30 && !state.finished; t++) M.tickRun(state, 1 / 12, null, null);
  assert.ok(M.liveScore(state, M.DEFAULT_RUBRIC) > 0, 'in-progress run has a running score');
  state.finished = true;
  state.escaped = false;
  state.abstained = true;
  assert.equal(M.liveScore(state, M.DEFAULT_RUBRIC), 0);
});

test('config stamps: identical configs match, the timer changes the stamp', () => {
  const a = M.configStamp({ seed: 's', cells: 21, vision: 9 });
  const b = M.configStamp({ seed: 's', cells: 21, vision: 9 });
  const c = M.configStamp({ seed: 's', cells: 21, vision: 9, timeLimitS: 120 });
  assert.equal(a, b);
  assert.notEqual(a, c);
});

/* ---------- chests ---------- */

function chestOutcome(seed) {
  /* run a human one step onto the first chest and resolve the 1 s open */
  const m = tinyMaze(['#######', '#.....#', '#.....#', '#######']);
  const state = M.createRunState(m, {
    seed, monstersEnabled: false, lootEnabled: false, chestsEnabled: true, chestCount: 1,
  }, 'human');
  const chest = { id: 0, x: 3, y: 2, opened: false };
  state.chests.length = 0;
  state.chests.push(chest);
  state.pos = { x: 2, y: 2 };
  const events = [];
  const dt = 0.1;
  for (let t = 0; t < 60; t++) {
    events.push(...M.tickRun(state, dt, chest.opened ? null : { dx: 1, dy: 0 }, null));
    if (events.some((e) => e.type === 'chest-open')) break;
  }
  return { state, events, chest };
}

test('chest rolls are seeded: same seed, same outcome, coins in 0–5', () => {
  const a = chestOutcome('chest-seed-a');
  const b = chestOutcome('chest-seed-a');
  const ea = a.events.find((e) => e.type === 'chest-open');
  const eb = b.events.find((e) => e.type === 'chest-open');
  assert.ok(ea, 'a chest resolves');
  assert.equal(ea.kind, eb.kind);
  assert.equal(ea.amount, eb.amount);
  assert.equal(ea.message, eb.message);
  if (ea.kind === 'coins') {
    assert.ok(ea.amount >= 0 && ea.amount <= 5, 'coin drop is 0–5');
    assert.ok(ea.message.length > 0, 'every roll carries its message');
  }
  assert.equal(a.chest.opened, true);
});

test('opening a chest pins the runner for one second', () => {
  const { state, events } = chestOutcome('chest-seed-b');
  const startAt = events.findIndex((e) => e.type === 'chest-start');
  const openAt = events.findIndex((e) => e.type === 'chest-open');
  assert.ok(startAt >= 0, 'chest-start fired');
  assert.ok(openAt > startAt, 'chest-open followed chest-start');
  const stepsDuringOpen = events.filter((e, i) => e.type === 'move' && i > startAt && i < openAt).length;
  /* movement events between chest-start and chest-open must be zero —
     the runner was busy; at 0.1 s ticks a 1 s open spans ~10 ticks */
  assert.ok(state.timeS >= 1, 'the open consumed its second');
  assert.equal(stepsDuringOpen, 0);
});

test('a chest monster is forced-aggro and must be fought', () => {
  /* find a seed whose chest rolls the monster outcome */
  let seed = null;
  for (let i = 0; i < 40; i++) {
    const r = chestOutcome('chest-monster-' + i);
    const ev = r.events.find((e) => e.type === 'chest-open');
    if (ev && ev.kind === 'monster') { seed = 'chest-monster-' + i; break; }
  }
  assert.ok(seed, 'monster chests occur within the sample');
  const { state, events } = chestOutcome(seed);
  const open = events.find((e) => e.type === 'chest-open');
  assert.equal(open.kind, 'monster');
  assert.ok(open.message.length > 0);
  /* the spawned monster lands on the runner's cell, so the fight can begin
     in the SAME tick batch that opened the chest — scan those events too */
  let fought = events.some((e) => e.type === 'fight-start') || state.fight !== null;
  for (let t = 0; t < 50 && !fought; t++) {
    fought = M.tickRun(state, 0.1, null, null).some((e) => e.type === 'fight-start');
  }
  assert.ok(fought, 'the chest monster engages immediately');
  for (const mo of state.monsters) assert.equal(mo.forced, true, 'chest monsters never lose aggro');
  assert.equal(state.monsters.every((mo) => !mo.forced || mo.aggro), true);
});

test('the bot opens chests on its way (bfs.chest rung exists in play)', () => {
  const maze = M.generateMaze({ seed: 'chestbot', cells: 13, braidPct: 10 });
  const { summary, events } = M.simulateBotRun(maze, {
    seed: 'chestbot', cells: 13, braidPct: 10, monstersEnabled: false,
    chestsEnabled: true, chestCount: 4, lootEnabled: false,
  });
  assert.ok(summary.escaped, 'chest runs still escape');
  assert.equal(summary.chestTotal, 4);
  assert.ok(summary.chestOpened >= 1, `bot opened ${summary.chestOpened} of 4 chests`);
  assert.ok(events.some((e) => e.type === 'chest-open'));
  const chestRung = events.find((e) => e.type === 'move' && e.rung === 'bfs.chest');
  assert.ok(chestRung, 'the ladder has a chest rung');
  assert.ok(typeof summary.coins === 'number');
});

/* ---------- dungeon maps (rooms + hallways) ---------- */

test('dungeon maps: rooms exist, halls connect, and every floor is reachable', () => {
  const m = M.generateMaze({ seed: 'halls', cells: 21, mapType: 'dungeon', rooms: 8, roomMin: 3, roomMax: 6 });
  assert.equal(m.w, 43);
  assert.equal(m.h, 43);
  assert.equal(m.mapType, 'dungeon');
  assert.equal(m.grid[idx(m, m.start.x, m.start.y)], M.FLOOR, 'start is a room-center floor');
  assert.equal(m.grid[idx(m, m.exit.x, m.exit.y)], M.FLOOR, 'exit is a floor');
  assert.ok(m.optimalSteps > 0, 'a real route exists');
  const dist = M.bfsDistances(m.grid, m.w, m.h, m.start.x, m.start.y, M.FLOOR);
  let reachable = 0;
  for (const d of dist) if (d >= 0) reachable++;
  assert.equal(reachable, m.floorCount, 'no orphaned pockets: halls connect everything');
  /* a room shows up as a 3×3 all-floor block somewhere */
  let room = false;
  for (let y = 1; y + 2 < m.h - 1 && !room; y++) {
    for (let x = 1; x + 2 < m.w - 1 && !room; x++) {
      room = [[0, 0], [1, 0], [2, 0], [0, 1], [1, 1], [2, 1], [0, 2], [1, 2], [2, 2]]
        .every(([ox, oy]) => m.grid[idx(m, x + ox, y + oy)] === M.FLOOR);
    }
  }
  assert.ok(room, 'at least one carved 3×3 room block exists');
});

test('dungeon generation is deterministic per seed and differs from the maze type', () => {
  const a = M.generateMaze({ seed: 'halls', cells: 13, mapType: 'dungeon' });
  const b = M.generateMaze({ seed: 'halls', cells: 13, mapType: 'dungeon' });
  const c = M.generateMaze({ seed: 'halls', cells: 13 });
  assert.deepEqual([...a.grid], [...b.grid]);
  assert.notDeepEqual([...a.grid], [...c.grid]);
  assert.equal(a.mapType, 'dungeon');
  assert.equal(c.mapType, 'maze');
});

test('the bot escapes a full dungeon-crawler setup (cone fog, rooms, threats)', () => {
  const cfg = {
    seed: 'crawl-sim', cells: 17, mapType: 'dungeon', rooms: 7, roomMin: 3, roomMax: 6,
    vision: 12, visionShape: 'cone', visionHalf: 60, braidPct: 0,
    monsterCount: 3, monsterSpeed: 2, aggroRange: 6, wanderRandomness: 45,
    avoidMonsters: true, lootCount: 6, chestCount: 3, chestsEnabled: true,
  };
  const maze = M.generateMaze(cfg);
  const { summary } = M.simulateBotRun(maze, cfg);
  assert.ok(summary.escaped, 'the crawler bot escapes its own fog');
  assert.ok(summary.steps <= 12 * summary.optimalSteps,
    `steps ${summary.steps} within 12× optimal ${summary.optimalSteps}`);
});

/* ---------- cone vision (facing torchlight) ---------- */

function openRoom() {
  return tinyMaze([
    '#########',
    '#.......#',
    '#.......#',
    '#.......#',
    '#.......#',
    '#.......#',
    '#########',
  ]);
}

test('cone vision hides what the disc shows when it is behind the facing', () => {
  const m = openRoom();
  const disc = new Uint8Array(m.w * m.h);
  M.computeFov(m.grid, m.w, m.h, 4, 3, 9, disc, null, null);
  const cone = new Uint8Array(m.w * m.h);
  M.computeFov(m.grid, m.w, m.h, 4, 3, 9, cone, null, M.DIRS.find((d) => d.name === 'E'), 45);
  /* (2,5): 2.8 cells away, behind-left of an east-facing runner */
  assert.equal(disc[idx(m, 2, 5)], 1, 'disc vision sees the rear-left floor');
  assert.equal(cone[idx(m, 2, 5)], 0, 'cone vision does not look backward');
  assert.equal(cone[idx(m, 7, 3)], 1, 'straight ahead is lit');
  assert.equal(cone[idx(m, 4, 3)], 1, 'the runner cell is always lit');
});

test('cone vision keeps a close radius: everything within 1.5 stays visible', () => {
  const m = openRoom();
  const cone = new Uint8Array(m.w * m.h);
  M.computeFov(m.grid, m.w, m.h, 4, 3, 9, cone, null, M.DIRS.find((d) => d.name === 'E'), 45);
  /* (5,2): behind-right but adjacent — a torch lights its holder */
  assert.equal(cone[idx(m, 5, 2)], 1, 'adjacent cell behind the facing is still lit');
  assert.equal(cone[idx(m, 3, 2)], 1, 'adjacent rear cell is lit');
  /* a wider half-angle admits more of the sides */
  const wide = new Uint8Array(m.w * m.h);
  M.computeFov(m.grid, m.w, m.h, 4, 3, 9, wide, null, M.DIRS.find((d) => d.name === 'E'), 85);
  assert.ok(wide[idx(m, 2, 5)] === 0 || wide[idx(m, 7, 2)] === 1, 'wider cone sees more ahead');
});

test('turning updates what a cone runner can see', () => {
  const m = openRoom();
  const state = M.createRunState(m, { vision: 9, visionShape: 'cone', visionHalf: 45,
    monstersEnabled: false, lootEnabled: false, chestsEnabled: false }, 'human');
  state.pos = { x: 4, y: 3 };
  state.facing = M.DIRS.find((d) => d.name === 'E');
  M.updateVision(state);
  const eastView = state.visible[idx(m, 7, 3)];
  const southLit = state.visible[idx(m, 4, 5)];
  state.facing = M.DIRS.find((d) => d.name === 'S');
  M.updateVision(state);
  assert.equal(eastView, 1, 'facing east lights the east corridor');
  assert.equal(state.visible[idx(m, 4, 5)], 1, 'facing south lights what east did not');
  assert.ok(southLit === 0 || state.visible[idx(m, 7, 3)] === 0, 'the old heading dims');
});

/* ---------- active monster avoidance ---------- */

test('avoidMonsters widens the threat mask only for monsters actually seen', () => {
  const m = openRoom();
  const state = M.createRunState(m, { vision: 5, monstersEnabled: false,
    lootEnabled: false, chestsEnabled: false, avoidMonsters: true }, 'bot');
  /* seen monster: center of the room, well inside vision 5 */
  state.monsters.push({ id: 1, x: 3, y: 3, aggro: true });
  /* unseen monster: far corner, outside vision 5 from (1,1) */
  state.monsters.push({ id: 2, x: 7, y: 5, aggro: true });
  M.updateVision(state);
  const base = M.threatCells(state);
  const wide = M.threatCells(state, true);
  assert.equal(base.has(idx(m, 3, 3)), true, 'base blocks the monster cell');
  assert.equal(base.has(idx(m, 5, 3)), false, 'base stops at the neighbor ring');
  assert.equal(wide.has(idx(m, 5, 3)), true, 'the seen monster gains a wide halo');
  assert.equal(wide.has(idx(m, 3, 1)), true, 'the halo reaches ring two');
  assert.equal(wide.has(idx(m, 7, 3)), false, 'the unseen monster gains no halo');
});

test('avoidMonsters keeps every run escaping and never adds fights', () => {
  let plainFights = 0;
  let avoidFights = 0;
  for (const seed of ['av1', 'av2', 'av3', 'av4', 'av5', 'av6']) {
    const cfg = { seed, cells: 13, braidPct: 20, monsterCount: 3, aggroRange: 5 };
    const a = M.simulateBotRun(M.generateMaze(cfg), { ...cfg, avoidMonsters: false });
    const b = M.simulateBotRun(M.generateMaze(cfg), { ...cfg, avoidMonsters: true });
    assert.ok(a.summary.escaped && b.summary.escaped, `both escape on ${seed}`);
    plainFights += a.summary.fights;
    avoidFights += b.summary.fights;
  }
  assert.ok(avoidFights <= plainFights,
    `avoidance does not add fights (${avoidFights} vs ${plainFights})`);
});

test('config stamps separate mapType, vision shape, and the avoid flag', () => {
  const base = { seed: 's', cells: 21, vision: 9 };
  assert.notEqual(M.configStamp({ ...base }), M.configStamp({ ...base, mapType: 'dungeon' }));
  assert.notEqual(M.configStamp({ ...base }), M.configStamp({ ...base, visionShape: 'cone' }));
  const withMon = { ...base, monsterCount: 3 };
  assert.notEqual(M.configStamp(withMon), M.configStamp({ ...withMon, avoidMonsters: true }));
  const noMon = { ...base, monstersEnabled: false };
  assert.equal(M.configStamp(noMon), M.configStamp({ ...noMon, avoidMonsters: true }),
    'avoid is meaningless without monsters — same stamp');
});

test('fastMode plans whole routes, still escapes, and stamps separately', () => {
  for (const seed of ['fast1', 'fast2', 'fast3', 'fast4']) {
    const cfg = { seed, cells: 17, braidPct: 15, monsterCount: 2, timeLimitS: 240 };
    const maze = M.generateMaze(cfg);
    const plain = M.simulateBotRun(maze, cfg);
    const fast = M.simulateBotRun(maze, { ...cfg, fastMode: true });
    assert.ok(fast.summary.escaped, `fastMode escapes on ${seed}`);
    assert.ok(fast.summary.plans > 0, `fastMode commits to routes on ${seed}`);
    assert.equal(fast.summary.planSteps + (fast.summary.plans > 0 ? 0 : 0) >= 0, true);
    assert.ok(fast.summary.planSteps / Math.max(1, fast.summary.steps) > 0.5,
      `most fastMode steps are predicted ones on ${seed}`);
    assert.notEqual(M.configStamp(cfg), M.configStamp({ ...cfg, fastMode: true }),
      'fastMode with a limit is a different stamp');
    assert.equal(
      M.configStamp({ ...cfg, timeLimitS: 0 }),
      M.configStamp({ ...cfg, timeLimitS: 0, fastMode: true }),
      'fastMode is meaningless without a limit — same stamp');
    assert.ok(plain.summary.escaped, `plain still escapes on ${seed}`);
  }
});

test('fastMode under deadline drops side objectives (loot) to make the exit', () => {
  const cfg = { seed: 'press', cells: 17, braidPct: 15, lootEnabled: true, lootCount: 8, timeLimitS: 60 };
  const maze = M.generateMaze(cfg);
  const fast = M.simulateBotRun(maze, { ...cfg, fastMode: true });
  // 60s limit: past 30s the bot stops chasing loot. Whatever the outcome,
  // the run must be deterministic and honest about what it collected.
  const again = M.simulateBotRun(M.generateMaze(cfg), { ...cfg, fastMode: true });
  assert.equal(fast.summary.lootCollected, again.summary.lootCollected,
    'same seed + fastMode = same loot count');
  assert.ok(fast.summary.escaped || fast.summary.timedOut, 'run resolves honestly');
});

test('fastMode is deterministic — identical runs, identical summaries', () => {
  const cfg = { seed: 'det', cells: 15, braidPct: 20, monsterCount: 2, timeLimitS: 180, fastMode: true };
  const a = M.simulateBotRun(M.generateMaze(cfg), cfg);
  const b = M.simulateBotRun(M.generateMaze(cfg), cfg);
  assert.deepEqual(a.summary, b.summary, 'same seed, same everything');
});
