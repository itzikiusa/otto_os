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

test('buildIndex: a stray commit long after delivery is NOT a fix (fix window)', () => {
  // add a commit mentioning TP-1 ~90 days after its merge (2026-06-05).
  git(['checkout', '-q', 'develop']);
  commit('TP-1: unrelated later touch', '2026-09-10T10:00:00Z', 'Alice A');
  git(['merge', '-q', '--no-ff', '-m', "Merge branch 'develop'", 'develop'], at('2026-09-10T10:05:00Z'));
  const def = buildIndex([{ name: 'fix', path: repoDir }], { git_fetch: false }).byKey.get('TP-1');
  assert.equal(def.fix_count, 1, 'only the in-window release fix counts (default 30d)');
  assert.ok(def.late_touches >= 1, 'the 90-day-later touch is a late_touch, not a fix');
  const wide = buildIndex([{ name: 'fix', path: repoDir }], { git_fetch: false, fix_window_days: 200 }).byKey.get('TP-1');
  assert.equal(wide.fix_count, 2, 'a wider window includes the later commit');
});

// ---------------------------------------------------------------------------
// Deploy-tag rule (deployed / hf / hotfix), hotfix-first deployment, target pref
// ---------------------------------------------------------------------------
const { isDeployTag, tagKind, deployTags, deployTagPatterns, resolveTarget, targetRefAgeDays } = require('../lib/gitscan.js');

test('isDeployTag: case-insensitive CONTAINS of deployed / hf / hotfix; tagKind', () => {
  const deploys = ['hotfixes-2026', 'prodHotfix1', 'v2hf', 'rel-HF2', 'v1-DEPLOYED'];
  for (const n of deploys) assert.equal(isDeployTag(n), true, n);
  assert.equal(isDeployTag('release-1.2'), false);
  assert.equal(isDeployTag(''), false);
  assert.deepEqual(deploys.map(tagKind), ['hotfix', 'hotfix', 'hotfix', 'hotfix', 'deploy']);
  assert.equal(tagKind('release-1.2'), 'deploy');
  // Config object or list both accepted; an override replaces the defaults.
  assert.equal(isDeployTag('v1-PROD', { deploy_tag_patterns: ['prod'] }), true);
  assert.equal(isDeployTag('v1-deployed', { deploy_tag_patterns: ['prod'] }), false);
  assert.equal(isDeployTag('v1-deployed', ['deployed']), true);
});

test('deployTagPatterns: legacy single key merged, defaults when absent', () => {
  assert.deepEqual(deployTagPatterns({}), ['deployed', 'hf', 'hotfix']);
  assert.deepEqual(deployTagPatterns({ deploy_tag_pattern: 'PROD' }), ['deployed', 'hf', 'hotfix', 'prod']);
  assert.deepEqual(deployTagPatterns({ deploy_tag_patterns: ['Deployed'], deploy_tag_pattern: 'deployed' }), ['deployed']);
});

