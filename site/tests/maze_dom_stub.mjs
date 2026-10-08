/* Shared minimal DOM stub for the maze shell suites (maze_shell_test.mjs,
   maze_deeplink_test.mjs). Importing it installs document/window globals;
   import it BEFORE maze-core.js / maze.js. Exposes the helpers the suites
   use to drive frames and read built UI. */

const rafQueue = [];
globalThis.requestAnimationFrame = (fn) => { rafQueue.push(fn); return rafQueue.length; };

function makeCtx() {
  /* every 2D call is a no-op; gradient objects need addColorStop */
  const gradient = { addColorStop() {} };
  return new Proxy({}, {
    get(_, prop) {
      if (prop === 'createRadialGradient' || prop === 'createLinearGradient') return () => gradient;
      if (prop === 'measureText') return () => ({ width: 10 });
      return () => {};
    },
    set() { return true; },
  });
}

/* input defaults the shell reads at boot (applied to the elements maze.js
   itself builds — ids set via setAttribute register + absorb these) */
const DEFAULTS = {
  'mz-seed': { value: 'shell-test' },
  'mz-size': { value: '600' },
  'mz-vision': { value: '9' },
  'mz-cells': { value: '13' },
  'mz-braid': { value: '10' },
  'mz-bot-speed': { value: '30' },
  'mz-timer': { value: '0' },
  'mz-loot-count': { value: '4' },
  'mz-chest-count': { value: '2' },
  'mz-mon-count': { value: '2' },
  'mz-mon-speed': { value: '2' },
  'mz-aggro': { value: '5' },
  'mz-wander': { value: '40' },
  'mz-loot-on': { checked: true },
  'mz-chest-on': { checked: true },
  'mz-mon-on': { checked: false },
  'mz-engine-on': { checked: false },
  'mz-audio': { checked: false },
  'mz-preset': { value: '' },
};

const registry = new Map();

/* Selector support: the shell wires up descendant selectors like
   ".mz-dpad .ctl", so the stub matches against the full path — a node
   matches the last simple selector and has ancestors matching the rest
   in order. */
function matchesSimple(node, simple) {
  if (!node) return false;
  if (simple.startsWith('#')) return node.id === simple.slice(1);
  if (simple.startsWith('.')) {
    return Boolean(node.classList) && node.classList.contains(simple.slice(1));
  }
  return node.tag === simple;
}

function matchesPath(node, sel) {
  const parts = sel.trim().split(/\s+/);
  if (!matchesSimple(node, parts[parts.length - 1])) return false;
  let up = node.parentElement;
  for (let i = parts.length - 2; i >= 0; i -= 1) {
    while (up && !matchesSimple(up, parts[i])) up = up.parentElement;
    if (!up) return false;
    up = up.parentElement;
  }
  return true;
}

function collectDescendants(node, out = []) {
  for (const c of node.children) {
    out.push(c);
    if (c.children) collectDescendants(c, out);
  }
  return out;
}

