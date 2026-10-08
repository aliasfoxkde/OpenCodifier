/* Maze Solver SHELL suite — boots the real site/maze.js under the shared
   DOM stub (tests/maze_dom_stub.mjs; the board_page_test.mjs pattern).
   Pins the wiring the core suite cannot see: boot, the rAF loop ticking
   the bot to an escape, mode switching, the leaderboard recording finished
   runs, the rubric editor's clamps, and hostile rubric JSON.

   The deep link gets its own process (tests/maze_deeplink_test.mjs) because
   location.search must be set before maze.js boots.

   Run: node --test tests/maze_shell_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

import { byId, pumpFrames, rafPending, textOf } from './maze_dom_stub.mjs';

/* ---------- import order matters: core first, then the shell ---------- */

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'maze-core.js'));
assert.ok(globalThis.MazeCore, 'maze-core.js must boot');
await import(path.join(siteDir, '..', 'maze.js'));

test('the shell boots: UI is built, canvas present, loop scheduled', () => {
  const rootEl = byId('maze-root');
  assert.ok(rootEl.children.length > 0, 'maze.js built its UI into #maze-root');
  const canvas = byId('mz-canvas');
  assert.ok(canvas && canvas.tag === 'canvas', 'the built canvas is registered');
  assert.ok(rafPending() > 0, 'rAF loop is scheduled');
});

test('bot mode plays to an escape and lands a row on the board', () => {
  /* default bot speed 30 steps/s; 27×27 grid escapes well inside 1200 frames */
  pumpFrames(1200);
  const boardBody = byId('mz-board-body');
  assert.ok(boardBody.children.length >= 1, 'a finished run lands on the board');
  const rowText = textOf(boardBody.children[0]);
  assert.match(rowText, /BOT/, 'the recorded row is the bot');
  assert.match(rowText, /escaped/, 'the bot escaped');
  assert.ok(byId('mz-rung').textContent !== '—', 'the rung chip shows a real rung');
});

test('mode switch to play keeps the human run in charge (d-pad drives it)', () => {
  byId('mz-mode-play').dispatch('click');
  pumpFrames(10);
  const before = byId('mz-coords').textContent;
  const dpad = byId('maze-root').querySelector('.mz-dpad');
  assert.ok(dpad, 'dpad exists');
  /* hold each direction in turn — the corridor out of the center cell may
     face any way; one of the four must move the runner */
  for (const dir of ['S', 'N', 'E', 'W']) {
    const btn = dpad.children.find((c) => c.attrs && c.attrs['data-dir'] === dir);
    btn.dispatch('pointerdown');
    pumpFrames(15);
    btn.dispatch('pointerup');
    if (byId('mz-coords').textContent !== before) break;
  }
  assert.notEqual(byId('mz-coords').textContent, before, 'the runner moved');
});

test('a keyboard tap shorter than one frame still moves the runner', () => {
  const before = byId('mz-coords').textContent;
  /* tap each key in turn — the open corridor may face any way; one tap
     (down + up within the same frame) must still step */
  for (const key of ['d', 'w', 's', 'a']) {
    globalThis.window.dispatchEvent({
      type: 'keydown', key, preventDefault() {}, repeat: false,
    });
    globalThis.window.dispatchEvent({
      type: 'keyup', key, preventDefault() {}, repeat: false,
    });
    pumpFrames(1);
    if (byId('mz-coords').textContent !== before) break;
  }
  assert.notEqual(byId('mz-coords').textContent, before, 'one tap = one step');
});

test('versus mode runs both sides and the scoreboard names them', () => {
  byId('mz-mode-versus').dispatch('click');
  const rowsBefore = byId('mz-board-body').children.length;
  pumpFrames(1400);
  assert.match(byId('mz-score-you').textContent, /^YOU/, 'scoreboard names the human side');
  assert.match(byId('mz-score-bot').textContent, /^BOT/, 'scoreboard names the bot side');
  const rowsAfter = byId('mz-board-body').children.length;
  assert.ok(rowsAfter > rowsBefore, 'the versus bot escapes and adds a row of its own');
});

test('god mode toggles the honest assisted flag path', () => {
  byId('mz-mode-play').dispatch('click');
  pumpFrames(5);
  byId('mz-god').dispatch('click');
  pumpFrames(5);
  assert.equal(byId('mz-god').getAttribute('aria-pressed'), 'true');
  byId('mz-god').dispatch('click'); /* back off */
});

test('rubric editor: click-add appends a criterion, clamps hold, JSON stays in sync', () => {
  const palette = byId('mz-palette');
  const steps = palette.children.find((c) => c.textContent === 'steps');
  const before = JSON.parse(byId('mz-rubric-json').value).criteria.length;
  steps.dispatch('click');
  const after = JSON.parse(byId('mz-rubric-json').value).criteria.length;
  assert.equal(after, before + 1, 'click-to-append works');
  /* clamp: weight − pressed 10 times bottoms out at −3 */
  const row = byId('mz-rubric-list').children.find((r) =>
    r.children[0] && r.children[0].textContent === 'steps');
  for (let i = 0; i < 10; i++) row.children[2].dispatch('click');
  const crit = JSON.parse(byId('mz-rubric-json').value).criteria.find((c) => c.tag === 'steps');
  assert.equal(crit.weight, -3, 'weight clamps at −3');
  row.children[4].dispatch('click'); /* × removes — row is [chip, weight, −, +, ×] */
  assert.ok(!JSON.parse(byId('mz-rubric-json').value).criteria.some((c) => c.tag === 'steps'));
});

test('hostile rubric JSON is sanitized, not trusted', () => {
  byId('mz-rubric-json').value = '{"criteria":[{"tag":"time","weight":999},{"tag":"bogus","weight":2}]}';
  byId('mz-rubric-apply').dispatch('click');
  const applied = JSON.parse(byId('mz-rubric-json').value);
  assert.ok(applied.criteria.every((c) => Math.abs(c.weight) <= 5), 'weights clamped');
  assert.ok(applied.criteria.every((c) => globalThis.MazeCore.TAGS.includes(c.tag)), 'unknown tags dropped');
});

test('a new seed regenerates and the log names it', () => {
  byId('mz-seed').value = 'regen-check';
  byId('mz-generate').dispatch('click');
  pumpFrames(5);
  assert.match(byId('mz-log').firstChild.textContent, /regen-check/, 'log names the new seed');
});
