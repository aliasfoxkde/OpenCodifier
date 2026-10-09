/* The Prisoner's Dilemma — core suite. Import order matters: the SDK
   first, then dilemma-core; read the globals.

   Spec-pinned tests (DECISIONS-SDK-PLAN §5.10): classic results
   reproduce (defect-vs-always-cooperate exploits; tit-for-tat ties
   itself at full cooperation), matrix edits change outcomes, the engine
   agent stays within the roster contract.

   Run: node --test tests/dilemma_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'dilemma-core.js'));
const DC = globalThis.DilemmaCore;

const recorder = (choice, outcome = 'accept') => {
  const reqs = [];
  return {
    reqs,
    choose: (req) => {
      reqs.push(req);
      if (outcome !== 'accept') return { outcome, answers: [] };
      return {
        outcome: 'accept',
        answers: [{ question_id: 'pd-move', type: 'choice', choice,
          confidence: 0.7, distribution: { entries: [] } }],
      };
    },
  };
};

const drive = (cfg, engineChoose) => {
  const g = DC.createGame(cfg);
  while (!g.over) DC.tickGame(g, engineChoose);
  return g;
};

test('config normalization clamps, keeps zeros, pins the defaults', () => {
  assert.deepEqual(DC.normalizeConfig(null).matrix, DC.DEFAULT_MATRIX);
  const x = DC.normalizeConfig({ rounds: 9999, noise: 3, askBudget: 99,
    left: 'nonsense', right: 'engine', matrix: { cc: 99, cd: -99 } });
  assert.equal(x.rounds, 500);
  assert.equal(x.noise, 0.5);
  assert.equal(x.askBudget, 10);
  assert.notEqual(x.left, 'nonsense');
  assert.equal(x.right, 'engine');
  assert.equal(x.matrix.cc, 9);
  assert.equal(x.matrix.cd, -9);
  assert.equal(x.matrix.dc, DC.DEFAULT_MATRIX.dc, 'untouched cells keep defaults');
  const z = DC.normalizeConfig({ noise: 0, askBudget: 0 });
  assert.equal(z.noise, 0);
  assert.equal(z.askBudget, 0);
});

test('SPEC: defection exploits unconditional cooperation (classic T/R ratio)', () => {
  const g = drive({ seed: 'exploit', rounds: 100, left: 'suspicious', right: 'trusting' });
  const s = DC.runSummary(g);
  assert.equal(s.scores.left, 500, 'suspicious takes T=5 every round');
  assert.equal(s.scores.right, 0, 'trusting eats S=0 every round');
  assert.equal(s.tally.left.dc, 100, 'every round is defect-vs-cooperate');
  assert.equal(s.coopRate.left, 0);
  assert.equal(s.coopRate.right, 1);
});

test('SPEC: tit-for-tat vs tit-for-tat ties at full cooperation', () => {
  const g = drive({ seed: 'tft', rounds: 100, left: 'titfortat', right: 'titfortat' });
  const s = DC.runSummary(g);
  assert.equal(s.tally.left.cc, 100, 'every round is mutual cooperation');
  assert.equal(s.scores.left, 300);
  assert.equal(s.scores.right, 300);
  assert.equal(s.coopRate.left, 1);
  assert.equal(s.coopRate.right, 1);
});

test('grim trigger forgives nothing: one defection ends cooperation forever', () => {
  const g = DC.createGame({ seed: 'grim', rounds: 60, left: 'grim', right: 'random' });
  // rig the coin so right defects on round 3, then cooperates forever
  let calls = 0;
  const orig = g.agents.right.rng;
  g.agents.right.rng = { next: null };
  g.agents.right.rng = () => {
    calls += 1;
    return (calls === 3 || calls === 4) ? 0.9 : 0.1;   /* 3rd random move defects */
  };
  void orig;
  while (!g.over) DC.tickGame(g, null);
  const defections = g.agents.right.mine.filter((m) => m === 'defect').length;
  assert.ok(defections >= 1, 'the coin produced a defection');
  // grim cooperated until the first seen defection, then defected to the end
  const seen = g.agents.left.seen;
  const firstD = seen.indexOf('defect');
  assert.ok(firstD >= 0, 'grim saw a defection');
  assert.ok(g.agents.left.mine.slice(0, firstD + 1).every((m) => m === 'cooperate'),
    'cooperated up to and through the trigger round (moves are simultaneous)');
  assert.ok(g.agents.left.mine.slice(firstD + 1).every((m) => m === 'defect'),
    'defected forever after');
});

