import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { RefBranch, WorktreeInfo } from '../src/lib/api/types';
import {
  describeRemoteTargets,
  isUnmerged,
  planBranchDelete,
  protectionReason,
  relativeAge,
  resultLine,
  splitPatch,
  stashTitle,
  summarizeDelete,
  worktreeFacts,
  worktreeRemovePlan,
  worktreeSummary,
} from '../src/modules/git/gitopsPlan.ts';

const wt = (over: Partial<WorktreeInfo> = {}): WorktreeInfo => ({
  path: '/src/repo-feat',
  head: 'abc',
  branch: 'feat',
  is_main: false,
  locked: false,
  lock_reason: null,
  prunable: false,
  dirty: false,
  dirty_known: true,
  changes: 0,
  untracked: 0,
  on_remote: true,
  upstream: 'origin/feat',
  ahead: 0,
  behind: 0,
  base: 'origin/main',
  unmerged: 0,
  unpushed: 0,
  risks: [],
  ...over,
});

const br = (name: string, over: Partial<RefBranch> = {}): RefBranch => ({
  name,
  is_current: false,
  upstream: null,
  remote: false,
  ...over,
});

test('a clean worktree removes with a plain confirm that keeps the branch', () => {
  const p = worktreeRemovePlan(wt());
  assert.equal(p.blocked, false);
  assert.equal(p.force, false);
  assert.equal(p.confirmLabel, 'Remove');
  assert.match(p.message, /branch “feat” and its commits are kept/);
});

test('a worktree holding work needs force and names every risk', () => {
  const risks = ['2 uncommitted changes', '1 untracked file', '3 commits not pushed to any remote and not in origin/main'];
  const p = worktreeRemovePlan(wt({ changes: 2, untracked: 1, unpushed: 3, risks }));
  assert.equal(p.force, true);
  assert.equal(p.confirmLabel, 'Force remove');
  for (const r of risks) assert.ok(p.message.includes(`• ${r}`), r);
});

test('unknown status blocks removal instead of asking for force', () => {
  const p = worktreeRemovePlan(wt({ dirty_known: false, risks: ['Otto can’t verify it belongs to this repo, so it won’t remove it'] }));
  assert.equal(p.blocked, true);
  assert.match(p.message, /verify/);
  assert.equal(worktreeRemovePlan(wt({ dirty_known: false })).blocked, true);
});

test('worktree facts and summary flag local-only branches and unpushed work', () => {
  const w = wt({ upstream: null, on_remote: false, unpushed: 2, unmerged: 2, changes: 1, untracked: 4 });
  const f = worktreeFacts(w);
  assert.deepEqual([f.changes, f.untracked, f.unpushed, f.localOnly, f.unknown], [1, 4, 2, true, false]);
  const s = worktreeSummary(w);
  assert.match(s, /1 uncommitted change/);
  assert.match(s, /4 untracked files/);
  assert.match(s, /branch not on any remote/);
  assert.match(s, /2 commits exist only here/);
  assert.match(s, /2 commits not in origin\/main/);
  assert.match(worktreeSummary(wt({ ahead: 3 })), /3 commits ahead of origin\/feat/);
  assert.equal(worktreeFacts(wt({ is_main: true, on_remote: false })).localOnly, false);
  assert.match(worktreeSummary(wt({ prunable: true })), /folder is gone/);
});

test('current, default and worktree-held branches are protected', () => {
  const ctx = { current: 'feat/a', base: 'develop', inWorktree: new Set(['feat/wt']) };
  assert.equal(protectionReason(br('feat/a', { is_current: true }), ctx), 'checked out');
  assert.equal(protectionReason(br('main'), ctx), 'default branch');
  assert.equal(protectionReason(br('master'), ctx), 'default branch');
  assert.equal(protectionReason(br('develop'), ctx), 'default branch');
  assert.equal(protectionReason(br('origin/main', { remote: true }), ctx), 'default branch');
  assert.equal(protectionReason(br('feat/wt'), ctx), 'checked out in a worktree');
  assert.equal(protectionReason(br('origin/feat/a', { remote: true }), ctx), null, 'the remote copy of the current branch may go');
  assert.equal(protectionReason(br('feat/b'), ctx), null);
});

test('the delete plan forces only unmerged locals and lists exact remote targets', () => {
  const merged = br('done', { upstream: 'origin/done', merged_into_base: true });
  const unmerged = br('wip', { upstream: 'fork/wip-renamed' });
  const remoteOnly = br('upstream/old', { remote: true });
  const dup = br('origin/done', { remote: true });
  assert.equal(isUnmerged(merged), false);
  assert.equal(isUnmerged(unmerged), true);

  const plan = planBranchDelete([merged, unmerged, remoteOnly, dup], { alsoRemote: true, forceUnmerged: true });
  assert.deepEqual(plan.forced, ['wip']);
  assert.deepEqual(plan.items, [
    { name: 'done', remote: 'origin' },
    { name: 'wip', force: true, remote: 'fork', remote_branch: 'wip-renamed' },
    { name: 'old', local: false, remote: 'upstream' },
  ]);
  assert.equal(describeRemoteTargets(plan.remoteTargets), 'origin: done\nfork: wip-renamed\nupstream: old');

  const localOnly = planBranchDelete([merged, unmerged], { alsoRemote: false, forceUnmerged: false });
  assert.deepEqual(localOnly.items, [{ name: 'done' }, { name: 'wip' }]);
  assert.equal(localOnly.remoteTargets.size, 0);
});

test('delete results summarize per branch', () => {
  const rs = [
    { name: 'done', remote: 'origin', local_deleted: true, remote_deleted: true, error: null, needs_force: false },
    { name: 'wip', remote: null, local_deleted: false, remote_deleted: false, error: "error: the branch 'wip' is not fully merged", needs_force: true },
    { name: 'x', remote: 'origin', local_deleted: true, remote_deleted: false, error: 'remote rejected', needs_force: false },
  ];
  const s = summarizeDelete(rs);
  assert.equal(s.okCount, 1);
  assert.deepEqual(s.needForce, ['wip']);
  assert.equal(s.failed.length, 2);
  assert.equal(resultLine(rs[0]), 'local deleted · deleted on origin');
  assert.equal(resultLine(rs[1]), 'not fully merged — needs force');
  assert.equal(resultLine(rs[2]), 'local deleted · remote rejected');
});

test('stash helpers: age, title and per-file patch split', () => {
  const now = Date.parse('2026-11-03T12:00:00Z');
  assert.equal(relativeAge('2026-11-03T11:59:30Z', now), 'just now');
  assert.equal(relativeAge('2026-11-03T11:30:00Z', now), '30 min ago');
  assert.equal(relativeAge('2026-11-03T09:00:00Z', now), '3 h ago');
  assert.equal(relativeAge('2026-10-31T12:00:00Z', now), '3 d ago');
  assert.equal(relativeAge('2026-01-02T12:00:00Z', now), '2026-01-02');
  assert.equal(relativeAge('nope', now), '');
  assert.equal(stashTitle('On main: my wip'), 'my wip');
  assert.equal(stashTitle('WIP on feat/x: abc123 subject'), 'abc123 subject');
  const patch = 'diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/dir/u.txt b/dir/u.txt\nnew file mode 100644\n+x\n';
  const parts = splitPatch(patch);
  assert.deepEqual(parts.map((p) => p.path), ['a.txt', 'dir/u.txt']);
  assert.ok(parts[1].text.startsWith('diff --git a/dir/u.txt'));
  assert.deepEqual(splitPatch(''), []);
});
