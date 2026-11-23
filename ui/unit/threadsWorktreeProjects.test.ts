// A worktree thread files under its main checkout (worktreeProjects.svelte.ts):
// one worktree read per repo, /var vs /private/var, remembered across reloads.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function fixture(stored: string | null = null) {
  const gets: { path: string; response: ReturnType<typeof deferred<any>> }[] = [];
  const saved: Record<string, string> = {};
  const { worktreeProjects } = loadSource(new URL('../src/modules/threads/worktreeProjects.svelte.ts', import.meta.url), {
    '../../lib/api/client': { api: { get: (path: string) => { const response = deferred<any>(); gets.push({ path, response }); return response.promise; } } },
    '../../lib/storage': { lsGet: () => stored, lsSet: (k: string, v: string) => { saved[k] = v; } },
  });
  return { wp: worktreeProjects, gets, saved };
}
const tick = () => new Promise((r) => setImmediate(r));
const repo = { id: 'r1', path: '/private/var/x/harbor', name: 'harbor' };

test('a linked worktree resolves to its main checkout, written the way the folder is', async () => {
  const h = fixture();
  h.wp.ensure(repo);
  h.wp.ensure(repo); // once per repo
  assert.equal(h.gets.length, 1);
  h.gets[0].response.resolve([
    { path: '/private/var/x/harbor', branch: 'main', is_main: true, prunable: false },
    { path: '/private/var/x/harbor-fix-flaky', branch: 'fix-flaky', is_main: false, prunable: false },
  ]);
  await tick();
  assert.equal(h.wp.root('/var/x/harbor-fix-flaky'), '/var/x/harbor');
  assert.equal(h.wp.root('/private/var/x/harbor-fix-flaky/src'), '/private/var/x/harbor');
  assert.equal(h.wp.of('/var/x/harbor-fix-flaky')?.branch, 'fix-flaky');
  // The main checkout and unrelated folders are themselves.
  assert.equal(h.wp.root('/var/x/harbor'), '/var/x/harbor');
  assert.equal(h.wp.root('/var/x/harbor-fix'), '/var/x/harbor-fix');
  assert.ok(h.saved['otto.threads.worktrees']);
});

test('a worktree Otto made is known at once and survives a reload', () => {
  const h = fixture();
  h.wp.note(repo, { path: '/private/var/x/harbor-new', branch: 'new', is_main: false });
  assert.equal(h.wp.root('/var/x/harbor-new'), '/var/x/harbor');
  const again = fixture(h.saved['otto.threads.worktrees']);
  assert.equal(again.wp.root('/var/x/harbor-new'), '/var/x/harbor');
});