test('forgiving (tit-for-two-tats) tolerates a lone defection, answers doubles', () => {
  const mk = (seenPrefix, last) => {
    const g = DC.createGame({ seed: 'forgive', rounds: 1, left: 'forgiving', right: 'trusting' });
    g.agents.left.seen = seenPrefix.concat([last]);
    return DC.decide(g, 'left', null);
  };
  assert.equal(mk([], 'defect'), 'cooperate', 'one defection alone is tolerated');
  assert.equal(mk(['defect'], 'defect'), 'defect', 'two in a row trigger');
  assert.equal(mk(['defect', 'cooperate'], 'defect'), 'cooperate', 'a make-up move resets');
});

test('pavlov stays on a win and shifts on a loss (matrix-aware)', () => {
  const mk = (mine, lastPayoff, R) => {
    const g = DC.createGame({ seed: 'pavlov', rounds: 1, left: 'pavlov', right: 'trusting' });
    g.agents.left.mine = mine;
    g.agents.left.lastPayoff = lastPayoff;
    return DC.decide(g, 'left', null);
  };
  assert.equal(mk(['cooperate'], 3, 3), 'cooperate', 'R is a win: stay');
  assert.equal(mk(['defect'], 5, 3), 'defect', 'T is a win: stay');
  assert.equal(mk(['cooperate'], 0, 3), 'defect', 'S is a loss: shift');
  assert.equal(mk(['defect'], 1, 3), 'cooperate', 'P is a loss: shift');
  assert.equal(mk(['cooperate'], 2, 3), 'defect', 'below R counts as a loss');
});

test('determinism: same seed, same match — different seed differs (random)', () => {
  const a = DC.runSummary(drive({ seed: 'det', rounds: 60, left: 'random', right: 'titfortat' }));
  const b = DC.runSummary(drive({ seed: 'det', rounds: 60, left: 'random', right: 'titfortat' }));
  assert.deepEqual(a, b);
  const c = DC.runSummary(drive({ seed: 'det-2', rounds: 60, left: 'random', right: 'titfortat' }));
  assert.notDeepEqual(a, c, 'a new seed re-rolls the coin');
});

test('noise garbles transmission; the scorer still pays on true moves', () => {
  const clean = drive({ seed: 'noise', rounds: 100, left: 'random', right: 'titfortat', noise: 0 });
  assert.equal(clean.flips, 0, 'no noise, no flips');
  const noisy = drive({ seed: 'noise', rounds: 100, left: 'random', right: 'titfortat', noise: 0.2 });
  assert.ok(noisy.flips > 0, 'flips happen at 0.2, got ' + noisy.flips);
  // the ledger: each score equals the sum of its per-round payoffs
  const sumA = noisy.moves.reduce((s, m) => s + m.pa, 0);
  const sumB = noisy.moves.reduce((s, m) => s + m.pb, 0);
  assert.equal(noisy.scores.left, sumA);
  assert.equal(noisy.scores.right, sumB);
  // a flip is logged, and tit-for-tat reacted to the SEEN move, not the truth
  assert.ok(noisy.log.some((l) => l.kind === 'noise'));
});

test('SPEC: matrix edits change outcomes — a temptation below reward flips the moral', () => {
  // with T=2 < R=3, an unconditional defector now LOSES the head-to-head
  const classic = DC.runSummary(drive({ seed: 'mx', rounds: 100,
    left: 'suspicious', right: 'trusting' }));
  assert.equal(classic.scores.left, 500, 'classic T=5 pays 5/round');
  const pacifist = DC.normalizeConfig({ seed: 'mx', rounds: 100,
    left: 'suspicious', right: 'trusting', matrix: { cc: 3, cd: 0, dc: 2, dd: 1 } });
  const g = drive(pacifist);
  const s = DC.runSummary(g);
  assert.equal(s.scores.left, 200, 'T=2 pays 2/round — the edit flows through');
  // and dd is editable too: a stalemate that pays is no longer a stalemate
  const rich = drive({ seed: 'mx2', rounds: 50, left: 'suspicious', right: 'suspicious',
    matrix: { cc: 3, cd: 0, dc: 5, dd: 7 } });
  assert.equal(DC.runSummary(rich).scores.left, 350, 'P=7 pays 7/round');
});

