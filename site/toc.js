/* toc.js — the on-this-page menu for every site page.
   Builds a fixed left rail from the page's real sections (main section[id]
   with a heading): always-visible labels, scrollspy highlight, an overall
   progress hairline, and the quick links at the bottom. The header nav
   stays page-only; everything in-page lives here. The rail is a guest in
   the left margin, not a layout column: the page stays centered in the
   full viewport and the rail hides itself (`.noroom`) whenever the
   centered content leaves less than its footprint of margin, so it can
   never sit on top of the page. Reduced-motion users get the rail too —
   it is navigation. Loaded by every page right after site-core.js. */

(() => {
  "use strict";

  const QUICK_LINKS = [
    { href: "https://github.com/aliasfoxkde/OpenCodifier", label: "GitHub" },
    { href: "https://github.com/aliasfoxkde/OpenCodifier/issues", label: "Issues" },
    { href: "https://github.com/aliasfoxkde/OpenCodifier/discussions", label: "Discuss" },
    { href: "decide.html", label: "Decide console" },
    { href: "api.html", label: "API docs" },
    { href: "https://huggingface.co/TaskWizerAI", label: "Hugging Face" },
  ];

  function sections() {
    const main = document.querySelector("main");
    if (!main) return [];
    const out = [];
    for (const sec of main.querySelectorAll("section[id]")) {
      const head = sec.querySelector("h2");
      if (!head) continue;
      /* label priority: explicit override > the section's kicker line
         (short by design) > the heading itself (often a full sentence) */
      const kicker = sec.querySelector(".kicker");
      let label = sec.getAttribute("data-toc-label")
        || (kicker && kicker.textContent.trim())
        || head.textContent.trim();
      if (label.length > 24) label = label.slice(0, 24).trimEnd() + "…";
      out.push({ id: sec.id, label });
    }
    return out;
  }

  function build() {
    const secs = sections();
    if (secs.length < 2) return;

    const rail = document.createElement("nav");
    rail.className = "toc-rail";
    rail.id = "toc-rail";
    rail.setAttribute("aria-label", "On this page");

    const head = document.createElement("div");
    head.className = "toc-head";
    head.textContent = "On this page";

    /* filter: only worth a box when the page carries enough sections that
       scanning beats scrolling (the games hub is the driver here) */
    let filterInput = null;
    if (secs.length >= 8) {
      filterInput = document.createElement("input");
      filterInput.type = "search";
      filterInput.className = "toc-filter";
      filterInput.placeholder = "Filter…";
      filterInput.setAttribute("aria-label", "Filter sections");
      filterInput.addEventListener("input", () => {
        const q = filterInput.value.trim().toLowerCase();
        for (const a of dots) {
          const hit = !q || a.textContent.toLowerCase().includes(q);
          a.parentElement.hidden = !hit;
        }
        for (const a of panelLinks) {
          const hit = !q || a.textContent.toLowerCase().includes(q);
          a.hidden = !hit;
        }
      });
    }

    const track = document.createElement("div");
    track.className = "toc-progress";
    track.setAttribute("aria-hidden", "true");
    const fill = document.createElement("div");
    fill.className = "toc-fill";
    track.appendChild(fill);

    const list = document.createElement("ul");
    list.className = "toc-list";
    const dots = [];
    for (const sec of secs) {
      const li = document.createElement("li");
      const a = document.createElement("a");
      a.href = "#" + sec.id;
      a.setAttribute("data-toc-target", sec.id);
      const dot = document.createElement("span");
      dot.className = "toc-dot";
      const label = document.createElement("span");
      label.className = "toc-label";
      label.textContent = sec.label;
      a.appendChild(dot);
      a.appendChild(label);
      li.appendChild(a);
      list.appendChild(li);
      dots.push(a);
    }

    const quick = document.createElement("div");
    quick.className = "toc-quick";
    for (const q of QUICK_LINKS) {
      const a = document.createElement("a");
      a.href = q.href;
      a.rel = "noopener";
      a.textContent = q.label;
      a.setAttribute("aria-label", q.label);
      a.target = q.href.startsWith("http") ? "_blank" : "_self";
      quick.appendChild(a);
    }

    rail.appendChild(head);
    if (filterInput) rail.appendChild(filterInput);
    rail.appendChild(track);
    rail.appendChild(list);
    rail.appendChild(quick);
    document.body.appendChild(rail);

    /* the rail hides below 1200px — mirror the sections into the mobile
       menu panel so in-page navigation survives on phones */
    const panel = document.querySelector(".nav-menu-panel");
    const panelLinks = [];
    if (panel) {
      const wrap = document.createElement("div");
      wrap.className = "nav-menu-toc";
      const phead = document.createElement("div");
      phead.className = "nav-menu-toc-head";
      phead.textContent = "On this page";
      wrap.appendChild(phead);
      for (const sec of secs) {
        const a = document.createElement("a");
        a.href = "#" + sec.id;
        a.setAttribute("data-toc-panel", sec.id);
        a.textContent = sec.label;
        wrap.appendChild(a);
        panelLinks.push(a);
      }
      panel.appendChild(wrap);
    }

    /* room gate: the rail hangs LEFTWARD off the content column's left edge
       (CSS: right edge = content edge - 1.25rem gap), so the margin must fit
       the fitted box itself plus a little air. Measured live — the box is
       shrink-to-fit, not a fixed width. */
    const RAIL_GAP = 20; /* keep in sync with the 1.25rem gap in styles.css */
    function hasRoom() {
      let left = Infinity;
      for (const sec of secs) {
        const el = document.getElementById(sec.id);
        if (el) left = Math.min(left, el.getBoundingClientRect().left);
      }
      return left >= RAIL_GAP + rail.offsetWidth + 12;
    }

    /* scrollspy: the section whose top most recently crossed the viewport's
       upper third is current; the fill shows overall scroll progress */
    function update() {
      const room = hasRoom();
      if (rail.classList.contains("noroom") === room) rail.classList.toggle("noroom", !room);
      if (!room) return;
      const doc = document.documentElement;
      const span = doc.scrollHeight - window.innerHeight;
      const frac = span > 0 ? Math.min(1, Math.max(0, window.scrollY / span)) : 0;
      fill.style.width = (frac * 100).toFixed(1) + "%";
      let current = secs[0].id;
      for (const sec of secs) {
        const el = document.getElementById(sec.id);
        if (el && el.getBoundingClientRect().top <= window.innerHeight * 0.4) {
          current = sec.id;
        }
      }
      for (const a of dots) {
        const on = a.getAttribute("data-toc-target") === current;
        if (a.classList.contains("on") !== on) a.classList.toggle("on", on);
      }
      for (const a of panelLinks) {
        const on = a.getAttribute("data-toc-panel") === current;
        if (a.classList.contains("on") !== on) a.classList.toggle("on", on);
      }
    }
    let ticking = false;
    window.addEventListener("scroll", () => {
      if (ticking) return;
      ticking = true;
      requestAnimationFrame(() => { update(); ticking = false; });
    }, { passive: true });
    window.addEventListener("resize", update);
    update();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", build);
  } else {
    build();
  }
})();