function makeElement(tag) {
  const listeners = {};
  const el = {
    tag,
    id: '',
    attrs: {},
    children: [],
    parentElement: null,
    className: '',
    /* DOM semantics: reading folds descendant text; writing replaces the
       children with one text node (maze.js does both). Backed by _text. */
    _text: '',
    value: '',
    dataset: {},
    style: {},
    checked: false,
    disabled: false,
    get firstChild() { return el.children[0] || null; },
    get lastChild() { return el.children[el.children.length - 1] || null; },
    classList: {
      add(...cls) { const s = new Set(el.className.split(' ').filter(Boolean)); for (const c of cls) s.add(c); el.className = [...s].join(' '); },
      remove(...cls) { const s = new Set(el.className.split(' ').filter(Boolean)); for (const c of cls) s.delete(c); el.className = [...s].join(' '); },
      toggle(cls, force) {
        const s = new Set(el.className.split(' ').filter(Boolean));
        const on = force !== undefined ? force : !s.has(cls);
        if (on) s.add(cls); else s.delete(cls);
        el.className = [...s].join(' ');
        return on;
      },
      contains(cls) { return el.className.split(' ').includes(cls); },
    },
    appendChild(kid) {
      kid.parentElement = el;
      el.children.push(kid);
      return kid;
    },
    insertBefore(kid, ref) {
      const i = el.children.indexOf(ref);
      if (i < 0) return el.appendChild(kid);
      el.children.splice(i, 0, kid);
      kid.parentElement = el;
      return kid;
    },
    removeChild(kid) {
      const i = el.children.indexOf(kid);
      if (i >= 0) el.children.splice(i, 1);
      return kid;
    },
    addEventListener(name, fn) { (listeners[name] = listeners[name] || []).push(fn); },
    removeEventListener() {},
    dispatch(name, ev) {
      for (const fn of listeners[name] || []) {
        fn(ev || { target: el, preventDefault() {}, stopPropagation() {} });
      }
    },
    setAttribute(k, v) {
      el.attrs[k] = String(v);
      if (k.startsWith('data-')) {
        /* the DOM maps data-x-y to dataset.xY; maze.js reads dataset.dir */
        const key = k.slice(5).replace(/-([a-z])/g, (_, ch) => ch.toUpperCase());
        el.dataset[key] = String(v);
      }
      if (k === 'id') {
        el.id = String(v);
        if (!registry.has(el.id)) {
          registry.set(el.id, el);
          if (DEFAULTS[el.id]) Object.assign(el, DEFAULTS[el.id]);
        }
      }
    },
    getAttribute(k) { return el.attrs[k] ?? null; },
    querySelector(sel) {
      return collectDescendants(el).find((c) => matchesPath(c, sel)) || null;
    },
    querySelectorAll(sel) {
      return collectDescendants(el).filter((c) => matchesPath(c, sel));
    },
  };
  if (tag === 'canvas') {
    el.getContext = () => makeCtx();
    el.clientWidth = 600;
    el.width = 600;
    el.height = 600;
  }
  Object.defineProperty(el, 'textContent', {
    get() { return (el._text || '') + el.children.map((c) => c.textContent).join(''); },
    set(v) { el.children = []; el._text = String(v); },
  });
  return el;
}

function findInTree(rootEl, id) {
  for (const c of rootEl.children) {
    if (c.id === id) return c;
    const deep = findInTree(c, id);
    if (deep) return deep;
  }
  return null;
}

const realCreate = makeElement;
globalThis.document = {
  getElementById(id) {
    if (registry.has(id)) return registry.get(id);
    const built = findInTree(globalThis.document.body, id);
    if (built) { registry.set(id, built); return built; }
    const el = realCreate('div');
    el.id = id;
    registry.set(id, el);
    return el;
  },
  createElement: realCreate,
  createTextNode: (t) => ({ tag: '#text', textContent: String(t) }),
  body: makeElement('body'),
};
/* key/input listeners the shell registers on window (keydown/keyup) */
const winListeners = {};
globalThis.window = {
  matchMedia: () => ({ matches: false }),
  addEventListener(name, fn) { (winListeners[name] = winListeners[name] || []).push(fn); },
  dispatchEvent(ev) {
    for (const fn of winListeners[ev.type] || []) fn(ev);
    return true;
  },
  devicePixelRatio: 1,
  location: { search: '' },
  AudioContext: undefined,
};
globalThis.URLSearchParams = URLSearchParams;
globalThis.location = window.location;

export function rafPending() {
  return rafQueue.length;
}

/* one module-level clock — restarting per pump would hand the loop dt=0
   (clamped) and silently freeze the sim in every later pump */
let clock = 0;
export function pumpFrames(n, dtMs = 16) {
  for (let i = 0; i < n; i++) {
    clock += dtMs;
    const batch = rafQueue.splice(0);
    for (const fn of batch) fn(clock);
  }
}

export function textOf(node) {
  let out = node.textContent || '';
  for (const c of node.children || []) out += ' ' + textOf(c);
  return out.trim();
}

export const byId = (id) => registry.get(id);
