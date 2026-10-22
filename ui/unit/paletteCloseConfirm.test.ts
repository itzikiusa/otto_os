// ux-design-3-03: the sidebar's bulk Delete / Archive and the ⌘K palette's
// close ask the same confirm — singular/plural title, up to five titles plus
// "…and N more", and a note when live agents will stop mid-turn.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import * as pluralMod from '../src/lib/plural.ts';

const { closeConfirm } = loadSource(new URL('../src/shell/closeConfirm.ts', import.meta.url), { '../lib/plural': pluralMod });

test('one session reads singular, never "Delete 1 sessions"', () => {
  const c = closeConfirm({ titles: ['Maldini'], working: 0, permanent: true });
  assert.equal(c.title, 'Delete session?');
  assert.equal(c.confirmLabel, 'Delete');
  assert.equal(c.danger, true);
  assert.match(c.message, /^Delete 1 session and its history permanently\?/);
  assert.match(c.message, /• Maldini/);
  assert.doesNotMatch(c.message, /sessions/);
});

test('many sessions list five titles, the rest as a count, and warn about working ones', () => {
  const titles = ['a', 'b', 'c', 'd', 'e', 'f', 'g'];
  const c = closeConfirm({ titles, working: 2, permanent: true });
  assert.equal(c.title, 'Delete sessions?');
  assert.match(c.message, /• e\n…and 2 more/);
  assert.doesNotMatch(c.message, /• f/);
  assert.match(c.message, /2 sessions are working right now and will stop mid-turn\./);
});

test('archive asks without the danger button and names the count', () => {
  const one = closeConfirm({ titles: ['x'], working: 1, permanent: false });
  assert.equal(one.title, 'Archive session?');
  assert.equal(one.confirmLabel, 'Archive session');
  assert.equal(one.danger, false);
  assert.match(one.message, /1 session is working right now/);
  const many = closeConfirm({ titles: ['x', 'y'], working: 0, permanent: false });
  assert.equal(many.confirmLabel, 'Archive 2 sessions');
  assert.doesNotMatch(many.message, /working/);
});
