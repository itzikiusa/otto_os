// Git blame rework worker (lib/rework.js) against a throwaway git repo.
// Run (from the plugin dir): node --test
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { execFileSync } = require('child_process');

const R = require('../lib/rework.js');

const DAY = 86400;
const NOW = Math.floor(Date.now() / 1000);

function makeRepo() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tp-rework-'));
  const g = (args, at) => execFileSync('git', args, {
    cwd: dir,
    stdio: 'pipe',
    env: {
      ...process.env,
      GIT_AUTHOR_NAME: 'Dev', GIT_AUTHOR_EMAIL: 'dev@example.com',
      GIT_COMMITTER_NAME: 'Dev', GIT_COMMITTER_EMAIL: 'dev@example.com',
      ...(at ? { GIT_AUTHOR_DATE: `@${at} +0000`, GIT_COMMITTER_DATE: `@${at} +0000` } : {}),
    },
  }).toString();
  const write = (f, lines) => fs.writeFileSync(path.join(dir, f), lines.join('\n') + '\n');
  const commit = (msg, at) => { g(['add', '-A']); g(['commit', '-q', '-m', msg], at); return g(['rev-parse', 'HEAD']).trim(); };
  g(['init', '-q', '-b', 'develop']);
  return { dir, g, write, commit };
}

function scenario() {
  const r = makeRepo();
  // ABC-0: legacy code, well outside the 120-day window.
  r.write('old.txt', ['o1', 'o2', 'o3', 'o4']);
  r.commit('ABC-0 legacy module', NOW - 300 * DAY);
  // ABC-1 writes 10 lines.
  const a = Array.from({ length: 10 }, (_, i) => `a${i + 1}`);
  r.write('a.txt', a);
  r.commit('ABC-1 new feature', NOW - 40 * DAY);
  // ABC-1 iterates on its own lines 9-10 (self churn, never rework).
  r.write('a.txt', [...a.slice(0, 8), 'a9b', 'a10b']);
  r.commit('ABC-1 polish', NOW - 39 * DAY);
  // ABC-2 rewrites 6 of ABC-1's lines inside the window.
  r.write('a.txt', ['b1', 'b2', 'b3', 'b4', 'b5', 'b6', 'a7', 'a8', 'a9b', 'a10b']);
  const abc2 = r.commit('ABC-2 rework of the feature', NOW - 20 * DAY);
  // ABC-3 rewrites 3 lines of the old module (older than the window).
  r.write('old.txt', ['c1', 'c2', 'c3', 'o4']);
  r.commit('ABC-3 refresh legacy', NOW - 10 * DAY);
  r.g(['update-ref', 'refs/remotes/origin/develop', 'HEAD']);
  return { ...r, abc2 };
}

const SINCE = new Date((NOW - 100 * DAY) * 1000).toISOString().slice(0, 10);

test('rework: exact blame charges, self-rewrite excluded, older code not rework', async () => {
  const r = scenario();
  const out = await R.run({ repos: [r.dir], since: SINCE, recent_days: 120 });
  assert.deepEqual(Object.keys(out.pairs), ['ABC-2>ABC-1']);
  assert.equal(out.pairs['ABC-2>ABC-1'].lines, 6);
  assert.equal(out.perKey['ABC-1'].reworkedBy, 6);
  assert.equal(out.perKey['ABC-1'].self, 2);
  assert.equal(out.perKey['ABC-1'].reworkOther, 0);
  assert.equal(out.perKey['ABC-2'].reworkOther, 6);
  assert.equal(out.perKey['ABC-3'].older, 3);
  assert.equal(out.perKey['ABC-3'].reworkOther, 0);
  assert.equal(out.perKey['ABC-0'], undefined, 'commits before SINCE are not scanned');
});

test('rework: recent_days narrows the window without re-blaming (cache reused)', async () => {
  const r = scenario();
  const cache = path.join(r.dir, '..', `${path.basename(r.dir)}-rework-cache.json`);
  const first = await R.run({ repos: [r.dir], since: SINCE, cache_path: cache });
  assert.ok(first.stats.processed >= 4);
  assert.equal(first.recent_days, 120);
  const narrow = await R.run({ repos: [r.dir], since: SINCE, cache_path: cache }, ['--recent-days=10']);
  assert.equal(narrow.stats.processed, 0, 'second run blames nothing new');
  assert.equal(narrow.recent_days, 10);
  assert.deepEqual(narrow.pairs, {}, 'ABC-1 code is 20 days old at rewrite → outside a 10-day window');
  const cached = JSON.parse(fs.readFileSync(cache, 'utf8'));
  assert.equal(Object.keys(cached.repos[r.dir]).length, first.stats.processed);
});

test('rework: only mainline + release refs, cherry-picks deduped by patch-id', async () => {
  const r = scenario();
  // An abandoned feature branch never reaches the mainline: not scanned.
  r.g(['checkout', '-q', '-b', 'feature/x']);
  r.write('a.txt', ['z1', 'z2', 'z3', 'z4', 'z5', 'z6', 'z7', 'z8', 'z9', 'z10']);
  r.commit('ABC-9 abandoned rewrite', NOW - 5 * DAY);
  r.g(['update-ref', 'refs/remotes/origin/feature/x', 'HEAD']);
  // ABC-2 cherry-picked onto a release branch.
  r.g(['checkout', '-q', '-b', 'release/1', 'develop~2']);
  r.g(['cherry-pick', r.abc2], NOW - 15 * DAY);
  r.g(['update-ref', 'refs/remotes/origin/release/1', 'HEAD']);
  const out = await R.run({ repos: [r.dir], since: SINCE });
  assert.equal(out.perKey['ABC-9'], undefined);
  assert.equal(out.pairs['ABC-2>ABC-1'].lines, 6, 'cherry-pick not double counted');
  assert.equal(out.perKey['ABC-2'].commits, 1);
  assert.ok(out.stats.duplicates >= 1);
});

test('rework: resolveOptions precedence argv > input > config > default', () => {
  assert.equal(R.resolveOptions({}).recentDays, 120);
  assert.equal(R.resolveOptions({ config: { rework: { recent_days: 60, since: '2026-01-01' } } }).since, '2026-01-01');
  assert.equal(R.resolveOptions({ recent_days: 30, config: { rework: { recent_days: 60 } } }).recentDays, 30);
  assert.equal(R.resolveOptions({ recent_days: 30 }, ['--recent-days=7']).recentDays, 7);
});
