import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

// `repo_status_changed` (otto-git watch.rs) → git.applyRepoChanged(): the Git
// page re-reads local status right away instead of on the next auto-fetch.

const status = (n: number) => ({
  branch: 'main', upstream: 'origin/main', ahead: 0, behind: 0,
  changes: Array.from({ length: n }, (_, i) => ({ path: `f${i}.ts`, kind: 'modified', staged: false, unstaged: true })),
});
const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };

function fixture() {
  const listeners = new Map<string, () => void>();
  const document = {
    hidden: false,
    hasFocus: () => true,
    addEventListener: (t: string, fn: () => void) => listeners.set(t, fn),
    removeEventListener: (t: string) => listeners.delete(t),
  };
  const gets: { url: string; result: ReturnType<typeof deferred<ReturnType<typeof status>>> }[] = [];
  const { git } = loadSource(new URL('../src/lib/stores/git.svelte.ts', import.meta.url), {
    '../api/client': { api: { get: (url: string) => {
      const result = deferred<ReturnType<typeof status>>();
      gets.push({ url, result });
      return result.promise;
    } } },
    '../loadError': { loadErrorText: (e: unknown) => (e instanceof Error ? e.message : String(e)) },
  }, { document, setTimeout, clearTimeout });
  return { git, gets, document, listeners };
}

test('a change to a shown repo re-reads its local status at once', async () => {
  const f = fixture();
  f.git.openRepoIds = ['r1'];
  f.git.applyRepoChanged('r1');
  assert.deepEqual(f.gets.map((g) => g.url), ['/repos/r1/status']);
  f.gets[0].result.resolve(status(2));
  await flush();
  assert.equal(f.git.statusById.r1.changes.length, 2);
  assert.equal(f.git.liveRev.r1, 1);
});

test('a repo nobody shows is ignored', () => {
  const f = fixture();
  f.git.openRepoIds = ['r1'];
  f.git.applyRepoChanged('other');
  assert.equal(f.gets.length, 0);
});

test('a burst while a read is in flight costs exactly one more read', async () => {
  const f = fixture();
  f.git.openRepoIds = ['r1'];
  f.git.applyRepoChanged('r1');
  f.git.applyRepoChanged('r1');
  f.git.applyRepoChanged('r1');
  f.git.applyRepoChanged('r1');
  assert.equal(f.gets.length, 1);
  f.gets[0].result.resolve(status(1));
  await flush();
  assert.equal(f.gets.length, 2);
  f.gets[1].result.resolve(status(3));
  await flush();
  assert.equal(f.gets.length, 2);
  assert.equal(f.git.statusById.r1.changes.length, 3);
});

test('a hidden window waits and reads once when visible again', async () => {
  const f = fixture();
  f.git.openRepoIds = ['r1'];
  f.document.hidden = true;
  f.git.applyRepoChanged('r1');
  f.git.applyRepoChanged('r1');
  assert.equal(f.gets.length, 0);
  f.document.hidden = false;
  f.listeners.get('visibilitychange')?.();
  assert.equal(f.gets.length, 1);
  assert.equal(f.listeners.has('visibilitychange'), false);
});
