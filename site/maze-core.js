/* Maze Solver core — pure logic for the maze.html + pacman.html #maze demos. No DOM, no
   canvas, no timers: everything here is deterministic given (seed, config)
   and unit-testable under `node --test` the same way benchmarks.js is
   (import for side effect, read the global).

   Design contract (docs/planning/maze-solver/PLAN.md):
   - walls are full grid cells, so Bresenham rays, fog, and movement share
     one representation;
   - vision is generalized integer Bresenham per destination cell — the ray
     reveals cells as it walks and STOPS at the first wall it hits;
   - the bot runs the cheapest-reliable-first ladder (rule → BFS → engine
     tie-break) with knowledge limited to what its own rays revealed;
   - the rubric is plain JSON scored by a pure function — the page never
     claims the WASM engine computes the arithmetic. */
"use strict";

(function () {
  const WALL = 1;
  const FLOOR = 0;
  const FIGHT_SECONDS = 2.0;
  /* deterministic direction order — N, E, S, W everywhere (rays, pathing,
     tie-breaks) so equal-cost choices resolve identically run to run */
  const DIRS = [
    { dx: 0, dy: -1, name: "N" },
    { dx: 1, dy: 0, name: "E" },
    { dx: 0, dy: 1, name: "S" },
    { dx: -1, dy: 0, name: "W" },
  ];
  const KNOWN_UNK = 0;
  const KNOWN_FLOOR = 1;
  const KNOWN_WALL = 2;

  /* chest flavor — the roll is seeded, the joke is deterministic per run */
  const CHEST_EMPTY_LINES = [
    "Nothing to see here.",
    "Dust. Just dust.",
    "A monster scurried out and took the coin with it. Rude.",
    "An IOU, dated 1987.",
    "Someone got here first. They left a polite note.",
    "The chest was a mimic. A very lazy one.",
  ];
  const CHEST_COIN_LINES = [
    "{} coins! The maze provides.",
    "{} coins, still warm. Concerning, but welcome.",
    "A tidy little stack: {} coins.",
    "{} coins and a faint smell of ozone.",
    "Jackpot adjacent — {} coins.",
  ];
  const CHEST_MONSTER_LINES = [
    "That was not coin.",
    "Something was sleeping in there. It has opinions.",
    "The lid was a jaw. Fight!",
    "Coins? It has teeth.",
  ];

  /* ---------- shared runtime (Decisions SDK) ----------
     rng and config clamping moved verbatim into sdk/decisions-sdk.js — the
     maze now re-exports them; the SDK must be loaded/imported first. */

  const DK = globalThis.DecisionsSDK;
  const hashSeed = DK.hashSeed;
  const mulberry32 = DK.mulberry32;
  const makeRng = DK.makeRng;
  const pick = DK.pick;

  /* ---------- config ---------- */

  const clamp = DK.clamp;

  function normalizeConfig(raw) {
    const c = raw || {};
    const cells = clamp(c.cells, 9, 29, 21);
    return {
      seed: String(c.seed || "oc-maze"),
      cells: cells,
      mapType: c.mapType === "dungeon" ? "dungeon" : "maze",
      rooms: clamp(c.rooms, 3, 12, 7),
      roomMin: clamp(c.roomMin, 3, 6, 3),
      roomMax: clamp(c.roomMax, 3, 9, 6),
      braidPct: clamp(c.braidPct, 0, 40, 10),
      vision: clamp(c.vision, 5, 15, 9),
      visionShape: c.visionShape === "cone" ? "cone" : "disc",
      visionHalf: clamp(c.visionHalf, 15, 85, 45),
      lootEnabled: c.lootEnabled !== false,
      lootCount: clamp(c.lootCount, 0, 16, 8),
      chestsEnabled: c.chestsEnabled === true,
      chestCount: clamp(c.chestCount, 0, 8, 3),
      monstersEnabled: c.monstersEnabled !== false,
      monsterCount: clamp(c.monsterCount, 0, 10, 3),
      monsterSpeed: clamp(c.monsterSpeed, 1, 5, 2),
      aggroRange: clamp(c.aggroRange, 2, 10, 5),
      wanderRandomness: clamp(c.wanderRandomness, 0, 100, 60),
      avoidMonsters: c.avoidMonsters === true,
      botSpeed: clamp(c.botSpeed, 2, 90, 12),
      timeLimitS: clamp(c.timeLimitS, 0, 1800, 0), /* 0 = no limit */
      fastMode: c.fastMode === true,
      godMode: c.godMode === true,
    };
  }

  /* ---------- map generation ---------- */

  /* Two map types share one representation (walls-as-cells grid):
     - "maze": recursive-backtracker corridors + braiding;
     - "dungeon": rectangular rooms connected by L-shaped hallways, with a
       few extra loop corridors so patrols and escapes have options.
     Both are fully deterministic given (seed, config). */
  function generateMaze(rawConfig) {
    const cfg = normalizeConfig(rawConfig);
    return cfg.mapType === "dungeon" ? generateDungeonMap(cfg) : generateMazeMap(cfg);
  }

  function finishMap(cfg, w, h, grid, startX, startY, exitX, exitY, optimalSteps, extra) {
    let floorCount = 0;
    for (let i = 0; i < grid.length; i++) if (grid[i] === FLOOR) floorCount++;
    return Object.assign({
      w, h, grid,
      start: { x: startX, y: startY },
      exit: { x: exitX, y: exitY },
      optimalSteps,
      floorCount,
      seed: cfg.seed,
      cells: cfg.cells,
      mapType: cfg.mapType,
    }, extra || {});
  }

  function generateMazeMap(cfg) {
    const w = 2 * cfg.cells + 1;
    const h = w;
    const grid = new Uint8Array(w * h).fill(WALL);
    const rng = makeRng(cfg.seed + ":maze");
    const idx = (x, y) => y * w + x;

    /* iterative recursive-backtracker carve over odd cells */
    const startX = (Math.floor(w / 2) % 2 === 1) ? Math.floor(w / 2) : Math.floor(w / 2) + 1;
    const startY = (Math.floor(h / 2) % 2 === 1) ? Math.floor(h / 2) : Math.floor(h / 2) + 1;
    const stack = [[startX, startY]];
    grid[idx(startX, startY)] = FLOOR;
    while (stack.length) {
      const [cx, cy] = stack[stack.length - 1];
      const options = [];
      for (const d of DIRS) {
        const nx = cx + d.dx * 2;
        const ny = cy + d.dy * 2;
        if (nx > 0 && nx < w - 1 && ny > 0 && ny < h - 1 && grid[idx(nx, ny)] === WALL) {
          options.push([nx, ny, cx + d.dx, cy + d.dy]);
        }
      }
      if (!options.length) { stack.pop(); continue; }
      const [nx, ny, wx, wy] = pick(rng, options);
      grid[idx(wx, wy)] = FLOOR;
      grid[idx(nx, ny)] = FLOOR;
      stack.push([nx, ny]);
    }

    /* braiding: convert a % of dead ends into loops so dodging monsters is
       possible when the monster objective is on. 0% keeps the perfect maze. */
    if (cfg.braidPct > 0) {
      for (let y = 1; y < h - 1; y += 2) {
        for (let x = 1; x < w - 1; x += 2) {
          const open = DIRS.filter((d) => grid[idx(x + d.dx, y + d.dy)] === FLOOR);
          if (open.length !== 1) continue;
          if (rng() * 100 >= cfg.braidPct) continue;
          const walls = DIRS.filter((d) => {
            const wx = x + d.dx;
            const wy = y + d.dy;
            const bx = x + d.dx * 2;
            const by = y + d.dy * 2;
            return bx > 0 && bx < w - 1 && by > 0 && by < h - 1 &&
              grid[idx(wx, wy)] === WALL && grid[idx(bx, by)] === FLOOR;
          });
          if (walls.length) {
            const d = pick(rng, walls);
            grid[idx(x + d.dx, y + d.dy)] = FLOOR;
          }
        }
      }
    }

    /* exit: the border ring cell whose adjacent interior floor is the
       farthest BFS point from the start — the hardest race on this map */
    const dist = bfsDistances(grid, w, h, startX, startY, FLOOR);
    let best = null;
    let bestD = -1;
    for (let y = 1; y < h - 1; y++) {
      for (let x = 1; x < w - 1; x++) {
        if (grid[idx(x, y)] !== FLOOR || dist[idx(x, y)] < 0) continue;
        const onEdge = x === 1 || x === w - 2 || y === 1 || y === h - 2;
        if (!onEdge) continue;
        const d = dist[idx(x, y)];
        if (d > bestD) { bestD = d; best = [x, y]; }
      }
    }
    /* row-major first-scan keeps this deterministic on distance ties; the
       outward ring cell is the one directly past the interior cell */
    let exitX = -1;
    let exitY = -1;
    if (best) {
      if (best[0] === 1) { exitX = 0; exitY = best[1]; }
      else if (best[0] === w - 2) { exitX = w - 1; exitY = best[1]; }
      else if (best[1] === 1) { exitX = best[0]; exitY = 0; }
      else { exitX = best[0]; exitY = h - 1; }
      grid[idx(exitX, exitY)] = FLOOR;
    } else {
      /* unreachable on a backtracker maze (odd column 1 is always carved) —
         defensive fallback keeps the map solvable rather than silent */
      exitX = 1; exitY = 1;
    }

    return finishMap(cfg, w, h, grid, startX, startY, exitX, exitY, bestD + 1,
      { braidPct: cfg.braidPct });
  }

  function generateDungeonMap(cfg) {
    const w = 2 * cfg.cells + 1;
    const h = w;
    const grid = new Uint8Array(w * h).fill(WALL);
    const rng = makeRng(cfg.seed + ":dungeon");
    const idx = (x, y) => y * w + x;

    const lo = Math.max(3, cfg.roomMin);
    const hi = Math.min(Math.max(lo, cfg.roomMax), Math.floor(w / 3));
    const rooms = [];
    const overlaps = (r) => rooms.some((o) =>
      r.x - 1 < o.x + o.w && r.x + r.w + 1 > o.x &&
      r.y - 1 < o.y + o.h && r.y + r.h + 1 > o.y);
    let attempts = cfg.rooms * 30;
    while (rooms.length < cfg.rooms && attempts-- > 0) {
      const rw = lo + Math.floor(rng() * (hi - lo + 1));
      const rh = lo + Math.floor(rng() * (hi - lo + 1));
      const rx = 1 + Math.floor(rng() * (w - rw - 2));
      const ry = 1 + Math.floor(rng() * (h - rh - 2));
      const r = { x: rx, y: ry, w: rw, h: rh };
      if (overlaps(r)) continue;
      for (let y = ry; y < ry + rh; y++) {
        for (let x = rx; x < rx + rw; x++) grid[idx(x, y)] = FLOOR;
      }
      rooms.push(r);
    }

    /* hallway between two points: horizontal leg then vertical leg (the
       corner order is rng-chosen so layouts vary without losing determinism) */
    const carveHall = (ax, ay, bx, by) => {
      if (rng() < 0.5) {
        carveRun(ax, ay, bx, ay);
        carveRun(bx, ay, bx, by);
      } else {
        carveRun(ax, ay, ax, by);
        carveRun(ax, by, bx, by);
      }
    };
    const carveRun = (x0, y0, x1, y1) => {
      const sx = Math.sign(x1 - x0);
      const sy = Math.sign(y1 - y0);
      let x = x0;
      let y = y0;
      grid[idx(x, y)] = FLOOR;
      while (x !== x1) { x += sx; grid[idx(x, y)] = FLOOR; }
      while (y !== y1) { y += sy; grid[idx(x, y)] = FLOOR; }
    };

    const center = (r) => ({ x: r.x + (r.w >> 1), y: r.y + (r.h >> 1) });
    for (let i = 1; i < rooms.length; i++) {
      const a = center(rooms[i - 1]);
      const b = center(rooms[i]);
      carveHall(a.x, a.y, b.x, b.y);
    }
    /* loop corridors: every ~3 rooms adds one shortcut elsewhere on the
       chain, so the dungeon is not a tree and monsters can be dodged */
    if (rooms.length >= 2) {
      const loops = Math.floor(rooms.length / 3);
      for (let i = 0; i < loops; i++) {
        const a = center(pick(rng, rooms));
        const b = center(pick(rng, rooms));
        if (a.x === b.x && a.y === b.y) continue;
        carveHall(a.x, a.y, b.x, b.y);
      }
    }

    /* start: center of the room nearest the middle of the map */
    let start = rooms.length ? center(rooms[0]) : { x: 1, y: 1 };
    let bestRoomDist = Infinity;
    for (const r of rooms) {
      const c = center(r);
      const d = Math.abs(c.x - (w >> 1)) + Math.abs(c.y - (h >> 1));
      if (d < bestRoomDist) { bestRoomDist = d; start = c; }
    }

    /* exit: the farthest reachable floor cell from the start — same "hardest
       race" rule as the maze, but interior: a vault door, not a border gap */
    const dist = bfsDistances(grid, w, h, start.x, start.y, FLOOR);
    let exitX = start.x;
    let exitY = start.y;
    let bestD = -1;
    for (let y = 1; y < h - 1; y++) {
      for (let x = 1; x < w - 1; x++) {
        if (grid[idx(x, y)] !== FLOOR || dist[idx(x, y)] < 0) continue;
        if ((x === start.x && y === start.y) || dist[idx(x, y)] <= bestD) continue;
        bestD = dist[idx(x, y)];
        exitX = x;
        exitY = y;
      }
    }
    if (bestD < 0) { exitX = start.x; exitY = start.y; bestD = 0; }

    return finishMap(cfg, w, h, grid, start.x, start.y, exitX, exitY, bestD,
      { rooms: rooms.length });
  }

  /* ---------- generalized Bresenham (all octants, integer only) ---------- */

  function bresenhamLine(x0, y0, x1, y1) {
    const pts = [];
    let dx = Math.abs(x1 - x0);
    let dy = -Math.abs(y1 - y0);
    const sx = x0 < x1 ? 1 : -1;
    const sy = y0 < y1 ? 1 : -1;
    let err = dx + dy;
    let x = x0;
    let y = y0;
    for (;;) {
      pts.push({ x, y });
      if (x === x1 && y === y1) break;
      const e2 = 2 * err;
      if (e2 >= dy) { err += dy; x += sx; }
      if (e2 <= dx) { err += dx; y += sy; }
    }
    return pts;
  }

  /* Walk one ray. visit(idx, isWall) is called for every cell until the ray
     stops: at the destination, or at the first wall (the wall cell itself is
     visited — you see the wall face; nothing behind it), or at a blocked
     diagonal corner (both orthogonal neighbors of the step are walls — no
     peeking through the joint). Returns the index of the cell that stopped
     the ray, or -1 when it reached the destination unblocked. */
  function castRay(grid, w, h, x0, y0, x1, y1, visit) {
    const idx = (x, y) => y * w + x;
    let dx = Math.abs(x1 - x0);
    let dy = -Math.abs(y1 - y0);
    const sx = x0 < x1 ? 1 : -1;
    const sy = y0 < y1 ? 1 : -1;
    let err = dx + dy;
    let x = x0;
    let y = y0;
    for (;;) {
      const here = idx(x, y);
      if (grid[here] === WALL) { visit(here, true); return here; }
      visit(here, false);
      if (x === x1 && y === y1) return -1;
      const px = x;
      const py = y;
      const e2 = 2 * err;
      if (e2 >= dy) { err += dy; x += sx; }
      if (e2 <= dx) { err += dx; y += sy; }
      if (px !== x && py !== y) {
        const sideA = idx(px + (x - px), py);
        const sideB = idx(px, py + (y - py));
        if (grid[sideA] === WALL && grid[sideB] === WALL) return here;
      }
    }
  }

  /* Vision: one Bresenham ray per destination cell inside the view shape.
     Default shape is the full disc (radius). `facing` (a DIR) + `halfDeg`
     switch to the crawler cone: only cells within the angular span of the
     facing direction get a ray — the same wall-stop and corner rules apply
     per ray. Immediate neighbors (≤ 1.5 cells) always count as seen, so a
     corner-facing spawn is never fully blind. Marks `visible` (this frame)
     and folds into `revealed` (memory). */
  function computeFov(grid, w, h, px, py, radius, visible, revealed, facing, halfDeg) {
    visible.fill(0);
    const r2 = radius * radius;
    const cone = !!facing;
    const cosHalf = cone ? Math.cos((halfDeg || 45) * Math.PI / 180) : 0;
    for (let dy = -radius; dy <= radius; dy++) {
      for (let dx = -radius; dx <= radius; dx++) {
        if (dx * dx + dy * dy > r2) continue;
        if (cone) {
          const d = Math.sqrt(dx * dx + dy * dy);
          if (d > 1.5) {
            const dot = (dx * facing.dx + dy * facing.dy) / d;
            if (dot < cosHalf) continue;
          }
        }
        const x = px + dx;
        const y = py + dy;
        if (x < 0 || y < 0 || x >= w || y >= h) continue;
        castRay(grid, w, h, px, py, x, y, (ci) => {
          visible[ci] = 1;
          if (revealed) revealed[ci] = 1;
        });
      }
    }
    return visible;
  }

  /* ---------- BFS over a grid ---------- */

  function bfsDistances(grid, w, h, sx, sy, passable) {
    /* passable: cell value, or a callback (bot pathing over the known map).
       head-index queue instead of shift() — grids are small but this runs
       on every bot decision */
    const dist = new Int32Array(w * h).fill(-1);
    const byValue = typeof passable !== "function";
    const qx = new Int16Array(w * h);
    const qy = new Int16Array(w * h);
    let head = 0;
    let tail = 0;
    qx[tail] = sx; qy[tail] = sy; tail++;
    dist[sy * w + sx] = 0;
    while (head < tail) {
      const x = qx[head];
      const y = qy[head];
      head++;
      for (const d of DIRS) {
        const nx = x + d.dx;
        const ny = y + d.dy;
        if (nx < 0 || ny < 0 || nx >= w || ny >= h) continue;
        const ni = ny * w + nx;
        if (dist[ni] !== -1) continue;
        if (byValue ? grid[ni] !== passable : !passable(nx, ny)) continue;
        dist[ni] = dist[y * w + x] + 1;
        qx[tail] = nx; qy[tail] = ny; tail++;
      }
    }
    return dist;
  }

  function bfsFirstStep(grid, w, h, sx, sy, tx, ty, passable) {
    /* BFS from the target back to the source, then descend — yields the
       first step of a shortest path without storing full paths */
    if (sx === tx && sy === ty) return null;
    const dist = new Int32Array(w * h).fill(-1);
    const ok = (x, y) => x >= 0 && y >= 0 && x < w && y < h && passable(x, y);
    const qx = new Int16Array(w * h);
    const qy = new Int16Array(w * h);
    let head = 0;
    let tail = 0;
    qx[tail] = tx; qy[tail] = ty; tail++;
    dist[ty * w + tx] = 0;
    while (head < tail) {
      const x = qx[head];
      const y = qy[head];
      head++;
      if (x === sx && y === sy) break;
      for (const d of DIRS) {
        const nx = x + d.dx;
        const ny = y + d.dy;
        if (!ok(nx, ny)) continue;
        const ni = ny * w + nx;
        if (dist[ni] !== -1) continue;
        dist[ni] = dist[y * w + x] + 1;
        qx[tail] = nx; qy[tail] = ny; tail++;
      }
    }
    if (dist[sy * w + sx] === -1) return null;
    /* descend from source toward target; the FIRST hop's direction is the
       answer even though the walk finishes next to the target */
    let first = null;
    let cx = sx;
    let cy = sy;
    for (;;) {
      let advanced = false;
      for (const d of DIRS) {
        const nx = cx + d.dx;
        const ny = cy + d.dy;
        if (nx === tx && ny === ty) return first || d;
        if (!ok(nx, ny)) continue;
        if (dist[ny * w + nx] === dist[cy * w + cx] - 1) {
          if (!first) first = d;
          cx = nx;
          cy = ny;
          advanced = true;
          break;
        }
      }
      if (!advanced) return null; /* defensive: should not happen on a mapped path */
    }
  }

  /* the WHOLE route, not just the first hop: BFS from the target, then walk
     the descent back up. Deterministic — the same DIRS order breaks every
     equal-distance tie, so a given (map, target) always yields one route.
     Returns [] when the target is unreachable over the passable set. */
  function bfsFullPath(grid, w, h, sx, sy, tx, ty, passable) {
    if (sx === tx && sy === ty) return [];
    /* distance map from the SOURCE, then climb down from the target:
       each hop lands on a cell one step closer to the source, so the
       collected directions (target→source) reverse into the route */
    const dist = bfsDistances(grid, w, h, sx, sy, passable);
    let d = dist[ty * w + tx];
    if (d < 0) return [];
    const path = [];
    let x = tx;
    let y = ty;
    while (d > 0) {
      let advanced = false;
      for (const dd of DIRS) {
        const nx = x + dd.dx;
        const ny = y + dd.dy;
        if (nx < 0 || ny < 0 || nx >= w || ny >= h) continue;
        if (dist[ny * w + nx] === d - 1) {
          path.push({ name: dd.name, dx: -dd.dx, dy: -dd.dy });
          x = nx;
          y = ny;
          d--;
          advanced = true;
          break;
        }
      }
      if (!advanced) return []; /* defensive: distance map desync */
    }
    path.reverse();
    return path;
  }

  /* ---------- run state (shared by the live shell and headless sim) ---------- */

  function createRunState(maze, rawConfig, runnerType) {
    const cfg = normalizeConfig(rawConfig);
    const n = maze.w * maze.h;
    const state = {
      maze,
      config: cfg,
      runnerType: runnerType === "human" ? "human" : "bot",
      revealed: new Uint8Array(n),
      visible: new Uint8Array(n),
      known: new Uint8Array(n),
      pos: { x: maze.start.x, y: maze.start.y },
      facing: openFacing(maze), /* cone vision: the way the runner looks */
      steps: 0,
      timeS: 0,
      cool: 0,
      escaped: false,
      botPlan: null, /* beat-the-clock: the committed route {kind,x,y,path,rung,note} */
      planCount: 0, planSteps: 0, planReplans: 0,
      finished: false,
      assisted: false,
      abstained: false,
      timedOut: false,
      fights: 0,
      fight: null, /* {monsterId, remaining} */
      chest: null, /* {id, remaining} while opening */
      coins: 0,
      chests: [],
      revealedFloors: 0,
      moveCount: 0,
      tickCount: 0,
      loot: [],
      monsters: [],
      monsterRng: makeRng(cfg.seed + ":monsters"),
      nextMonsterId: 1000, /* chest-spawned monsters id above placed ones */
      lastDecision: null,
      events: [],
    };
    for (let i = 0; i < n; i++) state.known[i] = KNOWN_UNK;

    /* loot: dead-end floors preferred, never next to the spawn */
    if (cfg.lootEnabled && cfg.lootCount > 0) {
      const dist = bfsDistances(maze.grid, maze.w, maze.h, maze.start.x, maze.start.y, FLOOR);
      const dead = [];
      const pool = [];
      const idx = (x, y) => y * maze.w + x;
      for (let y = 1; y < maze.h - 1; y++) {
        for (let x = 1; x < maze.w - 1; x++) {
          if (maze.grid[idx(x, y)] !== FLOOR) continue;
          if (dist[idx(x, y)] < 3) continue;
          if (x === maze.exit.x && y === maze.exit.y) continue;
          const open = DIRS.filter((d) => maze.grid[idx(x + d.dx, y + d.dy)] === FLOOR);
          (open.length === 1 ? dead : pool).push({ x, y });
        }
      }
      const chosen = [];
      const taken = new Set();
      const take = (arr) => {
        const avail = arr.filter((c) => !taken.has(idx(c.x, c.y)));
        if (!avail.length) return false;
        const c = pick(state.monsterRng, avail);
        taken.add(idx(c.x, c.y));
        chosen.push({ id: chosen.length, x: c.x, y: c.y, taken: false });
        return true;
      };
      for (let i = 0; i < cfg.lootCount; i++) {
        if (!take(dead)) { if (!take(pool)) break; }
      }
      state.loot = chosen;
    }

    /* monsters: far from spawn, unique cells, never on the exit */
    if (cfg.monstersEnabled && cfg.monsterCount > 0) {
      const dist = bfsDistances(maze.grid, maze.w, maze.h, maze.start.x, maze.start.y, FLOOR);
      const minD = Math.min(8, Math.max(4, Math.floor(maze.optimalSteps / 3)));
      const spots = [];
      const idx = (x, y) => y * maze.w + x;
      for (let y = 1; y < maze.h - 1; y++) {
        for (let x = 1; x < maze.w - 1; x++) {
          if (maze.grid[idx(x, y)] !== FLOOR) continue;
          if ((x === maze.exit.x && y === maze.exit.y) || dist[idx(x, y)] < minD) continue;
          spots.push({ x, y });
        }
      }
      const used = new Set();
      for (let i = 0; i < cfg.monsterCount; i++) {
        const avail = spots.filter((c) => !used.has(idx(c.x, c.y)));
        if (!avail.length) break;
        const c = pick(state.monsterRng, avail);
        used.add(idx(c.x, c.y));
        state.monsters.push({
          id: i, x: c.x, y: c.y, cool: 0, aggro: false, lastDir: null,
        });
      }
    }

    /* chests: same placement rules as loot, and never on a loot cell */
    if (cfg.chestsEnabled && cfg.chestCount > 0) {
      const dist = bfsDistances(maze.grid, maze.w, maze.h, maze.start.x, maze.start.y, FLOOR);
      const idx = (x, y) => y * maze.w + x;
      const lootCells = new Set(state.loot.map((l) => idx(l.x, l.y)));
      const spots = [];
      for (let y = 1; y < maze.h - 1; y++) {
        for (let x = 1; x < maze.w - 1; x++) {
          if (maze.grid[idx(x, y)] !== FLOOR) continue;
          if (dist[idx(x, y)] < 3) continue;
          if ((x === maze.exit.x && y === maze.exit.y) || lootCells.has(idx(x, y))) continue;
          spots.push({ x, y });
        }
      }
      const used = new Set();
      for (let i = 0; i < cfg.chestCount; i++) {
        const avail = spots.filter((c) => !used.has(idx(c.x, c.y)));
        if (!avail.length) break;
        const c = pick(state.monsterRng, avail);
        used.add(idx(c.x, c.y));
        state.chests.push({ id: i, x: c.x, y: c.y, opened: false });
      }
    }

    updateVision(state);
    return state;
  }

  /* cone spawn: face the first open neighbor (N,E,S,W order) so a runner
     never starts staring into solid rock; E is the fallback */
  function openFacing(maze) {
    const idx = (x, y) => y * maze.w + x;
    for (const d of DIRS) {
      const nx = maze.start.x + d.dx;
      const ny = maze.start.y + d.dy;
      if (nx >= 0 && ny >= 0 && nx < maze.w && ny < maze.h && maze.grid[idx(nx, ny)] === FLOOR) {
        return d;
      }
    }
    return DIRS[1];
  }

  function updateVision(state) {
    const cone = state.config.visionShape === "cone";
    computeFov(state.maze.grid, state.maze.w, state.maze.h,
      state.pos.x, state.pos.y, state.config.vision, state.visible, state.revealed,
      cone ? state.facing : null, state.config.visionHalf);
    const idx = (x, y) => y * state.maze.w + x;
    for (let dy = -state.config.vision; dy <= state.config.vision; dy++) {
      for (let dx = -state.config.vision; dx <= state.config.vision; dx++) {
        const x = state.pos.x + dx;
        const y = state.pos.y + dy;
        if (x < 0 || y < 0 || x >= state.maze.w || y >= state.maze.h) continue;
        const ci = idx(x, y);
        if (!state.visible[ci]) continue;
        state.known[ci] = state.maze.grid[ci] === WALL ? KNOWN_WALL : KNOWN_FLOOR;
      }
    }
  }

  function exploredPct(state) {
    return state.maze.floorCount > 0 ? state.revealedFloors / state.maze.floorCount : 0;
  }

  /* threat map for the bot pathfinder: every monster's own cell is blocked
     (never walk into one on purpose), and an aggro'd monster also poisons
     its 4-neighbors so the bot routes around, not through. Driven by the
     monster list, not the monsters toggle — chest-spawned monsters exist
     even on monster-free maps. */
  /* cells the ladder refuses to route through. Base: every monster's own
     cell plus an aggro monster's four neighbors. `wide` (avoidMonsters on)
     adds a two-ring halo around every monster the runner can actually SEE —
     route planning bends around known threats instead of through them. */
  function threatCells(state, wide) {
    const blocked = new Set();
    const maze = state.maze;
    const idx = (x, y) => y * maze.w + x;
    const inside = (x, y) => x >= 0 && y >= 0 && x < maze.w && y < maze.h;
    for (const m of state.monsters) {
      if (!inside(m.x, m.y)) continue;
      blocked.add(idx(m.x, m.y));
      if (!m.aggro) continue;
      for (const d of DIRS) {
        if (inside(m.x + d.dx, m.y + d.dy)) blocked.add(idx(m.x + d.dx, m.y + d.dy));
      }
    }
    if (wide && state.config.avoidMonsters) {
      const r = 2;
      for (const m of state.monsters) {
        if (!inside(m.x, m.y) || state.visible[idx(m.x, m.y)] !== 1) continue;
        for (let dy = -r; dy <= r; dy++) {
          for (let dx = -r; dx <= r; dx++) {
            if (inside(m.x + dx, m.y + dy)) blocked.add(idx(m.x + dx, m.y + dy));
          }
        }
      }
    }
    return blocked;
  }

  /* ---------- the bot decision ladder ---------- */

  /* Rungs, cheapest-first:
     fight → rule.exit → bfs.exit → rule.flee → bfs.loot → bfs.frontier → abstain.
     Every decision returns {rung, dir, note, tie?} — `tie` lists equal-cost
     alternatives the shell may hand to the WASM engine. */
  function botDecide(state) {
    const maze = state.maze;
    const idx = (x, y) => y * maze.w + x;
    const passableKnown = (x, y) => {
      if (x < 0 || y < 0 || x >= maze.w || y >= maze.h) return false;
      return state.known[idx(x, y)] === KNOWN_FLOOR;
    };

    if (state.fight) {
      return { rung: "fight", dir: null, note: "engaged — holding position" };
    }
    if (state.pos.x === maze.exit.x && state.pos.y === maze.exit.y) {
      return { rung: "rule.exit", dir: null, note: "standing on the exit" };
    }

    const threats = threatCells(state, true);
    const threatened = threats.has(idx(state.pos.x, state.pos.y));

    /* exit revealed → beeline over the known map */
    if (state.known[idx(maze.exit.x, maze.exit.y)] === KNOWN_FLOOR) {
      let dir = bfsFirstStep(maze.grid, maze.w, maze.h, state.pos.x, state.pos.y,
        maze.exit.x, maze.exit.y, passableKnown);
      if (dir) {
        const nx = state.pos.x + dir.dx;
        const ny = state.pos.y + dir.dy;
        if (threats.has(idx(nx, ny))) {
          const safe = evade(state, threats);
          if (safe) return { rung: "rule.flee", dir: safe, note: "exit path threatened — evading" };
          return { rung: "rule.flee", dir, note: "exit path threatened — no safe detour" };
        }
        return { rung: "bfs.exit", dir, note: "exit in sight — shortest known path" };
      }
    }

    /* fighting is forced by adjacency elsewhere; here: retreat when standing
       in a threat cell and no exit route above took us out of it */
    if (threatened) {
      const safe = evade(state, threats);
      if (safe) return { rung: "rule.flee", dir: safe, note: "monster too close — retreating" };
    }

    /* beat-the-clock deadline: past half the limit, side objectives lose to
       the exit (fastMode only — plain runs keep collecting) */
    const pressing = state.config.fastMode && state.config.timeLimitS > 0 &&
      state.timeS >= 0.5 * state.config.timeLimitS;

    /* committed-route follow: the plan was decided once, whole path and all
       (moves predicted ahead of time); each tick only re-checks that it
       still holds. No per-step re-decision, no frontier ping-pong, none of
       the retrace churn a greedy re-planner pays on every branch. */
    if (state.config.fastMode && state.botPlan) {
      const plan = state.botPlan;
      const wants = plan.kind === "loot"
        ? state.loot.some((l) => !l.taken && l.x === plan.x && l.y === plan.y)
        : plan.kind === "chest"
          ? state.chests.some((c) => !c.opened && c.x === plan.x && c.y === plan.y)
          : state.known[idx(plan.x, plan.y)] === KNOWN_FLOOR &&
            DIRS.some((d) => {
              const nx = plan.x + d.dx;
              const ny = plan.y + d.dy;
              return nx >= 0 && ny >= 0 && nx < maze.w && ny < maze.h &&
                state.known[idx(nx, ny)] === KNOWN_UNK;
            });
      if (!wants || (state.pos.x === plan.x && state.pos.y === plan.y)) {
        state.botPlan = null; /* served — pick the next objective below */
      } else if (!plan.path.length) {
        state.botPlan = null; /* route ran dry without landing — re-decide */
      } else {
        const d = plan.path[0];
        const nx = state.pos.x + d.dx;
        const ny = state.pos.y + d.dy;
        if (passableKnown(nx, ny) && !threats.has(idx(nx, ny))) {
          plan.path.shift();
          state.planSteps++;
          return { rung: plan.rung, dir: d, note: plan.note };
        }
        state.botPlan = null; /* interrupted (threat grew onto the route) */
        state.planReplans++;
      }
    }

    /* revealed loot (objective on) → nearest safe pickup. One distance map
       over the threat-free known map, then min-dist target (x/y tie-break) —
       never a direction-order pick, which can flip-flop between two piles. */
    if (state.config.lootEnabled && !pressing) {
      const ldist = bfsDistances(maze.grid, maze.w, maze.h, state.pos.x, state.pos.y,
        (x, y) => passableKnown(x, y) && !threats.has(idx(x, y)));
      let target = null;
      for (const l of state.loot) {
        if (l.taken) continue;
        if (state.known[idx(l.x, l.y)] !== KNOWN_FLOOR) continue;
        const d = ldist[idx(l.x, l.y)];
        if (d < 0) continue;
        if (!target || d < target.d ||
          (d === target.d && (l.x < target.x || (l.x === target.x && l.y < target.y)))) {
          target = { d, x: l.x, y: l.y };
        }
      }
      if (target) {
        const pass = (x, y) => passableKnown(x, y) && !threats.has(idx(x, y));
        const dir = bfsFirstStep(maze.grid, maze.w, maze.h, state.pos.x, state.pos.y,
          target.x, target.y, pass);
        if (dir) {
          if (state.config.fastMode) {
            const path = bfsFullPath(maze.grid, maze.w, maze.h,
              state.pos.x, state.pos.y, target.x, target.y, pass);
            if (path.length) {
              state.botPlan = { kind: "loot", x: target.x, y: target.y, path,
                rung: "bfs.loot", note: "planned route — " + path.length + " steps to loot" };
              state.planCount++;
              const d0 = state.botPlan.path.shift();
              state.planSteps++;
              return { rung: "bfs.loot", dir: d0, note: state.botPlan.note };
            }
          }
          return { rung: "bfs.loot", dir, note: "loot spotted — collecting" };
        }
      }
    }

    /* revealed unopened chests (objective on) → nearest safe chest, same
       nearest-by-distance selection as loot */
    if (state.config.chestsEnabled && !pressing) {
      const cdist = bfsDistances(maze.grid, maze.w, maze.h, state.pos.x, state.pos.y,
        (x, y) => passableKnown(x, y) && !threats.has(idx(x, y)));
      let target = null;
      for (const ch of state.chests) {
        if (ch.opened) continue;
        if (state.known[idx(ch.x, ch.y)] !== KNOWN_FLOOR) continue;
        const d = cdist[idx(ch.x, ch.y)];
        if (d < 0) continue;
        if (!target || d < target.d ||
          (d === target.d && (ch.x < target.x || (ch.x === target.x && ch.y < target.y)))) {
          target = { d, x: ch.x, y: ch.y };
        }
      }
      if (target) {
        const pass = (x, y) => passableKnown(x, y) && !threats.has(idx(x, y));
        const dir = bfsFirstStep(maze.grid, maze.w, maze.h, state.pos.x, state.pos.y,
          target.x, target.y, pass);
        if (dir) {
          if (state.config.fastMode) {
            const path = bfsFullPath(maze.grid, maze.w, maze.h,
              state.pos.x, state.pos.y, target.x, target.y, pass);
            if (path.length) {
              state.botPlan = { kind: "chest", x: target.x, y: target.y, path,
                rung: "bfs.chest", note: "planned route — " + path.length + " steps to chest" };
              state.planCount++;
              const d0 = state.botPlan.path.shift();
              state.planSteps++;
              return { rung: "bfs.chest", dir: d0, note: state.botPlan.note };
            }
          }
          return { rung: "bfs.chest", dir, note: "chest spotted — opening it" };
        }
      }
    }

    /* frontier: nearest known floor cell that still borders the unknown */
    let bestFrontier = null;
    let bestStep = null;
    let tied = [];
    const fdist = bfsDistances(maze.grid, maze.w, maze.h, state.pos.x, state.pos.y,
      (x, y) => passableKnown(x, y) && !threats.has(idx(x, y)));
    const fdistUnsafe = threats.size
      ? bfsDistances(maze.grid, maze.w, maze.h, state.pos.x, state.pos.y, passableKnown)
      : null;
    /* fastMode pattern logic: a frontier that borders MORE unknown is worth
       more per step — count unknown borders and score value = cost minus an
       information bonus (bigger under deadline). Plain runs keep pure
       nearest-frontier, so classic behavior and old replays are untouched. */
    const infoMode = state.config.fastMode;
    for (let y = 0; y < maze.h; y++) {
      for (let x = 0; x < maze.w; x++) {
        if (state.known[idx(x, y)] !== KNOWN_FLOOR) continue;
        let unknownBorders = 0;
        for (const d of DIRS) {
          const nx = x + d.dx;
          const ny = y + d.dy;
          if (nx >= 0 && ny >= 0 && nx < maze.w && ny < maze.h &&
            state.known[idx(nx, ny)] === KNOWN_UNK) unknownBorders++;
        }
        if (!unknownBorders) continue;
        const d = fdist[idx(x, y)];
        const du = fdistUnsafe ? fdistUnsafe[idx(x, y)] : d;
        if (d < 0 && (du < 0 || threats.size === 0)) continue;
        const useD = d >= 0 ? d : du;
        if (useD < 0) continue;
        const score = infoMode
          ? useD - (unknownBorders - 1) * (pressing ? 1.5 : 0.5)
          : useD;
        const better = !bestFrontier ||
          score < bestFrontier.score ||
          (score === bestFrontier.score &&
            (useD < bestFrontier.d ||
              (useD === bestFrontier.d && (x < bestFrontier.x || (x === bestFrontier.x && y < bestFrontier.y)))));
        if (better) {
          bestFrontier = { x, y, d: useD, score, ub: unknownBorders };
          tied = [[x, y, useD]];
        } else if (score === bestFrontier.score && useD === bestFrontier.d) {
          tied.push([x, y, useD]);
        }
      }
    }
    if (bestFrontier) {
      const step = bfsFirstStep(maze.grid, maze.w, maze.h, state.pos.x, state.pos.y,
        bestFrontier.x, bestFrontier.y,
        (x, y) => passableKnown(x, y) && !threats.has(idx(x, y)) && fdist[idx(x, y)] >= 0)
        || bfsFirstStep(maze.grid, maze.w, maze.h, state.pos.x, state.pos.y,
          bestFrontier.x, bestFrontier.y, passableKnown);
      if (step) {
        /* equal-cost frontiers reaching the runner through different first
           steps = a genuine junction tie → the shell may ask the engine */
        const altSteps = new Map();
        for (const [fx, fy, d] of tied) {
          const s = bfsFirstStep(maze.grid, maze.w, maze.h, state.pos.x, state.pos.y, fx, fy, passableKnown);
          if (s && d === bestFrontier.d && !altSteps.has(s.name)) {
            altSteps.set(s.name, { dir: s, at: { x: fx, y: fy } });
          }
        }
        if (altSteps.size > 1) {
          const cands = DIRS.filter((d) => altSteps.has(d.name)).map((d) => altSteps.get(d.name).dir);
          return {
            rung: "bfs.frontier",
            dir: altSteps.has(step.name) ? step : cands[0],
            tie: cands,
            note: "junction: " + cands.length + " equal-cost frontiers",
          };
        }
        /* fastMode: commit to the whole route NOW — moves predicted ahead of
           time, then followed without re-deliberation */
        if (infoMode) {
          const pass = (x, y) => passableKnown(x, y) && !threats.has(idx(x, y)) && fdist[idx(x, y)] >= 0;
          const path = bfsFullPath(maze.grid, maze.w, maze.h, state.pos.x, state.pos.y,
            bestFrontier.x, bestFrontier.y, pass) ||
            bfsFullPath(maze.grid, maze.w, maze.h, state.pos.x, state.pos.y,
              bestFrontier.x, bestFrontier.y, passableKnown);
          if (path.length) {
            state.botPlan = {
              kind: "frontier", x: bestFrontier.x, y: bestFrontier.y,
              path, rung: "bfs.frontier",
              note: "planned route — " + path.length + " steps to frontier" +
                (pressing ? " (deadline)" : ""),
            };
            state.planCount++;
            const d0 = state.botPlan.path.shift();
            state.planSteps++;
            return { rung: "bfs.frontier", dir: d0, note: state.botPlan.note };
          }
        }
        return { rung: "bfs.frontier", dir: step,
          note: infoMode ? "exploring best-value frontier" : "exploring nearest frontier" };
      }
    }

    /* crawler fog: the known map is fully explored but unknown cells remain —
       the frontier is behind the runner. Rotate toward the densest unknown
       fan (a real action, visible in the trace) rather than abstaining. */
    if (state.config.visionShape === "cone") {
      let hasUnknown = false;
      for (let i = 0; i < state.known.length; i++) {
        if (state.known[i] === KNOWN_UNK) { hasUnknown = true; break; }
      }
      if (hasUnknown) {
        let best = null;
        for (const d of DIRS) {
          const n = countUnknownInFan(state, d);
          if (!best || n > best.n) best = { d, n };
        }
        return {
          rung: "turn", dir: best.d,
          note: "no reachable frontier — turning (" + best.n + " unknown ahead)",
        };
      }
    }

    return { rung: "abstain", dir: null, note: "no known route — refusing to guess" };
  }

  /* how much unexplored area lies in the cone around direction d — the
     bot's lookahead for the turn rung. Scans the vision disc once per
     candidate; grid sizes here make that trivial. */
  function countUnknownInFan(state, d) {
    const maze = state.maze;
    const r = state.config.vision;
    const cosHalf = Math.cos((state.config.visionHalf || 45) * Math.PI / 180);
    let n = 0;
    for (let dy = -r; dy <= r; dy++) {
      for (let dx = -r; dx <= r; dx++) {
        if (dx * dx + dy * dy > r * r) continue;
        const dist = Math.sqrt(dx * dx + dy * dy);
        if (dist > 1.5) {
          if ((dx * d.dx + dy * d.dy) / dist < cosHalf) continue;
        }
        const x = state.pos.x + dx;
        const y = state.pos.y + dy;
        if (x < 0 || y < 0 || x >= maze.w || y >= maze.h) continue;
        if (state.known[y * maze.w + x] === KNOWN_UNK) n++;
      }
    }
    return n;
  }

  function evade(state, threats) {
    /* retreat: among safe known neighbors, the one farthest from every
       aggro'd monster (max of min-distance), N/E/S/W order as tie-break */
    const maze = state.maze;
    const idx = (x, y) => y * maze.w + x;
    let best = null;
    for (const d of DIRS) {
      const nx = state.pos.x + d.dx;
      const ny = state.pos.y + d.dy;
      if (state.known[idx(nx, ny)] !== KNOWN_FLOOR) continue;
      if (threats.has(idx(nx, ny))) continue;
      let nearest = Infinity;
      for (const m of state.monsters) {
        if (!m.aggro) continue;
        nearest = Math.min(nearest, Math.abs(m.x - nx) + Math.abs(m.y - ny));
      }
      const far = nearest === Infinity ? 9999 : nearest;
      if (!best || far > best.far ||
        (far === best.far && DIRS.findIndex((x) => x.name === d.name) < DIRS.findIndex((x) => x.name === best.dir.name))) {
        best = { dir: d, far };
      }
    }
    return best ? best.dir : null;
  }

  /* ---------- monsters ---------- */

  function hasLos(grid, w, h, ax, ay, bx, by) {
    return castRay(grid, w, h, ax, ay, bx, by, () => {}) === -1;
  }

  function monsterStep(state, m) {
    const maze = state.maze;
    const cfg = state.config;
    const idx = (x, y) => y * maze.w + x;
    const dx = state.pos.x - m.x;
    const dy = state.pos.y - m.y;
    const dist = Math.max(Math.abs(dx), Math.abs(dy));
    const los = dist <= 1 ? true : hasLos(maze.grid, maze.w, maze.h, m.x, m.y, state.pos.x, state.pos.y);

    if (!m.aggro) {
      if (dist <= cfg.aggroRange && los) {
        m.aggro = true;
        state.events.push({ type: "aggro", monster: m.id });
      }
    } else if (!m.forced && dist > cfg.aggroRange + 2 && !los) {
      /* chest-spawned monsters are forced-aggro: they never give up */
      m.aggro = false;
    }

    const occupied = new Set(state.monsters.filter((o) => o !== m).map((o) => idx(o.x, o.y)));
    const openCells = (x, y) => {
      const out = [];
      for (const d of DIRS) {
        const nx = x + d.dx;
        const ny = y + d.dy;
        if (nx < 0 || ny < 0 || nx >= maze.w || ny >= maze.h) continue;
        if (maze.grid[idx(nx, ny)] !== FLOOR) continue;
        if (nx === maze.exit.x && ny === maze.exit.y) continue;
        if (occupied.has(idx(nx, ny))) continue;
        out.push({ x: nx, y: ny, d });
      }
      return out;
    };

    let next = null;
    if (m.aggro) {
      /* monsters are maze denizens: they path over the real map (BFS) */
      const step = bfsFirstStep(maze.grid, maze.w, maze.h, m.x, m.y, state.pos.x, state.pos.y,
        (x, y) => (maze.grid[idx(x, y)] === FLOOR || (x === state.pos.x && y === state.pos.y)) &&
          !occupied.has(idx(x, y)));
      if (step) next = { x: m.x + step.dx, y: m.y + step.dy, d: step };
    } else {
      const opts = openCells(m.x, m.y);
      if (opts.length) {
        const straight = m.lastDir && opts.find((o) => o.d.name === m.lastDir.name);
        if (straight && state.monsterRng() * 100 >= cfg.wanderRandomness) {
          next = straight;
        } else {
          const noReverse = opts.filter((o) => !m.lastDir || o.d.name !== reverseOf(m.lastDir).name);
          next = pick(state.monsterRng, noReverse.length ? noReverse : opts);
        }
      }
    }
    if (!next) return false;
    m.lastDir = next.d;
    m.x = next.x;
    m.y = next.y;
    return true;
  }

  function reverseOf(d) {
    return DIRS.find((x) => x.dx === -d.dx && x.dy === -d.dy) || d;
  }

  /* collision shapes: same cell, runner stepped onto monster, or both
     swapped cells in the same tick — all one fight. Swap detection only
     counts monsters that actually moved this tick (stale positions must
     never fabricate a collision). */
  function detectCollision(state, prevPos) {
    for (const m of state.monsters) {
      if (m.x === state.pos.x && m.y === state.pos.y) return m;
      if (prevPos && m.movedTick === state.tickCount &&
        m.x === prevPos.x && m.y === prevPos.y &&
        m.prevX === state.pos.x && m.prevY === state.pos.y) return m;
    }
    return null;
  }

  /* ---------- one simulation tick ---------- */

  function tickRun(state, dtS, humanMove, hooks) {
    const events = state.events;
    state.events = [];
    if (state.finished) return events;
    const maze = state.maze;
    const idx = (x, y) => y * maze.w + x;
    state.tickCount++;
    state.timeS += dtS;

    /* the player-set timer outranks everything — expiring mid-fight is a
       loss, escaping the same instant would have finished the run first */
    if (state.config.timeLimitS > 0 && state.timeS >= state.config.timeLimitS) {
      state.finished = true;
      state.timedOut = true;
      events.push({ type: "game-over", reason: "time" });
      return events;
    }

    /* mid-fight: static 2.0 s count, everything else frozen */
    if (state.fight) {
      state.fight.remaining -= dtS;
      if (state.fight.remaining <= 0) {
        const m = state.monsters.find((x) => x.id === state.fight.monsterId);
        state.fight = null;
        if (m) {
          state.monsters = state.monsters.filter((x) => x !== m);
          state.fights++;
          events.push({ type: "fight-end", monster: m.id, fights: state.fights });
        }
      }
      return events;
    }

    /* chest opening: the runner is busy for 1.0 s, but the world is NOT
       frozen — monsters keep coming while you fiddle with the lid */
    if (state.chest) {
      state.chest.remaining -= dtS;
      if (state.chest.remaining <= 0) {
        const chest = state.chests.find((c) => c.id === state.chest.id);
        state.chest = null;
        if (chest && !chest.opened) {
          chest.opened = true;
          if (state.monsterRng() < 0.6) {
            const amount = Math.floor(state.monsterRng() * 6); /* 0–5 */
            const pool = amount === 0 ? CHEST_EMPTY_LINES : CHEST_COIN_LINES;
            const line = pool[Math.floor(state.monsterRng() * pool.length)];
            state.coins += amount;
            events.push({
              type: "chest-open", kind: "coins", amount,
              message: line.replace("{}", String(amount)),
              at: { x: chest.x, y: chest.y },
            });
          } else {
            const monster = {
              id: state.nextMonsterId++, x: chest.x, y: chest.y,
              cool: 0, aggro: true, forced: true, lastDir: null,
            };
            state.monsters.push(monster);
            events.push({
              type: "chest-open", kind: "monster", monster: monster.id,
              message: CHEST_MONSTER_LINES[Math.floor(state.monsterRng() * CHEST_MONSTER_LINES.length)],
              at: { x: chest.x, y: chest.y },
            });
          }
        }
      }
      /* the runner cannot move while opening; monsters still act below */
    }
    let moved = false;
    let prevPos = null;
    if (state.chest) {
      /* busy opening — no runner action this tick */
    } else if (state.runnerType === "bot") {
      state.cool -= dtS;
      if (state.cool <= 0) {
        state.cool += 1 / state.config.botSpeed;
        updateVision(state);
        let decision = botDecide(state);
        if (decision.tie && hooks && typeof hooks.tiebreak === "function") {
          const chosen = hooks.tiebreak(decision.tie, decision, state);
          if (chosen) {
            decision = {
              rung: "engine.tiebreak",
              dir: chosen,
              note: decision.note + " — engine decided",
            };
          }
        }
        if (decision.rung === "rule.exit") {
          state.escaped = true;
          state.finished = true;
          events.push({ type: "escaped" });
          return events;
        }
        if (decision.rung === "abstain") {
          state.abstained = true;
          state.finished = true;
          events.push({ type: "abstain" });
          return events;
        }
        if (decision.rung === "turn" && decision.dir) {
          /* crawler fog: nothing unexplored is reachable on the known map —
             spend the tick rotating in place; the new heading may reveal one */
          state.facing = decision.dir;
          updateVision(state);
          events.push({ type: "turn", to: decision.dir.name });
          state.lastDecision = decision;
        } else if (decision.dir) {
          prevPos = { x: state.pos.x, y: state.pos.y };
          const nx = state.pos.x + decision.dir.dx;
          const ny = state.pos.y + decision.dir.dy;
          if (state.known[idx(nx, ny)] === KNOWN_FLOOR) {
            state.pos = { x: nx, y: ny };
            state.steps++;
            state.facing = decision.dir;
            moved = true;
            events.push({ type: "move", from: prevPos, to: state.pos, rung: decision.rung, note: decision.note });
          }
        }
        state.lastDecision = decision;
      }
    } else if (humanMove && !state.fight) {
      const nx = state.pos.x + humanMove.dx;
      const ny = state.pos.y + humanMove.dy;
      if (nx >= 0 && ny >= 0 && nx < maze.w && ny < maze.h && maze.grid[idx(nx, ny)] === FLOOR) {
        prevPos = { x: state.pos.x, y: state.pos.y };
        state.pos = { x: nx, y: ny };
        state.steps++;
        state.facing = humanMove;
        moved = true;
        events.push({ type: "move", from: prevPos, to: state.pos, rung: "human", note: "player move" });
      } else if (state.config.visionShape === "cone") {
        /* walking into a wall under cone vision = turning to look at it */
        state.facing = humanMove;
        updateVision(state);
        events.push({ type: "turn", to: humanMove.name });
      }
    }

    if (moved) {
      updateVision(state);
      for (const l of state.loot) {
        if (!l.taken && l.x === state.pos.x && l.y === state.pos.y) {
          l.taken = true;
          events.push({ type: "loot", at: { x: l.x, y: l.y }, collected: state.loot.filter((x) => x.taken).length });
        }
      }
      for (const ch of state.chests) {
        if (!ch.opened && !state.chest && ch.x === state.pos.x && ch.y === state.pos.y) {
          state.chest = { id: ch.id, remaining: 1.0 };
          events.push({ type: "chest-start", at: { x: ch.x, y: ch.y } });
        }
      }
      if (state.pos.x === maze.exit.x && state.pos.y === maze.exit.y) {
        state.escaped = true;
        state.finished = true;
        events.push({ type: "escaped" });
        return events;
      }
    }

    /* monster turns at their own cadence; collision is checked for every
       monster every tick, mover or not — standing on one is still a fight */
    for (const m of state.monsters) {
      m.cool -= dtS;
      if (m.cool <= 0) {
        m.cool += 1 / state.config.monsterSpeed;
        m.prevX = m.x;
        m.prevY = m.y;
        m.movedTick = state.tickCount;
        if (monsterStep(state, m)) {
          events.push({ type: "monster-move", monster: m.id, at: { x: m.x, y: m.y }, aggro: m.aggro });
        }
      }
      const hit = detectCollision(state, moved ? prevPos : null);
      if (hit && !state.fight) {
        state.fight = { monsterId: hit.id, remaining: FIGHT_SECONDS };
        events.push({ type: "fight-start", monster: hit.id, at: { x: state.pos.x, y: state.pos.y } });
        break;
      }
    }

    /* safety net for unlimited runs — never reachable in normal play */
    if (state.timeS > 3600) {
      state.finished = true;
      state.timedOut = true;
      events.push({ type: "timeout" });
    }
    return events;
  }

  /* revealed-floor bookkeeping is derived once here rather than tracked
     incrementally, so live runs and headless sims can never drift */
  function countRevealedFloors(state) {
    let n = 0;
    const idx = (x, y) => y * state.maze.w + x;
    for (let y = 0; y < state.maze.h; y++) {
      for (let x = 0; x < state.maze.w; x++) {
        if (state.revealed[idx(x, y)] && state.maze.grid[idx(x, y)] === FLOOR) n++;
      }
    }
    state.revealedFloors = n;
    return n;
  }

  function runSummary(state) {
    countRevealedFloors(state);
    const lootTaken = state.loot.filter((l) => l.taken).length;
    return {
      runner: state.runnerType,
      escaped: state.escaped,
      assisted: state.assisted,
      abstained: state.abstained,
      timedOut: state.timedOut,
      timeS: Math.round(state.timeS * 10) / 10,
      steps: state.steps,
      explored: exploredPct(state),
      revealedFloors: state.revealedFloors,
      floorCount: state.maze.floorCount,
      lootCollected: lootTaken,
      lootTotal: state.config.lootEnabled ? state.loot.length : 0,
      coins: state.coins,
      chestOpened: state.chests.filter((c) => c.opened).length,
      chestTotal: state.config.chestsEnabled ? state.chests.length : 0,
      fights: state.fights,
      optimalSteps: state.maze.optimalSteps,
      fastMode: state.config.fastMode,
      plans: state.planCount,
      planSteps: state.planSteps,
      planReplans: state.planReplans,
      stepEfficiency: state.steps > 0
        ? Math.round((state.maze.optimalSteps / state.steps) * 100)
        : 0,
      monstersEnabled: state.config.monstersEnabled,
      lootEnabled: state.config.lootEnabled,
      chestsEnabled: state.config.chestsEnabled,
      seed: state.config.seed,
    };
  }

  /* headless full solve — the same tickRun the live shell drives, at a fixed
     cadence, so test outcomes and on-page outcomes share every code path */
  function simulateBotRun(maze, config, hooks) {
    const state = createRunState(maze, config, "bot");
    const dt = 1 / state.config.botSpeed;
    const allEvents = [];
    for (let i = 0; i < 200000 && !state.finished; i++) {
      allEvents.push(...tickRun(state, dt, null, hooks));
    }
    return { state, events: allEvents, summary: runSummary(state) };
  }

  /* ---------- rubric scoring ---------- */

  const TAGS = ["time", "steps", "explored", "loot", "coins", "chests", "fights"];
  const DEFAULT_RUBRIC = {
    version: 1,
    criteria: [
      { tag: "time", weight: 3 },
      { tag: "explored", weight: 2 },
      { tag: "loot", weight: 1 },
      { tag: "coins", weight: 1 },
      { tag: "chests", weight: 1 },
      { tag: "fights", weight: -2 },
    ],
    tiebreakers: ["time", "steps"],
  };

  /* sanitization moved into the SDK; the maze supplies its own weighted
     DEFAULT_RUBRIC as the empty-criteria fallback (the generic default
     weights every tag 1, which would change live scoring) */
  function normalizeRubric(raw) {
    return DK.rubric.normalize(raw, {
      tags: TAGS,
      minW: -3,
      maxW: 5,
      defWeight: 1,
      defRubric: DEFAULT_RUBRIC,
      defTie: ["time", "steps"],
    });
  }

  /* per-tag 0..1 normalization; pars derive from the map itself. A disabled
     objective normalizes to 0 — it can never inflate a run (and never
     differentiates two runs, since both score 0 on it). */
  function metricNorm(tag, s) {
    const parSteps = 4 * Math.max(1, s.optimalSteps);
    const parTime = 4 * (s.optimalSteps / 6);
    switch (tag) {
      case "time": return s.escaped ? Math.max(0, 1 - s.timeS / parTime) : 0;
      case "steps": return s.escaped ? Math.max(0, 1 - s.steps / parSteps) : 0;
      case "explored": return s.explored;
      case "loot": return (s.lootEnabled !== false && s.lootTotal > 0) ? s.lootCollected / s.lootTotal : 0;
      case "coins": return (s.chestsEnabled !== false && s.chestTotal > 0) ? Math.min(1, s.coins / (s.chestTotal * 5)) : 0;
      case "chests": return (s.chestsEnabled !== false && s.chestTotal > 0) ? s.chestOpened / s.chestTotal : 0;
      /* badness axis: 0 fights → 0, saturating toward 1 as fights pile up.
         With a negative weight this *penalizes*; a positive weight would
         reward brawling, which is the rubric author's call. */
      case "fights": return (s.monstersEnabled || s.chestsEnabled) ? s.fights / (1 + s.fights) : 0;
      default: return 0;
    }
  }

  /* the arithmetic lives in the SDK; the maze owns its metric normalization
     (pars derive from the map) and maps its run semantics onto the SDK's
     dnf/assisted sentinels (no escape = did not finish; assisted = excluded) */
  const MAZE_NORMS = {};
  for (const t of TAGS) MAZE_NORMS[t] = (s) => metricNorm(t, s);

  function scoreRun(s, rubric) {
    const r = normalizeRubric(rubric);
    return DK.rubric.score(
      { ...s, __dnf: !s.escaped, __assisted: !!s.assisted },
      r,
      MAZE_NORMS,
    );
  }

  function tagNorm(tag, s) { return metricNorm(tag, s); }

  function resolveWinner(summaries, rubric) {
    const scored = summaries.map((s) => ({ s, r: scoreRun(s, rubric) }));
    const eligible = scored.filter((x) => !x.r.dnf && !x.r.assisted);
    if (eligible.length === 1) {
      return { winner: eligible[0].s.runner, margin: 0, by: "forfeit", scored };
    }
    if (eligible.length < 2) return { winner: "none", margin: 0, scored };
    const [a, b] = eligible;
    if (a.r.score !== b.r.score) {
      const win = a.r.score > b.r.score ? a : b;
      const other = win === a ? b : a;
      return {
        winner: win.s.runner, margin: Math.abs(win.r.score - other.r.score),
        by: "score", scored,
      };
    }
    const r = normalizeRubric(rubric);
    for (const tag of r.tiebreakers) {
      const an = tagNorm(tag, a.s);
      const bn = tagNorm(tag, b.s);
      if (Math.abs(an - bn) > 1e-9) {
        const win = an > bn ? a : b;
        return { winner: win.s.runner, margin: 0, by: tag, scored };
      }
    }
    return { winner: "draw", margin: 0, by: "tiebreakers", scored };
  }

  /* running scoreboard value: score the run as if it ended right now, so
     the versus bar moves continuously under competitive pressure */
  function liveScore(state, rubric) {
    const s = runSummary(state);
    if (state.finished && !state.escaped) return 0;
    s.escaped = true;
    return scoreRun(s, rubric).score;
  }

  /* config stamp — winner verdicts only compare runs with identical stamps.
     Everything that changes the map or the information game is in; cosmetics
     (palette, gridlines, canvas size) are deliberately not. */
  function configStamp(cfg) {
    const c = normalizeConfig(cfg);
    return [c.seed, c.cells,
      c.mapType === "dungeon" ? "d:" + [c.rooms, c.roomMin, c.roomMax].join("-") : "b:" + c.braidPct,
      c.vision, c.visionShape === "cone" ? "cone" + c.visionHalf : "disc",
      c.lootEnabled ? c.lootCount : "off",
      c.chestsEnabled ? c.chestCount : "off",
      c.monstersEnabled
        ? [c.monsterCount, c.monsterSpeed, c.aggroRange, c.wanderRandomness, c.avoidMonsters ? 1 : 0].join("-")
        : "off",
      c.timeLimitS > 0 ? c.timeLimitS + (c.fastMode ? "/fast" : "") : "open",
    ].join("|");
  }

  /* ---------- exports ---------- */
  globalThis.MazeCore = {
    WALL, FLOOR, FIGHT_SECONDS, DIRS, KNOWN_UNK, KNOWN_FLOOR, KNOWN_WALL,
    hashSeed, mulberry32, makeRng,
    normalizeConfig, generateMaze,
    bresenhamLine, castRay, computeFov,
    bfsDistances, bfsFirstStep,
    createRunState, updateVision, exploredPct, threatCells,
    botDecide, monsterStep, detectCollision, tickRun,
    countRevealedFloors, runSummary, simulateBotRun, liveScore,
    TAGS, DEFAULT_RUBRIC, normalizeRubric, metricNorm, scoreRun, resolveWinner, configStamp,
  };
})();
