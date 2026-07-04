// gitscan tests against a real scripted temp git repo.
// Run: node --test test/gitscan.test.js
const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const { buildIndex, branchOfMerge } = require('../lib/gitscan.js');

let repoDir;

function git(args, env = {}) {
  return execFileSync('git', ['-C', repoDir, ...args], {
    encoding: 'utf8',
    env: { ...process.env, GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't@t', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't@t', ...env },
  });
}

function commit(msg, when, author) {
  fs.appendFileSync(path.join(repoDir, 'f.txt'), msg + '\n');
  git(['add', '.']);
  git(['commit', '-q', '-m', msg], {
    GIT_AUTHOR_DATE: when,
    GIT_COMMITTER_DATE: when,
    ...(author ? { GIT_AUTHOR_NAME: author, GIT_AUTHOR_EMAIL: `${author.replace(/\s+/g, '.').toLowerCase()}@x` } : {}),
  });
}

const at = (when) => ({ GIT_AUTHOR_DATE: when, GIT_COMMITTER_DATE: when });

before(() => {
  repoDir = fs.mkdtempSync(path.join(os.tmpdir(), 'tp-gitscan-'));
  git(['init', '-q', '-b', 'main']);
  commit('init', '2026-06-01T09:00:00Z');
  git(['checkout', '-q', '-b', 'develop']);
  commit('base develop', '2026-06-01T10:00:00Z');

  // TP-1: two-author feature branch merged into develop (multi-dev).
  git(['checkout', '-q', '-b', 'feature/TP-1-thing']);
  commit('TP-1: implement the thing', '2026-06-02T12:00:00Z', 'Alice A');
  commit('TP-1: polish', '2026-06-03T12:00:00Z', 'Bob B');
  git(['checkout', '-q', 'develop']);
  git(['merge', '-q', '--no-ff', '-m', "Merge branch 'feature/TP-1-thing' into develop", 'feature/TP-1-thing'], at('2026-06-05T11:00:00Z'));

  // TP-1 fix flows to a release branch, then the release is tagged -DEPLOYED.
  git(['checkout', '-q', '-b', 'release/1.0.0']);
  commit('TP-1: fix on release', '2026-06-09T10:00:00Z', 'Alice A');
  git(['tag', '-a', '1.0.0-abc-DEPLOYED', '-m', 'prod'], at('2026-06-11T09:00:00Z'));
  git(['checkout', '-q', 'develop']);

  // TP-2: committed on an unmerged branch — first commit only, not delivered.
  git(['checkout', '-q', '-b', 'feature/TP-2-wip']);
  commit('TP-2: wip', '2026-06-08T09:00:00Z');
  git(['checkout', '-q', 'develop']);

  // TP-3: only on main (not on develop) — with develop present it is NOT delivered.
  git(['checkout', '-q', 'main']);
  commit('TP-3: hotpatch directly on main', '2026-06-09T09:00:00Z');
  git(['checkout', '-q', 'develop']);

  // TP-4: direct commit to develop (no PR) — the direct commit IS the delivery.
  commit('TP-4 tiny version bump', '2026-06-10T09:00:00Z', 'Alice A');

  // A keyless feature branch (no Jira story) — feature extraction material.
  git(['checkout', '-q', '-b', 'feature/nightly-automation-suite']);
  commit('add nightly runner', '2026-06-12T10:00:00Z', 'Carol C');
  commit('wire reports', '2026-06-13T10:00:00Z', 'Carol C');
  git(['checkout', '-q', 'develop']);
  git(['merge', '-q', '--no-ff', '-m', "Merged in feature/nightly-automation-suite (pull request #7)", 'feature/nightly-automation-suite'], at('2026-06-15T09:00:00Z'));
  git(['tag', '-a', '1.1.0-def-deployed', '-m', 'prod'], at('2026-06-16T09:00:00Z')); // lower-case tag also matches
});

after(() => {
  fs.rmSync(repoDir, { recursive: true, force: true });
});

const IDX = () => buildIndex([{ name: 'fix', path: repoDir }], { git_fetch: false, feature_repos: ['fix'] });

test('buildIndex: merged feature is done at the merge event (subject carries the key)', () => {
  const idx = IDX();
  const e = idx.byKey.get('TP-1');
  assert.ok(e, 'TP-1 indexed');
  assert.equal(new Date(e.first_commit_at).toISOString(), '2026-06-02T12:00:00.000Z');
  assert.equal(new Date(e.done_git_at).toISOString(), '2026-06-05T11:00:00.000Z', 'done = merge commit, not first commit');
  assert.equal(idx.target_used.fix, 'develop');
});

test('buildIndex: release-branch commits after done are fixes; deploy tag dates the prod release', () => {
  const e = IDX().byKey.get('TP-1');
  assert.equal(e.fix_count, 1, 'release fix counted');
  assert.equal(new Date(e.last_fix_at).toISOString(), '2026-06-09T10:00:00.000Z');
  assert.equal(new Date(e.deployed_at).toISOString(), '2026-06-11T09:00:00.000Z', 'deployed at the tag creatordate');
});

