import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';
import * as livePaths from '../src/lib/gitLivePaths.ts';
import * as latest from '../src/lib/latest.ts';

// S13-03: switching A→B must never leave repo A's status under repo B.

const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };
const status = (branch: string) => ({ branch, upstream: null, ahead: 0, behind: 0, changes: [] });

function fixture() {
  const calls: { url: string; result: ReturnType<typeof deferred<any>> }[] = [];
  const call = (url: string) => {
    const result = deferred<any>();
    calls.push({ url, result });
    return result.promise;
  };
  // The store's typed client (lib/api/git.ts) runs for real over the same
  // transport stub: `relative` loads it, `lib/api/client` is its import.
  const imports: Record<string, unknown> = {
    '../api/client': { api: { get: call, post: call } },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../gitLivePaths': livePaths,
    '../latest': latest,
  };
  imports['lib/api/client'] = imports['../api/client'];
  const { git } = loadSource(new URL('../src/lib/stores/git.svelte.ts', import.meta.url), imports, { document: { hidden: false, hasFocus: () => true, addEventListener() {}, removeEventListener() {} }, setTimeout, clearTimeout }, { relative: true });
  const find = (url: string) => {
    const c = calls.find((x) => x.url === url);
    if (!c) throw new Error(`no call ${url}: ${calls.map((x) => x.url).join(' ')}`);
    return c.result;
  };
  return { git, calls, find };
}

const A = { id: 'A', name: 'a' };
const B = { id: 'B', name: 'b' };

test('out-of-order selectPrimary keeps the latest repo\'s status', async () => {
  const { git, find } = fixture();
  void git.selectPrimary(A);
  void git.selectPrimary(B);
  find('/repos/B/status').resolve(status('b-branch'));
  await flush();
  // A's slow response lands last.
  find('/repos/A/status').resolve(status('a-branch'));
  await flush();
  assert.equal(git.primary.id, 'B');
  assert.equal(git.primaryStatus.branch, 'b-branch');
  assert.equal(git.statusById.A?.branch, 'a-branch', 'A still lands in the per-repo cache');
});

test('a stale failed status for a superseded repo does not clear the primary\'s', async () => {
  const { git, find } = fixture();
  void git.selectPrimary(A);
  void git.selectPrimary(B);
  find('/repos/B/status').resolve(status('b-branch'));
  await flush();
  find('/repos/A/status').reject(new Error('gone'));
  await flush();
  assert.equal(git.primary.id, 'B');
  assert.equal(git.primaryStatus.branch, 'b-branch');
});

// maintainability-3-01: nothing renders a store-level PR list (PrList and
// FocusView fetch their own), so selecting a repo must not spend a forge call.
test('selecting a primary repo does not fetch its pull requests', async () => {
  const { git, calls, find } = fixture();
  void git.selectPrimary(A);
  find('/repos/A/status').resolve(status('main'));
  await flush();
  const before = calls.length;
  const refreshed = git.refreshPrimary();
  for (const c of calls.slice(before)) c.result.resolve(status('main'));
  await refreshed;
  await flush();
  assert.deepEqual(calls.filter((c) => c.url.includes('/prs')).map((c) => c.url), []);
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
