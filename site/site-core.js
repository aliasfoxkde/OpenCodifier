/* Site-wide shared behaviors: theme (System/Light/Dark), back-to-top,
   FAQ accordion, reveal-on-scroll, nav scrollspy. No dependencies, no
   network. Loaded by every page; page-specific scripts come after.
   The pre-paint inline <head> script on each page owns the FIRST paint
   (sets data-theme from localStorage); this module owns everything after. */
"use strict";

document.documentElement.classList.remove("no-js");
document.documentElement.classList.add("js");

/* ---------- theme: System (default) / Light / Dark ---------- */
const THEME_KEY = "oc-theme"; // "system" | "light" | "dark"
const ORDER = ["system", "light", "dark"];
const LABEL = {
  system: "Theme: system (matches your device) — activate for light",
  light: "Theme: light — activate for dark",
  dark: "Theme: dark — activate for system",
};

const mql = window.matchMedia("(prefers-color-scheme: light)");

function storedMode() {
  try {
    const v = localStorage.getItem(THEME_KEY);
    return ORDER.includes(v) ? v : "system";
  } catch (e) {
    return "system"; // storage can throw in privacy modes; default is honest
  }
}

/* Resolve "system" against the OS preference and publish both attributes. */
function applyTheme(mode) {
  const resolved = mode === "system" ? (mql.matches ? "light" : "dark") : mode;
  document.documentElement.setAttribute("data-theme", resolved);
  document.documentElement.setAttribute("data-theme-mode", mode);
  const t = document.getElementById("theme-toggle");
  if (t) t.setAttribute("aria-label", LABEL[mode]);
  return resolved;
}

/* OS theme changes must reach us while the user is on "system". */
mql.addEventListener("change", () => {
  if (document.documentElement.getAttribute("data-theme-mode") === "system") applyTheme("system");
});

const toggle = document.getElementById("theme-toggle");
if (toggle) {
  toggle.addEventListener("click", () => {
    const next = ORDER[(ORDER.indexOf(storedMode()) + 1) % ORDER.length];
    try { localStorage.setItem(THEME_KEY, next); } catch (e) { /* non-fatal */ }
    applyTheme(next);
  });
}
applyTheme(storedMode());

/* ---------- back to top (button exists only when JS runs) ---------- */
const toTop = document.createElement("button");
toTop.type = "button";
toTop.id = "to-top";
toTop.setAttribute("aria-label", "Back to top");
toTop.innerHTML =
  '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" aria-hidden="true">' +
  '<path d="M12 19V5M5 12l7-7 7 7" stroke="currentColor" stroke-width="2.4" ' +
  'stroke-linecap="round" stroke-linejoin="round"/></svg>';
