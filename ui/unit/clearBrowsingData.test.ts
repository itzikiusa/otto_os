import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const m = loadSource(new URL('../src/modules/browser/clearBrowsingData.ts', import.meta.url), {});

test('the clear-browsing-data confirm says what goes, for whom, and what stays', () => {
  const t: string = m.CLEAR_DATA_TEXT;
  for (const what of ['cookies', 'cache', 'site storage', 'every workspace', 'signed out of those sites', 'Otto’s own sign-in and settings are not affected']) {
    assert.ok(t.includes(what), what);
  }
});
