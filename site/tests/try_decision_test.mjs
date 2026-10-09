/* Try-a-Decision builder (demo.js) — the pure request-composition surface,
   pinned without a DOM. The NEGATORS mirror is diffed against the engine's
   Rust source so the page's polarity note can never silently drift.

   Run: node --test tests/try_decision_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';

const siteDir = path.dirname(fileURLToPath(import.meta.url));

/* demo.js is an IIFE that boots against a real page — evaluate the file
   with null-DOM stubs and with the trailing boot() call stripped, then lift
   TryDecision off the window object it publishes. */
const code = fs.readFileSync(path.join(siteDir, '..', 'demo.js'), 'utf8');
const unbooted = code.replace(/\n\s*boot\(\);\s*\n\}\)\(\);\s*$/, '\n})();');
if (unbooted === code) throw new Error('demo.js tail changed — update the test harness');
const stub = `
  const document = { getElementById: () => null,
    querySelectorAll: () => [], querySelector: () => null,
    createElement: () => ({ style: {}, classList: { add() {}, remove() {} },
      addEventListener() {}, appendChild() {}, querySelector() { return null; } }) };
  const window = { addEventListener() {}, location: { search: '' } };
  const performance = { now: () => 0, getEntriesByType: () => [] };
  const sessionStorage = { getItem: () => null, removeItem() {} };
  ${unbooted}
`;
const sandbox = new Function(stub + "; return window.TryDecision;");
const TD = sandbox();

test('buildRequest: boolean composes criteria negator-free and verbatim', () => {
  const req = TD.buildRequest({
    category: 'yes-no',
    state: 'replicas must be 2. prod-03 currently runs 2 replicas.',
    question: 'Is prod-03 compliant with the replica rule?',
    yesWhen: 'at least 2 replicas are running',
    noWhen: 'fewer than 2 replicas are running',
  });
  assert.equal(req.questions[0].type, 'boolean');
  assert.equal(
    req.questions[0].text,
    'Is prod-03 compliant with the replica rule?' +
    ' Criteria — satisfied when: at least 2 replicas are running.' +
    ' fail when: fewer than 2 replicas are running.',
  );
  assert.equal(TD.countNegators(req.questions[0].text), 0,
    'the shipped example must not flip the lexical polarity read');
});

test('buildRequest: the composition template itself adds no negators', () => {
  for (const [yes, no] of [['x', ''], ['', 'y'], ['x', 'y']]) {
    const text = TD.composeBooleanText('Q?', yes, no);
    const templateOnly = text.replace('x', '').replace('y', '');
    assert.equal(TD.countNegators(templateOnly), 0, JSON.stringify({ yes, no, text }));
  }
});

test('buildRequest: choice filters empty ids, keeps order', () => {
  const req = TD.buildRequest({
    category: 'choice', state: 's', question: 'q?',
    candidates: [
      { id: 'api', description: 'API tier' },
      { id: '   ', description: 'dropped' },
      { id: 'catalog', description: '' },
    ],
  });
  assert.deepEqual(req.questions[0].candidates, [
    { id: 'api', description: 'API tier' },
    { id: 'catalog', description: '' },
  ]);
});

test('buildRequest: score splits labels and drops empties', () => {
  const req = TD.buildRequest({
    category: 'score', state: 's', question: 'q?', levels: ' low ,high,,  ',
  });
  assert.deepEqual(req.questions[0].levels, [{ label: 'low' }, { label: 'high' }]);
});

test('buildRequest: unknown category refused, request carries policy + limits', () => {
  assert.equal(TD.buildRequest({ category: 'essay', state: 's', question: 'q' }), null);
  const req = TD.buildRequest({ category: 'yes-no', state: 's', question: 'q' });
  assert.equal(req.policy.risk, 'low');
  assert.ok(req.metadata.limits.max_questions >= 1);
  assert.deepEqual(req.state, { text: 's', facts: {} });
});

test('countNegators: token parity matches the engine semantics', () => {
  assert.equal(TD.countNegators('do not deploy'), 1);
  assert.equal(TD.countNegators('never not once'), 2);
  assert.equal(TD.countNegators('nobody knows'), 0, 'only whole tokens count');
  assert.equal(TD.countNegators('Cannot, nor neither.'), 3);
});

test('NEGATORS mirror matches the Rust source', () => {
  const src = fs.readFileSync(
    path.join(siteDir, '..', '..', 'crates', 'opencodifier-engine', 'src', 'lexical.rs'), 'utf8');
  const m = src.match(/pub const NEGATORS:\s*\[&str;\s*\d+\]\s*=\s*\[([^\]]+)\]/);
  assert.ok(m, 'NEGATORS constant found in lexical.rs');
  const rust = [...m[1].matchAll(/"([^"]+)"/g)].map((x) => x[1]).sort();
  assert.deepEqual([...TD.NEGATORS].sort(), rust);
});

test('parseRequest: round-trips every category the form can express', () => {
  for (const category of ['yes-no', 'choice', 'score']) {
    const built = TD.buildRequest({
      category, state: 'state text', question: 'the question?',
      yesWhen: 'yes terms', noWhen: 'no terms',
      candidates: [{ id: 'a', description: 'A' }],
      levels: 'low, high',
    });
    const back = TD.parseRequest(built);
    assert.ok(back, category);
    assert.equal(back.category, category);
    assert.equal(back.state, 'state text');
  }
});

test('parseRequest: hand-shaped JSON is refused, not half-applied', () => {
  assert.equal(TD.parseRequest(null), null);
  assert.equal(TD.parseRequest({}), null);
  assert.equal(TD.parseRequest({ state: { text: 'x' }, questions: [] }), null);
  assert.equal(TD.parseRequest({ state: { text: 'x' }, questions: [{ type: 'ranking' }] }), null);
});

test('looksLikeJson: JSON-paste detection is text-honest', () => {
  assert.equal(TD.looksLikeJson('{"a": 1}'), true);
  assert.equal(TD.looksLikeJson('[1, 2]'), true);
  assert.equal(TD.looksLikeJson('replicas must be 2.'), false);
  assert.equal(TD.looksLikeJson('{not json'), false, 'brace-shaped prose is not JSON');
});
