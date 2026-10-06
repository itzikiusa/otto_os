// S20-04 / S14-11: a typed folder is checked before an agent starts there.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { browsePath, checkFolder, statPath } from '../src/lib/folderCheck.ts';

const err = (status: number, message = 'no such folder') => Object.assign(new Error(message), { status });

/** A fake daemon: `/fs/stat` answers per `stat`, `/fs/browse` per `browse`. */
function daemon(stat: (p: string) => unknown, browse: (p: string) => unknown = () => { throw err(404); }) {
  const calls: string[] = [];
  const get = async (url: string) => {
    calls.push(url.split('?')[0]);
    const p = decodeURIComponent(url.split('path=')[1]);
    return url.startsWith('/fs/stat') ? stat(p) : browse(p);
  };
  return { get, calls };
}

test('an existing folder passes with the daemon-normalised path — one stat, no listing (S14-306)', async () => {
  const d = daemon((p) => ({ path: p.replace('~', '/Users/me'), is_dir: true, is_git_repo: false }));
  assert.deepEqual(await checkFolder(' ~/code ', d.get), { ok: true, path: '/Users/me/code' });
  assert.deepEqual(d.calls, ['/fs/stat'], 'never lists the folder');
});

test('a file blocks with an inline reason', async () => {
  const d = daemon((p) => ({ path: p, is_dir: false, is_git_repo: false }));
  const r = await checkFolder('/etc/hosts', d.get);
  assert.equal(r.ok, false);
  assert.match((r as { message: string }).message, /not a folder/);
});

test('a missing folder blocks (stat 404, confirmed by browse)', async () => {
  for (const s of [400, 404]) {
    const d = daemon(() => { throw err(404); }, () => { throw err(s); });
    const r = await checkFolder('/nope', d.get);
    assert.equal(r.ok, false);
    assert.match((r as { message: string }).message, /can’t open that folder \(no such folder\)/);
  }
  assert.equal((await checkFolder('  ', daemon(() => ({})).get)).ok, false);
});

test('a daemon without /fs/stat (route 404) falls back to browse', async () => {
  const d = daemon(() => { throw err(404, 'Not Found'); }, (p) => ({ path: p }));
  assert.deepEqual(await checkFolder('/x', d.get), { ok: true, path: '/x' });
  assert.deepEqual(d.calls, ['/fs/stat', '/fs/browse']);
});

test('busy / forbidden / network failures do not block (the create reports real failures)', async () => {
  for (const e of [err(409), err(502), err(403), new TypeError('Load failed')]) {
    assert.deepEqual(await checkFolder('/x', daemon(() => { throw e; }).get), { ok: true, path: '/x' });
  }
});

test('statPath / browsePath encode the path', () => {
  assert.equal(statPath('/a b/c'), '/fs/stat?path=%2Fa%20b%2Fc');
  assert.equal(browsePath('/a b/c'), '/fs/browse?path=%2Fa%20b%2Fc');
});

test('New Session and the first-run coach pre-check the folder', () => {
  for (const f of ['src/modules/agents/NewSession.svelte', 'src/modules/agents/FirstRunCoach.svelte']) {
    const src = readFileSync(join(import.meta.dirname, '..', f), 'utf8');
    assert.match(src, /checkFolder\(/, `${f} must pre-check the folder`);
  }
});
