import { test } from 'node:test';
import assert from 'node:assert/strict';
import { OWNER_ONLY_HINT, settingsSaveErrorText } from '../src/modules/settings/ownerOnly.ts';

// Sweep 2026-10-11 prefs S-4: a share-link / mobile owner saving Daemon or
// Providers settings got a bare 403 toast.
test('a refused settings save explains who can save', () => {
  const raw = (e: unknown) => (e as Error).message;
  const refused = Object.assign(new Error('a person signed in to Otto must do this'), { status: 403 });
  assert.equal(settingsSaveErrorText(refused, raw), OWNER_ONLY_HINT);
  const other = Object.assign(new Error('database is locked'), { status: 500 });
  assert.equal(settingsSaveErrorText(other, raw), 'database is locked');
  assert.equal(settingsSaveErrorText(new Error('offline'), raw), 'offline');
});
