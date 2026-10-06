// DORA metrics — pure, deterministic fixtures.
// Run: node --test test/dora.test.js
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { doraMetrics, bandFor } = require('../lib/dora.js');

const T = (s) => Date.parse(s);
const DAY = 86400000;

// 3 deploy tags (Mon / Wed / next Mon) in one repo; the Wed tag is a hotfix.
const tags = [
  { repo: 'svc', name: 'v1-deployed', ts: T('2026-06-01T10:00:00Z') },
  { repo: 'svc', name: 'v1.1-hf', ts: T('2026-06-03T10:00:00Z') },
  { repo: 'svc', name: 'v2-deployed', ts: T('2026-06-08T10:00:00Z') },
];
const records = [
  { key: 'ABC-1', type: 'Story', first_commit_at: T('2026-05-28T10:00:00Z'), deployed_at: T('2026-06-01T10:00:00Z'), deployed_tag: 'v1-deployed' }, // 2 bd
  { key: 'ABC-2', type: 'Story', first_commit_at: T('2026-05-29T10:00:00Z'), deployed_at: T('2026-06-01T10:00:00Z'), deployed_tag: 'v1-deployed' }, // 1 bd
  { key: 'ABC-3', type: 'Task', first_commit_at: T('2026-06-02T10:00:00Z'), deployed_at: T('2026-06-03T10:00:00Z'), deployed_tag: 'v1.1-hf' }, // 1 bd
  { key: 'ABC-4', type: 'Story', first_commit_at: T('2026-06-01T10:00:00Z'), deployed_at: T('2026-06-08T10:00:00Z'), deployed_tag: 'v2-deployed' }, // 5 bd
  { key: 'ABC-5', type: 'Bug', created: T('2026-06-10T10:00:00Z'), rework_of: ['ABC-4'] },
  // Sub-task + epic never feed lead time.
  { key: 'ABC-6', type: 'Sub-task', subtask: true, first_commit_at: T('2026-04-01T10:00:00Z'), deployed_at: T('2026-06-08T10:00:00Z') },
  { key: 'ABC-7', type: 'Epic', first_commit_at: T('2026-04-01T10:00:00Z'), deployed_at: T('2026-06-08T10:00:00Z') },
];
const window = { start: T('2026-06-01T00:00:00Z'), end: T('2026-06-15T00:00:00Z') }; // 2 weeks
const cfg = { failure_window_days: 7, window, timezone: 'UTC' };

test('doraMetrics: exact values on the deterministic fixture', () => {
  const m = doraMetrics(records, tags, cfg);
  assert.equal(m.deploy_frequency.total, 3);
  assert.equal(m.deploy_frequency.per_week, 1.5);
  assert.deepEqual(m.deploy_frequency.by_repo, { svc: 3 });
  assert.deepEqual(m.deploy_frequency.by_kind, { regular: 2, hotfix: 1 });
  assert.equal(m.deploy_frequency.band, 'high');

  assert.equal(m.lead_time.basis, 'ticket'); // no PRs given → ticket fallback
  assert.equal(m.lead_time.n, 4);
  assert.equal(m.lead_time.p50, 1.5);
  assert.equal(m.lead_time.p75, 2.75);
  assert.equal(m.lead_time.p85, 3.65);
  assert.equal(m.ticket_lead_time.n, 4);

  const c = m.change_failure_rate;
  assert.equal(c.failed, 2);
  assert.equal(c.total, 3);
  assert.equal(c.label, '2 of 3');
  assert.equal(c.value, 0.67);
  assert.equal(c.by_signal.hotfix_tag, 1);
  assert.equal(c.by_signal.bug, 1);
  assert.deepEqual(c.evidence.map((e) => e.deploy), ['v1-deployed', 'v2-deployed']);
  assert.equal(c.evidence[1].signals[0].ref, 'ABC-5');

  assert.equal(m.mttr.n, 1);
  assert.equal(m.mttr.incidents, 2);
  assert.equal(m.mttr.p50, 48);
  assert.equal(m.mttr.evidence[0].from_deploy, true);

  const ids = m.guardrails.map((g) => g.id).sort();
  assert.deepEqual(ids, ['dora_mttr']); // 3 deploys / 4 leads ≥ min_n 3; mttr n=1
});

