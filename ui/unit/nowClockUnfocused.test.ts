// The shared relative-time clock (lib/stores/now.svelte.ts) steps every 15 s,
// not every second, while the window is unfocused, and catches up on focus.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

test('an unfocused window ticks the shared clock at 15 s steps; focus catches up', async () => {
  let tick: (() => void) | null = null;
  let onFocus: (() => void) | null = null;
  let focused = false;
  const m = loadSource(new URL('../src/lib/stores/now.svelte.ts', import.meta.url), { svelte: { untrack: (f: () => unknown) => f() } }, {
    window: { addEventListener: (t: string, fn: () => void) => { if (t === 'focus') onFocus = fn; } },
    document: { hidden: false, hasFocus: () => focused, addEventListener() {} },
    setInterval: (fn: () => void) => { tick = fn; return 1; },
  });
  const t0 = m.now();
  await new Promise((r) => setTimeout(r, 5));
  tick!();
  assert.equal(m.now(), t0, 'unfocused, inside 15 s: no re-run');
  focused = true; onFocus!();
  assert.ok(m.now() > t0, 'focus catches up at once');
  const t1 = m.now();
  await new Promise((r) => setTimeout(r, 5));
  tick!();
  assert.ok(m.now() > t1, 'focused: every second');
});
