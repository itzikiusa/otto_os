// Jira-detected rework classifier (lib/jira-rework.js). Pure — no I/O.
// Run (from the plugin dir): node --test
const { test } = require('node:test');
const assert = require('node:assert/strict');

const J = require('../lib/jira-rework.js');

const T = (s) => Date.parse(s);
const DAY = 86400000;

const origin = { key: 'ABC-1', type: 'Story', summary: 'Checkout page', dev_days: 4, deployed_at: T('2026-06-01T00:00:00Z'), flags: [] };
const rec = (o) => ({ type: 'Story', summary: 'Something new', dev_days: 2, flags: [], created: T('2026-06-10T00:00:00Z'), ...o });
const corpus = (...rs) => new Map([origin, ...rs].map((r) => [r.key, r]));

test('link: "is caused by" (raw Jira issuelinks) → high, origin found', () => {
  const r = rec({ key: 'ABC-2', issuelinks: [{ type: { name: 'Problem/Incident', inward: 'is caused by', outward: 'causes' }, inwardIssue: { key: 'ABC-1' } }] });
  const v = J.classifyRework(r, corpus(r));
  assert.deepEqual(v, { is_rework: true, origin_key: 'ABC-1', confidence: 'high', signals: ['link'] });
});

test('link: "fixes" outward → high', () => {
  const r = rec({ key: 'ABC-2', links: [{ rel: 'fixes', key: 'ABC-1' }] });
  assert.equal(J.classifyRework(r, corpus(r)).confidence, 'high');
});

test('link: reverse "causes" on the origin counts via reverseLinks', () => {
  const o2 = { ...origin, links: [{ rel: 'causes', key: 'ABC-2' }] };
  const r = rec({ key: 'ABC-2' });
  const all = [o2, r];
  const v = J.classifyRework(r, new Map(all.map((x) => [x.key, x])), { reverse: J.reverseLinks(all) });
  assert.equal(v.is_rework, true);
  assert.equal(v.origin_key, 'ABC-1');
});

test('link: "relates to" alone is low confidence and NOT rework', () => {
  const r = rec({ key: 'ABC-2', links: [{ rel: 'relates to', key: 'ABC-1' }] });
  const v = J.classifyRework(r, corpus(r));
  assert.equal(v.confidence, 'low');
  assert.equal(v.is_rework, false);
  assert.equal(v.origin_key, 'ABC-1');
});

test('relates + follow-up keyword → medium rework', () => {
  const r = rec({ key: 'ABC-2', summary: 'Follow-up on checkout', links: [{ rel: 'relates to', key: 'ABC-1' }] });
  const v = J.classifyRework(r, corpus(r));
  assert.equal(v.confidence, 'medium');
  assert.equal(v.is_rework, true);
  assert.deepEqual(v.signals.sort(), ['link', 'title_keyword']);
});

test('title_ref: summary names another ticket → medium; + keyword → high', () => {
  const r = rec({ key: 'ABC-2', summary: 'Adjust ABC-1 layout' });
  assert.deepEqual(J.classifyRework(r, corpus(r)), { is_rework: true, origin_key: 'ABC-1', confidence: 'medium', signals: ['title_ref'] });
  const r2 = rec({ key: 'ABC-3', summary: 'Fix for ABC-1 totals' });
  assert.equal(J.classifyRework(r2, corpus(r2)).confidence, 'high');
});

test('title_keyword alone (no origin) → low, not rework', () => {
  const r = rec({ key: 'ABC-2', summary: 'Regression in search' });
  assert.deepEqual(J.classifyRework(r, corpus(r)), { is_rework: false, origin_key: null, confidence: 'low', signals: ['title_keyword'] });
});

test('negative: version-only mention ("JDK-17", "UTF-8") is not a reference', () => {
  const r = rec({ key: 'ABC-2', summary: 'Upgrade runtime to JDK-17 and UTF-8 everywhere' });
  assert.deepEqual(J.classifyRework(r, corpus(r)), { is_rework: false, origin_key: null, confidence: null, signals: [] });
});

test('negative: own key and parent key in the title are not rework', () => {
  const r = rec({ key: 'ABC-2', parent_key: 'ABC-1', summary: 'ABC-2 part of ABC-1' });
  assert.equal(J.classifyRework(r, corpus(r)).is_rework, false);
});