test('bandFor: every cutoff boundary', () => {
  assert.equal(bandFor('deploy_frequency', 5), 'elite');
  assert.equal(bandFor('deploy_frequency', 4.99), 'high');
  assert.equal(bandFor('deploy_frequency', 1), 'high');
  assert.equal(bandFor('deploy_frequency', 0.99), 'medium');
  assert.equal(bandFor('deploy_frequency', 0.25), 'medium');
  assert.equal(bandFor('deploy_frequency', 0.24), 'low');
  assert.equal(bandFor('lead_time_days', 1), 'elite');
  assert.equal(bandFor('lead_time_days', 1.01), 'high');
  assert.equal(bandFor('lead_time_days', 5), 'high');
  assert.equal(bandFor('lead_time_days', 22), 'medium');
  assert.equal(bandFor('lead_time_days', 22.01), 'low');
  assert.equal(bandFor('change_failure_rate', 0.05), 'elite');
  assert.equal(bandFor('change_failure_rate', 0.15), 'high');
  assert.equal(bandFor('change_failure_rate', 0.3), 'medium');
  assert.equal(bandFor('change_failure_rate', 0.31), 'low');
  assert.equal(bandFor('mttr_hours', 1), 'elite');
  assert.equal(bandFor('mttr_hours', 24), 'high');
  assert.equal(bandFor('mttr_hours', 168), 'medium');
  assert.equal(bandFor('mttr_hours', 168.01), 'low');
  assert.equal(bandFor('lead_time_days', null), null);
});

test('two repos: by_repo and per_week', () => {
  const t = [
    { repo: 'a', name: 'a1-deployed', ts: T('2026-06-01T10:00:00Z') },
    { repo: 'a', name: 'a2-deployed', ts: T('2026-06-02T10:00:00Z') },
    { repo: 'b', name: 'b1-deployed', ts: T('2026-06-01T11:00:00Z') },
    { repo: 'a', name: 'build-123', ts: T('2026-06-03T10:00:00Z') }, // not a deploy tag
  ];
  const m = doraMetrics([], t, cfg);
  assert.equal(m.deploy_frequency.total, 3);
  assert.deepEqual(m.deploy_frequency.by_repo, { a: 2, b: 1 });
  assert.deepEqual(m.deploy_frequency.per_week_by_repo, { a: 1, b: 0.5 });
  assert.equal(m.deploy_frequency.per_week, 1.5);
});

test('deploys dedupe by repo + tagged sha: same commit collapses, different commits same day do not', () => {
  const t = [
    { repo: 'svc', name: 'v1-deployed', sha: 'aaa', ts: T('2026-06-01T09:00:00Z') },
    { repo: 'svc', name: 'v1-prod-deployed', sha: 'aaa', ts: T('2026-06-01T09:30:00Z') },
    { repo: 'svc', name: 'v1b-deployed', sha: 'bbb', ts: T('2026-06-01T12:00:00Z') },
    { repo: 'svc', name: 'v1-hotfix', sha: 'ccc', ts: T('2026-06-01T15:00:00Z') },
  ];
  const m = doraMetrics([], t, cfg);
  assert.equal(m.deploy_frequency.total, 3);
  assert.deepEqual(m.deploy_frequency.by_kind, { regular: 2, hotfix: 1 });
  assert.equal(m.change_failure_rate.failed, 1);
  assert.equal(m.mttr.p50, 3); // 12:00 deploy (latest before) → 15:00 hotfix
});

test('null repo / missing sha: each tag name is its own deploy', () => {
  const t = [
    { repo: null, name: 'a-deployed', sha: 'aaa', ts: T('2026-06-01T09:00:00Z') },
    { repo: null, name: 'b-deployed', sha: 'aaa', ts: T('2026-06-01T10:00:00Z') },
    { repo: 'svc', name: 'c-deployed', ts: T('2026-06-01T11:00:00Z') },
    { repo: 'svc', name: 'd-deployed', ts: T('2026-06-01T12:00:00Z') },
  ];
  assert.equal(doraMetrics([], t, cfg).deploy_frequency.total, 4);
});

