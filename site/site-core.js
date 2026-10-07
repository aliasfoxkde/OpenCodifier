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

/* ---------- FAQ accordion: opening one closes its siblings ---------- */
document.querySelectorAll(".faq-list").forEach((list) => {
  const items = Array.from(list.querySelectorAll(":scope > details"));
  items.forEach((d) => {
    d.addEventListener("toggle", () => {
      if (!d.open) return;
      items.forEach((other) => { if (other !== d) other.open = false; });
    });
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
