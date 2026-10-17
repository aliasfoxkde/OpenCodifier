/* Decisions SDK — the shared runtime behind every OpenCodifier playground
   game. One global (DecisionsSDK), one IIFE, no dependencies; DOM-free except
   the explicitly-marked `shell` section. What lives here is what every game
   shares: seeded rng, config normalization, rubric scoring, a session-only
   leaderboard, per-game meters (fps / decision time / engine latency), the
   single sanctioned WASM engine bridge, and the DOM shell helpers.

   Honesty rules the SDK enforces by construction:
   - seeded determinism everywhere (no Math.random in game logic);
   - the engine bridge meters every call and auto-disables after repeated
     failures — a game never stalls and never pretends it asked;
   - boards are memory-only: nothing a visitor does leaves the tab.
   See docs/planning/playground/DECISIONS-SDK-PLAN.md. */

(() => {
  "use strict";

  /* ---------- rng: string seed → mulberry32 ---------- */

  function hashSeed(str) {
    /* FNV-1a 32-bit — stable across engines, no Math.random anywhere */
    let h = 0x811c9dc5;
    const s = String(str);
    for (let i = 0; i < s.length; i++) {
      h ^= s.charCodeAt(i);
      h = Math.imul(h, 0x01000193);
    }
    return h >>> 0;
  }

  function mulberry32(a) {
    let t = a >>> 0;
    return function () {
      t = (t + 0x6d2b79f5) | 0;
      let r = Math.imul(t ^ (t >>> 15), 1 | t);
      r = (r + Math.imul(r ^ (r >>> 7), 61 | r)) ^ r;
      return ((r ^ (r >>> 14)) >>> 0) / 4294967296;
    };
  }

  function makeRng(seedStr) {
    return mulberry32(hashSeed(seedStr));
  }

  function pick(rng, arr) {
    return arr[Math.floor(rng() * arr.length)];
  }

  /* random seed as a short numeric string (6 digits, like the maze) */
  function randomSeed(rng) {
    const r = rng || Math.random;
    return String(100000 + Math.floor(r() * 900000));
  }

  /* ---------- config ---------- */

  function clamp(v, lo, hi, dflt) {
    const n = Number(v);
    if (!isFinite(n)) return dflt;
    return Math.min(hi, Math.max(lo, Math.round(n)));
  }

  function clampf(v, lo, hi, dflt) {
    const n = Number(v);
    if (!isFinite(n)) return dflt;
    return Math.min(hi, Math.max(lo, n));
  }

  /* spec-driven config normalization for game shells:
     normalize(raw, { cells: { def: 17, min: 13, max: 25 }, fog: { bool: true, def: false } })
     - {def, min, max}  → integer clamp (range inputs)
     - {fdef, min, max} → float clamp (rates, multipliers)
     - {bool: true}     → strict boolean (=== true)
     - {oneOf: [...]}   → enum with def as fallback            */
  function normalize(raw, spec) {
    const src = raw && typeof raw === "object" ? raw : {};
    const out = {};
    for (const key of Object.keys(spec)) {
      const s = spec[key];
      const v = src[key];
      if (s.bool) { out[key] = v === true; continue; }
      if (s.oneOf) { out[key] = s.oneOf.includes(v) ? v : s.def; continue; }
      if (s.fdef !== undefined) { out[key] = clampf(v, s.min, s.max, s.fdef); continue; }
      out[key] = clamp(v, s.min, s.max, s.def);
    }
    return out;
  }

  /* ---------- rubric: weighted criteria over normalized metrics ---------- */

  /* normalizeRubric(raw, opts):
     opts = { tags: ["time", ...], defWeight, minW, maxW, defTie: ["time"] }
     → { version: 1, criteria: [{tag, weight}], tiebreakers } */
  function normalizeRubric(raw, opts) {
    const tags = opts.tags;
    const minW = opts.minW === undefined ? -3 : opts.minW;
    const maxW = opts.maxW === undefined ? 5 : opts.maxW;
    const defWeight = opts.defWeight === undefined ? 1 : opts.defWeight;
    const src = raw && typeof raw === "object" ? raw : {};
    const seen = new Set();
    const criteria = [];
    const arr = Array.isArray(src.criteria) ? src.criteria : [];
    for (const c of arr) {
      if (!c || typeof c !== "object") continue;
      if (!tags.includes(c.tag) || seen.has(c.tag)) continue;
      seen.add(c.tag);
      criteria.push({ tag: c.tag, weight: clamp(c.weight, minW, maxW, defWeight) });
    }
    if (!criteria.length) {
      /* a game can ship a curated default (the maze's does — mixed weights,
         and not every tag present); otherwise every tag gets defWeight */
      if (opts.defRubric) return JSON.parse(JSON.stringify(opts.defRubric));
      return { version: 1, criteria: tags.map((t) => ({ tag: t, weight: defWeight })),
        tiebreakers: opts.defTie || [] };
    }
    const tieSeen = new Set();
    const tiebreakers = [];
    for (const t of (Array.isArray(src.tiebreakers) ? src.tiebreakers : [])) {
      if (tags.includes(t) && !tieSeen.has(t)) { tieSeen.add(t); tiebreakers.push(t); }
    }
    return {
      version: 1,
      criteria,
      tiebreakers: tiebreakers.length ? tiebreakers : (opts.defTie || []),
    };
  }

  /* scoreRun(summary, rubric, normFns):
     normFns = { tag: (summary) => 0..1 }; entries with weight 0 are kept in
     the breakdown for auditability but contribute nothing. A summary can
     opt out via summary.__dnf / summary.__assisted (games set their own
     semantic fields; the SDK only honors these two sentinels). */
  function scoreRun(s, rubric, normFns) {
    if (!rubric || !rubric.criteria) throw new Error("scoreRun: rubric required");
    if (s.__dnf || s.__assisted) {
      return { score: 0, dnf: !!s.__dnf, assisted: !!s.__assisted, breakdown: [] };
    }
    let num = 0;
    let den = 0;
    const breakdown = [];
    for (const c of rubric.criteria) {
      const fn = normFns[c.tag];
      const norm = fn ? Math.max(0, Math.min(1, Number(fn(s)) || 0)) : 0;
      if (c.weight === 0) { breakdown.push({ tag: c.tag, weight: 0, norm, contrib: 0 }); continue; }
      num += c.weight * norm;
      den += Math.abs(c.weight);
      breakdown.push({
        tag: c.tag, weight: c.weight,
        norm: Math.round(norm * 1000) / 1000,
        contrib: Math.round(c.weight * norm * 1000) / 1000,
      });
    }
    const score = den > 0 ? Math.round((100 * num) / den) : 0;
    return { score: Math.max(0, score), dnf: false, assisted: false, breakdown };
  }

  /* resolveWinner(pairs) — takes already-scored entries
     [{ summary, result }], prefers the top score; `tiebreak` (optional)
     breaks exact ties with the entry's own ordering rules. */
  function resolveWinner(scored, tiebreak) {
    const eligible = scored.filter((x) => !x.result.dnf && !x.result.assisted);
    if (eligible.length === 1) {
      return { winner: eligible[0].summary.runner, margin: 0, by: "forfeit", scored };
    }
    if (eligible.length < 2) return { winner: "none", margin: 0, scored };
    const [a, b] = eligible;
    if (a.result.score !== b.result.score) {
      const win = a.result.score > b.result.score ? a : b;
      const other = win === a ? b : a;
      return {
        winner: win.summary.runner, margin: Math.abs(win.result.score - other.result.score),
        by: "score", scored,
      };
    }
    if (tiebreak) {
      const by = tiebreak(a.summary, b.summary);
      if (by && by.winner) return { ...by, scored };
    }
    return { winner: "draw", margin: 0, by: "tiebreakers", scored };
  }

  /* ---------- session board: memory only, dies with the tab ---------- */

  function sessionBoard() {
    const rows = [];
    return {
      record(row) { rows.unshift(row); return row; },
      all() { return rows.slice(); },
      clear() { rows.length = 0; },
      /* filterFn(row) → bool, sortFn(a, b) → number — the game owns its
         column semantics (who/result filters, time-vs-score ordering) */
      filtered(filterFn, sortFn) {
        const out = rows.filter((r) => (filterFn ? filterFn(r) : true));
        return sortFn ? out.sort(sortFn) : out;
      },
    };
  }

  /* ---------- meters: measured numbers for the stats overlay ----------
     EMA averages plus slowly-decaying maxima: "recent typical" and "recent
     worst" without unbounded history. Counts are exact. */

  function nowMs() {
    return (typeof performance !== "undefined" && performance.now)
      ? performance.now() : Date.now();
  }

  function meters() {
    const store = new Map(); /* key → { ema, max, count, last } */
    function entry(key) {
      let e = store.get(key);
      if (!e) { e = { ema: 0, max: 0, count: 0, sum: 0, last: null }; store.set(key, e); }
      return e;
    }
    return {
      /* duration sample in ms */
      sample(key, ms) {
        const e = entry(key);
        e.ema = e.count ? e.ema * 0.9 + ms * 0.1 : ms;
        e.max = Math.max(ms, e.max * 0.995);
        e.count += 1;
        e.sum += ms;
        e.last = ms;
      },
      count(key, by) { entry(key).count += by === undefined ? 1 : by; },
      get(key) { return store.get(key) || null; },
      snapshot() {
        const out = {};
        for (const [k, e] of store) {
          out[k] = { avg: e.ema, worst: e.max, count: e.count, last: e.last };
        }
        return out;
      },
      now: nowMs,
    };
  }

  /* ---------- engine bridge: the one sanctioned WASM call path ----------

     engineBridge({ importBase, onStatus, failLimit })
       .ensure()   → Promise<engine | null> (lazy; resolves null on failure)
       .choose(req)→ engine's decision object | null (any error → null)
       .status     → "off" | "loading" | "ready" | "unavailable"
       .meters     → SDK meters: engine.calls, engine.ms

     After `failLimit` (default 5) consecutive failures the bridge turns
     itself off — the maze's proven behavior, generalized. Call sites must
     always have a local fallback for null. */

  function engineBridge(opts) {
    const o = opts || {};
    const base = o.importBase || "../wasm/";
    const failLimit = o.failLimit === undefined ? 5 : o.failLimit;
    const m = meters();
    let engine = null;
    let busy = false;
    let pending = null;          /* the in-flight load — ensure() hands it
                                    back so concurrent callers actually wait */
    let fails = 0;
    let status = "off";
    const api = {
      meters: m,
      get status() { return status; },
      get engine() { return engine; },
      setEnabled(on) {
        status = on ? "loading" : "off";
        if (o.onStatus) o.onStatus(status);
        if (on) api.ensure();
      },
      ensure() {
        if (engine) return Promise.resolve(engine);
        if (pending) return pending;
        busy = true;
        status = "loading";
        if (o.onStatus) o.onStatus(status);
        pending = import(base + "opencodifier_wasm.js")
          .then((mod) => mod.default().then(() => mod))
          .then((mod) => {
            engine = new mod.WasmEngine();
            busy = false;
            pending = null;
            status = "ready";
            if (o.onStatus) o.onStatus(status);
            return engine;
          })
          .catch(() => {
            busy = false;
            pending = null;
            status = "unavailable";
            if (o.onStatus) o.onStatus(status);
            return null;
          });
        return pending;
      },
      /* request: { state, questions, policy?, metadata? } — the same
         contract the server runtime speaks. Returns null on any failure;
         abstention from the engine itself is a valid non-null result. */
      choose(request) {
        if (!engine) return null;
        const t0 = nowMs();
        try {
          /* the wasm boundary speaks JSON strings BOTH ways: a plain object
             into decide() hangs the engine (measured, see FINDINGS.md), and
             decide() returns a JSON string — callers get an object back */
          const wire = typeof request === "string" ? request : JSON.stringify(request);
          const res = engine.decide(wire);
          fails = 0;
          return typeof res === "string" ? JSON.parse(res) : res;
        } catch (err) {
          fails += 1;
          if (fails > failLimit) {
            status = "unavailable";
            if (o.onStatus) o.onStatus(status);
          }
          return null;
        } finally {
          m.sample("engine.ms", nowMs() - t0);
          m.count("engine.calls");
        }
      },
    };
    return api;
  }

  /* ---------- fmt: shared number/time formatting ---------- */

  function fmtMs(ms) {
    if (ms === null || ms === undefined || !isFinite(ms)) return "—";
    return ms >= 100 ? Math.round(ms) + " ms" : ms.toFixed(2) + " ms";
  }
  function fmtPct(f) {
    return Math.round((Number(f) || 0) * 100) + "%";
  }
  function fmtTime(sec) {
    const s = Math.max(0, Math.floor(Number(sec) || 0));
    return Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0");
  }
  function fmtNum(n) {
    return (Number(n) || 0).toLocaleString("en-US");
  }

  /* ---------- shell (DOM): the shared game chrome ----------
     Everything below touches the DOM — it is the only section here that does,
     and the headless cores must never import it. */

  /* element factory: h("div", {class: "x", onclick: fn, "aria-…": v}, ...kids).
     Strings/numbers become text nodes; null/undefined are skipped. */
  function h(tag, attrs, ...kids) {
    const el = document.createElement(tag);
    if (attrs) {
      for (const [k, v] of Object.entries(attrs)) {
        if (v === null || v === undefined || v === false) continue;
        if (k === "class") el.className = v;
        else if (k === "text") el.textContent = v;
        else if (k.startsWith("on") && typeof v === "function") {
          el.addEventListener(k.slice(2).toLowerCase(), v);
        } else el.setAttribute(k, String(v));
      }
    }
    for (const kid of kids.flat()) {
      if (kid === null || kid === undefined || kid === false) continue;
      el.appendChild(typeof kid === "object" ? kid : document.createTextNode(String(kid)));
    }
    return el;
  }

  /* single-open accordion: items = [{ id, label, open, kids: [...] }] */
  function accordion(items) {
    const root = h("div", { class: "mz-acc" });
    for (const item of items) {
      root.appendChild(h("div", {
        class: "mz-acc-item" + (item.open ? " open" : ""),
        "data-acc-item": item.id,
      },
      h("button", {
        class: "mz-acc-head", type: "button", "aria-expanded": String(!!item.open),
        "data-acc": item.id,
      },
      h("span", { class: "mz-acc-label" }, item.label),
      h("span", { class: "mz-acc-chev", "aria-hidden": "true" }, "▾")),
      h("div", { class: "mz-acc-body" }, ...item.kids)));
    }
    root.addEventListener("click", (e) => {
      const head = e.target && e.target.closest ? e.target.closest(".mz-acc-head") : null;
      if (!head || !root.contains(head)) return;
      const id = head.getAttribute("data-acc");
      for (const itemEl of root.querySelectorAll(".mz-acc-item")) {
        const wasOpen = itemEl.classList.contains("open");
        const on = itemEl.getAttribute("data-acc-item") === id && !wasOpen;
        itemEl.classList.toggle("open", on);
        const hd = itemEl.querySelector(".mz-acc-head");
        if (hd) hd.setAttribute("aria-expanded", String(on));
      }
    });
    return root;
  }

  /* tabs: panes = [{ id, label, kids: [...], on }] — label buttons above,
     one visible pane; data-tab / data-pane attributes like the maze. */
  function tabs(panes) {
    const bar = h("div", { class: "mz-tabs", role: "tablist" });
    const body = h("div", { class: "mz-tabpanes" });
    const select = (id) => {
      for (const p of panes) {
        const on = p.id === id;
        const btn = bar.querySelector('[data-tab="' + p.id + '"]');
        const pane = body.querySelector('[data-pane="' + p.id + '"]');
        if (btn) btn.setAttribute("aria-selected", String(on));
        if (btn && btn.classList) btn.classList.toggle("on", on);
        if (pane && pane.classList) pane.classList.toggle("on", on);
      }
    };
    for (const p of panes) {
      const btn = h("button", {
        class: "mz-tab" + (p.on ? " on" : ""), type: "button", role: "tab",
        "data-tab": p.id, "aria-selected": String(!!p.on),
      }, p.label);
      btn.addEventListener("click", () => select(p.id));
      bar.appendChild(btn);
      body.appendChild(h("div", {
        class: "mz-tabpane" + (p.on ? " on" : ""), role: "tabpanel",
        "data-pane": p.id,
      }, ...p.kids));
    }
    return h("div", { class: "mz-info" }, bar, body);
  }

  /* key/value readout row: pairs = [[label, value, attr?], ...] */
  function kvRow(pairs) {
    return h("div", { class: "mz-under" },
      ...pairs.map(([label, value, attr]) => {
        const el = h("span", { class: "mz-kv" }, label + " ");
        if (attr) el.setAttribute("data-state", attr);
        el.appendChild(h("b", null, value));
        return el;
      }));
  }

  /* fullscreen toggle for a game window: requests fullscreen on `el`,
     flips `btn`'s label, and keeps the label honest if the user exits
     with Esc. Safe no-op where the API is missing (older webviews). */
  function fullscreen(el, btn) {
    /* defensive against DOM-stubbed test hosts: fullscreen is chrome */
    if (typeof document === "undefined" || !document.addEventListener
      || !el || !el.requestFullscreen) return;
    el.classList.add("game-fs");   /* shared :fullscreen chrome */
    const on = () => !!document.fullscreenElement;
    const set = () => { if (btn) btn.textContent = on() ? "⛶ exit full screen" : "⛶ full screen"; };
    if (btn) btn.addEventListener("click", () => {
      if (on()) { document.exitFullscreen().catch(() => {}); return; }
      el.requestFullscreen().catch(() => {});
    });
    document.addEventListener("fullscreenchange", set);
    set();
  }

  /* numeric game seed: guarantees the box always carries a numeric seed —
     a fresh page gets a random one, a cleared box gets one on next read,
     and the dice button rerolls. get() is the only read path a game needs.
     onChange fires after a dice roll (the game restarts itself). */
  function numericSeed(input, diceBtn, onChange) {
    if (!input) return { get: function () { return randomSeed(); }, roll: function () {} };
    input.type = "number";
    if (!/^\d+$/.test(String(input.value || "").trim())) input.value = randomSeed();
    const roll = () => {
      input.value = randomSeed();
      if (onChange) onChange();
    };
    if (diceBtn && diceBtn.addEventListener) diceBtn.addEventListener("click", roll);
    return {
      get() {
        const raw = String(input.value || "").trim();
        if (!/^\d+$/.test(raw)) { input.value = randomSeed(); }
        return String(parseInt(input.value, 10));
      },
      roll,
    };
  }

  /* restore-defaults: snapshots every form control under `root` at boot,
     and on `btn` click puts the boot values back, re-firing change/input
     so the game's own listeners apply them. `apply` runs once afterwards
     for games whose restart isn't wired to a control change. */
  function resetParams(btn, root, apply) {
    if (!btn || !btn.addEventListener || !root || !root.querySelectorAll) return;
    const sel = 'select, input[type="range"], input[type="number"],'
      + ' input[type="checkbox"], input[type="text"]';
    const boot = [...root.querySelectorAll(sel)].map((el) => [el, el.value, el.checked]);
    btn.addEventListener("click", () => {
      boot.forEach(([el, v, c]) => {
        if (el.type === "checkbox") { if (el.checked !== c) { el.checked = c; fire(el); } }
        else if (el.value !== v) { el.value = v; fire(el); }
      });
      if (apply) apply();
    });
    function fire(el) {
      if (typeof Event !== "function") return;
      el.dispatchEvent(new Event("input", { bubbles: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
    }
  }

  globalThis.DecisionsSDK = {
    /* rng */
    hashSeed, mulberry32, makeRng, pick, randomSeed,
    /* config */
    clamp, clampf, normalize,
    /* rubric */
    rubric: { normalize: normalizeRubric, score: scoreRun, resolve: resolveWinner },
    /* boards + meters */
    sessionBoard, meters, nowMs,
    /* engine */
    engineBridge,
    /* format */
    fmtMs, fmtPct, fmtTime, fmtNum,
    /* shell */
    shell: { h, accordion, tabs, kvRow, fullscreen, numericSeed, resetParams },
  };
})();