test('failure window edge: hotfix at exactly 7d counts, 7d+1ms does not', () => {
  const base = T('2026-06-01T10:00:00Z');
  const mk = (off) => [
    { repo: 'svc', name: 'v1-deployed', ts: base },
    { repo: 'svc', name: 'v1-hf', ts: base + off },
  ];
  assert.equal(doraMetrics([], mk(7 * DAY), cfg).change_failure_rate.failed, 1);
  assert.equal(doraMetrics([], mk(7 * DAY + 1), cfg).change_failure_rate.failed, 0);
});

test('hotfix in another repo does not fail this repo', () => {
  const t = [
    { repo: 'a', name: 'a1-deployed', ts: T('2026-06-01T10:00:00Z') },
    { repo: 'b', name: 'b1-hotfix', ts: T('2026-06-02T10:00:00Z') },
  ];
  const m = doraMetrics([], t, cfg);
  assert.equal(m.change_failure_rate.failed, 0);
  assert.equal(m.change_failure_rate.label, '0 of 2');
});

test('chained hotfixes collapse into one incident', () => {
  const t = [
    { repo: 'svc', name: 'v1-deployed', ts: T('2026-06-01T10:00:00Z') },
    { repo: 'svc', name: 'v1-hf1', ts: T('2026-06-02T10:00:00Z') },
    { repo: 'svc', name: 'v1-hf2', ts: T('2026-06-03T10:00:00Z') },
  ];
  const m = doraMetrics([], t, cfg);
  assert.equal(m.change_failure_rate.total, 3);
  assert.equal(m.change_failure_rate.failed, 1);
  assert.equal(m.change_failure_rate.evidence[0].signals.length, 2);
  assert.equal(m.mttr.p50, 48); // restored by the last hotfix in the chain
});

test('MTTR from bug detection to the bug\'s own deploy in the same repo', () => {
  const t = [
    { repo: 'svc', name: 'v1-deployed', ts: T('2026-06-01T10:00:00Z') },
    { repo: 'svc', name: 'v2-deployed', ts: T('2026-06-03T10:00:00Z') },
  ];
  const r = [
    { key: 'ABC-1', type: 'Story', deployed_at: T('2026-06-01T10:00:00Z'), deployed_tag: 'v1-deployed' },
    { key: 'ABC-2', type: 'Bug', created: T('2026-06-02T10:00:00Z'), rework_of: 'ABC-1', deployed_at: T('2026-06-03T10:00:00Z'), deployed_tag: 'v2-deployed' },
    { key: 'ABC-3', type: 'Story', created: T('2026-06-02T12:00:00Z'), rework_of: 'ABC-1', rework_signals: ['title_ref', 'title_keyword'], rework_confidence: 'medium' },
    { key: 'ABC-4', type: 'Bug', created: T('2026-06-02T10:00:00Z') }, // unlinked
  ];
  const m = doraMetrics(r, t, cfg);
  assert.equal(m.change_failure_rate.failed, 1);
  assert.equal(m.change_failure_rate.by_signal.bug, 1);
  assert.equal(m.change_failure_rate.by_signal.jira_rework, 1);
  assert.equal(m.mttr.p50, 24);
  assert.equal(m.mttr.evidence[0].from_source, 'detected');
  const g = m.guardrails.find((x) => x.id === 'unlinked_bugs_in_window');
  assert.deepEqual(g.keys, ['ABC-4']);
});

test('PR lead time: first commit → first tag containing the merge sha', () => {
  const t = [
    { repo: 'svc', name: 'v1-deployed', ts: T('2026-06-02T10:00:00Z'), contains: ['m0'] },
    { repo: 'svc', name: 'v2-deployed', ts: T('2026-06-04T10:00:00Z'), contains: ['m0', 'm1'] },
  ];
  const prs = [{ repo: 'svc', first_commit_at: T('2026-06-01T10:00:00Z'), merged_at: T('2026-06-01T18:00:00Z'), merge_commit: 'm1' }];
  const m = doraMetrics([], t, { ...cfg, prs });
  assert.equal(m.lead_time.basis, 'pr');
  assert.equal(m.lead_time.n, 1);
  assert.equal(m.lead_time.p50, 3); // Mon 10:00 → Thu 10:00
  assert.equal(m.lead_time.sources.tag_contains_merge, 1);
});