test('deploy tags: exact matches in a fixture repo, hf tag sets deployed_at', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tp-tags-'));
  const g = (args, env = {}) =>
    execFileSync('git', ['-C', dir, ...args], {
      encoding: 'utf8',
      env: { ...process.env, GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't@t', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't@t', ...env },
    });
  const c = (msg, when) => {
    fs.appendFileSync(path.join(dir, 'f.txt'), msg + '\n');
    g(['add', '.']);
    g(['commit', '-q', '-m', msg], at(when));
  };
  try {
    g(['init', '-q', '-b', 'develop']);
    c('ABC-1 first', '2026-06-01T09:00:00Z');
    g(['tag', '-a', 'v1-DEPLOYED', '-m', 'p'], at('2026-06-02T09:00:00Z'));
    c('ABC-2 urgent', '2026-06-03T09:00:00Z');
    g(['tag', '-a', 'rel-HF2', '-m', 'p'], at('2026-06-03T15:00:00Z'));
    c('ABC-3 more', '2026-06-04T09:00:00Z');
    g(['tag', 'prod_Hotfix_3'], at('2026-06-04T09:00:00Z')); // lightweight
    g(['tag', '-a', 'release-1.0', '-m', 'p'], at('2026-06-05T09:00:00Z'));
    g(['tag', '-a', 'nightly-build', '-m', 'p'], at('2026-06-05T09:00:00Z'));
    c('ABC-4 later', '2026-06-06T09:00:00Z');
    g(['tag', '-a', 'v2-deployed', '-m', 'p'], at('2026-06-07T09:00:00Z'));

    const tags = deployTags(dir, ['deployed', 'hf', 'hotfix']);
    assert.deepEqual(tags.map((t) => t.name), ['v1-DEPLOYED', 'rel-HF2', 'prod_Hotfix_3', 'v2-deployed']);
    assert.deepEqual(tags.map((t) => t.kind), ['deploy', 'hotfix', 'hotfix', 'deploy']);

    const idx = buildIndex([{ name: 'r', path: dir }], { git_fetch: false });
    const e2 = idx.byKey.get('ABC-2');
    assert.equal(e2.deployed_at, Date.parse('2026-06-03T15:00:00Z'), 'first reached by the hf tag');
    assert.equal(e2.deployed_tag, 'rel-HF2');
    assert.equal(e2.deployed_kind, 'hotfix');
    assert.equal(idx.byKey.get('ABC-1').deployed_at, Date.parse('2026-06-02T09:00:00Z'));
    assert.equal(idx.byKey.get('ABC-4').deployed_tag, 'v2-deployed');
    assert.deepEqual(idx.deploy_tags.map((t) => t.name), ['v1-DEPLOYED', 'rel-HF2', 'prod_Hotfix_3', 'v2-deployed']);
    assert.equal(typeof idx.target_ref_age_days.r, 'number');
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('resolveTarget: origin/develop beats a stale local develop; age is recorded', () => {
  const up = fs.mkdtempSync(path.join(os.tmpdir(), 'tp-up-'));
  const clone = fs.mkdtempSync(path.join(os.tmpdir(), 'tp-clone-'));
  const env = { ...process.env, GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't@t', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't@t' };
  const g = (d, args, e = {}) => execFileSync('git', ['-C', d, ...args], { encoding: 'utf8', env: { ...env, ...e } });
  try {
    g(up, ['init', '-q', '-b', 'develop']);
    fs.writeFileSync(path.join(up, 'a'), '1');
    g(up, ['add', '.']);
    g(up, ['commit', '-q', '-m', 'ABC-10 old'], at('2026-01-01T09:00:00Z'));
    execFileSync('git', ['clone', '-q', up, clone], { env });
    fs.writeFileSync(path.join(up, 'a'), '2');
    g(up, ['commit', '-qam', 'ABC-11 new'], at('2026-06-01T09:00:00Z'));
    g(clone, ['fetch', '-q']); // origin/develop advances; local develop stays stale
    assert.equal(resolveTarget(clone, ['develop', 'main', 'master']), 'origin/develop');
    const idx = buildIndex([{ name: 'c', path: clone }], { git_fetch: false });
    assert.ok(idx.byKey.has('ABC-11'), 'newer remote commit is indexed');
    const age = targetRefAgeDays(clone, 'origin/develop', Date.parse('2026-06-11T09:00:00Z'));
    assert.equal(age, 10);
  } finally {
    fs.rmSync(up, { recursive: true, force: true });
    fs.rmSync(clone, { recursive: true, force: true });
  }
});

// ---------------------------------------------------------------------------
// Deploy attribution: creatordate order, tag.contains, last-commit deploy
// ---------------------------------------------------------------------------
function tagFixture(prefix) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), prefix));
  const g = (args, env = {}) =>
    execFileSync('git', ['-C', dir, ...args], {
      encoding: 'utf8',
      env: { ...process.env, GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't@t', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't@t', ...env },
    });
  const c = (msg, when) => {
    fs.appendFileSync(path.join(dir, 'f.txt'), msg + '\n');
    g(['add', '.']);
    g(['commit', '-q', '-m', msg], at(when));
    return g(['rev-parse', 'HEAD']).trim();
  };
  return { dir, g, c };
}

