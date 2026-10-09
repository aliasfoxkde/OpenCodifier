/* Ask the Engine — core suite. Import order matters: the SDK first, then
   quiz-core; read the globals.

   Run: node --test tests/quiz_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'quiz-core.js'));
const QC = globalThis.QuizCore;

const ENGINE_OK = (pickIdx, conf) => () => ({
  outcome: 'accept',
  answers: [{ question_id: 'quiz-q1', type: 'choice', choice: 'option-' + pickIdx,
    confidence: conf, distribution: { entries: [] } }],
});

const goodDeck = () => QC.normalizeDeck({
  name: 'unit',
  questions: [
    { text: 'pick A', options: ['A', 'B'], key: 0 },
    { text: 'pick B', options: ['A', 'B', 'C'], key: 2 },
  ],
}).deck;

test('deck validation names every way an editor can be wrong', () => {
  assert.ok(QC.validateDeck(goodDeck()).length === 0, 'the good deck is clean');
  const cases = [
    [null, /must be an object/],
    [[], /must be an object/],
    [{ name: 'x', questions: [] }, /non-empty array/],
    [{ name: '', questions: [{ text: 't', options: ['a', 'b'], key: 0 }] }, /name/],
    [{ name: 'x', questions: [{ text: '  ', options: ['a', 'b'], key: 0 }] }, /text is empty/],
    [{ name: 'x', questions: [{ text: 't', options: ['a'], key: 0 }] }, /at least 2/],
    [{ name: 'x', questions: [{ text: 't', options: ['a', 'b', 'c', 'd', 'e', 'f', 'g'], key: 0 }] }, /at most 6/],
    [{ name: 'x', questions: [{ text: 't', options: ['a', ''], key: 0 }] }, /non-empty text/],
    [{ name: 'x', questions: [{ text: 't', options: ['a', 'b'], key: '0' }] }, /integer/],
    [{ name: 'x', questions: [{ text: 't', options: ['a', 'b'], key: 2 }] }, /out of range/],
    [{ name: 'x', questions: [{ text: 't', options: ['a', 'b'] }] }, /integer/],
  ];
  for (const [deck, re] of cases) {
    assert.ok(QC.validateDeck(deck).some((e) => re.test(e)), String(re));
  }
});

test('normalizeDeck trims, assigns stable ids, and rejects with reasons', () => {
  const bad = QC.normalizeDeck({ name: 'x', questions: [{ text: 't', options: ['a'], key: 0 }] });
  assert.equal(bad.ok, false);
  assert.ok(bad.errors.length >= 1);
  const good = QC.normalizeDeck({
    name: ' round-trip ',
    questions: [{ text: '  what?  ', options: [' a ', 'b'], key: 1 }],
  });
  assert.equal(good.ok, true);
  assert.equal(good.deck.name, 'round-trip');
  assert.equal(good.deck.questions[0].id, 'q1');
  assert.deepEqual(good.deck.questions[0].options, ['a', 'b']);
});

test('the built-in deck passes its own validation and is sized for the demo', () => {
  const round = QC.normalizeDeck(QC.BUILT_IN_DECK);
  assert.deepEqual(round.errors, [], 'built-in deck is valid as shipped');
  assert.equal(round.deck.name, QC.BUILT_IN_DECK.name);
  assert.ok(QC.BUILT_IN_DECK.questions.length >= 10, 'enough questions for a run');
  assert.ok(QC.BUILT_IN_DECK.questions.every((q) => q.options.length >= 2 && q.options.length <= 6));
  assert.ok(QC.BUILT_IN_DECK.questions.every((q) => q.text && q.options[q.key]));
});

test('question order is seeded: same seed replays, a new seed reshuffles', () => {
  const d = QC.BUILT_IN_DECK;
  const a = QC.makeQuiz(d, { seed: 'one' });
  const b = QC.makeQuiz(d, { seed: 'one' });
  const c = QC.makeQuiz(d, { seed: 'two' });
  assert.deepEqual(a.order, b.order);
  assert.notDeepEqual(a.order, c.order);
  assert.equal(a.order.length, d.questions.length);
  assert.deepEqual([...a.order].sort((x, y) => x - y), d.questions.map((_, i) => i), 'a permutation');
});

test('every engine request carries the FULL wire contract (the racer rule)', () => {
  const q = goodDeck().questions[0];
  const req = QC.buildRequest(q);
  assert.equal(typeof req.state.text, 'string');
  assert.ok(req.state.text.includes(q.text), 'the question text is in the state');
  assert.deepEqual(req.state.facts, {}, 'facts is the tagged-enum empty object');
  assert.equal(req.questions.length, 1);
  assert.equal(req.questions[0].type, 'choice');
  assert.equal(req.questions[0].id, 'quiz-' + q.id);
  assert.equal(req.questions[0].text, q.text);
  assert.deepEqual(req.questions[0].candidates,
    q.options.map((o, i) => ({ id: 'option-' + i, description: o })));
  assert.equal(req.questions[0].candidates.length, q.options.length);
  assert.deepEqual(req.policy, QC.POLICY);
  assert.equal(req.policy.risk, 'low');
});

test('grading: right, wrong, abstain, and bridge-dead are distinct outcomes', () => {
  const q = goodDeck().questions[1];               /* key: 2 */
  const right = QC.grade(ENGINE_OK(2, 0.7)(), q);
  assert.deepEqual([right.pick, right.correct, right.outcome], [2, true, 'accept']);
  assert.equal(right.confidence, 0.7);
  const wrong = QC.grade(ENGINE_OK(0, 0.6)(), q);
  assert.deepEqual([wrong.pick, wrong.correct, wrong.outcome], [0, false, 'accept']);
  const abstain = QC.grade({ outcome: 'abstain', answers: [] }, q);
  assert.equal(abstain.outcome, 'abstain');
  assert.equal(abstain.correct, null, 'an abstention is never scored wrong');
  const dead = QC.grade(null, q);
  assert.equal(dead.outcome, 'unavailable');
  assert.equal(dead.correct, null);
  const bogus = QC.grade({ outcome: 'accept', answers: [{ choice: 'option-9' }] }, q);
  assert.equal(bogus.outcome, 'unavailable', 'a non-candidate choice is not a pick');
});

