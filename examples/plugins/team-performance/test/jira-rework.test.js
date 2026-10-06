// Jira-detected rework classifier (lib/jira-rework.js). Pure — no I/O.
// Run (from the plugin dir): node --test
const { test } = require('node:test');
const assert = require('node:assert/strict');

const J = require('../lib/jira-rework.js');
const A = require('../lib/analytics.js');

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

// ---- P3: table-driven classification (positives + negatives) ----
const OTHER = { key: 'XYZ-7', type: 'Story', summary: 'Partner API', dev_days: 3, deployed_at: T('2026-06-05T00:00:00Z'), flags: [] };
const OLD = { key: 'ABC-0', type: 'Story', summary: 'Legacy login', dev_days: 3, deployed_at: T('2026-01-01T00:00:00Z'), flags: [] };
const STORY = { key: 'ABC-50', type: 'Story', summary: 'Parent story', dev_days: 1, flags: [] };
const CASES = [
  { name: 'strong "is caused by" link', r: { key: 'ABC-2', links: [{ rel: 'is caused by', key: 'ABC-1' }] }, origin: 'ABC-1', conf: 'high' },
  { name: '"fixes" link', r: { key: 'ABC-2', links: [{ rel: 'fixes', key: 'ABC-1' }] }, origin: 'ABC-1', conf: 'high' },
  { name: 'title names key + "fix for"', r: { key: 'ABC-2', summary: 'Fix for ABC-1 totals' }, origin: 'ABC-1', conf: 'high' },
  { name: 'title names key only', r: { key: 'ABC-2', summary: 'Adjust ABC-1 layout' }, origin: 'ABC-1', conf: 'medium' },
  { name: 'relates-to + follow-up keyword (in window)', r: { key: 'ABC-2', summary: 'Follow-up', links: [{ rel: 'relates to', key: 'ABC-1' }] }, origin: 'ABC-1', conf: 'medium' },
  { name: 'cross-project origin named in title', r: { key: 'ABC-2', summary: 'Missing field from XYZ-7' }, origin: 'XYZ-7', conf: 'high' },
  { name: 'rework ticket that is a sub-task of another story', r: { key: 'ABC-51', type: 'Sub-task', parent_key: 'ABC-50', links: [{ rel: 'is caused by', key: 'ABC-1' }] }, origin: 'ABC-1', conf: 'high' },
  { name: 'bug in window with blame overlap', r: { key: 'ABC-2', type: 'Bug', created: origin.deployed_at + 5 * DAY }, pairs: { 'ABC-2>ABC-1': { lines: 4 } }, origin: 'ABC-1', conf: 'high' },
  { name: 'NEG: "missing" in title without any reference', r: { key: 'ABC-2', summary: 'Missing translations on page' }, origin: null },
  { name: 'NEG: relates-to an older ticket outside the window', r: { key: 'ABC-2', summary: 'Follow-up', links: [{ rel: 'relates to', key: 'ABC-0' }] }, origin: null },
  { name: 'NEG: bug long after delivery (blame only)', r: { key: 'ABC-2', type: 'Bug', created: origin.deployed_at + 120 * DAY }, pairs: { 'ABC-2>ABC-1': { lines: 4 } }, origin: null },
  { name: 'NEG: sub-task naming its own parent', r: { key: 'ABC-51', type: 'Sub-task', parent_key: 'ABC-50', summary: 'Fix for ABC-50 checklist' }, origin: null },
];
for (const c of CASES) {
  test(`classifyRework table: ${c.name}`, () => {
    const r = rec(c.r);
    const all = [origin, OTHER, OLD, STORY, r];
    const by = new Map(all.map((x) => [x.key, x]));
    const v = J.classifyRework(r, by, { reverse: J.reverseLinks(all), blamePairs: c.pairs, bug_window_days: 30 });
    if (c.origin === null) {
      assert.equal(v.is_rework, false, JSON.stringify(v));
    } else {
      assert.equal(v.is_rework, true, JSON.stringify(v));
      assert.equal(v.origin_key, c.origin);
      assert.equal(v.confidence, c.conf);
    }
  });
}

test('deliveredScope excludes every rework ticket\'s estimate (incl. a rework sub-task)', () => {
  const sub = rec({ key: 'ABC-51', type: 'Sub-task', parent_key: 'ABC-50', links: [{ rel: 'is caused by', key: 'ABC-1' }] });
  const fix = rec({ key: 'ABC-2', summary: 'Fix for ABC-1 totals' });
  const est = { 'ABC-1': 5, 'ABC-2': 3, 'ABC-51': 1, 'ABC-50': 2 };
  const out = J.applyRework([origin, STORY, sub, fix]);
  assert.equal(J.deliveredScope(out, (x) => est[x.key]), 7);
});

test('applyRework: chain A←B←C is order-independent and conserves the team total', () => {
  const a = { key: 'ABC-10', type: 'Story', summary: 'Base', dev_days: 5, deployed_at: T('2026-06-01T00:00:00Z'), flags: [] };
  const b = rec({ key: 'ABC-11', summary: 'Fix for ABC-10', dev_days: 3 });
  const c = rec({ key: 'ABC-12', summary: 'Fix for ABC-11', dev_days: 2 });
  const sum = (rs) => Math.round(rs.reduce((s, r) => s + A.actualDays(r), 0) * 100) / 100;
  const pick = (rs) => Object.fromEntries(rs.map((r) => [r.key, [A.actualDays(r), r.rework_in || 0, r.rework_out || 0]]));
  const fwd = J.applyRework([a, b, c]);
  const rev = J.applyRework([c, b, a]);
  assert.deepEqual(pick(fwd), pick(rev));
  assert.equal(sum(fwd), 10);
  assert.equal(sum(rev), 10);
  assert.deepEqual(pick(fwd)['ABC-10'], [8, 3, 0], 'A carries B\'s own 3 days, not C\'s');
  assert.deepEqual(pick(fwd)['ABC-11'], [2, 2, 3]);
  assert.deepEqual(pick(fwd)['ABC-12'], [0, 0, 2]);
});

test('applyRework: a manual-time side is never adjusted, flagged instead', () => {
  const r = rec({ key: 'ABC-2', summary: 'Fix for ABC-1', dev_days: 2, manual_days: 4 });
  const by = Object.fromEntries(J.applyRework([origin, r]).map((x) => [x.key, x]));
  assert.equal(by['ABC-2'].rework_skipped_manual, true);
  assert.equal(by['ABC-2'].scope_excluded, true);
  assert.equal(by['ABC-1'].rework_in, undefined);
  assert.equal(A.actualDays(by['ABC-2']), 4);
  assert.equal(A.reworkGuardrails(Object.values(by))[0].code, 'rework_manual_skip');
});

test('bug window is configurable (config.bug_window_days) and exposed as meta', () => {
  const late = rec({ key: 'ABC-2', type: 'Bug', created: origin.deployed_at + 45 * DAY });
  const pairs = { 'ABC-2>ABC-1': { lines: 5 } };
  assert.equal(J.classifyRework(late, corpus(late), { blamePairs: pairs }).is_rework, false);
  assert.equal(J.classifyRework(late, corpus(late), { blamePairs: pairs, config: { bug_window_days: 60 } }).is_rework, true);
  assert.equal(J.bugWindowDays({}), 30);
  assert.equal(J.bugWindowDays({ config: { bug_window_days: 14 } }), 14);
  assert.equal(J.reworkMeta({ config: { bug_window_days: 14 } }).bug_window_days, 14);
  assert.equal(J.reworkRate([], { config: { bug_window_days: 14 } }).bug_window_days, 14);
});