test('deploy: partial commit in tag N, final commit in tag N+2 → deployed_at is N+2; contains populated', () => {
  const { dir, g, c } = tagFixture('tp-last-');
  try {
    g(['init', '-q', '-b', 'develop']);
    const s1 = c('ABC-1 part one', '2026-06-01T09:00:00Z');
    g(['tag', '-a', 'n1-deployed', '-m', 'p'], at('2026-06-02T09:00:00Z'));
    const s2 = c('ABC-2 other', '2026-06-03T09:00:00Z');
    g(['tag', '-a', 'n2-deployed', '-m', 'p'], at('2026-06-04T09:00:00Z'));
    const s3 = c('ABC-1 final part', '2026-06-05T09:00:00Z');
    g(['tag', '-a', 'n3-deployed', '-m', 'p'], at('2026-06-06T09:00:00Z'));
    const idx = buildIndex([{ name: 'r', path: dir }], { git_fetch: false });
    const e = idx.byKey.get('ABC-1');
    assert.equal(e.deployed_at, Date.parse('2026-06-06T09:00:00Z'));
    assert.equal(e.deployed_tag, 'n3-deployed');
    assert.equal(e.first_deployed_at, Date.parse('2026-06-02T09:00:00Z'));
    assert.equal(e.first_deployed_tag, 'n1-deployed');
    const byName = Object.fromEntries(idx.deploy_tags.map((t) => [t.name, t]));
    assert.deepEqual(byName['n1-deployed'].contains, [s1]);
    assert.deepEqual(byName['n2-deployed'].contains, [s2]);
    assert.deepEqual(byName['n3-deployed'].contains, [s3]);
    assert.deepEqual(idx.matched_tags.r, ['n1-deployed', 'n2-deployed', 'n3-deployed']);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('deploy: a late tag on an old commit is ordered by creatordate, not commit date', () => {
  const { dir, g, c } = tagFixture('tp-late-');
  try {
    g(['init', '-q', '-b', 'develop']);
    const old = c('ABC-5 old work', '2026-06-01T09:00:00Z');
    c('ABC-6 newer work', '2026-06-02T09:00:00Z');
    g(['tag', '-a', 'b-deployed', '-m', 'p'], at('2026-06-03T09:00:00Z')); // ships both
    g(['tag', '-a', 'a-late-hotfix', '-m', 'p', old], at('2026-06-10T09:00:00Z')); // later tag, old commit
    const tags = deployTags(dir, ['deployed', 'hf', 'hotfix']);
    assert.deepEqual(tags.map((t) => t.name), ['b-deployed', 'a-late-hotfix']);
    const idx = buildIndex([{ name: 'r', path: dir }], { git_fetch: false });
    assert.equal(idx.byKey.get('ABC-5').deployed_tag, 'b-deployed', 'already shipped by the earlier-created tag');
    assert.equal(idx.byKey.get('ABC-5').deployed_at, Date.parse('2026-06-03T09:00:00Z'));
    const late = idx.deploy_tags.find((t) => t.name === 'a-late-hotfix');
    assert.deepEqual(late.contains, [], 'nothing new in the late tag');
    assert.equal(late.kind, 'hotfix');
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('deploy: custom hotfix pattern list + hf word boundary', () => {
  assert.equal(tagKind('v1-urgent', { hotfix_tag_patterns: ['urgent'] }), 'hotfix');
  assert.equal(tagKind('v1-hf', { hotfix_tag_patterns: ['urgent'] }), 'deploy');
  assert.equal(isDeployTag('v1-urgent', { hotfix_tag_patterns: ['urgent'] }), true, 'hotfix patterns are deployments');
  assert.equal(isDeployTag('v1-deployed', { hotfix_tag_patterns: ['urgent'] }), true, 'contains-deployed rule kept');
  assert.equal(isDeployTag('pdfhf', { hf_word_boundary: true }), false);
  assert.equal(isDeployTag('rel-HF2', { hf_word_boundary: true }), true);
  assert.equal(isDeployTag('pdfhf'), true, 'default is plain substring');
  const { dir, g, c } = tagFixture('tp-hfpat-');
  try {
    g(['init', '-q', '-b', 'develop']);
    c('ABC-7 fix', '2026-06-01T09:00:00Z');
    g(['tag', '-a', 'prod-urgent-1', '-m', 'p'], at('2026-06-02T09:00:00Z'));
    const idx = buildIndex([{ name: 'r', path: dir }], { git_fetch: false, hotfix_tag_patterns: ['urgent'] });
    assert.equal(idx.byKey.get('ABC-7').deployed_kind, 'hotfix');
    assert.deepEqual(idx.matched_tags.r, ['prod-urgent-1']);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('deploy: bare remote — origin/develop past a stale local develop; remote merge scanned and deployed', () => {
  const env = { ...process.env, GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't@t', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't@t' };
  const bare = fs.mkdtempSync(path.join(os.tmpdir(), 'tp-bare-'));
  const work = fs.mkdtempSync(path.join(os.tmpdir(), 'tp-work-'));
  const clone = fs.mkdtempSync(path.join(os.tmpdir(), 'tp-cl-'));
  const g = (d, args, e = {}) => execFileSync('git', ['-C', d, ...args], { encoding: 'utf8', env: { ...env, ...e } });
  try {
    g(bare, ['init', '-q', '--bare', '-b', 'develop']);
    execFileSync('git', ['clone', '-q', bare, work], { env });
    g(work, ['checkout', '-q', '-b', 'develop']);
    fs.writeFileSync(path.join(work, 'a'), '1');
    g(work, ['add', '.']);
    g(work, ['commit', '-q', '-m', 'ABC-20 base'], at('2026-05-01T09:00:00Z'));
    g(work, ['push', '-q', 'origin', 'develop']);
    execFileSync('git', ['clone', '-q', '-b', 'develop', bare, clone], { env }); // local develop frozen here
    g(work, ['checkout', '-q', '-b', 'feature/ABC-21-x']);
    fs.writeFileSync(path.join(work, 'a'), '2');
    g(work, ['commit', '-qam', 'ABC-21 feature work'], at('2026-06-01T09:00:00Z'));
    g(work, ['checkout', '-q', 'develop']);
    g(work, ['merge', '-q', '--no-ff', '-m', "Merge branch 'feature/ABC-21-x' into develop", 'feature/ABC-21-x'], at('2026-06-02T09:00:00Z'));
    g(work, ['tag', '-a', '2.0-DEPLOYED', '-m', 'p'], at('2026-06-03T09:00:00Z'));
    g(work, ['push', '-q', 'origin', 'develop', '--tags']);
    g(clone, ['fetch', '-q', '--tags']);
    assert.equal(resolveTarget(clone, ['develop']), 'origin/develop');
    const idx = buildIndex([{ name: 'c', path: clone }], { git_fetch: false });
    const e = idx.byKey.get('ABC-21');
    assert.ok(e && e.done_git_at, 'remote merge is scanned as the delivery');
    assert.equal(e.deployed_at, Date.parse('2026-06-03T09:00:00Z'));
    assert.equal(e.deployed_tag, '2.0-DEPLOYED');
    const t = idx.deploy_tags.find((x) => x.name === '2.0-DEPLOYED');
    assert.equal(t.contains.length, 3, 'base + feature commit + merge commit');
  } finally {
    for (const d of [bare, work, clone]) fs.rmSync(d, { recursive: true, force: true });
  }
});
