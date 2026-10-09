/* SDK shell chrome — numericSeed + resetParams contracts, pinned against
   minimal element stubs (no real DOM needed: the helpers only touch value,
   type, checked, addEventListener/dispatchEvent, classList.add).

   Run: node --test tests/sdk_shell_test.mjs */

import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const siteDir = path.dirname(fileURLToPath(import.meta.url));
await import(path.join(siteDir, '..', 'sdk', 'decisions-sdk.js'));
const DK = globalThis.DecisionsSDK;

function fakeEl(tag, value) {
  return {
    tagName: tag, value: value ?? '', type: tag === 'input' ? 'text' : undefined,
    checked: false, listeners: {},
    classList: { add() {}, remove() {} },
    addEventListener(t, fn) { (this.listeners[t] ||= []).push(fn); },
    dispatchEvent(ev) { try { ev.target = this; } catch {} for (const f of this.listeners[ev.type] || []) f(ev); return true; },
  };
}
function fire(el, type) { el.dispatchEvent({ type }); }

test('randomSeed is a numeric string', () => {
  for (let i = 0; i < 20; i++) assert.match(DK.randomSeed(), /^\d+$/);
});

test('numericSeed: non-numeric boot value is replaced by a numeric one', () => {
  const input = fakeEl('input', 'oc-pong');
  const kit = DK.shell.numericSeed(input, null);
  assert.equal(input.type, 'number', 'the box becomes a number box');
  assert.match(input.value, /^\d+$/, 'garbage boot value becomes a random numeric seed');
  assert.equal(kit.get(), input.value, 'get() reads what the box shows');
});

test('numericSeed: get() repairs a box the user emptied', () => {
  const input = fakeEl('input', '123456');
  const kit = DK.shell.numericSeed(input, null);
  input.value = '   ';
  const v = kit.get();
  assert.match(v, /^\d+$/);
  assert.equal(input.value, v, 'the repair is written back so the user sees it');
});

test('numericSeed: dice rerolls and restarts via onChange', () => {
  const input = fakeEl('input', '111111');
  const dice = fakeEl('button');
  let restarts = 0;
  const kit = DK.shell.numericSeed(input, dice, () => { restarts += 1; });
  const before = input.value;
  fire(dice, 'click');
  assert.match(input.value, /^\d+$/);
  assert.notEqual(input.value, before, 'a roll is always a new seed');
  assert.equal(kit.get(), input.value);
  assert.equal(restarts, 1, 'the game restart hook fired once');
  assert.equal(input.type, 'number');
});

test('resetParams: changed controls return to boot values and refire events', () => {
  const speed = fakeEl('input', '40'); speed.type = 'range';
  const wrap = fakeEl('input'); wrap.type = 'checkbox'; wrap.checked = true;
  const seed = fakeEl('input', '42'); seed.type = 'number';
  const root = { querySelectorAll: () => [speed, wrap, seed] };
  const btn = fakeEl('button');
  const fired = [];
  for (const el of [speed, wrap, seed]) {
    el.addEventListener('input', () => fired.push('input:input'));
    el.addEventListener('change', () => fired.push('input:change'));
  }
  DK.shell.resetParams(btn, root);

  speed.value = '80';            // user drags a slider
  wrap.checked = false;          // user toggles wrap off
  seed.value = '999';            // user types a new seed
  fire(btn, 'click');

  assert.equal(speed.value, '40', 'slider restored');
  assert.equal(wrap.checked, true, 'checkbox restored');
  assert.equal(seed.value, '42', 'seed restored');
  /* untouched controls do not fire; touched ones fire input+change */
  assert.deepEqual(fired, [
    'input:input', 'input:change',
    'input:input', 'input:change',
    'input:input', 'input:change',
  ]);
});

test('resetParams is a defensive no-op without real chrome', () => {
  assert.doesNotThrow(() => DK.shell.resetParams(null, { querySelectorAll: () => [] }));
  assert.doesNotThrow(() => DK.shell.resetParams(fakeEl('button'), null));
});

test('numericSeed without an input still returns a usable kit', () => {
  const kit = DK.shell.numericSeed(null, null);
  assert.match(kit.get(), /^\d+$/);
  assert.doesNotThrow(() => kit.roll());
});
