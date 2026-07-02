// gitscan tests against a real scripted temp git repo.
// Run: node --test test/gitscan.test.js
const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const { buildIndex } = require('../lib/gitscan.js');

let repoDir;

function git(args, env = {}) {
  return execFileSync('git', ['-C', repoDir, ...args], {
    encoding: 'utf8',
    env: { ...process.env, GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't@t', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't@t', ...env },
  });
}

function commit(msg, when) {
  fs.appendFileSync(path.join(repoDir, 'f.txt'), msg + '\n');
  git(['add', '.']);
  git(['commit', '-q', '-m', msg], { GIT_AUTHOR_DATE: when, GIT_COMMITTER_DATE: when });
}

before(() => {
  repoDir = fs.mkdtempSync(path.join(os.tmpdir(), 'tp-gitscan-'));
  git(['init', '-q', '-b', 'main']);
  commit('init', '2026-06-01T09:00:00Z');
  git(['checkout', '-q', '-b', 'develop']);
  commit('base develop', '2026-06-01T10:00:00Z');

  // TP-1: feature branch merged into develop.
  git(['checkout', '-q', '-b', 'feature/TP-1-thing']);
  commit('TP-1: implement the thing', '2026-06-02T12:00:00Z');
  commit('TP-1: polish', '2026-06-03T12:00:00Z');
  git(['checkout', '-q', 'develop']);
  git(['merge', '-q', '--no-ff', '-m', "Merge branch 'feature/TP-1-thing' into develop", 'feature/TP-1-thing'], {
    GIT_AUTHOR_DATE: '2026-06-05T11:00:00Z',
    GIT_COMMITTER_DATE: '2026-06-05T11:00:00Z',
  });

  // TP-2: committed on an unmerged branch — first commit only, not delivered.
  git(['checkout', '-q', '-b', 'feature/TP-2-wip']);
  commit('TP-2: wip', '2026-06-08T09:00:00Z');
  git(['checkout', '-q', 'develop']);

  // TP-3: only on main (not on develop) — with develop present it is NOT delivered.
  git(['checkout', '-q', 'main']);
  commit('TP-3: hotpatch directly on main', '2026-06-09T09:00:00Z');
  git(['checkout', '-q', 'develop']);
});

after(() => {
  fs.rmSync(repoDir, { recursive: true, force: true });
});

test('buildIndex: merged feature is delivered (merge-commit subject carries the key)', () => {
  const idx = buildIndex([{ name: 'fix', path: repoDir }], {});
  const tp1 = idx.byKey.get('TP-1');
  assert.ok(tp1, 'TP-1 indexed');
  assert.equal(new Date(tp1.first_commit_at).toISOString(), '2026-06-02T12:00:00.000Z');
  // delivered_at = the merge commit on develop (max ts of key-mentioning commits in target history)
  assert.equal(new Date(tp1.delivered_at).toISOString(), '2026-06-05T11:00:00.000Z');
});

test('buildIndex: unmerged branch has first commit but no delivery', () => {
  const idx = buildIndex([{ name: 'fix', path: repoDir }], {});
  const tp2 = idx.byKey.get('TP-2');
  assert.ok(tp2);
  assert.equal(new Date(tp2.first_commit_at).toISOString(), '2026-06-08T09:00:00.000Z');
  assert.equal(tp2.delivered_at, null);
});

test('buildIndex: commit only on main is not delivered when develop exists', () => {
  const idx = buildIndex([{ name: 'fix', path: repoDir }], {});
  const tp3 = idx.byKey.get('TP-3');
  assert.ok(tp3);
  assert.equal(tp3.delivered_at, null);
  assert.equal(idx.target_used.fix, 'develop');
});

test('buildIndex: falls back to main when develop is absent', () => {
  const other = fs.mkdtempSync(path.join(os.tmpdir(), 'tp-gitscan2-'));
  const g = (args, env = {}) =>
    execFileSync('git', ['-C', other, ...args], {
      encoding: 'utf8',
      env: { ...process.env, GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't@t', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't@t', GIT_AUTHOR_DATE: '2026-06-10T10:00:00Z', GIT_COMMITTER_DATE: '2026-06-10T10:00:00Z', ...env },
    });
  g(['init', '-q', '-b', 'main']);
  fs.writeFileSync(path.join(other, 'a.txt'), 'x');
  g(['add', '.']);
  g(['commit', '-q', '-m', 'TP-9: straight to main']);
  const idx = buildIndex([{ name: 'o', path: other }], {});
  assert.equal(new Date(idx.byKey.get('TP-9').delivered_at).toISOString(), '2026-06-10T10:00:00.000Z');
  assert.equal(idx.target_used.o, 'main');
  fs.rmSync(other, { recursive: true, force: true });
});

test('buildIndex: nonexistent repo path is tolerated', () => {
  const idx = buildIndex([{ name: 'gone', path: '/nonexistent/nope' }], {});
  assert.equal(idx.byKey.size, 0);
  assert.equal(idx.hasRepos, false);
});
