import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const m = loadSource(new URL('../src/modules/threads/threadTitle.ts', import.meta.url), {});

test('a placeholder title drops lead-ins and keeps the first sentence', () => {
  assert.equal(m.placeholderTitle('can you please fix the login redirect bug? It loops forever.', 'x'), 'Fix the login redirect bug');
  assert.equal(m.placeholderTitle('I want you to add a dark mode toggle to settings', 'x'), 'Add a dark mode toggle to settings');
  assert.equal(m.placeholderTitle('\n\n  hey, refactor the parser\nmore details', 'x'), 'Refactor the parser');
});

test('a long message is cut at a word with an ellipsis', () => {
  const t = m.placeholderTitle('investigate why the nightly performance job times out on the linux runner every other day', 'x');
  assert.ok(t.length <= 56, t);
  assert.ok(t.endsWith('…'), t);
  assert.ok(!/\s…$/.test(t), t);
  assert.ok(t.startsWith('Investigate why the nightly'), t);
});

test('empty text falls back', () => {
  assert.equal(m.placeholderTitle('   ', 'Claude thread'), 'Claude thread');
});
