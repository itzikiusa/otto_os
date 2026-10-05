import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';
import * as livePaths from '../src/lib/gitLivePaths.ts';
import * as latest from '../src/lib/latest.ts';

// S13-03: switching A→B must never leave repo A's status / PRs under repo B.

const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };
const status = (branch: string) => ({ branch, upstream: null, ahead: 0, behind: 0, changes: [] });

function fixture() {
  const calls: { url: string; result: ReturnType<typeof deferred<any>> }[] = [];
  const call = (url: string) => {
    const result = deferred<any>();
    calls.push({ url, result });
    return result.promise;
  };
  const { git } = loadSource(new URL('../src/lib/stores/git.svelte.ts', import.meta.url), {
    '../api/client': { api: { get: call, post: call } },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../gitLivePaths': livePaths,
    '../latest': latest,
  }, { document: { hidden: false, hasFocus: () => true, addEventListener() {}, removeEventListener() {} }, setTimeout, clearTimeout });
  const find = (url: string) => {
    const c = calls.find((x) => x.url === url);
    if (!c) throw new Error(`no call ${url}: ${calls.map((x) => x.url).join(' ')}`);
    return c.result;
  };
  return { git, calls, find };
}

const A = { id: 'A', name: 'a' };
const B = { id: 'B', name: 'b' };

test('out-of-order selectPrimary keeps the latest repo\'s status and PRs', async () => {
  const { git, find } = fixture();
  void git.selectPrimary(A);
  void git.selectPrimary(B);
  find('/repos/B/status').resolve(status('b-branch'));
  await flush();
  find('/repos/B/prs?state=open').resolve({ items: [{ number: 2 }] });
  await flush();
  // A's slow responses land last.
  find('/repos/A/status').resolve(status('a-branch'));
  await flush();
  assert.equal(git.primary.id, 'B');
  assert.equal(git.primaryStatus.branch, 'b-branch');
  assert.equal(git.statusById.A?.branch, 'a-branch', 'A still lands in the per-repo cache');
  assert.deepEqual(git.prs.map((p: any) => p.number), [2]);
});

test('a stale PR list for a repo that is no longer primary is dropped', async () => {
  const { git, find } = fixture();
  git.primary = A;
  void git.loadPrs('A');
  git.primary = B;
  void git.loadPrs('B');
  find('/repos/B/prs?state=open').resolve({ items: [{ number: 2 }] });
  await flush();
  find('/repos/A/prs?state=open').resolve({ items: [{ number: 1 }] });
  await flush();
  assert.deepEqual(git.prs.map((p: any) => p.number), [2]);
  assert.equal(git.prsLoading, false);
});

test('workspace switch: the older repo list never overwrites the newer one', async () => {
  const { git, find } = fixture();
  void git.loadRepos('w1');
  void git.loadRepos('w2');
  find('/workspaces/w2/repos').resolve([B]);
  await flush();
  find('/workspaces/w1/repos').resolve([A]);
  await flush();
  assert.deepEqual(git.repos.map((r: any) => r.id), ['B']);
  assert.equal(git.primary.id, 'B');
});

test('a failed loadRepos is not sticky: the next call retries', async () => {
  const { git, calls, find } = fixture();
  void git.loadRepos('w1');
  find('/workspaces/w1/repos').reject(new Error('down'));
  await flush();
  void git.loadRepos('w1');
  assert.equal(calls.filter((c) => c.url === '/workspaces/w1/repos').length, 2);
});