toTop.addEventListener("click", () => {
  window.scrollTo({ top: 0, behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
});
document.body.appendChild(toTop);

let toTopTick = false;
function toTopUpdate() {
  toTop.classList.toggle("show", window.scrollY > 480);
  toTopTick = false;
}
window.addEventListener("scroll", () => {
  if (!toTopTick) { toTopTick = true; requestAnimationFrame(toTopUpdate); }
}, { passive: true });
toTopUpdate();

/* ---------- accordion rule, SITE-WIDE: opening one closes the rest ---------- */
/* Not scoped to FAQs. Every <details> on every page — option panels,
   advanced settings, the nav menu — participates: an open disclosure is
   the user's focused context, and two open at once is clutter. */
const allDetails = Array.from(document.querySelectorAll("details"));
allDetails.forEach((d) => {
  d.addEventListener("toggle", () => {
    if (!d.open) return;
    allDetails.forEach((other) => { if (other !== d && other.open) other.open = false; });
  });
});

/* ---------- reveal on scroll (moved here so every page has it) ----------
   Gated on html.js in CSS: without JS everything stays visible. */
const prefersReduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
if (prefersReduced) {
  document.querySelectorAll(".reveal").forEach((el) => el.classList.add("in"));
} else {
  const io = new IntersectionObserver(
    (entries) => entries.forEach((e) => {
      if (e.isIntersecting) { e.target.classList.add("in"); io.unobserve(e.target); }
    }),
    { threshold: 0.12 }
  );
  document.querySelectorAll(".reveal").forEach((el) => io.observe(el));
  /* safety net: if the observer never fires for an element (layout quirks,
     hidden containers, missed intersections), stop hiding it — content
     must never be permanently invisible because of an animation gate */
  setTimeout(() => {
    document.querySelectorAll(".reveal:not(.in)").forEach((el) => el.classList.add("in"));
  }, 1600);
}

/* ---------- nav scrollspy (same-page anchors only) ---------- */
const navAnchors = Array.from(document.querySelectorAll(".nav-links a[href^='#']"))
  .map((a) => ({ a, section: document.getElementById(a.hash.slice(1)) }))
  .filter((x) => x.section);
if (navAnchors.length && "IntersectionObserver" in window) {
  const spy = new IntersectionObserver(
    (entries) => entries.forEach((e) => {
      if (!e.isIntersecting) return;
      navAnchors.forEach(({ a, section }) => {
        const on = section === e.target;
        a.classList.toggle("spy-active", on);
        if (on) a.setAttribute("aria-current", "true");
        else a.removeAttribute("aria-current");
      });
    }),
    { rootMargin: "-30% 0px -60% 0px" }
  );
  navAnchors.forEach(({ section }) => spy.observe(section));
}

/* ---------- background layer: pointer glow + section parallax ----------
   Both are decoration: gated off for prefers-reduced-motion, and the glow
   additionally off for coarse pointers (no cursor to follow). Writes CSS
   vars from rAF; listeners are passive. */
(function () {
  const mesh = document.querySelector(".bg-mesh");
  const fine = window.matchMedia("(pointer: fine)").matches;

  if (mesh && fine && !prefersReduced) {
    let mx = -9999, my = -9999, raf = 0;
    const paint = () => {
      raf = 0;
      mesh.style.setProperty("--mx", mx + "px");
      mesh.style.setProperty("--my", my + "px");
    };
    window.addEventListener("pointermove", (e) => {
      if (e.pointerType !== "mouse" && e.pointerType !== "pen") return;
      mx = e.clientX; my = e.clientY;
      if (!raf) raf = requestAnimationFrame(paint);
    }, { passive: true });
    document.documentElement.addEventListener("mouseleave", () => {
      mx = -9999; my = -9999;
      if (!raf) raf = requestAnimationFrame(paint);
    });
  }

  const secs = (!prefersReduced && "IntersectionObserver" in window)
    ? Array.from(document.querySelectorAll("main > section[id]")) : [];
  if (!secs.length) return;

  /* only sections near the viewport get a --par write each frame */
  const near = new Set();
  const io = new IntersectionObserver((entries) => entries.forEach((e) => {
    if (e.isIntersecting) {
      near.add(e.target);
      driftOne(e.target); /* first write immediately — don't wait for a scroll */
    } else {
      near.delete(e.target);
    }
  }), { rootMargin: "25% 0px 25% 0px" });
  secs.forEach((s) => io.observe(s));

  let ticking = false;
  const driftOne = (s) => {
    const r = s.getBoundingClientRect();
    const pf = parseFloat(getComputedStyle(s).getPropertyValue("--pf")) || 0;
    s.style.setProperty("--par", ((r.top + r.height / 2 - window.innerHeight / 2) * pf).toFixed(1));
  };
  const drift = () => {
    ticking = false;
    for (const s of near) driftOne(s);
  };
  window.addEventListener("scroll", () => {
    if (!ticking) { ticking = true; requestAnimationFrame(drift); }
  }, { passive: true });
  window.addEventListener("resize", drift, { passive: true });
  drift();
})();

/* ---------- ocLazyBoot: one-time work deferred until its element is in
   view (rootMargin starts it ~200px early). Above-fold elements run
   synchronously — nothing above the fold waits. Falls back to immediate
   execution when IntersectionObserver is missing or el is null. ---------- */
window.ocLazyBoot = function (el, fn) {
  if (!el || !("IntersectionObserver" in window)) { fn(); return; }
  const r = el.getBoundingClientRect();
  if (r.top < window.innerHeight + 200 && r.bottom > -200) { fn(); return; }
  const io = new IntersectionObserver((entries) => entries.forEach((e) => {
    if (!e.isIntersecting) return;
    io.disconnect();
    fn();
  }), { rootMargin: "200px 0px" });
  io.observe(el);
};

/* ---------- mobile: the theme toggle lives in the menu panel ---------- */
/* Same wired button node, moved between its header spot and the panel's
   foot slot at the 760px breakpoint — listeners survive the move. */
(function () {
  const toggle = document.getElementById("theme-toggle");
  const slot = document.querySelector(".nav-menu-themeslot");
  if (!toggle || !slot) return;
  const gh = document.querySelector(".nav-gh");
  const mq = window.matchMedia("(max-width: 760px)");
  const place = () => {
    if (mq.matches) {
      slot.appendChild(toggle);
      slot.removeAttribute("aria-hidden");
    } else if (gh && toggle.parentElement !== gh.parentElement) {
      gh.parentNode.insertBefore(toggle, gh);
    }
  };
  if (mq.addEventListener) mq.addEventListener("change", place);
  else if (mq.addListener) mq.addListener(place);
  place();
})();

/* ---------- game switcher: one dropdown, every game page ---------- */
/* Slugged off the URL, so each game page shows the same list with itself
   preselected; picking another name navigates there. Only game pages get
   the bar — the hub (playground.html) is its own directory. */
(function () {
  const GAMES = [
    ["antcolony", "Ant Colony"],
    ["civ60", "AI Civilization in 60 Seconds"],
    ["cube", "Cube Solver"],
    ["dilemma", "Dilemma"],
    ["door", "Door Policy"],
    ["fighter", "Fighter"],
    ["gridwars", "GridWars"],
    ["life", "Game of Life"],
    ["maze", "Maze Solver"],
    ["microworld", "MicroWorld"],
    ["pacman", "Pac-Man"],
    ["pong", "Pong"],
    ["quiz", "Quiz"],
    ["racing", "Racing"],
    ["rpsls", "RPSLS"],
    ["spacecraft", "Spacecraft"],
    ["tower", "Tower Defense"],
    ["traffic", "Traffic"],
  ];
  const page = location.pathname.replace(/.*\//, "").replace(/\.html$/, "");
  const onHub = page === "playground"; /* the hub gets a bare jump menu too */
  const isGame = GAMES.some((g) => g[0] === page);
  if (!isGame && !onHub) return;
  const main = document.querySelector("main");
  if (!main) return;

  const bar = document.createElement("div");
  bar.className = "game-switch-bar";
  const label = document.createElement("span");
  label.className = "game-switch-label";
  label.textContent = "Game";
  const sel = document.createElement("select");
  sel.className = "game-switch";
  sel.setAttribute("aria-label", "Switch game");
  if (onHub) {
    const o = document.createElement("option");
    o.value = "";
    o.textContent = "jump to a game…";
    o.selected = true;
    sel.appendChild(o);
  }
  for (const [slug, name] of GAMES) {
    const o = document.createElement("option");
    o.value = slug + ".html";
    o.textContent = name;
    if (slug === page) o.selected = true;
    sel.appendChild(o);
  }
  sel.addEventListener("change", () => {
    if (sel.value && sel.value !== page + ".html") location.href = sel.value;
  });
  if (onHub) {
    /* the hub also gets a text filter over its game cards — find, then go */
    const cards = document.getElementById("pg-cards");
    if (cards) {
      const search = document.createElement("input");
      search.type = "search";
      search.className = "game-switch-search";
      search.placeholder = "search the games…";
      search.setAttribute("aria-label", "Filter games");
      const count = document.createElement("span");
      count.className = "game-switch-count";
      const all = cards.querySelectorAll(".pg-flip");
      const sync = () => {
        const q = search.value.trim().toLowerCase();
        let shown = 0;
        for (const card of all) {
          const hay = (card.getAttribute("data-name") || "" + " ") + card.textContent;
          const hit = !q || hay.toLowerCase().includes(q);
          card.hidden = !hit;
          if (hit) shown++;
        }
        count.textContent = shown + "/" + all.length + " games";
      };
      search.addEventListener("input", sync);
      bar.appendChild(search);
      bar.appendChild(count);
      sync();
    }
  }
  bar.appendChild(label);
  bar.appendChild(sel);
  main.insertBefore(bar, main.firstChild);
})();

/* ---------- game ratings: 5 stars + feedback, local-first ---------- */
/* The site is zero-network, so ratings live in YOUR browser (localStorage)
   and stay there. The feedback box exports as a prefilled GitHub issue —
   that link is the pipe from "this game confused me" to the improvement
   backlog. Nothing is transmitted by this page itself. */
(function () {
  const NAMES = {
    antcolony: "Ant Colony", civ60: "AI Civilization in 60 Seconds",
    cube: "Cube Solver", dilemma: "Dilemma", door: "Door Policy",
    fighter: "Fighter", life: "Game of Life", maze: "Maze Solver",
    microworld: "MicroWorld", pacman: "Pac-Man", pong: "Pong", quiz: "Quiz",
    racing: "Racing", rpsls: "RPSLS", spacecraft: "Spacecraft",
    tower: "Tower Defense", traffic: "Traffic", gridwars: "GridWars",
  };
  const KEY = (slug) => "oc-rating:" + slug;
  const load = (slug) => {
    try { return JSON.parse(localStorage.getItem(KEY(slug)) || "null"); }
    catch (e) { return null; }
  };
  const save = (slug, rec) => {
    try { localStorage.setItem(KEY(slug), JSON.stringify(rec)); } catch (e) { /* private mode */ }
  };

  const page = location.pathname.replace(/.*\//, "").replace(/\.html$/, "");

  /* ---- game pages: the rating strip under the switcher ---- */
  const name = NAMES[page];
  if (name && document.querySelector(".game-switch-bar")) {
    const bar = document.querySelector(".game-switch-bar");
    const strip = document.createElement("div");
    strip.className = "rating-strip";

    const lab = document.createElement("span");
    lab.className = "rating-label";
    lab.textContent = "rate " + name;

    const stars = [];
    const starRow = document.createElement("div");
    starRow.className = "rating-stars";
    starRow.setAttribute("role", "group");
    starRow.setAttribute("aria-label", "Rate " + name + " out of five stars");
    let current = load(page);

    const paint = (n) => {
      stars.forEach((st, i) => {
        st.classList.toggle("on", i < n);
        st.setAttribute("aria-pressed", String(i + 1 === (current && current.stars)));
      });
    };
    for (let i = 1; i <= 5; i++) {
      const st = document.createElement("button");
      st.type = "button";
      st.className = "rating-star";
      st.textContent = "★";
      st.setAttribute("aria-label", i + (i === 1 ? " star" : " stars"));
      st.addEventListener("click", () => {
        current = { stars: i, feedback: (current && current.feedback) || "", ts: Date.now() };
        paint(i);
        box.hidden = false;
        note.textContent = i + "/5 — say why (optional), then save or file it.";
      });
      stars.push(st);
      starRow.appendChild(st);
    }

    const note = document.createElement("span");
    note.className = "rating-note";
    note.textContent = current ? "saved: " + current.stars + "/5 in this browser" : "unrated";

    const box = document.createElement("div");
    box.className = "rating-box";
    box.hidden = true;
    const ta = document.createElement("textarea");
    ta.className = "rating-feedback";
    ta.rows = 3;
    ta.placeholder = "what worked, what confused you, what to change…";
    ta.value = (current && current.feedback) || "";
    const saveBtn = document.createElement("button");
    saveBtn.type = "button";
    saveBtn.className = "btn btn-ghost btn-small";
    saveBtn.textContent = "save in this browser";
    saveBtn.addEventListener("click", () => {
      if (!current) return;
      current.feedback = ta.value.trim();
      current.ts = Date.now();
      save(page, current);
      note.textContent = "saved: " + current.stars + "/5 in this browser";
      saveBtn.textContent = "saved ✓";
      setTimeout(() => { saveBtn.textContent = "save in this browser"; }, 1400);
    });
    const ghBtn = document.createElement("a");
    ghBtn.className = "btn btn-ghost btn-small";
    ghBtn.target = "_blank";
    ghBtn.rel = "noopener";
    ghBtn.textContent = "file it on GitHub ↗";
    ghBtn.title = "opens a prefilled issue — the improvement pass reads these";
    const syncGh = () => {
      const st = current ? current.stars : 0;
      ghBtn.href = "https://github.com/aliasfoxkde/OpenCodifier/issues/new?title=" +
        encodeURIComponent("[" + name + "] " + (st ? st + "\u2605 " : "") + "game feedback") +
        "&body=" + encodeURIComponent(
          (st ? "Rating: " + st + "/5\n" : "") +
          (ta.value.trim() ? "\n" + ta.value.trim() + "\n" : "") +
          "\n\u2014 sent from " + page + ".html on the OpenCodifier site");
    };
    ta.addEventListener("input", syncGh);
    box.appendChild(ta);
    const row = document.createElement("div");
    row.className = "rating-actions";
    row.appendChild(saveBtn);
    row.appendChild(ghBtn);
    box.appendChild(row);

    strip.appendChild(lab);
    strip.appendChild(starRow);
    strip.appendChild(note);
    strip.appendChild(box);
    if (current) { paint(current.stars); }
    bar.appendChild(strip);
  }

  /* ---- the hub: badges + sort/filter over the game cards ---- */
  if (page === "playground" && document.getElementById("pg-cards")) {
    const cards = Array.from(document.querySelectorAll("#pg-cards .pg-flip"));
    /* slug = first token of data-name */
    const slugOf = (card) => (card.getAttribute("data-name") || "").split(/\s+/)[0];
    for (const card of cards) {
      const rec = load(slugOf(card));
      if (!rec || !rec.stars) continue;
      const front = card.querySelector(".pg-front");
      if (!front) continue;
      const badge = document.createElement("span");
      badge.className = "pg-rating-badge";
      badge.textContent = "your rating: " + "\u2605".repeat(rec.stars) + "\u2606".repeat(5 - rec.stars);
      front.appendChild(badge);
    }

    const bar = document.querySelector(".game-switch-bar");
    if (bar) {
      const wrap = document.createElement("div");
      wrap.className = "rating-hub";
      const sel = document.createElement("select");
      sel.className = "game-switch";
      sel.setAttribute("aria-label", "Sort or filter games by your rating");
      for (const [v, t] of [
        ["default", "sort: site default"],
        ["rated", "filter: rated only"],
        ["3plus", "filter: \u2605\u2605\u2605 and up"],
        ["4plus", "filter: \u2605\u2605\u2605\u2605 and up"],
        ["5", "filter: \u2605\u2605\u2605\u2605\u2605"],
        ["best", "sort: your best first"],
      ]) {
        const o = document.createElement("option");
        o.value = v;
        o.textContent = t;
        sel.appendChild(o);
      }
      const original = cards.slice();
      const stars = (card) => { const r = load(slugOf(card)); return r && r.stars ? r.stars : 0; };
      sel.addEventListener("change", () => {
        const grid = document.getElementById("pg-cards");
        let list = original.slice();
        const v = sel.value;
        if (v === "rated") list = list.filter((c) => stars(c) > 0);
        else if (v === "3plus") list = list.filter((c) => stars(c) >= 3);
        else if (v === "4plus") list = list.filter((c) => stars(c) >= 4);
        else if (v === "5") list = list.filter((c) => stars(c) === 5);
        else if (v === "best") list.sort((a, b) => stars(b) - stars(a));
        for (const c of list) grid.appendChild(c);
        /* anything filtered out must not linger hidden by the text search */
        for (const c of original) if (!list.includes(c)) c.hidden = true;
      });
      wrap.appendChild(sel);
      bar.appendChild(wrap);
    }
  }
})();
