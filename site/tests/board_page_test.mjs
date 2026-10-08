/* Board data page smoke — runs the real site/benchmarks.js boot→render path
   against the real site/board.csv under a minimal DOM stub. No browser, no
   dependencies, no network: fetch is answered from disk, so the test fails
   whenever the generator's schema and the page's parser drift apart.

   Run: node --test site/tests/  (or `just site-test`). */

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

function el(tag) {
  return {
    tag, children: [], className: '', textContent: '', title: '', type: '',
    innerHTML: '', value: '', checked: false, scope: '', id: '',
    setAttribute(k, v) { this[k] = v; },
    getAttribute(k) { return this[k]; },
    appendChild(c) { this.children.push(c); return c; },
    addEventListener() {},
    classList: { add() {}, toggle() {}, remove() {} },
  };
}

/* every element id the page script touches */
const byId = {};
for (const id of ['thead', 'tbody', 'f-boards', 'f-quant', 'f-tier', 'f-prov',
  'f-size', 'f-search', 'f-rank', 'f-minacc', 'f-maxece', 'f-maxp50',
  'f-mintrust', 'f-cols', 'f-lambda', 'lambda-out', 'f-hasece', 'f-hassize',
  'f-reset', 'sum-count', 'sum-best', 'table-note']) {
  byId[id] = el('div');
  byId[id].id = id;
}

globalThis.document = {
  getElementById: (id) => byId[id] || null,
  createElement: el,
  createDocumentFragment: () => el('fragment'),
  createTextNode: (t) => ({ tag: '#text', textContent: String(t) }),
};
globalThis.window = { matchMedia: () => ({ matches: false }) };
globalThis.fetch = () => Promise.resolve({
  ok: true,
  status: 200,
  text: () => Promise.resolve(readFileSync(path.join(siteDir, 'board.csv'), 'utf8')),
});

await import(path.join(siteDir, 'benchmarks.js'));

/* boot is fetch-driven; give the microtask chain a beat to finish */
await new Promise((r) => setTimeout(r, 300));

function flatten(node, out = []) {
  for (const c of node.children || []) { out.push(c); flatten(c, out); }
  return out;
}
const flat = flatten(byId.tbody);
const rows = flat.filter((c) => c.tag === 'tr' && c.children.length > 1);

test('board renders every default-visible row', () => {
  /* 196 CSV rows − 7 REPORT.md tuning rows (that surface ships off by
     default) = 189; drift in either number should update this deliberately */
  assert.equal(rows.length, 189);
});

test('row IDs are unique across the whole rendered table', () => {
  const ids = rows.map((r) => r.children[0].textContent);
  assert.equal(new Set(ids).size, ids.length);
});

test('RFC 4180 quoting: names with embedded commas survive parsing', () => {
  const names = new Set(flat.map((c) => c.textContent));
  assert.ok(names.has('Engine rung (rules + lexical, JevBench public split)'));
});

test('modeled-cost surface lands with stable IDs E-01..E-04', () => {
  const ids = new Set(rows.map((r) => r.children[0].textContent));
  assert.deepEqual(['E-01', 'E-02', 'E-03', 'E-04'].filter((id) => !ids.has(id)), []);
});

test('the explainer note names the derived-score disclosures', () => {
  const note = byId['table-note'].textContent;
  for (const phrase of ['Trust %', '%/GiB', 'modeled on the benchmarkheaven CPU basis']) {
    assert.ok(note.includes(phrase), `note should mention ${phrase}`);
  }
});
