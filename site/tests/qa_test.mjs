/* QA tool core suite (#196). Import order matters: the SDK first, then
   qa-core; read the globals.

   Run: node --test tests/qa_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'qa-core.js'));
const QA = globalThis.QaCore;

test('built-in rules pass their own validation', () => {
  for (const rule of QA.BUILT_IN_RULES) {
    assert.equal(QA.validateRule(rule), null, rule.keyword);
  }
});

test('validateRule names every way an editor can be wrong', () => {
  const cases = [
    [null, /must be an object/],
    [{ keyword: '', factKey: 'a', factValue: 'b', answer: 'yes' }, /keyword is empty/],
    [{ keyword: 'x', factKey: '', factValue: 'b', answer: 'yes' }, /fact key is empty/],
    [{ keyword: 'x', factKey: 'a', factValue: '', answer: 'yes' }, /fact value is empty/],
    [{ keyword: 'x', factKey: 'a', factValue: 'b', answer: 'maybe' }, /yes or no/],
    [{ keyword: 'x', factKey: 'a', factValue: 'b' }, /yes or no/],
  ];
  for (const [rule, re] of cases) {
    assert.ok(re.test(QA.validateRule(rule)), String(re));
  }
});

test('normalizeRules keeps good rules and refuses bad ones with reasons', () => {
  const r = QA.normalizeRules([
    { keyword: ' sky ', factKey: 'sky_color', factValue: ' blue ', answer: 'yes' },
    { keyword: '', factKey: 'a', factValue: 'b', answer: 'no' },
    'not even an object',
  ]);
  assert.equal(r.ok, false);
  assert.equal(r.rules.length, 1);
  assert.equal(r.rules[0].keyword, 'sky');
  assert.equal(r.rules[0].factValue, 'blue');
  assert.equal(r.errors.length, 2);
  assert.ok(r.errors[1].includes('rule 3'));
});

test('matchingRules fires on word boundaries and respects case', () => {
  const rules = QA.normalizeRules([
    { keyword: 'sky', factKey: 'sky_color', factValue: 'blue', answer: 'yes' },
    { keyword: 'deploy', factKey: 'deploy_policy', factValue: 'needs approval', answer: 'no' },
  ]).rules;
  assert.equal(QA.matchingRules('Is the SKY blue?', rules).length, 1);
  assert.equal(QA.matchingRules('should I deploy today', rules).length, 1);
  assert.equal(QA.matchingRules('is the skyscraper tall', rules).length, 0, 'no substring hits');
  assert.equal(QA.matchingRules('nothing relevant here', rules).length, 0);
});

test('composeRequest grounds the state with firing facts and keeps facts empty', () => {
  const rules = QA.normalizeRules(QA.BUILT_IN_RULES).rules;
  const { request, fired } = QA.composeRequest('is the sky blue', '', rules);
  assert.equal(fired.length, 1);
  assert.ok(request.state.text.includes('blue'), 'the fact value travels in the text');
  assert.ok(!request.state.text.includes('is the sky blue'),
    'the question must NOT travel in the state — lexical self-match inflates confidence');
  assert.deepEqual(request.state.facts, {}, 'facts stay empty (tagged-enum contract)');
  assert.equal(request.questions[0].type, 'boolean');
  assert.equal(request.questions[0].id, 'qa');
  assert.equal(request.policy.min_confidence, QA.POLICY.min_confidence);
});

test('a bare question with no rules produces a fact-free state', () => {
  const { request, fired } = QA.composeRequest('should I sell my house tomorrow', '', []);
  assert.equal(fired.length, 0);
  assert.equal(request.state.text, '', 'empty state = nothing for the lexical rung to agree with');
});

test('composeRequest carries the context text when given', () => {
  const rules = QA.normalizeRules([]).rules;
  const { request } = QA.composeRequest('ship it?', 'the working tree is green', rules);
  assert.ok(request.state.text.includes('the working tree is green'));
});

test('classify maps the engine outcomes onto the three honest states', () => {
  assert.equal(QA.classify({ outcome: 'accept', confidence: { calibrated_confidence: 0.9 } }), 'decided');
  assert.equal(QA.classify({ outcome: 'abstain', confidence: { calibrated_confidence: 0.1 } }), 'abstained');
  assert.equal(QA.classify({ outcome: 'verify', confidence: { calibrated_confidence: 0.4 } }), 'escalated');
  assert.equal(QA.classify({ outcome: 'escalate', confidence: { calibrated_confidence: 0.4 } }), 'escalated');
  assert.equal(QA.classify(null), 'unavailable');
  assert.equal(QA.classify({}), 'unavailable');
});

test('the escalation payload names the gate, the confidence, and the request', () => {
  const rules = QA.normalizeRules(QA.BUILT_IN_RULES).rules;
  const response = { outcome: 'verify', confidence: { calibrated_confidence: 0.4 } };
  const payload = QA.escalationPayload('is the sky blue', '', rules, response);
  assert.ok(payload.reason.includes('0.4'), payload.reason);
  assert.ok(payload.reason.includes(String(QA.ESCALATE_BELOW)));
  assert.equal(payload.request.questions[0].id, 'qa');
  assert.deepEqual(payload.confidence, response.confidence);
});
