/* Maze Solver game deep link — games.html links into the playground with
   ?game=<preset>; like the seed link this is read once at wire() time, so
   it gets its own process with location.search set BEFORE maze.js boots.

   Run: node --test tests/maze_game_link_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

import { byId, pumpFrames } from './maze_dom_stub.mjs';

globalThis.window.location.search = '?game=crawler&maze=90210';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
await import(path.join(siteDir, '..', 'maze-core.js'));
await import(path.join(siteDir, '..', 'maze.js'));

test('the crawler game link applies the dungeon preset and keeps the seed', () => {
  assert.equal(byId('mz-seed').value, '90210', 'the seed from the link survives the preset');
  assert.equal(byId('mz-maptype').value, 'dungeon', '?game=crawler is a dungeon map');
  assert.equal(byId('mz-vshape').value, 'cone', 'crawler plays with cone vision');
  assert.equal(byId('mz-vhalf').value, '60', 'crawler cone half-angle applied');
  assert.equal(byId('mz-avoid').checked, true, 'crawler routes around monsters');
  assert.match(byId('mz-log').textContent, /dungeon/, 'the booted map really is a dungeon');
});

test('the linked crawler game is live', () => {
  pumpFrames(400);
  const trace = byId('mz-trace').textContent || '';
  const log = byId('mz-log').textContent || '';
  assert.ok(trace.length > 0 || log.length > 0, 'the bot acts under cone fog');
});
