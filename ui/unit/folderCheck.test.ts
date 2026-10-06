// S20-04 / S14-11: a typed folder is checked before an agent starts there.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { browsePath, checkFolder } from '../src/lib/folderCheck.ts';

const err = (status: number, message = 'no such folder') => Object.assign(new Error(message), { status });

test('an existing folder passes with the daemon-normalised path', async () => {
  assert.deepEqual(await checkFolder(' ~/code ', async (p) => ({ path: p.replace('~', '/Users/me') })), { ok: true, path: '/Users/me/code' });
});

test('a missing folder or a file blocks with an inline reason', async () => {
  for (const s of [400, 404]) {
    const r = await checkFolder('/nope', async () => Promise.reject(err(s)));
    assert.equal(r.ok, false);
    assert.match((r as { message: string }).message, /can’t open that folder \(no such folder\)/);
  }
  assert.equal((await checkFolder('  ', async () => ({ path: '/' }))).ok, false);
});

test('busy / forbidden / network failures do not block (the create reports real failures)', async () => {
  for (const e of [err(409), err(502), err(403), new TypeError('Load failed')]) {
    assert.deepEqual(await checkFolder('/x', async () => Promise.reject(e)), { ok: true, path: '/x' });
  }
});

test('browsePath encodes the path', () => {
  assert.equal(browsePath('/a b/c'), '/fs/browse?path=%2Fa%20b%2Fc');
});

test('New Session and the first-run coach pre-check the folder', () => {
  for (const f of ['src/modules/agents/NewSession.svelte', 'src/modules/agents/FirstRunCoach.svelte']) {
    const src = readFileSync(join(import.meta.dirname, '..', f), 'utf8');
    assert.match(src, /checkFolder\(/, `${f} must pre-check the folder`);
  }
});
