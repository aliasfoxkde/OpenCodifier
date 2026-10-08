/* Maze Solver deep link — `?maze=<seed>` (+ optional &mode=bot|play|versus)
   is read once at wire() time, so it gets its own process with
   location.search set BEFORE maze.js boots (the shared stub imports first,
   the query is set on its plain-object location, then the shell imports).

   Run: node --test tests/maze_deeplink_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

import { byId, pumpFrames } from './maze_dom_stub.mjs';

globalThis.window.location.search = '?maze=pinned-seed-77&mode=play';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'maze-core.js'));
await import(path.join(siteDir, '..', 'maze.js'));

test('the deep link pins the seed and the mode before first render', () => {
  assert.equal(byId('mz-seed').value, 'pinned-seed-77', 'seed input carries the link');
  assert.equal(byId('mz-mode-play').getAttribute('aria-pressed'), 'true', 'mode from the link is active');
  assert.equal(byId('mz-mode-bot').getAttribute('aria-pressed'), 'false', 'default bot mode was displaced');
});

test('the linked game is live under the pinned seed', () => {
  pumpFrames(1200);
  const log = byId('mz-log').textContent || '';
  assert.match(log, /pinned-seed-77/, 'the log names the deep-linked seed');
});
