/* RPSLS++ — core suite. Import order matters: the SDK first, then
   rpsls-core; read the globals.

   Spec-pinned tests (DECISIONS-SDK-PLAN §5.9): move-graph closure
   (every pair resolves), the predictor beats a fixed-cycle bot above
   chance, determinism per seed, the EV-tie path.

   Run: node --test tests/rpsls_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'rpsls-core.js'));
const RC = globalThis.RpslsCore;

const recorder = (choice, outcome = 'accept') => {
  const reqs = [];
  return {
    reqs,
    choose: (req) => {
      reqs.push(req);
      if (outcome !== 'accept') return { outcome, answers: [] };
      return {
        outcome: 'accept',
        answers: [{ question_id: 'rpsls-move', type: 'choice', choice,
          confidence: 0.7, distribution: { entries: [] } }],
      };
    },
  };
};

test('the base graph is a regular tournament: every pair resolves 2-2', () => {
  const moves = RC.baseGraph();
  assert.equal(moves.length, 5);
  const rules = RC.buildRules(moves);
  const keys = Object.keys(rules);
  assert.equal(keys.length, 10, '5 moves -> 10 unordered pairs');
  for (const k of keys) assert.ok(rules[k], k + ' resolves');
  const v = RC.validateGraph(moves);
  assert.ok(v.ok, v.reason);
  for (const m of moves) assert.equal(m.beats.length, 2, m.id + ' beats exactly 2');
  // the classic facts hold
  assert.equal(RC.result(rules, 'rock', 'scissors'), 'win');
  assert.equal(RC.result(rules, 'spock', 'rock'), 'win');
  assert.equal(RC.result(rules, 'lizard', 'spock'), 'win');
  assert.equal(RC.result(rules, 'paper', 'paper'), 'tie');
  assert.equal(RC.result(rules, 'scissors', 'rock'), 'lose');
});

test('the validator refuses broken graphs, with a reason', () => {
  assert.ok(!RC.validateGraph([
    { id: 'a', beats: ['a'] }, { id: 'b', beats: [] }]).ok, 'self-beat');
  assert.ok(!RC.validateGraph([
    { id: 'a', beats: [] }, { id: 'b', beats: ['a'] }]).ok, 'beats nothing');
  const dup = [{ id: 'x', beats: ['y'] }, { id: 'y', beats: ['x'] }, { id: 'x', beats: [] }];
  assert.ok(!RC.validateGraph(dup).ok, 'duplicate id');
  // a 4-move graph where one move is 3-0: closure holds, fairness fails
  const unfair = [
    { id: 'a', beats: ['b', 'c', 'd'] },
    { id: 'b', beats: ['c'] },
    { id: 'c', beats: ['d'] },
    { id: 'd', beats: ['b'] },
  ];
  const v = RC.validateGraph(unfair);
  assert.ok(!v.ok, 'unfair roster refused');
  assert.ok(v.reason.includes('too unfair'), v.reason);
});

test('SPEC: the move graph is extendable — a fair add keeps closure', () => {
  const g = RC.createGame({ seed: 'extend' });
  // wrong beat counts are refused with the right reason
  const bad = RC.addMove(g, { id: 'water', beats: ['rock'] });
  assert.ok(!bad.ok, 'one beat is not a fair add');
  assert.ok(bad.reason.includes('exactly 2'), bad.reason);
  // the fair add: water beats rock and paper (floor(5/2) = 2)
  const ok = RC.addMove(g, { id: 'water', label: 'Water', glyph: '💧',
    beats: ['rock', 'paper'] });
  assert.ok(ok.ok, ok.reason);
  assert.equal(g.moves.length, 6);
  const rules = RC.buildRules(g.moves);
  assert.equal(Object.keys(rules).length, 15, '6 moves -> 15 pairs');
  for (const k of Object.keys(rules)) assert.ok(rules[k], k + ' resolves');
  const v = RC.validateGraph(g.moves);
  assert.ok(v.ok, v.reason);
  // water plays: 2 wins 3 losses, inside the even-roster +-1 rule
  assert.equal(RC.result(rules, 'water', 'rock'), 'win');
  assert.equal(RC.result(rules, 'water', 'paper'), 'win');
  assert.equal(RC.result(rules, 'water', 'scissors'), 'lose');
  // a round happens on the extended graph without throwing
  RC.playRound(g, 'water', null);
  assert.equal(g.round, 1);
  assert.equal(g.histories.left[0], 'water');
  // a further add from the even roster is refused
  const even = RC.addMove(g, { id: 'fire', beats: ['rock', 'paper', 'water'] });
  assert.ok(!even.ok, 'even rosters take no add');
  assert.ok(even.reason.includes('even'), even.reason);
});

test('SPEC: the predictor beats a fixed-cycle bot well above chance', () => {
  const g = RC.createGame({ seed: 'cycle-hunter', mode: 'you' });
  const cycle = ['rock', 'scissors', 'paper'];
  for (let i = 0; i < 30; i++) RC.playRound(g, cycle[i % 3], null);
  const s = RC.runSummary(g);
  const decided = s.scores.left + s.scores.right;
  assert.ok(decided > 20, 'the cycle mostly avoids ties: ' + JSON.stringify(s.scores));
  assert.ok(s.scores.right > decided * 0.6,
    'the bot wins well above chance: ' + JSON.stringify(s.scores));
  assert.equal(s.rungs.engine, 0, 'no bridge in this run — pure rules');
});

test('the close-EV path escalates with the FULL wire contract', () => {
  const eng = recorder('rock');
  const g = RC.createGame({ seed: 'wire', mode: 'you', band: 0.99, askBudget: 5 });
  RC.playRound(g, 'rock', eng.choose);
  assert.equal(eng.reqs.length, 1, 'a tie-band call went to the engine');
  const req = eng.reqs[0];
  assert.equal(typeof req.state.text, 'string');
  assert.ok(req.state.text.includes('rpsls') || req.state.text.includes('Rock'), req.state.text);
  assert.ok(req.state.text.includes('vs'), 'the history table rides in state: ' + req.state.text.slice(0, 200));
  assert.deepEqual(req.state.facts, {}, 'facts is the tagged-enum empty object');
  assert.equal(req.questions.length, 1);
  assert.equal(req.questions[0].type, 'choice');
  assert.equal(req.questions[0].id, 'rpsls-move');
  assert.deepEqual(req.questions[0].candidates.map((c) => c.id),
    ['rock', 'paper', 'scissors', 'lizard', 'spock']);
  assert.deepEqual(req.policy,
    { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: 'low' });
  assert.equal(g.rungs.engine, 1, 'the honored answer landed on the engine rung');
  assert.equal(g.histories.right[0], 'rock');
});

test('refusal falls to the same EV argmax, on the record (no captain here)', () => {
  const mk = () => RC.createGame({ seed: 'refuse', mode: 'you', band: 0.99, askBudget: 5 });
  // abstain
  const g1 = mk();
  RC.playRound(g1, 'paper', recorder(null, 'abstain').choose);
  assert.equal(g1.rungs.rule, 1);
  assert.equal(g1.rungs.engine, 0);
  assert.ok(g1.lastDecision.why.includes('abstained'), g1.lastDecision.why);
  // garbage
  const g2 = mk();
  RC.playRound(g2, 'paper', recorder('flame').choose);
  assert.equal(g2.rungs.rule, 1);
  assert.ok(g2.lastDecision.why.includes('named nothing live'), g2.lastDecision.why);
  // engine off entirely: the argmax still decides, and says it had no lane
  const g3 = mk();
  RC.playRound(g3, 'paper', null);
  assert.equal(g3.rungs.rule, 1);
  assert.ok(g3.lastDecision.why.includes('no engine lane'), g3.lastDecision.why);
});

test('the ask budget is hard and bot-vs-bot never asks', () => {
  const eng = recorder('spock');
  const g = RC.createGame({ seed: 'budget', mode: 'you', band: 0.99, askBudget: 3 });
  for (let i = 0; i < 20; i++) RC.playRound(g, 'rock', eng.choose);
  const s = RC.runSummary(g);
  assert.equal(s.engineCalls, 3, 'exactly the budget was spent');
  assert.equal(s.tokens, 0);
  assert.equal(s.rungs.engine, 3);
  assert.equal(s.rungs.rule, 17, 'the rest is the argmax rule');
  // bot-vs-bot: both sides are rules even with a bridge in hand
  const duel = RC.createGame({ seed: 'duel', mode: 'bots', botRounds: 30, band: 0.99 });
  let guard = 0;
  while (!duel.over && guard++ < 100) RC.playRound(duel, null, eng.choose);
  const d = RC.runSummary(duel);
  assert.ok(duel.over, 'the duel finished');
  assert.equal(d.engineCalls, 0, 'watch mode asks nobody');
  assert.equal(d.rungs.rule, d.decisions);
  assert.equal(d.scores.left + d.scores.right + d.scores.ties, 30, 'the ledger closes');
});

test('determinism: bot duels replay per seed, and seeds separate', () => {
  const duel = (seed) => {
    const g = RC.createGame({ seed, mode: 'bots', botRounds: 40 });
    let guard = 0;
    while (!g.over && guard++ < 200) RC.playRound(g, null, null);
    return RC.runSummary(g);
  };
  assert.deepEqual(duel('det-a'), duel('det-a'));
  assert.notDeepEqual(duel('det-a'), duel('det-b'));
});

test('streaks and usage track the story the rounds tell', () => {
  const g = RC.createGame({ seed: 'streaks', mode: 'you' });
  // paper always beats rock: right (the no-lane bot would need to play
  // rock — force the story by scripting BOTH sides via histories is
  // out of contract, so play a run and check bookkeeping instead)
  for (let i = 0; i < 25; i++) RC.playRound(g, ['rock', 'paper', 'scissors'][i % 3], null);
  const s = RC.runSummary(g);
  const played = s.scores.left + s.scores.right + s.scores.ties;
  assert.equal(played, 25);
  const usage = Object.values(s.usage).reduce((x, y) => x + y, 0);
  assert.equal(usage, 50, 'both sides count in usage');
  if (s.streak.side) {
    assert.ok(s.streak.count >= 1 && s.streak.count <= played);
    assert.ok(s.streak.best[s.streak.side] >= s.streak.count);
  }
});

test('config and stamp: stamps separate every knob', () => {
  const base = RC.rpslsStamp(RC.normalizeConfig({ seed: 'x' }));
  assert.notEqual(base, RC.rpslsStamp(RC.normalizeConfig({ seed: 'y' })));
  assert.notEqual(base, RC.rpslsStamp(RC.normalizeConfig({ seed: 'x', mode: 'bots' })));
  assert.notEqual(base, RC.rpslsStamp(RC.normalizeConfig({ seed: 'x', botRounds: 99 })));
  assert.notEqual(base, RC.rpslsStamp(RC.normalizeConfig({ seed: 'x', askBudget: 1 })));
  assert.notEqual(base, RC.rpslsStamp(RC.normalizeConfig({ seed: 'x', band: 0.5 })));
  assert.equal(base, RC.rpslsStamp(RC.normalizeConfig({ seed: 'x' })), 'stable');
  const n = RC.normalizeConfig({ botRounds: 9999, askBudget: -5, band: 9 });
  assert.equal(n.botRounds, 500);
  assert.equal(n.askBudget, 0);
  assert.equal(n.band, 1);
});