test('reopened: self rework, origin is the ticket itself', () => {
  const r = rec({ key: 'ABC-2', flags: ['reopened'] });
  assert.deepEqual(J.classifyRework(r, corpus(r)), { is_rework: true, origin_key: 'ABC-2', confidence: 'medium', signals: ['reopened'] });
});

test('bug_after_delivery: bug within window that blame-overlaps the origin → high', () => {
  const r = rec({ key: 'ABC-2', type: 'Bug', created: origin.deployed_at + 10 * DAY });
  const v = J.classifyRework(r, corpus(r), { blamePairs: { 'ABC-2>ABC-1': { lines: 5 } }, bug_window_days: 30 });
  assert.equal(v.is_rework, true);
  assert.equal(v.confidence, 'high');
  assert.deepEqual(v.signals.sort(), ['blame', 'bug_after_delivery']);
});

test('negative: bug outside the window, or without blame overlap, or a story with blame only', () => {
  const late = rec({ key: 'ABC-2', type: 'Bug', created: origin.deployed_at + 45 * DAY });
  assert.equal(J.classifyRework(late, corpus(late), { blamePairs: new Set(['ABC-2>ABC-1']) }).is_rework, false);
  const noBlame = rec({ key: 'ABC-3', type: 'Bug', created: origin.deployed_at + 3 * DAY });
  assert.equal(J.classifyRework(noBlame, corpus(noBlame), { blamePairs: {} }).is_rework, false);
  const story = rec({ key: 'ABC-4', created: origin.deployed_at + 3 * DAY });
  assert.equal(J.classifyRework(story, corpus(story), { blamePairs: { 'ABC-4>ABC-1': { lines: 9 } } }).is_rework, false);
});

test('applyRework: charges actual to origin, marks scope_excluded, inputs untouched', () => {
  const r = rec({ key: 'ABC-2', summary: 'Fix for ABC-1', dev_days: 2 });
  const plain = rec({ key: 'ABC-3', dev_days: 3 });
  const input = [origin, r, plain];
  const out = J.applyRework(input);
  const by = Object.fromEntries(out.map((x) => [x.key, x]));
  assert.equal(by['ABC-2'].rework_of, 'ABC-1');
  assert.equal(by['ABC-2'].scope_excluded, true);
  assert.equal(by['ABC-2'].rework_out, 2);
  assert.equal(by['ABC-1'].rework_in, 2);
  assert.equal(by['ABC-3'].scope_excluded, undefined);
  assert.equal(origin.rework_in, undefined, 'input not mutated');
});

test('applyRework: already blame-charged time is not charged twice', () => {
  const r = rec({ key: 'ABC-2', summary: 'Fix for ABC-1', dev_days: 2, rework_out: 0.5 });
  const o = { ...origin, rework_in: 0.5 };
  const by = Object.fromEntries(J.applyRework([o, r]).map((x) => [x.key, x]));
  assert.equal(by['ABC-1'].rework_in, 2, '0.5 by blame + 1.5 remaining');
  assert.equal(by['ABC-2'].rework_out, 2);
});

test('weighted throughput excludes a rework ticket\'s estimate', () => {
  const est = { 'ABC-1': 5, 'ABC-2': 3, 'ABC-3': 2 };
  const r = rec({ key: 'ABC-2', summary: 'Follow-up: missing validation from ABC-1' });
  const plain = rec({ key: 'ABC-3' });
  const out = J.applyRework([origin, r, plain]);
  assert.equal(J.deliveredScope([origin, r, plain], (x) => est[x.key]), 10);
  assert.equal(J.deliveredScope(out, (x) => est[x.key]), 7);
});

test('reworkRate = rework / (dev + rework)', () => {
  const out = J.applyRework([origin, rec({ key: 'ABC-2', summary: 'Fix for ABC-1', dev_days: 2 })]);
  const v = J.reworkRate(out);
  // time actually spent: 4 (origin) + 2 (redo); rework = 2 → 2 / 6
  assert.deepEqual(v, { rate: 0.33, rework_days: 2, dev_days: 4 });
  assert.equal(J.reworkRate([]).rate, null);
});