test('askEngine makes exactly one bridge call per question and records it', () => {
  const qz = QC.makeQuiz(goodDeck(), { seed: 'calls' });
  let calls = 0;
  const bridge = { choose: () => { calls += 1; return ENGINE_OK(0, 0.66)(); } };
  const row = QC.askEngine(qz, bridge);
  assert.equal(calls, 1);
  assert.equal(row.who, 'engine');
  assert.equal(row.qid, QC.current(qz).id);
  assert.equal(typeof row.ms, 'number');
  assert.equal(qz.results.length, 1);
  QC.next(qz);
  QC.askEngine(qz, bridge);
  assert.equal(calls, 2);
});

test('a dead bridge records unavailable without throwing, and the run finishes', () => {
  const qz = QC.makeQuiz(goodDeck(), { seed: 'dead' });
  while (!QC.finished(qz)) {
    const row = QC.askEngine(qz, null);            /* no engine at all */
    assert.equal(row.outcome, 'unavailable');
    QC.next(qz);
  }
  const s = QC.scoreSummary(qz, 'engine');
  assert.equal(s.asked, 2);
  assert.equal(s.answered, 0);
  assert.equal(s.abstained, 2);
  assert.equal(s.accuracy, null, 'no answers, no accuracy — not a zero');
});

test('scoring: accuracy over answered, abstain rate, streaks, median time', () => {
  const qz = QC.makeQuiz(goodDeck(), { seed: 'score' });
  /* hand-built result sequence: right, wrong, abstain, right, right */
  const script = [
    { pick: 0, outcome: 'accept' }, { pick: 1, outcome: 'accept' },
    { pick: null, outcome: 'abstain' }, { pick: 0, outcome: 'accept' },
    { pick: 0, outcome: 'accept' },
  ];
  const questions = [goodDeck().questions[0], goodDeck().questions[1], goodDeck().questions[0],
    goodDeck().questions[0], goodDeck().questions[0]];
  let i = 0;
  for (const sc of script) {
    const graded = sc.pick === null
      ? { pick: null, correct: null, confidence: null, outcome: 'abstain', raw: null }
      : { pick: sc.pick, correct: sc.pick === questions[i].key, confidence: 0.6,
          outcome: 'accept', raw: null };
    QC.record(qz, questions[i], 'engine', graded, 10 + i * 10);
    i++;
  }
  const s = QC.scoreSummary(qz, 'engine');
  assert.equal(s.asked, 5);
  assert.equal(s.answered, 4);
  assert.equal(s.correct, 3);
  assert.equal(s.wrong, 1);
  assert.equal(s.abstained, 1);
  assert.equal(s.accuracy, 0.75, 'accuracy is over ANSWERED');
  assert.equal(s.abstainRate, 0.2);
  assert.equal(s.bestStreak, 2, 'streak resets on a wrong, ignores abstains');
  assert.equal(s.medianMs, 30);
});

test('median handles even counts and non-numeric noise', () => {
  assert.equal(QC.median([30, 10, 20]), 20);
  assert.equal(QC.median([40, 10, 30, 20]), 25);
  assert.equal(QC.median([5, null, 'x']), 5);
  assert.equal(QC.median([]), null);
});

