import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';

for (const [file, name, width] of [
  ['UsagePage', 'showXLabel', 'SVG_W'],
  ['ModelBreakdown', 'showX', 'W'],
] as const) {
  test(`${file} date labels stay at least 52 rendered pixels apart`, () => {
    for (const chartWidth of [260, 390, 700]) {
      const state = componentFunctions(new URL(`../src/modules/usage/${file}.svelte`, import.meta.url), [name], { [width]: chartWidth, AXIS_L: 52 });
      for (const days of [7, 30, 90]) {
        const visible = Array.from({ length: days }, (_, i) => i).filter(i => state[name](i, days));
        assert.ok(visible.length > 0);
        for (let i = 1; i < visible.length; i++) {
          const gap = (visible[i] - visible[i - 1]) * (chartWidth - 52) / days;
          assert.ok(gap >= 52, `${chartWidth}px / ${days} days: gap ${gap}`);
        }
      }
    }
  });
}
