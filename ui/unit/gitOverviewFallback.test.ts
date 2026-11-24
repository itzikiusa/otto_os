// W3: a repository's status asked for while its overview read is in flight
// (the tab strip, Threads' details card) must not stay empty when that read
// fails — the store falls back to the status read (lib/stores/git.svelte.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';
import * as livePaths from '../src/lib/gitLivePaths.ts';
import * as latest from '../src/lib/latest.ts';

const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };

test('a failed overview read falls back to the status read', async () => {
  const overview = deferred<unknown>();
  const gets: string[] = [];
  const client = { api: {
    get: (url: string) => {
      gets.push(url);
      if (url.endsWith('/overview')) return overview.promise;
      if (url.includes('/status')) return Promise.resolve({ branch: 'main', upstream: null, ahead: 0, behind: 0, changes: [] });
      return Promise.reject(new Error(`no GET ${url}`));
    },
    post: () => new Promise(() => {}),
  } };
  const imports: Record<string, unknown> = {
    '../api/client': client,
    '../loadError': { loadErrorText: (e: unknown) => (e instanceof Error ? e.message : String(e)) },
    '../gitLivePaths': livePaths, '../latest': latest,
  };
  imports['lib/api/client'] = client;
  const { git } = loadSource(new URL('../src/lib/stores/git.svelte.ts', import.meta.url), imports, {
    document: { hidden: false, hasFocus: () => true },
    setTimeout: () => 0, clearTimeout: () => {},
  }, { relative: true });
  git.openRepoTab('one'); // its arrival reads the overview
  git.ensureStatus('one'); // the tab strip asks meanwhile: the overview will bring it…
  assert.equal(gets.filter((u) => u.includes('/status')).length, 0);
  overview.reject(new Error('404 — an older daemon'));
  await flush();
  // …and when it fails, the status is read instead of staying empty.
  assert.equal(gets.filter((u) => u.includes('/status')).length, 1);
  assert.equal(git.statusById.one?.branch, 'main');
});