test('buildIndex: authorship is per non-merge commit (multi-dev)', () => {
  const e = IDX().byKey.get('TP-1');
  const names = e.authors.map((a) => a.name).sort();
  assert.deepEqual(names, ['Alice A', 'Bob B']);
  const alice = e.authors.find((a) => a.name === 'Alice A');
  assert.equal(alice.commits, 2, 'feature commit + release fix');
});

test('buildIndex: unmerged branch -> first commit only, no done', () => {
  const e = IDX().byKey.get('TP-2');
  assert.ok(e.first_commit_at, 'first commit tracked');
  assert.equal(e.done_git_at, null);
  assert.equal(e.deployed_at, null);
});

test('buildIndex: main-only commit is not delivered when develop is the target', () => {
  const e = IDX().byKey.get('TP-3');
  assert.ok(e, 'seen via --all');
  assert.equal(e.done_git_at, null);
});

test('buildIndex: direct develop commit (no PR) is its own delivery', () => {
  const e = IDX().byKey.get('TP-4');
  assert.equal(new Date(e.done_git_at).toISOString(), '2026-06-10T09:00:00.000Z');
  assert.equal(e.fix_count, 0);
});

test('featureIndex: keyless merged branch becomes a git-only feature with authors + deploy tag', () => {
  const idx = IDX();
  const feat = idx.features.find((f) => f.branch === 'feature/nightly-automation-suite');
  assert.ok(feat, `feature extracted (got: ${idx.features.map((f) => f.branch).join(', ')})`);
  assert.deepEqual(feat.jira_keys, []);
  assert.equal(feat.commit_count, 2);
  assert.equal(new Date(feat.first_commit_at).toISOString(), '2026-06-12T10:00:00.000Z');
  assert.equal(new Date(feat.merged_at).toISOString(), '2026-06-15T09:00:00.000Z');
  assert.equal(feat.authors[0].name, 'Carol C');
  assert.equal(new Date(feat.deployed_at).toISOString(), '2026-06-16T09:00:00.000Z', 'case-insensitive -deployed tag');
  assert.equal(feat.summary, 'nightly automation suite');
});

test('featureIndex: keyed feature branches carry their jira keys (excluded from the keyless view)', () => {
  const feat = IDX().features.find((f) => f.branch === 'feature/TP-1-thing');
  assert.ok(feat);
  assert.deepEqual(feat.jira_keys, ['TP-1']);
});

test('branchOfMerge: recognizes Bitbucket/GitHub/plain merges, ignores merge-backs', () => {
  assert.equal(branchOfMerge("Merged in feature/ABC-1-x (pull request #241)"), 'feature/ABC-1-x');
  assert.equal(branchOfMerge("Merge pull request #3 from org/feature/thing"), 'feature/thing');
  assert.equal(branchOfMerge("Merge branch 'bugfix/leak'"), 'bugfix/leak');
  assert.equal(branchOfMerge("Merge branch 'develop'"), null);
  assert.equal(branchOfMerge("Merge branch 'release/5.02.03' into develop"), null);
  assert.equal(branchOfMerge('ABC-1: normal commit'), null);
});

test('buildIndex: depth 0 walks the full history', () => {
  const idx = buildIndex([{ name: 'fix', path: repoDir }], { git_fetch: false, git_depth: 0 });
  assert.ok(idx.byKey.get('TP-1'), 'unbounded log still finds everything');
});

test('placeholder keys (ABC-0000 style) become per-author unscoped fix work, not issues', () => {
  // TP-0000 commit exists? add one on develop now.
  git(['checkout', '-q', 'develop']);
  commit('TP-0000 hotfix prod issue', '2026-06-25T10:00:00Z', 'Dave D');
  const idx = IDX();
  assert.ok(!idx.byKey.has('TP-0000'), 'placeholder is not an issue');
  const dave = idx.unscoped.find((u) => u.name === 'Dave D');
  assert.ok(dave, 'unscoped author tracked');
  assert.equal(dave.commits, 1);
  assert.equal(dave.monthly['2026-06'], 1);
});

test('feature repos: direct-to-master commits become per-author repo activity', () => {
  git(['checkout', '-q', 'develop']);
  commit('add login smoke test', '2026-06-26T10:00:00Z', 'Quinn QA');
  commit('add checkout smoke test', '2026-06-27T10:00:00Z', 'Quinn QA');
  const idx = IDX(); // feature_repos: ['fix']
  const quinn = (idx.repo_activity || []).find((a) => a.name === 'Quinn QA');
  assert.ok(quinn, `repo activity tracked (${JSON.stringify((idx.repo_activity||[]).map(a=>a.name))})`);
  assert.ok(quinn.commits >= 2);
  assert.equal(quinn.repos.fix >= 2, true);
  assert.ok(quinn.monthly['2026-06'] >= 2);
  // Non-feature repo: no repo_activity collected.
  const plain = buildIndex([{ name: 'fix', path: repoDir }], { git_fetch: false });
  assert.equal((plain.repo_activity || []).length, 0);
});
