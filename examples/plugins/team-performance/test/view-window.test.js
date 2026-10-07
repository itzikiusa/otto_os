'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { resolveViewWindow } = require('../lib/view-window.js');
const until = Date.parse('2026-10-07T12:30:00Z');

test('all-time view window: earliest record activity includes its whole UTC capacity day', () => {
  const scope = { records: [{ created: '2026-06-01T15:00:00+03:00', eff_start_at: Date.parse('2026-06-03T10:00:00Z'), done_at: Date.parse('2026-06-05T12:00:00Z') }] };
  assert.deepEqual(resolveViewWindow(scope, 0, until), { since: Date.parse('2026-06-01T00:00:00Z'), until });
  const cutoff = Date.parse('2026-06-04T12:34:00Z');
  assert.deepEqual(resolveViewWindow(scope, cutoff, until), { since: cutoff, until });
});

test('all-time view window: owned commit and deployment dates bound records without Jira creation', () => {
  const record = { first_commit_at: '2026-04-03T23:00:00Z', first_deployed_at: '2026-05-05T12:00:00Z', deployed_at: '2026-06-12T12:00:00Z' };
  assert.equal(resolveViewWindow({ records: [record] }, 0, until).since, Date.parse('2026-04-03T00:00:00Z'));
  delete record.first_commit_at;
  assert.equal(resolveViewWindow({ records: [record] }, 0, until).since, Date.parse('2026-05-05T00:00:00Z'));
});

test('all-time view window: invalid, absent and future activity never creates an epoch window', () => {
  const scope = { records: [{ created: null, started_at: '', done_at: Infinity, eff_done_at: NaN, first_commit_at: -1 }], side: { tags: [{ ts: until + 1000 }], prs: [{ opened_at: 'invalid' }] } };
  assert.deepEqual(resolveViewWindow(scope, 0, until), { since: until, until });
  assert.deepEqual(resolveViewWindow({}, 0, until), { since: until, until });
  assert.deepEqual(resolveViewWindow({}, until + 1000, until), { since: until + 1000, until });
});


test('all-time view window: unrelated global side caches cannot expand a selected or empty scope', () => {
  const records = [{ key: 'TP-1', created: '2026-06-01T10:00:00Z', deployed_tag: 'current-deployed', git_change: { repos: [{ name: 'owned' }] } }];
  const side = {
    tags: [
      { name: 'old-deployed', repo: 'other', ts: Date.parse('2001-01-01T00:00:00Z') },
      { name: 'old-same-repo-deployed', repo: 'owned', ts: Date.parse('2002-01-01T00:00:00Z') },
      { name: 'current-deployed', repo: 'other', ts: Date.parse('2003-01-01T00:00:00Z') },
    ],
    prs: [{ keys: ['OTHER-1'], opened_at: '2000-01-01T00:00:00Z' }],
  };
  assert.deepEqual(resolveViewWindow({ records, side }, 0, until), { since: Date.parse('2026-06-01T00:00:00Z'), until });
  assert.deepEqual(resolveViewWindow({ records: [], side }, 0, until), { since: until, until });
});

test('all-time view window: matching global keys and repo tag names do not prove account ownership', () => {
  const records = [{ key: 'TP-1', created: '2026-06-01T10:00:00Z', deployed_tag: 'release-deployed', git_change: { repos: [{ name: 'repo-a' }, { name: 'repo-b' }] } }];
  // Another Jira account may also own TP-1; an aggregate repo list does not
  // establish which repository supplied the record's deployment tag.
  const side = {
    prs: [{ keys: ['TP-1'], opened_at: '2000-01-01T00:00:00Z' }],
    tags: [{ repo: 'repo-b', name: 'release-deployed', ts: Date.parse('2001-01-01T00:00:00Z') }],
  };
  assert.deepEqual(resolveViewWindow({ records, side }, 0, until), { since: Date.parse('2026-06-01T00:00:00Z'), until });
  assert.deepEqual(resolveViewWindow({ records: [], side }, 0, until), { since: until, until });
});
