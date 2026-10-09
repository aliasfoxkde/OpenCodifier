/* Pac-maze core — pure simulation + ghost policy for the Playground Pacman
   game. No DOM, no canvas, no timers: deterministic given (seed, config,
   input script), unit-testable under `node --test` like pong-core.js and
   life-core.js (import for side effect, read the global; sdk/decisions-sdk.js
   AND maze-core.js must load first — the board is MazeCore's SDK-backed
   generator, and the BFS helpers are MazeCore's, not reimplemented here).

   Design contract (docs/planning/playground/DECISIONS-SDK-PLAN.md §5.3):
   - tile-locked movement: entities anchor on a tile, move along one axis,
     and only turn at tile arrivals; reversal mid-segment is allowed for
     the player (classic feel), ghosts never reverse (classic AI rule);
   - each ghost's personality is a reward-weight vector over candidate
     intents (pursue / intercept / patrol / retreat) scored at every tile
     arrival — the pong mechanism transplanted to a chase;
   - a genuine tie between the top two intents escalates to the engine as a
     real choice question; frightened retreat is a RULE, never a choice,
     and every move records which rung answered;
   - power pellets, ghost eating with a score chain, lives, level clear,
     and a bot player are all in the core so tests can drive whole games. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const MC = globalThis.MazeCore;

  const TICKS_PER_S = 60;
  const DIRS = {
    up: { dx: 0, dy: -1 },
    down: { dx: 0, dy: 1 },
    left: { dx: -1, dy: 0 },
    right: { dx: 1, dy: 0 },
  };
  const REVERSE_OF = { up: "down", down: "up", left: "right", right: "left" };
  const DIR_ORDER = ["up", "left", "down", "right"];   /* stable tie-break order */
  const NAME_OF = {};                                   /* {dx,dy} -> dir name */

  /* ghost personalities — the incentive surface, pong-style weights */
  const INTENTS = ["pursue", "intercept", "patrol", "retreat"];
  const PERSONALITIES = [
    { name: "blinky", color: "#ff4d4d", weights: { pursue: 5, intercept: 1, patrol: 0, retreat: 0 } },
    { name: "pinky", color: "#ff9ad5", weights: { pursue: 1, intercept: 5, patrol: 1, retreat: 0 } },
    { name: "inky", color: "#4de3ff", weights: { pursue: 2, intercept: 2, patrol: 4, retreat: 0 } },
    { name: "clyde", color: "#ffb84d", weights: { pursue: 3, intercept: 0, patrol: 1, retreat: 3 } },
    { name: "spunky", color: "#b8ff4d", weights: { pursue: 4, intercept: 0, patrol: 2, retreat: 0 } },
    { name: "shy", color: "#c94dff", weights: { pursue: 0, intercept: 1, patrol: 2, retreat: 4 } },
  ];
  for (const name of DIR_ORDER) NAME_OF[DIRS[name].dx + "," + DIRS[name].dy] = name;

  const DEFAULT_CONFIG = {
    version: 1,
    board: "classic",   /* "classic" = the 28x31 arcade layout; "generated" = SDK maze */
    seed: "oc-pac",
    cells: 15,          /* board is 2*cells+1 square (generated boards) */
    braidPct: 14,
    ghosts: 4,
    ghostSpeed: 4.2,    /* tiles per second */
    playerSpeed: 5.4,
    frightS: 7,
    pelletFill: 0.85,
    lives: 3,
    botPlayer: false,
    maxTicks: 54000,    /* 15 min sim cap */
  };

  function normalizeConfig(raw) {
    const c = raw || {};
    return {
      version: 1,
      board: c.board === "generated" ? "generated" : "classic",
      seed: String(c.seed || DEFAULT_CONFIG.seed),
      cells: DK.clamp(c.cells, 9, 29, DEFAULT_CONFIG.cells),
      braidPct: DK.clamp(c.braidPct, 0, 40, DEFAULT_CONFIG.braidPct),
      ghosts: DK.clamp(c.ghosts, 1, 6, DEFAULT_CONFIG.ghosts),
      ghostSpeed: DK.clampf(c.ghostSpeed, 1.5, 9, DEFAULT_CONFIG.ghostSpeed),
      playerSpeed: DK.clampf(c.playerSpeed, 2, 9, DEFAULT_CONFIG.playerSpeed),
      frightS: DK.clampf(c.frightS, 2, 15, DEFAULT_CONFIG.frightS),
      pelletFill: DK.clampf(c.pelletFill, 0.4, 1, DEFAULT_CONFIG.pelletFill),
      lives: DK.clamp(c.lives, 1, 5, DEFAULT_CONFIG.lives),
      botPlayer: c.botPlayer === true,
      maxTicks: DK.clamp(c.maxTicks, 3600, 162000, DEFAULT_CONFIG.maxTicks),
    };
  }

  /* stamp — session rows compare like-for-like only */
  function pacStamp(cfg) {
    const c = normalizeConfig(cfg);
    if (c.board === "classic") {
      return [c.seed, "arcade", "g" + c.ghosts, "gs" + c.ghostSpeed,
        "ps" + c.playerSpeed, "fr" + c.frightS, "l" + c.lives].join("|");
    }
    return [c.seed, c.cells + "x" + c.cells, "braid" + c.braidPct,
      "g" + c.ghosts, "gs" + c.ghostSpeed, "ps" + c.playerSpeed,
      "fr" + c.frightS, "fill" + c.pelletFill, "l" + c.lives].join("|");
  }

  /* ---------- board ---------- */

  /* the SDK-backed generator builds the walls; this layers the pac furniture:
     pellets on a fraction of floor tiles, 4 power pellets pinning the
     corners, the player on the generator's start tile, ghosts around the
     center. Deterministic given (seed, config). */
  function buildBoard(cfg) {
    const c = normalizeConfig(cfg);
    const map = MC.generateMaze({ seed: c.seed, cells: c.cells, mapType: "maze",
      braidPct: c.braidPct, monstersEnabled: false, lootEnabled: false });
    const { w, h, grid } = map;
    const idx = (x, y) => y * w + x;
    const rng = DK.makeRng(c.seed + ":pac:" + c.cells);

    const pellets = new Set();
    const floors = [];
    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        if (grid[idx(x, y)] === MC.FLOOR) floors.push([x, y]);
      }
    }
    for (const [x, y] of floors) {
      if (rng() < c.pelletFill) pellets.add(idx(x, y));
    }
    /* power pellets: the four corner-most floor tiles, never plain pellets */
    const powers = [];
    for (const [cx, cy] of [[1, 1], [w - 2, 1], [1, h - 2], [w - 2, h - 2]]) {
      let best = -1, bestD = Infinity;
      for (const [x, y] of floors) {
        const key = idx(x, y);
        if (powers.includes(key)) continue;
        const d = Math.abs(x - cx) + Math.abs(y - cy);
        if (d < bestD) { bestD = d; best = key; }
      }
      pellets.delete(best);
      powers.push(best);
    }
    /* ghosts spawn on distinct floor tiles near the board center */
    const centerX = w >> 1, centerY = h >> 1;
    const genStartKey = idx(map.start.x, map.start.y);
    const spawns = floors.slice().sort((a, b) => {
      const da = Math.abs(a[0] - centerX) + Math.abs(a[1] - centerY);
      const db = Math.abs(b[0] - centerX) + Math.abs(b[1] - centerY);
      return da - db || idx(a[0], a[1]) - idx(b[0], b[1]);
    });
    const ghostTiles = [];
    for (const [x, y] of spawns) {
      if (Math.abs(x - centerX) + Math.abs(y - centerY) > 6) break;
      const key = idx(x, y);
      if (key === genStartKey || powers.includes(key)) continue;
      ghostTiles.push({ x, y });
      if (ghostTiles.length >= 6) break;
    }
    /* the player starts at the floor tile FARTHEST from the ghost home —
       the generator's own start sits beside the center cluster, which is
       an instant death loop (measured: seed 'tie' lost 3 lives in ~60
       ticks with zero decisions). BFS from home, argmax distance. */
    const home = ghostTiles[0] || { x: centerX, y: centerY };
    const fromHome = MC.bfsDistances(grid, w, h, home.x, home.y, MC.FLOOR);
    let start = null;
    let startD = -1;
    for (const [x, y] of floors) {
      /* interior tiles only: the generator's border stub is a one-way
         dead end, and power tiles belong to the corners */
      const key2 = idx(x, y);
      if (x < 1 || x > w - 2 || y < 1 || y > h - 2) continue;
      if (powers.includes(key2)) continue;
      const d = fromHome[key2];
      if (d > startD) { startD = d; start = { x, y }; }
    }
    if (!start) start = { x: map.start.x, y: map.start.y };
    pellets.delete(idx(start.x, start.y));
    return {
      cfg: c, w, h, grid,
      start, home,
      ghostTiles,
      pellets, powers,
      pelletsTotal: pellets.size + powers.length,
    };
  }

  /* ---------- the arcade board ---------- */

  /* the 28x31 original: outer walls, power pellets in the four corners,
     the dotless house ring, the ghost house with its door (-), and the
     row-14 wrap tunnel. '#' wall, '.' pellet, 'o' power, '-' door,
     ' ' open floor without a pellet. */
  const CLASSIC_ROWS = [
    "############################",
    "#............##............#",
    "#.####.#####.##.#####.####.#",
    "#o####.#####.##.#####.####o#",
    "#.####.#####.##.#####.####.#",
    "#..........................#",
    "#.####.##.########.##.####.#",
    "#.####.##.########.##.####.#",
    "#......##....##....##......#",
    "######.##### ## #####.######",
    "     #.##### ## #####.#     ",
    "     #.##          ##.#     ",
    "     #.## ###--### ##.#     ",
    "######.## #      # ##.######",
    "          #      #          ",
    "######.## #      # ##.######",
    "     #.## ######## ##.#     ",
    "     #.##          ##.#     ",
    "     #.## ######## ##.#     ",
    "######.## ######## ##.######",
    "#............##............#",
    "#.####.#####.##.#####.####.#",
    "#.####.#####.##.#####.####.#",
    "#o..##.......  .......##..o#",
    "###.##.##.########.##.##.###",
    "###.##.##.########.##.##.###",
    "#......##....##....##......#",
    "#.##########.##.##########.#",
    "#.##########.##.##########.#",
    "#..........................#",
    "############################",
  ];
  const DOOR = 2;                       /* grid code: ghost-door tile */
  const TUNNEL_ROW = 14;
  const FRUIT_AT = [70, 170];           /* pellets eaten that spawn fruit */
  const FRUIT_TILE = { x: 13, y: 17 };
  const CLASSIC_ROSTER = [              /* exit order + house slot */
    { name: "blinky", release: 0, slot: null },
    { name: "pinky", release: 60, slot: 0 },
    { name: "inky", release: 240, slot: 1 },
    { name: "clyde", release: 420, slot: 2 },
    { name: "spunky", release: 600, slot: 3 },
    { name: "shy", release: 780, slot: 4 },
  ];
  const SCATTER_CORNER = {
    blinky: { x: 25, y: 0 }, pinky: { x: 2, y: 0 },
    inky: { x: 27, y: 31 }, clyde: { x: 0, y: 31 },
    spunky: { x: 25, y: 31 }, shy: { x: 2, y: 31 },
  };

  function buildClassicBoard(cfg) {
    const c = normalizeConfig(cfg);
    const h = CLASSIC_ROWS.length, w = CLASSIC_ROWS[0].length;
    const grid = new Uint8Array(w * h);
    const pellets = new Set();
    const powers = [];
    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        const ch = CLASSIC_ROWS[y][x];
        const key = y * w + x;
        if (ch === "#") { grid[key] = 1; continue; }
        if (ch === "-") { grid[key] = DOOR; continue; }
        grid[key] = MC.FLOOR;
        if (ch === ".") pellets.add(key);
        else if (ch === "o") powers.push(key);
      }
    }
    return {
      cfg: c, w, h, grid, classic: true, tunnelRow: TUNNEL_ROW,
      start: { x: 13, y: 23 },          /* the classic spawn, between the dots */
      home: { x: 13, y: 11 },           /* the tile above the ghost-house door */
      doorTiles: [12 * w + 13, 12 * w + 14],
      houseSlots: [[13, 14], [11, 14], [15, 14], [12, 14], [14, 14], [16, 14]],
      pellets, powers,
      pelletsTotal: pellets.size + powers.length,
    };
  }

  /* tunnel-aware passability: x wraps on the tunnel row, doors stay solid */
  function passableC(board, x, y) {
    if (y < 0 || y >= board.h) return false;
    const wx = ((x % board.w) + board.w) % board.w;
    return board.grid[y * board.w + wx] === MC.FLOOR;
  }

  function passable(board, x, y) {
    return x >= 0 && y >= 0 && x < board.w && y < board.h &&
      board.grid[y * board.w + x] === MC.FLOOR;
  }

  /* BFS distances from one tile across the whole board (MazeCore's — shared
     with the crawler, not reimplemented) */
  function distFrom(board, x, y) {
    return MC.bfsDistances(board.grid, board.w, board.h, x, y, MC.FLOOR);
  }

  /* nearest passable tile to (tx,ty) — the intercept anchor */
  function nearestFloor(board, tx, ty) {
    tx = Math.max(0, Math.min(board.w - 1, tx));
    ty = Math.max(0, Math.min(board.h - 1, ty));
    if (passable(board, tx, ty)) return { x: tx, y: ty };
    let best = { x: tx, y: ty }, bestD = Infinity;
    for (let y = 0; y < board.h; y++) {
      for (let x = 0; x < board.w; x++) {
        if (!passable(board, x, y)) continue;
        const d = Math.abs(x - tx) + Math.abs(y - ty);
        if (d < bestD) { bestD = d; best = { x, y }; }
      }
    }
    return best;
  }

  /* ---------- entities ---------- */

  function makePlayer(board, cfg) {
    return {
      kind: "player",
      x: board.start.x, y: board.start.y,
      px: board.start.x, py: board.start.y,
      dir: "left", want: null,
      prog: 0, speed: cfg.playerSpeed,
    };
  }

  function makeGhosts(board, cfg) {
    const out = [];
    for (let i = 0; i < cfg.ghosts; i++) {
      const tile = board.ghostTiles[i % board.ghostTiles.length];
      const p = PERSONALITIES[i % PERSONALITIES.length];
      out.push({
        kind: "ghost", slot: i,
        name: p.name, color: p.color, weights: { ...p.weights },
        x: tile.x, y: tile.y, px: tile.x, py: tile.y,
        dir: DIR_ORDER[i % 4], prog: 0,
        speed: cfg.ghostSpeed,
        state: i === 0 ? "active" : "house",     /* staggered release */
        releaseAt: i * 3 * TICKS_PER_S,
        respawnAt: 0,
      });
    }
    return out;
  }

  function makeClassicGhosts(board, cfg) {
    const out = [];
    for (let i = 0; i < cfg.ghosts; i++) {
      const roster = CLASSIC_ROSTER[i];
      const p = PERSONALITIES.find((q) => q.name === roster.name);
      const tile = roster.slot === null
        ? { x: board.home.x, y: board.home.y }
        : { x: board.houseSlots[roster.slot][0], y: board.houseSlots[roster.slot][1] };
      out.push({
        kind: "ghost", slot: i,
        name: p.name, color: p.color, weights: { ...p.weights },
        x: tile.x, y: tile.y, px: tile.x, py: tile.y,
        dir: i === 0 ? "left" : "up", prog: 0,
        speed: cfg.ghostSpeed,
        phase: i === 0 ? "active" : "house",   /* staggered exit, classic order */
        releaseAt: roster.release,
        homeSlot: tile,
      });
    }
    return out;
  }

  /* ---------- ghost policy: weighted intents, argmax, genuine ties ---------- */

  /* score the open directions at a tile arrival. pursue/intercept read as
     proximity (1 - d/diag), retreat the mirror, patrol = momentum. Ghosts
     never reverse unless it is the only open direction (classic AI rule).
     Returns candidates in stable scored order plus the genuine tie pair. */
  function ghostCandidates(board, ghost, ctx) {
    const diag = Math.hypot(board.w, board.h);
    const prox = (v) => (v < 0 ? 0 : 1 - v / diag);
    const open = DIR_ORDER.filter((name) => {
      if (!passable(board, ghost.x + DIRS[name].dx, ghost.y + DIRS[name].dy)) return false;
      if (name === REVERSE_OF[ghost.dir]) {
        /* reverse only when it is the only way out */
        return !DIR_ORDER.some((o) => o !== REVERSE_OF[ghost.dir] &&
          passable(board, ghost.x + DIRS[o].dx, ghost.y + DIRS[o].dy));
      }
      return true;
    });
    const cands = open.map((name) => {
      const nx = ghost.x + DIRS[name].dx, ny = ghost.y + DIRS[name].dy;
      const key = ny * board.w + nx;
      const dp = ctx.playerDist[key];
      const di = ctx.interceptDist[key];
      return {
        dir: name,
        score: ghost.weights.pursue * prox(dp) +
               ghost.weights.intercept * prox(di) +
               ghost.weights.patrol * (name === ghost.dir ? 1 : 0) -
               ghost.weights.retreat * prox(dp),
        playerDist: dp, interceptDist: di,
      };
    });
    cands.sort((a, b) => b.score - a.score ||
      DIR_ORDER.indexOf(a.dir) - DIR_ORDER.indexOf(b.dir));
    const tie = cands.length >= 2 &&
      Math.abs(cands[0].score - cands[1].score) < 1e-9
      ? [cands[0], cands[1]] : null;
    return { cands, tie };
  }

  /* the engine's view of a tie — a real choice question over the tied dirs */
  function tieRequest(board, ghost, ctx, tie) {
    const fmt = (c) => c.dir + " — player " + (c.playerDist < 0 ? "?" : c.playerDist) +
      " tiles away, intercept anchor " + (c.interceptDist < 0 ? "?" : c.interceptDist);
    return {
      state: {
        text: "Pac-maze tie: ghost " + ghost.name + " at tile (" + ghost.x + "," +
          ghost.y + ") scores two directions exactly equal under its reward " +
          "weights (pursue " + ghost.weights.pursue + ", intercept " +
          ghost.weights.intercept + ", patrol " + ghost.weights.patrol +
          ", retreat " + ghost.weights.retreat + "). Pick a direction.",
        facts: {},
      },
      questions: [{
        type: "choice", id: "ghost-dir",
        text: "Which way should " + ghost.name + " go?",
        candidates: tie.map((c) => ({ id: c.dir, description: fmt(c) })),
      }],
      policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" },
    };
  }

  /* ---------- game state ---------- */

  function createGame(rawCfg) {
    const cfg = normalizeConfig(rawCfg);
    const board = cfg.board === "classic" ? buildClassicBoard(cfg) : buildBoard(cfg);
    return {
      cfg, board,
      player: makePlayer(board, cfg),
      ghosts: board.classic ? makeClassicGhosts(board, cfg) : makeGhosts(board, cfg),
      t: 0, ticks: 0,
      score: 0, lives: cfg.lives, level: 1,
      pelletsEaten: 0, powersEaten: 0, ghostsEaten: 0,
      frightUntil: 0, chain: 0,
      eatenPowers: new Set(),
      over: false,
      engineCalls: 0,
      rungs: { weights: 0, engine: 0, fright: 0, eyes: 0 },
      lastDecision: null,          /* {who, at, dir, rung} for the trace line */
      rng: DK.makeRng(cfg.seed + ":runtime"),
      interceptAnchor: null,
      engineChoose: null,
      /* classic-only: READY gate + scatter/chase clock + fruit */
      readyUntil: board.classic ? 90 : 0,
      phaseT: 0,
      fruit: null,
      fruitEaten: 0,
    };
  }

  /* level N speeds everything up 5% per level — escalation without chaos */
  function speedScale(level) { return 1 + 0.05 * (level - 1); }

  function resetPositions(g) {
    g.player = makePlayer(g.board, g.cfg);
    g.ghosts = makeGhosts(g.board, g.cfg);
    g.frightUntil = 0;
    g.chain = 0;
  }

  function levelClear(g) {
    g.level += 1;
    g.board = buildBoard(g.cfg);           /* fresh furniture, same maze */
    g.eatenPowers = new Set();
    resetPositions(g);
    g.player.speed = g.cfg.playerSpeed * speedScale(g.level);
    for (const gh of g.ghosts) gh.speed = g.cfg.ghostSpeed * speedScale(g.level);
  }

  function killPlayer(g) {
    g.lives -= 1;
    if (g.lives <= 0) { g.over = true; return; }
    if (g.board.classic) classicReset(g);
    else resetPositions(g);
  }

  /* classic reset: back to the house and the spawn, READY gate, scatter
     clock restarts — the arcade's between-lives state */
  function classicReset(g) {
    g.player = makePlayer(g.board, g.cfg);
    g.ghosts = makeClassicGhosts(g.board, g.cfg);
    g.frightUntil = 0;
    g.chain = 0;
    g.fruit = null;
    g.phaseT = 0;
    g.readyUntil = g.t + 2 * TICKS_PER_S;
  }

  function classicLevelClear(g) {
    g.level += 1;
    g.board = buildClassicBoard(g.cfg);    /* fresh pellets, same arcade */
    g.eatenPowers = new Set();
    classicReset(g);
  }

  /* scatter/chase schedule, arcade-shaped: seconds per phase, level-scaled */
  function scatterSchedule(level) {
    if (level >= 5) return [5, 17, 5, 17, 3, 17, 3];
    if (level >= 2) return [5, 20, 5, 20, 3, 20, 3];
    return [7, 20, 7, 20, 5, 20, 5];
  }

  function isScatter(g) {
    const sched = scatterSchedule(g.level);
    let acc = 0;
    for (let i = 0; i < sched.length; i++) {
      acc += sched[i] * TICKS_PER_S;
      if (g.phaseT < acc) return i % 2 === 0;
    }
    return false;                          /* schedule spent: permanent chase */
  }

  /* the classic targeting rules, one per ghost — the decision layer here
     is WHICH WAY to move toward the target, never the target itself */
  function classicTarget(g, gh) {
    const p = g.player;
    const d = DIRS[p.dir];
    if (g.t >= g.frightUntil && !isScatter(g)) {
      switch (gh.name) {
        case "blinky":
          return { x: p.x, y: p.y };
        case "pinky":
          return { x: p.x + d.dx * 4, y: p.y + d.dy * 4 };
        case "inky": {
          const blinky = g.ghosts.find((o) => o.name === "blinky") || gh;
          const vx = p.x + d.dx * 2, vy = p.y + d.dy * 2;
          return { x: vx + (vx - blinky.x), y: vy + (vy - blinky.y) };
        }
        case "clyde": {
          const dist = Math.hypot(p.x - gh.x, p.y - gh.y);
          return dist > 8 ? { x: p.x, y: p.y } : SCATTER_CORNER.clyde;
        }
        case "shy": {
          const dist = Math.hypot(p.x - gh.x, p.y - gh.y);
          return dist > 5 ? { x: p.x, y: p.y } : SCATTER_CORNER.shy;
        }
        default:                            /* spunky: cut off two behind */
          return { x: p.x - d.dx * 2, y: p.y - d.dy * 2 };
      }
    }
    return SCATTER_CORNER[gh.name] || SCATTER_CORNER.blinky;
  }

  /* ---------- movement ---------- */

  function stepEntity(g, e, dt, chooseDir, wrapX) {
    e.prog += e.speed * speedScale(g.level) * dt;
    let arrived = false;
    while (e.prog >= 1) {
      /* decide FIRST at the tile we occupy, then move into the vetted
         direction — moving in the old direction teleports blocked entities
         through walls (measured: a player walked to x = -290) */
      const next = chooseDir(e);
      if (next === null || !DIRS[next]) { e.prog = 0; break; }   /* hold, wait */
      e.dir = next;
      e.x += DIRS[next].dx; e.y += DIRS[next].dy;
      if (wrapX) {
        if (e.x < 0) e.x += g.board.w;
        else if (e.x >= g.board.w) e.x -= g.board.w;
      }
      e.prog -= 1;
      arrived = true;
    }
    return arrived;
  }

  function playerChoose(g) {
    const p = g.player;
    const b = g.board;
    return () => {
      /* queued intent first, then current dir, then stop */
      if (p.want && passable(b, p.x + DIRS[p.want].dx, p.y + DIRS[p.want].dy)) {
        return p.want;
      }
      if (passable(b, p.x + DIRS[p.dir].dx, p.y + DIRS[p.dir].dy)) return p.dir;
      return null;
    };
  }

  /* mid-segment reversal — the player's classic escape */
  function playerReverse(g, want) {
    const p = g.player;
    if (want !== REVERSE_OF[p.dir]) return;
    const d = DIRS[p.dir];
    if (!passable(g.board, p.x + d.dx, p.y + d.dy)) return;
    p.x += d.dx; p.y += d.dy;              /* anchor becomes the tile ahead */
    p.dir = REVERSE_OF[p.dir];
    p.prog = 1 - p.prog;
    p.want = null;
  }

  /* the bot player: step toward the nearest pellet (each candidate scored
     by its OWN BFS distance to that pellet — distance-from-player is 1 for
     every neighbor and dithers into dead stubs, measured on seed 'bravo'),
     penalize steps that land within 5 tiles of an active hunter, and add a
     hair against reversing to keep corridors from ping-ponging. A readable
     greedy, honest about being a baseline. */
  function botIntent(g) {
    const b = g.board;
    const p = g.player;
    const fright = g.t < g.frightUntil;
    const dist = distFrom(b, p.x, p.y);
    let bestD = Infinity, target = -1;
    for (const key of b.pellets) {
      const d = dist[key];
      if (d >= 0 && d < bestD) { bestD = d; target = key; }
    }
    for (const key of b.powers) {
      if (g.eatenPowers.has(key)) continue;
      const d = dist[key];
      if (d >= 0 && d < bestD) { bestD = d; target = key; }
    }
    if (target < 0) return null;
    /* classic ghosts carry `phase`, generated ghosts `state` */
    const hunting = (gh) => gh.phase === "active" || gh.state === "active";
    const threats = fright ? [] : g.ghosts.filter(hunting)
      .map((gh) => distFrom(b, gh.x, gh.y));
    const danger = (key) => threats.some((gd) => gd[key] >= 0 && gd[key] <= 5) ? 8 : 0;
    let bestScore = Infinity, want = null;
    for (const name of DIR_ORDER) {
      const nx = p.x + DIRS[name].dx, ny = p.y + DIRS[name].dy;
      if (!passable(b, nx, ny)) continue;
      const key = ny * b.w + nx;
      const toTarget = distFrom(b, nx, ny)[target];
      const s = (toTarget < 0 ? 99 : toTarget) + danger(key) +
        (name === REVERSE_OF[p.dir] ? 0.5 : 0);
      if (s < bestScore) { bestScore = s; want = name; }
    }
    return want;
  }

  /* ---------- the tick ---------- */

  /* every ghost decision names its rung: eyes (a rule), fright (a rule),
     weights (the local argmax), or engine (a broken genuine tie) */
  function ghostThink(g, gh) {
    const b = g.board;
    const ctx = {
      playerDist: distFrom(b, g.player.x, g.player.y),
      interceptDist: distFrom(b, g.interceptAnchor.x, g.interceptAnchor.y),
    };
    const record = (dir, rung) => {
      gh.dir = dir;
      g.rungs[rung] += 1;
      g.lastDecision = { who: gh.name, at: { x: gh.x, y: gh.y }, dir, rung };
    };
    if (gh.state === "eyes") {
      /* head home: shortest path, a rule — never a choice question */
      const step = MC.bfsFirstStep(b.grid, b.w, b.h, gh.x, gh.y, b.home.x, b.home.y,
        (x, y) => passable(b, x, y));
      record(step ? NAME_OF[step.dx + "," + step.dy] || gh.dir : gh.dir, "eyes");
      return;
    }
    if (g.t < g.frightUntil) {
      /* frightened retreat is a RULE: maximize distance from the player */
      const { cands } = ghostCandidates(b, gh, ctx);
      if (!cands.length) return;
      let best = cands[0];
      for (const c of cands) if (c.playerDist > best.playerDist) best = c;
      record(best.dir, "fright");
      return;
    }
    const { cands, tie } = ghostCandidates(b, gh, ctx);
    if (!cands.length) return;
    let pick = { best: cands[0], tie };
    if (tie && g.engineChoose) {
      const res = g.engineChoose(tieRequest(b, gh, ctx, tie));
      if (res) {
        g.engineCalls += 1;
        const ans = res && res.answers && res.answers[0];
        const chosen = tie.find((c) => c.dir === (ans && ans.choice));
        if (chosen) pick = { best: chosen, engineBroke: true };
      }
    }
    /* honesty bookkeeping: "engine" only when the engine actually broke a
       genuine tie; everything else was the local argmax ("weights") */
    record(pick.best.dir, pick.engineBroke ? "engine" : "weights");
  }

  function ghostChoose(g, gh) {
    return () => {
      ghostThink(g, gh);
      return gh.dir;
    };
  }

  function eatAt(g) {
    const b = g.board;
    const p = g.player;
    const key = p.y * b.w + p.x;
    if (b.pellets.delete(key)) {
      g.score += 10;
      g.pelletsEaten += 1;
      return;
    }
    if (b.powers.includes(key) && !g.eatenPowers.has(key)) {
      g.eatenPowers.add(key);
      g.score += 50;
      g.powersEaten += 1;
      g.frightUntil = g.t + g.cfg.frightS * TICKS_PER_S;
      g.chain = 0;
      for (const gh of g.ghosts) {
        if (gh.state === "active") gh.dir = REVERSE_OF[gh.dir];  /* classic turnabout */
      }
    }
  }

  function collide(g) {
    const p = g.player;
    if (g.t >= g.frightUntil) {
      for (const gh of g.ghosts) {
        if (gh.state !== "active") continue;
        const same = gh.x === p.x && gh.y === p.y;
        const swap = gh.px === p.x && gh.py === p.y && gh.x === p.px && gh.y === p.py;
        if (same || swap) { killPlayer(g); return; }
      }
      return;
    }
    for (const gh of g.ghosts) {
      if (gh.state !== "active") continue;
      const same = gh.x === p.x && gh.y === p.y;
      const swap = gh.px === p.x && gh.py === p.y && gh.x === p.px && gh.y === p.py;
      if (!same && !swap) continue;
      g.chain += 1;
      g.score += 200 * g.chain;
      g.ghostsEaten += 1;
      gh.state = "eyes";
      gh.prog = 0;
    }
  }

  /* ---------- the arcade tick ---------- */

  function recordClassic(g, gh, dir, rung) {
    gh.dir = dir;
    g.rungs[rung] += 1;
    g.lastDecision = { who: gh.name, at: { x: gh.x, y: gh.y }, dir, rung };
  }

  /* open directions for an active ghost: no doors, no reverse unless it is
     the only way out — the arcade's movement contract */
  function classicOpen(g, gh) {
    const b = g.board;
    const forced = DIR_ORDER.every((name) => name === REVERSE_OF[gh.dir] ||
      !passableC(b, gh.x + DIRS[name].dx, gh.y + DIRS[name].dy));
    return DIR_ORDER.filter((name) => {
      if (!passableC(b, gh.x + DIRS[name].dx, gh.y + DIRS[name].dy)) return false;
      if (name === REVERSE_OF[gh.dir]) return forced;
      return true;
    });
  }

  /* candidates scored by straight-line distance to the ghost's target —
     the arcade rule. Ties are genuine ties and go to the engine. */
  function classicCandidates(g, gh, target) {
    const cands = classicOpen(g, gh).map((name) => {
      const nx = gh.x + DIRS[name].dx, ny = gh.y + DIRS[name].dy;
      return { dir: name, d2: (nx - target.x) ** 2 + (ny - target.y) ** 2 };
    });
    cands.sort((a, b) => a.d2 - b.d2 ||
      DIR_ORDER.indexOf(a.dir) - DIR_ORDER.indexOf(b.dir));
    const tie = cands.length >= 2 && cands[0].d2 === cands[1].d2
      ? [cands[0], cands[1]] : null;
    return { cands, tie };
  }

  function classicTieRequest(g, gh, target, tie) {
    return {
      state: {
        text: "Pacman tie: ghost " + gh.name + " at tile (" + gh.x + "," +
          gh.y + ") measures two directions equidistant from its target (" +
          target.x + "," + target.y + ") under the arcade rule. Pick a direction.",
        facts: {},
      },
      questions: [{
        type: "choice", id: "ghost-dir",
        text: "Which way should " + gh.name + " go?",
        candidates: tie.map((c) => ({ id: c.dir, description: c.dir + " — distance² " + c.d2 })),
      }],
      policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" },
    };
  }

  function classicGhostThink(g, gh, fright) {
    if (gh.phase === "eyes") {
      const b = g.board;
      const step = MC.bfsFirstStep(b.grid, b.w, b.h, gh.x, gh.y, b.home.x, b.home.y,
        (x, y) => passableC(b, x, y));
      recordClassic(g, gh, step ? NAME_OF[step.dx + "," + step.dy] || gh.dir : gh.dir, "eyes");
      return;
    }
    if (fright) {
      /* frightened flight is a RULE: a seeded pick from the open directions */
      const open = classicOpen(g, gh);
      if (!open.length) return;
      recordClassic(g, gh, open[Math.floor(g.rng() * open.length)], "fright");
      return;
    }
    const target = classicTarget(g, gh);
    const { cands, tie } = classicCandidates(g, gh, target);
    if (!cands.length) return;
    let pick = { dir: cands[0].dir, rung: "weights" };
    if (tie && g.engineChoose) {
      const res = g.engineChoose(classicTieRequest(g, gh, target, tie));
      if (res) {
        g.engineCalls += 1;
        const ans = res && res.answers && res.answers[0];
        const chosen = tie.find((c) => c.dir === (ans && ans.choice));
        if (chosen) pick = { dir: chosen.dir, rung: "engine" };
      }
    }
    recordClassic(g, gh, pick.dir, pick.rung);
  }

  /* scripted exit: to the door column, up through the door, out */
  function classicLeave(g, gh, dt) {
    gh.speed = g.cfg.ghostSpeed * 0.7;
    stepEntity(g, gh, dt, () => {
      if (gh.x < 13) return "right";
      if (gh.x > 13) return "left";
      if (gh.y > 11) return "up";
      gh.phase = "active";
      return null;                          /* door clear: hunt begins */
    }, false);
  }

  /* scripted re-entry: down through the door, then to the house slot */
  function classicEnter(g, gh, dt) {
    gh.speed = g.cfg.ghostSpeed * 0.8;
    stepEntity(g, gh, dt, () => {
      if (gh.y < 14) return "down";
      if (gh.x !== gh.homeSlot.x) return gh.x < gh.homeSlot.x ? "right" : "left";
      gh.phase = "house";
      gh.releaseAt = g.t + 2 * TICKS_PER_S;  /* respawn gate */
      return null;
    }, false);
  }

  function classicGhostSpeed(g, gh, fright) {
    const base = g.cfg.ghostSpeed;
    if (gh.phase === "eyes") return base * 1.7;
    if (fright) return base * 0.55;
    if (gh.y === g.board.tunnelRow) return base * 0.6;   /* the tunnel slows hunters */
    return base;
  }

  function classicGhostTick(g, gh, dt, fright) {
    if (gh.phase === "house") {
      if (g.t >= gh.releaseAt) { gh.phase = "leaving"; gh.prog = 0; }
      return;
    }
    if (gh.phase === "leaving") { classicLeave(g, gh, dt); return; }
    if (gh.phase === "entering") { classicEnter(g, gh, dt); return; }
    gh.speed = classicGhostSpeed(g, gh, fright);
    stepEntity(g, gh, dt, () => {
      classicGhostThink(g, gh, fright && gh.phase === "active");
      return gh.dir;
    }, true);
    if (gh.phase === "eyes" && gh.x === g.board.home.x && gh.y === g.board.home.y) {
      gh.phase = "entering";
      gh.prog = 0;
    }
  }

  function classicReverse(g, want) {
    const p = g.player;
    if (want !== REVERSE_OF[p.dir]) return;
    const d = DIRS[p.dir];
    if (!passableC(g.board, p.x + d.dx, p.y + d.dy)) return;
    p.x += d.dx; p.y += d.dy;
    if (p.x < 0) p.x += g.board.w; else if (p.x >= g.board.w) p.x -= g.board.w;
    p.dir = REVERSE_OF[p.dir];
    p.prog = 1 - p.prog;
    p.want = null;
  }

  function eatAtClassic(g) {
    const b = g.board;
    const p = g.player;
    const key = p.y * b.w + p.x;
    if (b.pellets.delete(key)) {
      g.score += 10;
      g.pelletsEaten += 1;
      if (!g.fruit && FRUIT_AT.includes(g.pelletsEaten)) {
        g.fruit = { until: g.t + 9 * TICKS_PER_S };
      }
      return;
    }
    if (b.powers.includes(key) && !g.eatenPowers.has(key)) {
      g.eatenPowers.add(key);
      g.score += 50;
      g.powersEaten += 1;
      g.frightUntil = g.t + g.cfg.frightS * TICKS_PER_S;
      g.chain = 0;
      for (const gh of g.ghosts) {
        if (gh.phase === "active") gh.dir = REVERSE_OF[gh.dir];  /* the turnabout */
      }
      return;
    }
    if (g.fruit) {
      if (g.t > g.fruit.until) { g.fruit = null; return; }
      if (p.x === FRUIT_TILE.x && p.y === FRUIT_TILE.y) {
        g.score += Math.min(500, 100 * g.level);
        g.fruitEaten += 1;
        g.fruit = null;
      }
    }
  }

  function classicCollide(g) {
    const p = g.player;
    const fright = g.t < g.frightUntil;
    for (const gh of g.ghosts) {
      if (gh.phase !== "active") continue;
      const same = gh.x === p.x && gh.y === p.y;
      const swap = gh.px === p.x && gh.py === p.y && gh.x === p.px && gh.y === p.py;
      if (!same && !swap) continue;
      if (!fright) { killPlayer(g); return; }
      g.chain += 1;
      g.score += Math.min(1600, 200 * 2 ** (g.chain - 1));   /* 200-400-800-1600 */
      g.ghostsEaten += 1;
      gh.phase = "eyes";
      gh.prog = 0;
    }
  }

  function classicTick(g, input, engineChoose) {
    if (g.over) return g;
    g.engineChoose = engineChoose || null;
    const dt = 1 / TICKS_PER_S;
    g.t += 1;
    g.ticks += 1;
    g.phaseT += 1;

    /* READY: nothing moves */
    if (g.t < g.readyUntil) return g;

    const p = g.player;
    p.px = p.x; p.py = p.y;
    for (const gh of g.ghosts) { gh.px = gh.x; gh.py = gh.y; }

    const want = g.cfg.botPlayer ? botIntent(g) : (input && input.want) || null;
    if (want) {
      classicReverse(g, want);
      p.want = want;
    }

    const fright = g.t < g.frightUntil;
    p.speed = g.cfg.playerSpeed;
    stepEntity(g, p, dt, () => {
      if (p.want && passableC(g.board, p.x + DIRS[p.want].dx, p.y + DIRS[p.want].dy)) {
        return p.want;
      }
      if (passableC(g.board, p.x + DIRS[p.dir].dx, p.y + DIRS[p.dir].dy)) return p.dir;
      return null;
    }, true);
    eatAtClassic(g);

    for (const gh of g.ghosts) classicGhostTick(g, gh, dt, fright);

    classicCollide(g);

    if (!g.over && g.board.pellets.size === 0 &&
        g.board.powers.every((k) => g.eatenPowers.has(k))) {
      classicLevelClear(g);
    }

    if (g.ticks >= g.cfg.maxTicks) g.over = true;
    return g;
  }

  function tickGame(g, input, engineChoose) {
    if (g.over) return g;
    if (g.board.classic) return classicTick(g, input, engineChoose);
    g.engineChoose = engineChoose || null;
    const dt = 1 / TICKS_PER_S;
    g.t += 1;
    g.ticks += 1;

    const p = g.player;
    p.px = p.x; p.py = p.y;
    for (const gh of g.ghosts) { gh.px = gh.x; gh.py = gh.y; }

    /* player intent: the shell's queue, or the bot's read */
    const want = g.cfg.botPlayer ? botIntent(g) : (input && input.want) || null;
    if (want) {
      playerReverse(g, want);
      p.want = want;
    }

    /* intercept anchor: 3 tiles ahead of the player, snapped to floor */
    const pd = DIRS[p.dir];
    g.interceptAnchor = nearestFloor(g.board, p.x + pd.dx * 3, p.y + pd.dy * 3);

    eatAt(g);
    stepEntity(g, p, dt, playerChoose(g));
    eatAt(g);

    for (const gh of g.ghosts) {
      if (gh.state === "house") {
        const gate = gh.respawnAt || gh.releaseAt;
        if (g.t >= gate) {
          gh.state = "active";
          gh.respawnAt = 0;
          gh.speed = g.cfg.ghostSpeed;
        }
        continue;
      }
      stepEntity(g, gh, dt, ghostChoose(g, gh));
      if (gh.state === "eyes" && gh.x === g.board.home.x && gh.y === g.board.home.y) {
        gh.state = "house";
        gh.respawnAt = g.t + 2 * TICKS_PER_S;
        gh.prog = 0;
      }
    }

    collide(g);

    /* level clear: every pellet and power pellet eaten */
    if (!g.over && g.board.pellets.size === 0 &&
        g.board.powers.every((k) => g.eatenPowers.has(k))) {
      levelClear(g);
    }

    if (g.ticks >= g.cfg.maxTicks) g.over = true;
    return g;
  }

  function runSummary(g) {
    return {
      seed: g.cfg.seed,
      board: g.cfg.board,
      fruitEaten: g.fruitEaten,
      score: g.score,
      level: g.level,
      lives: g.lives,
      over: g.over,
      pelletsEaten: g.pelletsEaten,
      pelletsTotal: g.board.pelletsTotal,
      powersEaten: g.powersEaten,
      ghostsEaten: g.ghostsEaten,
      ticks: g.ticks,
      engineCalls: g.engineCalls,
      rungs: { ...g.rungs },
      lastDecision: g.lastDecision,
    };
  }

  /* drive a whole game headless — the bot supplies intents, an optional
     engine breaks ties; identical to driving tickGame by hand */
  function simulateGame(rawCfg, engineChoose, maxTicksOverride) {
    const g = createGame(rawCfg);
    const cap = maxTicksOverride || g.cfg.maxTicks;
    while (!g.over && g.ticks < cap) tickGame(g, null, engineChoose);
    return g;
  }

  globalThis.PacCore = {
    TICKS_PER_S, DIRS, DIR_ORDER, REVERSE_OF, INTENTS, PERSONALITIES,
    DEFAULT_CONFIG, normalizeConfig, pacStamp,
    buildBoard, buildClassicBoard, passable, passableC, distFrom, nearestFloor,
    CLASSIC_ROWS, isScatter, classicTarget,
    makePlayer, makeGhosts, ghostCandidates, tieRequest,
    createGame, tickGame, runSummary, simulateGame, botIntent,
  };
})();
