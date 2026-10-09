/* GridWars shell — the arena, the input, the neon, and the panels.
   gridwars-core.js owns the deterministic sim (seeded waves, the enemy rule
   ladder, the spring grid, the particles); this file is chrome: a DPR canvas,
   a fixed-timestep loop that boots paused, mouse + keyboard input, the
   warping-grid render, the options accordion on the left, the live game-state
   panel on the right, the session-only scoreboard, and the engine bridge.

   Honesty rules this file keeps: the scoreboard is page memory and says so,
   the engine chip names its real status, and the rung counters are the core's
   own — nothing here invents a decision the core did not make. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const GC = globalThis.GridWarsCore;
  const canvas = document.getElementById("gridwars-canvas");
  if (!canvas || !DK || !GC) return;
  const ctx = canvas.getContext("2d");
  const byId = (id) => document.getElementById(id);
  const h = DK.shell.h;
  const nowMs = DK.nowMs;
  const W = GC.W, H = GC.H, DT = 1 / GC.TICKS_PER_S;
  const REDUCED = window.matchMedia
    && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  const TYPE = GC.ENEMY_TYPES, WEP = GC.WEAPONS;

  /* ---------- state ---------- */
  const S = {
    game: null,
    running: false,          /* boots paused — the play button starts it */
    last: 0, acc: 0, raf: 0, frames: 0,
    scaleX: 1, scaleY: 1,
    mouse: { x: W / 2, y: H * 0.25, inside: false },
    firing: false,
    recorded: false,         /* this run already landed on the board */
    board: DK.sessionBoard(),
    hudEls: null, tagEls: null, kvEls: null, engEls: null,
  };

  /* no importBase: the SDK's default "../wasm/" is right for a root page */
  const engine = DK.engineBridge({ onStatus: setChip });
  function engineAsk(request) {
    const t0 = nowMs();
    const res = engine.choose(request);
    if (res) S.metersMs = (S.metersMs || 0) + (nowMs() - t0);
    return res;
  }

  /* ---------- config from the panels ---------- */
  function wepModsFromUI() {
    const mods = {};
    for (const input of document.querySelectorAll("#gw-acc-weapons input[data-wep]")) {
      const k = input.getAttribute("data-wep"), key = input.getAttribute("data-key");
      const v = Number(input.value);
      mods[k] = mods[k] || {};
      mods[k][key] = key === "spread" ? v : Math.round(v);
    }
    return mods;
  }

  function cfgFromUI(seedOverride) {
    return {
      seed: seedOverride !== undefined ? seedOverride
        : (byId("gw-seed").value.trim() || "oc-gridwars"),
      difficulty: byId("gw-difficulty").value,
      spawnRate: Number(byId("gw-spawnrate").value),
      maxEnemies: Number(byId("gw-maxenemies").value),
      shipSpeed: Number(byId("gw-speed").value),
      inertia: Number(byId("gw-inertia").value),
      lives: Number(byId("gw-lives").value),
      invuln: Number(byId("gw-invuln").value),
      warp: Number(byId("gw-warp").value),
      density: Number(byId("gw-density").value),
      weaponMods: wepModsFromUI(),
    };
  }

  function newGame() {
    recordIfDue("dnf");
    S.game = GC.createGame(cfgFromUI());
    S.game.player.weapon = byId("gw-weapon-start").value;
    S.running = false;       /* restarts land paused, same as boot */
    S.recorded = false;
    S.acc = 0;
    setPlayLabels();
    syncWeaponButtons();
    render();
  }

  /* live tuning: the seed stays put, so the run stays comparable */
  function applyLive() {
    const g = S.game;
    if (!g) return;
    const before = g.cfg.density;
    g.cfg = GC.normalizeConfig(cfgFromUI(g.cfg.seed));
    if (g.cfg.density !== before) g.grid = GC.makeGrid(g.cfg.density);
    syncWeaponButtons();
  }

  function start() {
    if (!S.game) newGame();
    if (S.game.over) { newGame(); }
    S.running = true;
    setPlayLabels();
  }

  function pause() {
    S.running = false;
    setPlayLabels();
    render();
  }

  function togglePause() { if (S.running) pause(); else start(); }

  function setPlayLabels() {
    const on = S.running;
    for (const id of ["gw-play", "gw-play2"]) {
      const b = byId(id);
      if (b) b.textContent = on ? "⏸ pause" : (S.game && S.game.over ? "▶ play again" : "▶ play");
    }

  }

  function selectWeapon(key) {
    const g = S.game;
    if (!g || !GC.WEAPONS[key]) return;
    g.player.weapon = key;
    g.player.cd = Math.min(g.player.cd, GC.weaponOf(g, key).cooldown);
    syncWeaponButtons();
  }

  function syncWeaponButtons() {
    const g = S.game;
    const cur = g ? g.player.weapon : byId("gw-weapon-start").value;
    for (const b of document.querySelectorAll(".gw-wepbtn")) {
      const on = b.getAttribute("data-weapon") === cur;
      b.classList.toggle("on", on);
      b.setAttribute("aria-pressed", String(on));
    }
  }

  function bomb() {
    const g = S.game;
    if (!g) return;
    GC.useBomb(g);
  }

  /* ---------- session board (page memory, nothing stored or sent) ---------- */
  function recordIfDue(outcome) {
    const g = S.game;
    if (!g || S.recorded) return;
    if (!g.over && g.ticks < 60) return;
    const s = GC.runSummary(g);
    S.recorded = true;
    S.board.record({
      ts: Date.now(), outcome: g.over ? "done" : outcome,
      score: s.score, wave: s.wave, tier: s.tier, mult: s.multPeak,
      kills: s.kills, timeS: Math.round(s.timeS), seed: g.cfg.seed,
      weapon: g.player.weapon,
    });
  }

  /* ---------- canvas sizing ---------- */
  function sizeCanvas() {
    const rect = canvas.getBoundingClientRect();
    const cssW = rect.width || W;
    const cssH = cssW * (H / W);
    const dpr = window.devicePixelRatio || 1;
    const pw = Math.max(1, Math.round(cssW * dpr));
    const ph = Math.max(1, Math.round(cssH * dpr));
    if (canvas.width !== pw || canvas.height !== ph) {
      canvas.width = pw;
      canvas.height = ph;
    }
    S.scaleX = pw / W;
    S.scaleY = ph / H;
  }

  /* ---------- rendering ---------- */
  const ARENA_BG = "#06080f";
  const GRID_HI = "rgba(96,165,250,0.55)";
  const GRID_LO = "rgba(52,96,168,0.15)";

  function strokeTwice(color, wide, thin) {
    ctx.strokeStyle = color;
    ctx.globalAlpha = 0.20;
    ctx.lineWidth = wide;
    ctx.stroke();
    ctx.globalAlpha = 1;
    ctx.lineWidth = thin;
    ctx.stroke();
  }

  function drawGrid(g) {
    const { cols, rows, nodes } = g.grid;
    ctx.beginPath();
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        const n = nodes[r * cols + c];
        if (c === 0) ctx.moveTo(n.x, n.y); else ctx.lineTo(n.x, n.y);
      }
    }
    for (let c = 0; c < cols; c++) {
      for (let r = 0; r < rows; r++) {
        const n = nodes[r * cols + c];
        if (r === 0) ctx.moveTo(n.x, n.y); else ctx.lineTo(n.x, n.y);
      }
    }
    ctx.lineJoin = "round";
    strokeTwice(GRID_LO, 4.5, 1);
    strokeTwice(GRID_HI, 1.4, 0.5);
  }

  function shapePath(e, t, size) {
    const s = size === undefined ? e.size : size;
    switch (t.shape) {
      case "diamond":
        ctx.moveTo(e.x, e.y - s); ctx.lineTo(e.x + s, e.y);
        ctx.lineTo(e.x, e.y + s); ctx.lineTo(e.x - s, e.y);
        ctx.closePath();
        return true;
      case "square": {
        const a = e.t * 0.02, c = Math.cos(a) * s, d = Math.sin(a) * s;
        ctx.moveTo(e.x + c, e.y + d); ctx.lineTo(e.x - d, e.y + c);
        ctx.lineTo(e.x - c, e.y - d); ctx.lineTo(e.x + d, e.y - c);
        ctx.closePath();
        return true;
      }
      case "pinwheel": {
        const a0 = e.t * 0.06;
        for (let i = 0; i < 4; i++) {
          const a = a0 + (Math.PI / 2) * i;
          ctx.moveTo(e.x, e.y);
          ctx.lineTo(e.x + Math.cos(a) * s, e.y + Math.sin(a) * s);
          ctx.lineTo(e.x + Math.cos(a + 0.9) * s * 0.55, e.y + Math.sin(a + 0.9) * s * 0.55);
        }
        ctx.closePath();
        return true;
      }
      case "rocket": {
        const a = Math.atan2(e.vy, e.vx);
        ctx.moveTo(e.x + Math.cos(a) * s * 1.5, e.y + Math.sin(a) * s * 1.5);
        ctx.lineTo(e.x + Math.cos(a + 2.5) * s, e.y + Math.sin(a + 2.5) * s);
        ctx.lineTo(e.x + Math.cos(a - 2.5) * s, e.y + Math.sin(a - 2.5) * s);
        ctx.closePath();
        return true;
      }
      case "snake": {
        for (let i = e.segs.length - 1; i >= 0; i--) {
          const q = e.segs[i];
          ctx.moveTo(q.x + s * 0.7, q.y);
          ctx.arc(q.x, q.y, s * 0.7, 0, Math.PI * 2);
        }
        ctx.moveTo(e.x + s, e.y);
        ctx.arc(e.x, e.y, s, 0, Math.PI * 2);
        return true;
      }
      case "well": {
        ctx.moveTo(e.x + s, e.y);
        ctx.arc(e.x, e.y, s, 0, Math.PI * 2);
        ctx.moveTo(e.x + s * 0.45, e.y);
        ctx.arc(e.x, e.y, s * 0.45, 0, Math.PI * 2);
        const a0 = e.t * 0.05;
        for (let i = 0; i < 4; i++) {
          const a = a0 + (Math.PI / 2) * i;
          ctx.moveTo(e.x + Math.cos(a) * s * 0.5, e.y + Math.sin(a) * s * 0.5);
          ctx.lineTo(e.x + Math.cos(a) * (s + 5 + Math.sin(e.t * 0.1) * 3),
            e.y + Math.sin(a) * (s + 5 + Math.sin(e.t * 0.1) * 3));
        }
        return true;
      }
      default: {
        ctx.moveTo(e.x + s, e.y);
        ctx.arc(e.x, e.y, s, 0, Math.PI * 2);
        return true;
      }
    }
  }

  function drawEnemies(g) {
    for (const e of g.enemies) {
      const t = TYPE[e.type];
      if (!t) continue;
      /* snakes show their body armour dimmer than the head */
      const alpha = e.type === "snake" ? 0.95 : 1;
      ctx.beginPath();
      shapePath(e, t);
      ctx.globalAlpha = 0.18 * alpha;
      ctx.strokeStyle = t.color;
      ctx.lineWidth = 7;
      ctx.stroke();
      ctx.globalAlpha = alpha;
      ctx.lineWidth = 1.8;
      ctx.stroke();
      /* a wounded splitter shows its cracks */
      if (e.maxHp > 1 && e.hp < e.maxHp) {
        ctx.globalAlpha = 0.8;
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(e.x - e.size, e.y);
        ctx.lineTo(e.x + e.size, e.y);
        ctx.stroke();
      }
      /* feed counter on wells: how close to bursting */
      if (e.type === "well" && e.feed > 0) {
        ctx.globalAlpha = 0.55;
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(e.x, e.y, e.size + 9, -Math.PI / 2,
          -Math.PI / 2 + (Math.PI * 2) * Math.min(1, e.feed / GC.WELL_BURST));
        ctx.stroke();
      }
    }
    ctx.globalAlpha = 1;
  }

  function drawPlayer(g) {
    const p = g.player;
    if (!p.alive) return;
    const a = p.aim;
    ctx.save();
    if (!REDUCED) { ctx.shadowColor = "#67e8f9"; ctx.shadowBlur = 14; }
    ctx.beginPath();
    ctx.moveTo(p.x + Math.cos(a) * 13, p.y + Math.sin(a) * 13);
    ctx.lineTo(p.x + Math.cos(a + 2.5) * 10, p.y + Math.sin(a + 2.5) * 10);
    ctx.lineTo(p.x + Math.cos(a - 2.5) * 10, p.y + Math.sin(a - 2.5) * 10);
    ctx.closePath();
    ctx.strokeStyle = "#e0f2fe";
    ctx.lineWidth = 1.8;
    ctx.stroke();
    ctx.globalAlpha = 0.22;
    ctx.lineWidth = 6;
    ctx.stroke();
    ctx.globalAlpha = 1;
    ctx.restore();
    if (p.invuln > 0) {
      ctx.beginPath();
      ctx.arc(p.x, p.y, 16 + Math.sin(g.ticks * 0.2) * 2, 0, Math.PI * 2);
      ctx.strokeStyle = "#7dd3fc";
      ctx.globalAlpha = 0.5;
      ctx.setLineDash([4, 5]);
      ctx.lineWidth = 1.4;
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.globalAlpha = 1;
    }
  }

  function drawBullets(g) {
    for (const b of g.bullets) {
      const sp = Math.hypot(b.vx, b.vy) || 1;
      const ux = b.vx / sp, uy = b.vy / sp;
      ctx.beginPath();
      ctx.moveTo(b.x - ux * 6, b.y - uy * 6);
      ctx.lineTo(b.x + ux * 6, b.y + uy * 6);
      strokeTwice(b.color, 5, 2);
    }
  }

  function drawRings(g) {
    for (const r of g.rings) {
      const k = r.r / r.r1;
      ctx.beginPath();
      ctx.arc(r.x, r.y, r.r, 0, Math.PI * 2);
      ctx.globalAlpha = 0.85 * (1 - k * 0.6);
      ctx.strokeStyle = r.color;
      ctx.lineWidth = 3.5;
      ctx.stroke();
      ctx.globalAlpha = 0.18 * (1 - k * 0.6);
      ctx.lineWidth = 12;
      ctx.stroke();
      ctx.globalAlpha = 1;
    }
  }

  function drawBeams(g) {
    for (const b of g.beams) {
      const k = b.life / 9;
      ctx.beginPath();
      ctx.moveTo(b.x1, b.y1);
      ctx.lineTo(b.x2, b.y2);
      ctx.strokeStyle = b.color;
      ctx.globalAlpha = 0.25 * k;
      ctx.lineWidth = 14;
      ctx.stroke();
      ctx.globalAlpha = 0.9 * k;
      ctx.lineWidth = 3;
      ctx.stroke();
      ctx.strokeStyle = "#fff";
      ctx.globalAlpha = 0.8 * k;
      ctx.lineWidth = 1;
      ctx.stroke();
      ctx.globalAlpha = 1;
    }
  }

  function drawGeoms(g) {
    ctx.fillStyle = "#5eead4";
    for (const q of g.geoms) {
      const s = 2.4;
      ctx.fillRect(q.x - s, q.y - s, s * 2, s * 2);
    }
  }

  function drawParticles(g) {
    for (const q of g.particles) {
      ctx.globalAlpha = Math.max(0, Math.min(1, q.life));
      ctx.fillStyle = q.color;
      const s = q.size;
      ctx.fillRect(q.x - s, q.y - s, s * 2, s * 2);
    }
    ctx.globalAlpha = 1;
  }

  function render() {
    const g = S.game;
    ctx.setTransform(S.scaleX, 0, 0, S.scaleY, 0, 0);
    ctx.globalCompositeOperation = "source-over";
    ctx.fillStyle = ARENA_BG;
    ctx.fillRect(0, 0, W, H);
    if (!g) return;

    ctx.save();
    if (g.shake > 0 && !REDUCED) {
      ctx.translate((Math.random() - 0.5) * 8 * g.shake, (Math.random() - 0.5) * 8 * g.shake);
    }
    drawGrid(g);
    ctx.globalCompositeOperation = "lighter";
    drawGeoms(g);
    drawParticles(g);
    drawBullets(g);
    drawRings(g);
    drawBeams(g);
    drawEnemies(g);
    drawPlayer(g);
    ctx.globalCompositeOperation = "source-over";
    ctx.restore();

    if (g.flash > 0 && !REDUCED) {
      ctx.fillStyle = "rgba(191,219,254," + (g.flash * 0.30).toFixed(3) + ")";
      ctx.fillRect(0, 0, W, H);
    }
    if (!S.running && !g.over) {
      ctx.fillStyle = "rgba(226,232,240,0.85)";
      ctx.font = "600 22px ui-sans-serif, system-ui, sans-serif";
      ctx.textAlign = "center";
      ctx.fillText("paused — press play or P", W / 2, H / 2 - 40);
      ctx.font = "400 14px ui-sans-serif, system-ui, sans-serif";
      ctx.fillStyle = "rgba(148,163,184,0.9)";
      ctx.fillText("mouse aims · left click fires · right click swaps · WASD moves", W / 2, H / 2 - 14);
    }
    if (g.over) {
      ctx.fillStyle = "rgba(226,232,240,0.92)";
      ctx.font = "600 26px ui-sans-serif, system-ui, sans-serif";
      ctx.textAlign = "center";
      ctx.fillText("run over — " + DK.fmtNum(g.score) + " pts, wave " + g.wave, W / 2, H / 2 - 30);
    }
  }

  /* ---------- panels ---------- */
  function buildHud() {
    const host = byId("gw-hud");
    host.textContent = "";
    const fields = ["score", "multiplier", "lives", "bombs", "wave", "tier"];
    S.hudEls = {};
    for (const f of fields) {
      const el = h("span", { class: "mz-kv" }, f + " ");
      const b = h("b", null, "—");
      el.appendChild(b);
      host.appendChild(el);
      S.hudEls[f] = b;
    }
  }

  function buildStatePanel() {
    const kv = byId("gw-kv");
    kv.textContent = "";
    S.kvEls = {};
    for (const f of ["geometry on field", "particles alive", "kills", "deaths",
      "shots", "geoms taken", "peak multiplier", "run time", "bomb"])
    {
      const el = h("div", { class: "gw-kv" }, h("span", null, f + " "));
      const b = h("b", null, "—");
      el.appendChild(b);
      kv.appendChild(el);
      S.kvEls[f] = b;
    }
    const tags = byId("gw-tags");
    tags.textContent = "";
    S.tagEls = {};
    for (const tag of GC.TAGS) {
      const row = h("div", { class: "gw-tagrow" });
      const name = h("span", { class: "gw-tagname" }, tag);
      const bar = h("span", { class: "gw-tagbar" }, h("i", { class: "gw-tagfill" }));
      const n = h("b", { class: "gw-tagn" }, "0");
      row.append(name, bar, n);
      tags.appendChild(row);
      S.tagEls[tag] = { fill: bar.firstChild, n };
    }
    const eng = byId("gw-engine");
    eng.textContent = "";
    S.engEls = {};
    for (const f of ["engine asks", "answered by engine", "answered by rule", "last pick"]) {
      const el = h("div", { class: "gw-kv" }, h("span", null, f + " "));
      const b = h("b", null, "—");
      el.appendChild(b);
      eng.appendChild(el);
      S.engEls[f] = b;
    }
  }

  function updatePanels() {
    const g = S.game;
    if (!g) return;
    const s = GC.runSummary(g);
    const tier = GC.TIERS[s.tier - 1];
    S.hudEls.score.textContent = DK.fmtNum(s.score);
    S.hudEls.multiplier.textContent = "x" + s.mult;
    S.hudEls.lives.textContent = String(Math.max(0, s.lives));
    S.hudEls.bombs.textContent = String(s.bombs);
    S.hudEls.wave.textContent = String(s.wave);
    S.hudEls.tier.textContent = tier ? tier.id : "?";

    const k = S.kvEls;
    k["geometry on field"].textContent = String(s.enemies);
    k["particles alive"].textContent = s.particles + " / " + GC.PARTICLE_CAP;
    k.kills.textContent = String(s.kills);
    k.deaths.textContent = String(s.deaths);
    k.shots.textContent = String(s.shots);
    k["geoms taken"].textContent = String(s.geoms);
    k["peak multiplier"].textContent = "x" + s.multPeak;
    k["run time"].textContent = DK.fmtTime(s.timeS);
    k.bomb.textContent = String(s.bombs) + " (+" + (Math.round(g.nextBombAt) - s.score) + " pts)";

    const maxTag = Math.max(1, s.decisions.total);
    for (const tag of GC.TAGS) {
      const n = s.decisions.tags[tag] || 0;
      S.tagEls[tag].n.textContent = String(n);
      S.tagEls[tag].fill.style.width = (100 * n / maxTag).toFixed(1) + "%";
    }

    const e = S.engEls;
    e["engine asks"].textContent = String(s.engineCalls);
    e["answered by engine"].textContent = String(s.rungs.engine);
    e["answered by rule"].textContent = String(s.rungs.rule);
    e["last pick"].textContent = g.lastEngine
      ? g.lastEngine.pick + " (" + g.lastEngine.why + ", " + g.lastEngine.rung + ")"
      : "none yet";

    const tr = byId("gw-trace");
    if (g.lastDecision) {
      const d = g.lastDecision;
      tr.textContent = "tick " + d.tick + " — " + (TYPE[d.type] ? TYPE[d.type].label.toLowerCase() : d.type)
        + " #" + d.id + " decided " + d.tag;
    }
    if (g.lastEngine) {
      tr.textContent += "  ·  engine: " + g.lastEngine.rung + " → " + g.lastEngine.pick
        + " for " + g.lastEngine.why;
    }
  }

  /* ---------- legend ---------- */
  function buildLegend() {
    const host = byId("gw-legend");
    host.textContent = "";
    for (const key of GC.WEAPON_ORDER) {
      const w = WEP[key];
      host.appendChild(h("div", { class: "gw-leg" },
        h("span", { class: "gw-leg-key" }, w.key),
        h("span", { class: "gw-swatch", style: "background:" + w.color }),
        h("span", null, h("b", null, w.label + " — "), w.desc)));
    }
    const controls = [
      ["LMB", "hold to fire"], ["RMB", "swap weapon"], ["W A S D", "thrust"],
      ["space", "bomb — clears the field, scores nothing"],
      ["P", "pause"], ["R", "restart"],
    ];
    for (const [k, v] of controls) {
      host.appendChild(h("div", { class: "gw-leg" },
        h("span", { class: "gw-leg-key" }, k), h("span", { class: "gw-swatch" }),
        h("span", null, v)));
    }
    host.appendChild(h("div", { class: "gw-leg gw-leg-wide" },
      h("span", { class: "gw-leg-key" }, "the geometry")));
    for (const key of Object.keys(TYPE)) {
      if (TYPE[key].child) continue;
      host.appendChild(h("div", { class: "gw-leg" },
        h("span", { class: "gw-swatch", style: "background:" + TYPE[key].color }),
        h("span", null, h("b", null, TYPE[key].label + " — "), TYPE[key].desc)));
    }
  }

  /* ---------- dialogs: keybindings + session scoreboard ---------- */
  function overlay(id, title, bodyId, closeId) {
    return h("dialog", { class: "mz-overlay", id },
      h("div", { class: "mz-board-card" },
        h("div", { class: "mz-overlay-head" },
          h("span", { class: "mz-overlay-title" }, title),
          h("button", { class: "mz-chip", id: closeId, type: "button" }, "✕ close")),
        h("div", { class: "mz-overlay-scroll", id: bodyId })));
  }

  function buildOverlays() {
    document.getElementById("gridwars-root").appendChild(
      overlay("gw-keys-overlay", "Keybindings", "gw-keys-body", "gw-keys-close"));
    document.getElementById("gridwars-root").appendChild(
      overlay("gw-board-overlay", "Session scoreboard", "gw-board-body", "gw-board-close"));

    const kb = byId("gw-keys-body");
    kb.textContent = "";
    const rows = [
      ["mouse move", "aim — the ship always faces the pointer"],
      ["left click (hold)", "fire the current weapon"],
      ["right click", "swap to the next weapon (same as 1→2→3→4)"],
      ["W / ↑", "thrust up"],
      ["A / ←", "thrust left"],
      ["S / ↓", "thrust down"],
      ["D / →", "thrust right"],
      ["1", "pulse — fast single shot"],
      ["2", "scatter — 3-way spread"],
      ["3", "laser — piercing beam, long cooldown"],
      ["4", "wave — slow expanding ring"],
      ["space", "bomb — clears the field (scores nothing, drops no geoms)"],
      ["P", "pause / resume"],
      ["R", "restart the run with the same seed"],
    ];
    for (const [k, v] of rows) {
      kb.appendChild(h("div", { class: "gw-keyrow" },
        h("kbd", null, k), h("span", null, v)));
    }
    kb.appendChild(h("p", { class: "game-note" },
      "Sliders live in the accordion on the left; changing one tunes the run in place. ",
      "The seed field and the dice restart from wave 1 with a fresh sequence."));
  }

  function renderBoard() {
    const body = byId("gw-board-body");
    body.textContent = "";
    const rows = S.board.all();
    body.appendChild(h("p", { class: "game-note" },
      "Session-only — nothing is stored or sent. Close the tab and this list is gone."));
    if (!rows.length) {
      body.appendChild(h("p", { class: "game-note" },
        "No finished runs yet. Survive to a game-over or restart mid-run and the run lands here."));
      return;
    }
    const table = h("table", { class: "gw-board" },
      h("thead", null, h("tr", null,
        ["#", "outcome", "score", "wave", "tier", "peak ×", "kills", "time", "weapon", "seed"]
          .map((x) => h("th", null, x)))));
    const tb = h("tbody");
    rows.slice(0, 40).forEach((r, i) => {
      tb.appendChild(h("tr", null,
        [String(i + 1), r.outcome || "—", DK.fmtNum(r.score), String(r.wave),
          String(r.tier), "x" + r.mult, String(r.kills), DK.fmtTime(r.timeS),
          r.weapon || "—", String(r.seed)].map((x) => h("td", null, x))));
    });
    table.appendChild(tb);
    body.appendChild(table);
  }

  function setChip() {
    const chip = byId("gw-engine-chip");
    if (!chip) return;
    const st = engine.status;
    chip.textContent = "engine: " + st;
    chip.classList.toggle("on", st === "ready");
    chip.classList.toggle("off", st !== "ready");
    const t = byId("gw-engine-toggle");
    if (t) t.textContent = st === "off" ? "engine: off" : "engine: on";
  }

  /* ---------- input ---------- */
  const KEYMAP = {
    KeyW: "up", ArrowUp: "up",
    KeyA: "left", ArrowLeft: "left",
    KeyS: "down", ArrowDown: "down",
    KeyD: "right", ArrowRight: "right",
  };

  function bindInput() {
    window.addEventListener("keydown", (e) => {
      const tag = e.target && e.target.tagName;
      if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") return;
      const dir = KEYMAP[e.code];
      if (dir && S.game) { S.game.input[dir] = true; e.preventDefault(); return; }
      if (e.code === "Space") { bomb(); e.preventDefault(); return; }
      if (e.code === "Digit1") { selectWeapon("pulse"); return; }
      if (e.code === "Digit2") { selectWeapon("scatter"); return; }
      if (e.code === "Digit3") { selectWeapon("laser"); return; }
      if (e.code === "Digit4") { selectWeapon("wave"); return; }
      if (e.code === "KeyP") { togglePause(); return; }
      if (e.code === "KeyR") { newGame(); return; }
    });
    window.addEventListener("keyup", (e) => {
      const dir = KEYMAP[e.code];
      if (dir && S.game) S.game.input[dir] = false;
    });
    window.addEventListener("blur", () => {
      if (S.running) pause();
      if (S.game) S.game.input = GC.makeInput();
    });

    canvas.addEventListener("mousemove", (e) => {
      const rect = canvas.getBoundingClientRect();
      S.mouse.x = ((e.clientX - rect.left) / rect.width) * W;
      S.mouse.y = ((e.clientY - rect.top) / rect.height) * H;
      S.mouse.inside = true;
    });
    canvas.addEventListener("mouseleave", () => {
      S.mouse.inside = false;
      S.firing = false;
      if (S.game) S.game.input.fire = false;
    });
    canvas.addEventListener("mousedown", (e) => {
      if (e.button === 0) { S.firing = true; if (S.game) S.game.input.fire = true; }
      if (e.button === 2) cycleWeapon();
      e.preventDefault();
    });
    window.addEventListener("mouseup", (e) => {
      if (e.button === 0) { S.firing = false; if (S.game) S.game.input.fire = false; }
    });
    canvas.addEventListener("contextmenu", (e) => e.preventDefault());
    canvas.addEventListener("touchstart", (e) => {
      const t = e.touches[0];
      if (!t) return;
      const rect = canvas.getBoundingClientRect();
      S.mouse.x = ((t.clientX - rect.left) / rect.width) * W;
      S.mouse.y = ((t.clientY - rect.top) / rect.height) * H;
      e.preventDefault();
    }, { passive: false });
  }

  const WEP_CYCLE = GC.WEAPON_ORDER;
  function cycleWeapon() {
    const g = S.game;
    if (!g) return;
    const i = WEP_CYCLE.indexOf(g.player.weapon);
    selectWeapon(WEP_CYCLE[(i + 1) % WEP_CYCLE.length]);
  }

  /* ---------- loop ---------- */
  function loop(t) {
    S.raf = requestAnimationFrame(loop);
    if (!S.last) S.last = t;
    let ms = t - S.last;
    S.last = t;
    if (ms > 100) ms = 100;         /* a background tab owes us nothing */
    S.acc += ms / 1000;
    let steps = 0;
    while (S.running && S.acc >= DT && steps < 3) {
      if (S.game) {
        S.game.input.aim = Math.atan2(S.mouse.y - S.game.player.y,
          S.mouse.x - S.game.player.x);
        GC.tickGame(S.game, engine.status === "ready" ? engineAsk : null);
        if (S.game.over) { recordIfDue("done"); S.running = false; setPlayLabels(); }
      }
      S.acc -= DT;
      steps += 1;
    }
    if (steps === 3) S.acc = 0;
    render();
    S.frames += 1;
    if (S.frames % 3 === 0) updatePanels();
  }

  /* ---------- wire ---------- */
  function bindPanel() {
    for (const id of ["gw-speed", "gw-inertia", "gw-warp", "gw-density",
      "gw-spawnrate", "gw-maxenemies", "gw-lives", "gw-invuln"]) {
      const input = byId(id);
      if (!input) continue;
      const out = byId(id + "-val");
      const show = () => { if (out) out.textContent = input.value; };
      input.addEventListener("input", () => { show(); applyLive(); });
      input.addEventListener("change", applyLive);
      show();
    }
    for (const input of document.querySelectorAll("#gw-acc-weapons input[data-wep]")) {
      input.addEventListener("input", applyLive);
      input.addEventListener("change", applyLive);
    }
    byId("gw-difficulty").addEventListener("change", applyLive);
    byId("gw-weapon-start").addEventListener("change", () => {
      selectWeapon(byId("gw-weapon-start").value);
    });

    for (const id of ["gw-play", "gw-play2"]) byId(id).addEventListener("click", togglePause);
    byId("gw-pause2").addEventListener("click", pause);
    for (const id of ["gw-restart", "gw-restart2"]) byId(id).addEventListener("click", newGame);
    byId("gw-bomb").addEventListener("click", bomb);
    for (const b of document.querySelectorAll(".gw-wepbtn")) {
      b.addEventListener("click", () => selectWeapon(b.getAttribute("data-weapon")));
    }

    byId("gw-dice").addEventListener("click", () => {
      byId("gw-seed").value = DK.randomSeed();
      newGame();
    });
    byId("gw-seed").addEventListener("change", newGame);
    byId("gw-reset-params").addEventListener("click", () => {
      const c = GC.DEFAULT_CONFIG;
      byId("gw-difficulty").value = c.difficulty;
      byId("gw-spawnrate").value = String(c.spawnRate);
      byId("gw-maxenemies").value = String(c.maxEnemies);
      byId("gw-speed").value = String(c.shipSpeed);
      byId("gw-inertia").value = String(c.inertia);
      byId("gw-lives").value = String(c.lives);
      byId("gw-invuln").value = String(c.invuln);
      byId("gw-warp").value = String(c.warp);
      byId("gw-density").value = String(c.density);
      for (const input of document.querySelectorAll("#gw-acc-weapons input[data-wep]")) {
        const w = GC.WEAPON_LIMITS[input.getAttribute("data-wep")];
        const key = input.getAttribute("data-key");
        input.value = String(GC.WEAPONS[input.getAttribute("data-wep")][key]);
        const out = input.parentElement.querySelector(".game-sliderval");
        if (out) out.textContent = input.value;
      }
      for (const id of ["gw-speed", "gw-inertia", "gw-warp", "gw-density",
        "gw-spawnrate", "gw-maxenemies", "gw-lives", "gw-invuln"]) {
        const out = byId(id + "-val");
        if (out) out.textContent = byId(id).value;
      }
      applyLive();
    });

    byId("gw-keys").addEventListener("click", () => byId("gw-keys-overlay").showModal());
    byId("gw-keys-close").addEventListener("click", () => byId("gw-keys-overlay").close());
    byId("gw-board-btn").addEventListener("click", () => { renderBoard(); byId("gw-board-overlay").showModal(); });
    byId("gw-board-close").addEventListener("click", () => byId("gw-board-overlay").close());

    byId("gw-engine-toggle").addEventListener("click", () => {
      const on = engine.status === "off" || engine.status === "unavailable";
      engine.setEnabled(on);
      setChip();
    });
  }

  function build() {
    const deepSeed = new URLSearchParams(window.location.search).get("seed");
    if (deepSeed) byId("gw-seed").value = deepSeed;

    buildHud();
    buildStatePanel();
    buildLegend();
    buildOverlays();
    bindInput();
    bindPanel();

    if (window.ResizeObserver) {
      new ResizeObserver(() => sizeCanvas()).observe(canvas);
    } else {
      window.addEventListener("resize", sizeCanvas);
    }
    sizeCanvas();

    newGame();
    setChip();
    engine.setEnabled(true);   /* default on: wave leads reach the runtime */
    /* the loop always runs — reduced motion is honoured in render(), where
       shake and the bomb flash are gated off */
    requestAnimationFrame(loop);
  }

  build();
})();
