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

// S13-305: App's loadRepos and GitPanel's detectFor race on a workspace switch.
test('a detect that resolves before the repo list keeps the detected primary', async () => {
  const { git, find } = fixture();
  const R1 = { id: 'R1', workspace_id: 'w2', name: 'first' };
  const R2 = { id: 'R2', workspace_id: 'w2', name: 'session-repo' };
  void git.loadRepos('w2');
  void git.detectFor('w2', '/src/session-repo');
  find('/workspaces/w2/repos/detect').resolve(R2);
  await flush();
  find('/repos/R2/status').resolve(status('feature'));
  await flush();
  // The list predates the detect's registration: it lists R1 only.
  find('/workspaces/w2/repos').resolve([R1]);
  await flush();
  assert.equal(git.primary.id, 'R2', 'still the focused session\'s repo, not repos[0]');
  assert.equal(JSON.stringify(git.repos.map((r: any) => r.id).sort()), '["R1","R2"]');
});

test('a failed repo load surfaces an error and retries on demand', async () => {
  const { git, calls, find } = fixture();
  void git.loadRepos('w1');
  find('/workspaces/w1/repos').reject(new Error('daemon restarting'));
  await flush();
  assert.match(git.reposError, /daemon restarting/);
  const before = calls.length;
  void git.retryRepos('w1');
  assert.equal(calls.length, before + 1, 'Retry fetches again (loadedFor is not sticky)');
  calls[calls.length - 1].result.resolve([{ id: 'R1', workspace_id: 'w1', name: 'r' }]);
  await flush();
  assert.equal(git.reposError, null);
  assert.equal(git.primary.id, 'R1');
});