test('SPEC: the engine agent carries the FULL wire contract', () => {
  const eng = recorder('cooperate');
  const g = DC.createGame({ seed: 'wire', rounds: 50, left: 'engine', right: 'titfortat', askBudget: 5 });
  DC.tickGame(g, eng.choose);   // round 1: the ask happens
  assert.ok(eng.reqs.length >= 1, 'round 1 asked (a token was available)');
  const req = eng.reqs[0];
  assert.equal(typeof req.state.text, 'string');
  assert.ok(req.state.text.includes('prisoner'), req.state.text);
  assert.deepEqual(req.state.facts, {}, 'facts is the tagged-enum empty object');
  assert.equal(req.questions.length, 1);
  assert.equal(req.questions[0].type, 'choice');
  assert.equal(req.questions[0].id, 'pd-move');
  assert.deepEqual(req.questions[0].candidates.map((c) => c.id),
    ['cooperate', 'defect']);
  assert.deepEqual(req.policy,
    { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: 'low' });
  assert.equal(g.rungs.engine, 1, 'the honored answer landed on the engine rung');
  assert.ok(g.moves[0].a === 'cooperate', 'the honored move was played');
  // later rounds put the interaction history on the wire
  DC.tickGame(g, eng.choose);
  const t2 = eng.reqs[1].state.text;
  assert.ok(t2.includes('Score me'), 'scores ride along: ' + t2);
});

test('the engine agent stays within the roster contract: refusal falls to the mood rule', () => {
  const mk = () => DC.createGame({ seed: 'contract', rounds: 30,
    left: 'engine', right: 'suspicious', askBudget: 10 });
  // abstain: the mood rule (defect against a defector) answers, rung rule
  const g1 = mk();
  const abst = recorder(null, 'abstain');
  DC.tickGame(g1, abst.choose);
  assert.equal(g1.rungs.rule, 2, 'left mood rule + right suspicious, both rules');
  assert.equal(g1.rungs.engine, 0);
  assert.equal(g1.moves[0].a, 'cooperate',
    'round 1 has no history — the mood rule opens cooperative');
  // garbage answer: ignored, mood rule, rung rule — never a crash
  const g2 = mk();
  DC.tickGame(g2, recorder('betray').choose);
  assert.equal(g2.moves[0].a, 'cooperate', 'no history yet — the mood rule opens C');
  assert.equal(g2.rungs.rule, 2);
  // honoring works
  const g3 = mk();
  DC.tickGame(g3, recorder('cooperate').choose);
  assert.equal(g3.moves[0].a, 'cooperate');
  assert.equal(g3.rungs.engine, 1);
});

test('the ask budget is hard: asks never exceed it, the mood rule covers the rest', () => {
  const eng = recorder('defect');
  const g = drive({ seed: 'budget', rounds: 80, left: 'engine', right: 'trusting', askBudget: 3 },
    eng.choose);
  const s = DC.runSummary(g);
  assert.equal(s.engineCalls, 3, 'exactly the budget was spent');
  assert.equal(s.rungs.engine, 3);
  assert.equal(s.rungs.rule, 157, 'both sides every round, minus 3 asks');
  const zero = drive({ seed: 'budget', rounds: 20, left: 'engine', right: 'trusting', askBudget: 0 },
    eng.choose);
  assert.equal(DC.runSummary(zero).engineCalls, 0);
  // no bridge at all: a rules-only engine agent still plays
  const bare = drive({ seed: 'budget', rounds: 20, left: 'engine', right: 'titfortat' });
  const bs = DC.runSummary(bare);
  assert.equal(bs.engineCalls, 0);
  assert.equal(bs.rungs.rule, 40, 'both sides are rules without a bridge');
});

test('roster contract: every strategy answers only cooperate/defect, any history', () => {
  const rng = globalThis.DecisionsSDK.makeRng('fuzz');
  for (const strat of DC.ROSTER.filter((s) => s !== 'engine')) {
    for (let trial = 0; trial < 50; trial++) {
      const st = { seen: [], mine: [], lastPayoff: Math.floor(rng() * 8) };
      const n = Math.floor(rng() * 12);
      for (let i = 0; i < n; i++) {
        const m = rng() < 0.5 ? 'cooperate' : 'defect';
        st.seen.push(m);
        st.mine.push(rng() < 0.5 ? 'cooperate' : 'defect');
      }
      const move = DC.step(strat, st, { rng, R: 3 });
      assert.ok(move === 'cooperate' || move === 'defect',
        strat + ' answered ' + move);
    }
  }
});

test('score and tally ledgers close: cells count every round on both sides', () => {
  const g = drive({ seed: 'ledger', rounds: 120, left: 'pavlov', right: 'grim', noise: 0.1 });
  const s = DC.runSummary(g);
  for (const side of ['left', 'right']) {
    const t = s.tally[side];
    assert.equal(t.cc + t.cd + t.dc + t.dd, 120, side + ' counted every round');
  }
  assert.equal(g.round, 120);
});