test('compete: human and engine sessions are independent rows on the same deck', () => {
  const deck = goodDeck();
  const you = QC.makeQuiz(deck, { seed: 'vs', mode: 'compete' });
  const engine = QC.makeQuiz(deck, { seed: 'vs', mode: 'compete' });
  assert.deepEqual(you.order, engine.order, 'same seed: same question order');
  QC.answerHuman(you, deck.questions[you.order[0]].key, 1234);
  QC.askEngine(engine, { choose: () => ({ outcome: 'abstain', answers: [] }) });
  QC.next(you); QC.next(engine);
  const sy = QC.scoreSummary(you, 'you');
  const se = QC.scoreSummary(engine, 'engine');
  assert.equal(sy.correct, 1);
  assert.equal(sy.medianMs, 1234);
  assert.equal(se.abstained, 1);
  assert.equal(QC.scoreSummary(you, 'engine').asked, 0, 'no cross-contamination');
});

test('audit verdicts score the human judgment against the keyed truth', () => {
  const qz = QC.makeQuiz(goodDeck(), { seed: 'audit' });
  assert.equal(QC.verdictAudit(qz, true), null, 'no engine answer yet, no verdict');
  const abstainBridge = { choose: () => ({ outcome: 'abstain', answers: [] }) };
  QC.askEngine(qz, abstainBridge);
  assert.equal(QC.verdictAudit(qz, true), null, 'still nothing to judge');
  QC.askEngine(qz, { choose: ENGINE_OK(0, 0.8) });  /* deck q1 key is 0 → right */
  const v = QC.verdictAudit(qz, true);
  assert.equal(v.who, 'audit');
  assert.equal(v.engineCorrect, true);
  assert.equal(v.correct, true, 'agreeing with a right answer is a correct judgment');
  const v2 = QC.verdictAudit(qz, false);
  assert.equal(v2.correct, false, 'disagreeing with a right answer is a wrong judgment');
});

test('deck JSON round-trips and revalidates; garbage fails with a reason', () => {
  const deck = goodDeck();
  const text = QC.deckToJSON(deck);
  const back = QC.deckFromJSON(text);
  assert.equal(back.ok, true);
  assert.deepEqual(back.deck.questions, deck.questions);
  assert.equal(QC.deckFromJSON('{nope').ok, false);
  assert.ok(QC.deckFromJSON('{"name":"x","questions":[]}').errors.length > 0);
});

test('deck save is opt-in and load-verifies; a broken saved deck loads as null', () => {
  const backing = new Map();
  const storage = { setItem: (k, v) => backing.set(k, v), getItem: (k) => backing.get(k) };
  assert.equal(QC.quizLoadDeck(storage), null, 'nothing saved yet');
  const deck = goodDeck();
  assert.equal(QC.quizSaveDeck(deck, storage), true);
  const loaded = QC.quizLoadDeck(storage);
  assert.deepEqual(loaded, deck);
  backing.set(QC.DECK_KEY, '{"name":"broken"}');
  assert.equal(QC.quizLoadDeck(storage), null, 'an invalid saved deck never loads');
  backing.set(QC.DECK_KEY, '{bad json');
  assert.equal(QC.quizLoadDeck(storage), null);
  assert.equal(QC.quizSaveDeck(deck, null), false, 'no storage: save is a no-op, not a throw');
});

test('config normalization clamps the mode and stringifies the seed', () => {
  assert.deepEqual(QC.normalizeConfig({ mode: 'compete', seed: 7 }), { seed: '7', mode: 'compete' });
  assert.deepEqual(QC.normalizeConfig({ mode: 'tournament' }), { seed: 'oc-quiz', mode: 'speed' });
  assert.deepEqual(QC.normalizeConfig(null), { seed: 'oc-quiz', mode: 'speed' });
});

test('a full engine session walks every question exactly once', () => {
  const deck = QC.BUILT_IN_DECK;
  const qz = QC.makeQuiz(deck, { seed: 'full' });
  const seen = [];
  while (!QC.finished(qz)) {
    const row = QC.askEngine(qz, { choose: ENGINE_OK(0, 0.55) });
    seen.push(row.qid);
    QC.next(qz);
  }
  assert.equal(seen.length, deck.questions.length);
  assert.deepEqual([...new Set(seen)].length, seen.length, 'no repeats');
  const s = QC.scoreSummary(qz, 'engine');
  assert.equal(s.asked, deck.questions.length);
  assert.ok(s.answered >= 1, 'the engine answered something');
  assert.equal(s.medianMs !== null, true);
});
