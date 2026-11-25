import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import * as livePaths from '../src/lib/gitLivePaths.ts';
import * as latest from '../src/lib/latest.ts';

// The Git page has no Changes view any more: History's WIP row is the working
// tree. A remembered "changes" tab, a #/git/<repo>/changes link and an old
// setSubTab caller all resolve to History (which lands on the WIP row).

function load(stored: Record<string, string> = {}) {
  const storage = new Map(Object.entries(stored));
  const never = () => new Promise(() => {});
  const imports: Record<string, unknown> = {
    '../api/client': { api: { get: never, post: never } },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../gitLivePaths': livePaths,
    '../latest': latest,
  };
  imports['lib/api/client'] = imports['../api/client'];
  const { git } = loadSource(new URL('../src/lib/stores/git.svelte.ts', import.meta.url), imports, {
    document: { hidden: false, hasFocus: () => true, addEventListener() {}, removeEventListener() {} },
    localStorage: { getItem: (k: string) => storage.get(k) ?? null, setItem: (k: string, v: string) => void storage.set(k, v), removeItem: (k: string) => void storage.delete(k) },
  }, { relative: true });
  return { git, storage };
}

test('a link or caller naming the old Changes tab opens History', () => {
  const { git } = load();
  git.openRepoTab('A', 'changes');
  assert.equal(git.subTabFor('A'), 'graph');
  git.setSubTab('A', 'prs');
  git.setSubTab('A', 'changes');
  assert.equal(git.subTabFor('A'), 'graph');
});

test('a remembered "changes" tab restores as History', () => {
  const { git, storage } = load({ otto_git_open_tabs: JSON.stringify({ openRepoIds: ['A', 'B'], activeRepoId: 'A', sub: { A: 'changes', B: 'review' } }) });
  git.allRepos = [{ id: 'A', name: 'a', path: '/w/a' }, { id: 'B', name: 'b', path: '/w/b' }];
  git.restoreOpenTabs();
  assert.equal(git.subTabFor('A'), 'graph');
  assert.equal(git.subTabFor('B'), 'review');
  assert.equal(JSON.parse(storage.get('otto_git_open_tabs')!).sub.A, 'graph', 'the stale value is rewritten, not kept');
});
