// The context window meter (runtime/contextMeter.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { contextMeter, tokens } from '../src/modules/threads/runtime/contextMeter.ts';

test('tokens read short', () => {
  assert.equal(tokens(950), '950');
  assert.equal(tokens(1234), '1.2k');
  assert.equal(tokens(122_400), '122k');
  assert.equal(tokens(1_048_576), '1M');
});

test('the meter: percent and tone at 80 % / 95 %, nothing without a window', () => {
  assert.equal(contextMeter(null), null);
  assert.equal(contextMeter({ used_tokens: 500 }), null, 'a provider that reports no window: no meter');
  assert.deepEqual(contextMeter({ used_tokens: 122_000, max_tokens: 200_000 }), { pct: 61, used: 122_000, max: 200_000, tone: 'ok', text: '122k of 200k tokens (61 %)' });
  assert.equal(contextMeter({ used_tokens: 160_000, max_tokens: 200_000 })?.tone, 'warning');
  assert.equal(contextMeter({ used_tokens: 190_000, max_tokens: 200_000 })?.tone, 'danger');
  assert.equal(contextMeter({ used_tokens: 260_000, max_tokens: 200_000 })?.pct, 100);
});