test('zero deploys → not_available, no band, never 0/NaN', () => {
  const m = doraMetrics([], [{ repo: 'svc', name: 'release-1', ts: T('2026-06-02T10:00:00Z') }], { ...cfg, branch: 'develop' });
  const d = m.deploy_frequency;
  assert.equal(d.value, null);
  assert.equal(d.status, 'not_available');
  assert.equal(d.reason, 'no_deploy_tags');
  assert.deepEqual(d.patterns, ['deployed', 'hf', 'hotfix']);
  assert.deepEqual(d.repos, ['svc']);
  assert.equal(d.branch, 'develop');
  assert.equal(d.band, null);
  assert.equal(m.lead_time.value, null);
  assert.equal(m.change_failure_rate.value, null);
  assert.equal(m.mttr.value, null);
  assert.equal(m.mttr.band, null);
  assert.ok(m.guardrails.some((g) => g.id === 'dora_deploy_frequency' && g.severity === 'not_available'));
});

test('legacy object form is still accepted', () => {
  const m = doraMetrics({ records, tags, window, cfg: { failure_window_days: 7 } });
  assert.equal(m.deploy_frequency.total, 3);
});

test('hotfix kind comes from hotfix_tag_patterns, or a tagKind callback', () => {
  const t = [
    { repo: 'svc', name: 'v1-deployed', sha: 'a', ts: T('2026-06-01T10:00:00Z') },
    { repo: 'svc', name: 'v1-urgent-deployed', sha: 'b', ts: T('2026-06-02T10:00:00Z') },
    { repo: 'svc', name: 'v1-hf', sha: 'c', ts: T('2026-06-03T10:00:00Z') },
  ];
  assert.deepEqual(doraMetrics([], t, cfg).deploy_frequency.by_kind, { regular: 2, hotfix: 1 });
  const custom = doraMetrics([], t, { ...cfg, hotfix_tag_patterns: ['URGENT'] });
  assert.deepEqual(custom.deploy_frequency.by_kind, { regular: 2, hotfix: 1 });
  assert.equal(custom.change_failure_rate.evidence[0].signals[0].ref, 'v1-urgent-deployed');
  const cb = doraMetrics([], t, { ...cfg, tagKind: (name) => (name === 'v1-deployed' ? 'hotfix' : 'deploy') });
  assert.deepEqual(cb.deploy_frequency.by_kind, { regular: 2, hotfix: 1 });
  assert.equal(cb.hotfix_rate.value, 0.33);
});

test('PR lead time: earliest tag containing the merge sha (O(1) index), repo-scoped, coverage reported', () => {
  const t = [
    { repo: 'svc', name: 'v1-deployed', sha: 's1', ts: T('2026-06-03T10:00:00Z'), contains: ['m1'] },
    { repo: 'svc', name: 'v2-deployed', sha: 's2', ts: T('2026-06-05T10:00:00Z'), contains: ['m1', 'm2'] },
    { repo: 'other', name: 'o1-deployed', sha: 'o1', ts: T('2026-06-02T10:00:00Z'), contains: ['m2'] },
  ];
  const prs = [
    { repo: 'svc', merge_commit: 'm1', first_commit_at: '2026-06-01T10:00:00Z', merged_at: '2026-06-02T10:00:00Z' }, // → v1: 2 bd
    { repo: 'svc', merge_commit: 'm2', first_commit_at: '2026-06-01T10:00:00Z', merged_at: '2026-06-01T12:00:00Z' }, // → v2 (not other repo): 4 bd
    { repo: 'svc', merge_commit: 'zz', first_commit_at: '2026-06-01T10:00:00Z', merged_at: '2026-06-02T10:00:00Z' }, // no tag, no ticket
  ];
  const m = doraMetrics([], t, { ...cfg, prs });
  assert.equal(m.lead_time.basis, 'pr');
  assert.equal(m.lead_time.n, 2);
  assert.equal(m.lead_time.p50, 3);
  assert.equal(m.lead_time.sources.tag_contains_merge, 2);
  assert.equal(m.lead_time.sources.considered, 3);
  assert.equal(m.lead_time.sources.tag_contains_merge_coverage, 0.67);
  // Fallback callback: binary-searched to the first tag at/after the merge.
  const seen = [];
  const cb = doraMetrics([], t.map(({ contains, ...x }) => x), { ...cfg, prs: [prs[0]], tagContains: (tag, sha) => { seen.push(tag.name); return sha === 'm1'; } });
  assert.equal(cb.lead_time.n, 1);
  assert.deepEqual(seen, ['v1-deployed']);
});