test('history samples every 10 rounds and is time-ordered', () => {
  const g = drive({ seed: 'hist', rounds: 55, left: 'trusting', right: 'suspicious' });
  assert.equal(g.history.length, 6, 'samples at r=10..50 plus the r=55 tail');
  assert.deepEqual(g.history.map((h) => h.r), [10, 20, 30, 40, 50, 55]);
  const even = drive({ seed: 'hist', rounds: 50, left: 'trusting', right: 'suspicious' });
  assert.equal(even.history.length, 5, 'no duplicate tail when 50 % 10 === 0');
  assert.deepEqual(even.history.map((h) => h.r), [10, 20, 30, 40, 50]);
  assert.ok(g.history.every((h, i) => i === 0 || h.r > g.history[i - 1].r));
  assert.ok(g.history.every((h) => h.coopL >= 0 && h.coopL <= 1 && h.coopR >= 0
    && h.coopR <= 1 && Number.isFinite(h.avg)));
});

test('engine-vs-engine: both sides ask, both budgets spend', () => {
  const eng = recorder('cooperate');
  const g = drive({ seed: 'mirror', rounds: 20, left: 'engine', right: 'engine', askBudget: 2 },
    eng.choose);
  const s = DC.runSummary(g);
  assert.equal(s.engineCalls, 2, 'the pool is per-match: both sides draw it');
  assert.equal(s.rungs.engine, 2);
  assert.equal(s.rungs.rule, 38, 'the rest of both sides is rules');
  assert.equal(s.scores.left, 60, 'mutual cooperation at R=3');
  assert.equal(s.scores.right, 60);
  assert.ok(eng.reqs.length === 2);
  assert.ok(eng.reqs.every((r) => r.questions[0].id === 'pd-move'));
});

test('tournament: full round-robin, deterministic, engine bounded per pairing', () => {
  const eng = recorder('cooperate');
  const t1 = DC.tournament({ seed: 'tour', rounds: 40, noise: 0.05, askBudget: 2 }, eng.choose);
  assert.deepEqual(t1.roster.sort(), ['engine', 'forgiving', 'grim', 'pavlov',
    'random', 'suspicious', 'titfortat', 'trusting'].sort());
  const n = t1.roster.length;
  assert.equal(t1.table.length, n * (n - 1) / 2, 'every pair plays once');
  assert.equal(t1.interactions, 40 * t1.table.length);
  assert.equal(t1.engineCalls, 2 * 7, 'the engine sits in 7 pairings, budget 2 each');
  assert.ok(t1.series.length > 0 && t1.series[0].r === 10);
  assert.ok(t1.table.every((r) => Number.isFinite(r.sa) && Number.isFinite(r.sb)));
  // classic check inside the table: suspicious beats trusting head-to-head
  const st = t1.table.find((r) => (r.a === 'suspicious' && r.b === 'trusting')
    || (r.b === 'suspicious' && r.a === 'trusting'));
  assert.ok(st.sa !== st.sb, 'exploitation happened');
  const t2 = DC.tournament({ seed: 'tour', rounds: 40, noise: 0.05, askBudget: 2 }, eng.choose);
  assert.deepEqual(t1.table, t2.table, 'the round-robin replays identically');
  // without the engine, no engine asks at all
  const t3 = DC.tournament({ seed: 'tour', rounds: 20, engineInTournament: false }, null);
  assert.equal(t3.roster.includes('engine'), false);
  assert.equal(t3.engineCalls, 0);
});

test('config and stamp: stamps separate every knob that changes the match', () => {
  const base = DC.dilemmaStamp(DC.normalizeConfig({ seed: 'x' }));
  assert.notEqual(base, DC.dilemmaStamp(DC.normalizeConfig({ seed: 'y' })));
  assert.notEqual(base, DC.dilemmaStamp(DC.normalizeConfig({ seed: 'x', rounds: 99 })));
  assert.notEqual(base, DC.dilemmaStamp(DC.normalizeConfig({ seed: 'x', noise: 0.2 })));
  assert.notEqual(base, DC.dilemmaStamp(DC.normalizeConfig({ seed: 'x', askBudget: 1 })));
  assert.notEqual(base, DC.dilemmaStamp(DC.normalizeConfig({ seed: 'x', left: 'grim' })));
  assert.notEqual(base, DC.dilemmaStamp(DC.normalizeConfig({ seed: 'x',
    matrix: { cc: 4, cd: 0, dc: 5, dd: 1 } })));
  assert.notEqual(base, DC.dilemmaStamp(DC.normalizeConfig({ seed: 'x',
    engineInTournament: false })));
  assert.equal(base, DC.dilemmaStamp(DC.normalizeConfig({ seed: 'x' })), 'stable');
});