test('Fri/Sat weekend changes lead time vs Sat/Sun', () => {
  // Thu 10:00 first commit → Sun 10:00 deploy.
  const recs = [{ key: 'ABC-1', type: 'Story', first_commit_at: T('2026-06-04T10:00:00Z'), deployed_at: T('2026-06-07T10:00:00Z') }];
  const tg = [{ repo: 'svc', name: 'v1-deployed', sha: 'a', ts: T('2026-06-07T10:00:00Z') }];
  const satSun = doraMetrics(recs, tg, { ...cfg, weekend: [0, 6] }).lead_time.p50;
  const friSat = doraMetrics(recs, tg, { ...cfg, weekend: [5, 6] }).lead_time.p50;
  assert.notEqual(satSun, friSat);
});

test('new DORA metrics: hotfix rate, batch size, time to first commit, detect vs restore, open incidents, capacity weeks', () => {
  const t = [
    { repo: 'svc', name: 'v1-deployed', sha: 'a', ts: T('2026-06-01T10:00:00Z') },
    { repo: 'svc', name: 'v2-deployed', sha: 'b', ts: T('2026-06-08T10:00:00Z') },
    { repo: 'svc', name: 'v3-hotfix', sha: 'c', ts: T('2026-06-09T10:00:00Z') },
    { repo: 'svc', name: 'v4-deployed', sha: 'd', ts: T('2026-06-12T10:00:00Z') },
  ];
  const recs = [
    { key: 'ABC-1', type: 'Story', first_active_at: T('2026-05-27T10:00:00Z'), first_commit_at: T('2026-05-28T10:00:00Z'), deployed_at: T('2026-06-01T10:00:00Z'), deployed_tag: 'v1-deployed' },
    { key: 'ABC-2', type: 'Story', first_active_at: T('2026-05-27T10:00:00Z'), first_commit_at: T('2026-05-29T10:00:00Z'), deployed_at: T('2026-06-01T10:00:00Z'), deployed_tag: 'v1-deployed' },
    { key: 'ABC-3', type: 'Story', first_active_at: T('2026-06-03T10:00:00Z'), first_commit_at: T('2026-06-03T10:00:00Z'), deployed_at: T('2026-06-08T10:00:00Z'), deployed_tag: 'v2-deployed' },
    // Bug detected 1 day after v2, restored by the v3 hotfix 1 day later.
    { key: 'ABC-4', type: 'Bug', created: T('2026-06-08T22:00:00Z'), rework_of: 'ABC-3', deployed_at: T('2026-06-09T10:00:00Z'), deployed_tag: 'v3-hotfix' },
    { key: 'ABC-6', type: 'Story', first_commit_at: T('2026-06-10T10:00:00Z'), deployed_at: T('2026-06-12T10:00:00Z'), deployed_tag: 'v4-deployed' },
    // Bug on v4 never restored inside the window → open incident.
    { key: 'ABC-5', type: 'Bug', created: T('2026-06-13T10:00:00Z'), rework_of: 'ABC-6' },
  ];
  const m = doraMetrics(recs, t, { ...cfg, capacity_days: 20 });
  assert.equal(m.hotfix_rate.value, 0.25);
  assert.equal(m.batch_size.p50, 1);
  assert.equal(m.batch_size.p85, 1.55);
  assert.equal(m.time_to_first_commit.n, 3);
  assert.equal(m.time_to_first_commit.p50, 1);
  assert.equal(m.mttr.time_to_detect.n, 2);
  assert.equal(m.mttr.time_to_detect.p50, 18); // 12h and 24h
  assert.equal(m.mttr.time_to_restore.n, 1);
  assert.equal(m.mttr.time_to_restore.p50, 12);
  assert.equal(m.open_incidents.n, 1);
  assert.equal(m.open_incidents.oldest_age, 2.58); // Jun 12 10:00 → Jun 15 00:00
  assert.equal(m.deploys_per_capacity_week, 1); // 4 deploys / (20 / 5) capacity weeks
  assert.equal(m.mttr.from_deploy_share, 0);
});
