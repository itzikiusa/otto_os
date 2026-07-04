// Unit tests for the AI estimation machinery (no network — agentRun is faked).
// Run (from the plugin dir): node --test test/estimates.test.js
const { test } = require('node:test');
const assert = require('node:assert/strict');

const E = require('../lib/estimates.js');

const T = (s) => Date.parse(s);
const NOW = T('2026-07-01T00:00:00Z');

function rec(over) {
  return {
    key: 'K-1',
    type: 'Story',
    points: 3,
    summary: 'Implement the thing',
    description_snippet: 'details',
    done_at: T('2026-06-20T00:00:00Z'),
    eff_done_at: T('2026-06-20T00:00:00Z'),
    updated: T('2026-06-20T00:00:00Z'),
    ...over,
  };
}

test('contentHash: stable on same content, changes when the text changes', () => {
  const a = E.contentHash(rec({}));
  assert.equal(a, E.contentHash(rec({})));
  assert.notEqual(a, E.contentHash(rec({ summary: 'Implement the OTHER thing' })));
  assert.notEqual(a, E.contentHash(rec({ points: 5 })));
});

test('selectTargets: window filter, cache hits skipped, hash mismatch re-selected', () => {
  const recent = rec({ key: 'R-1' });
  const old = rec({ key: 'R-2', done_at: T('2025-01-01T00:00:00Z'), eff_done_at: T('2025-01-01T00:00:00Z') });
  const open = rec({ key: 'R-3', done_at: null, eff_done_at: null });
  const cached = rec({ key: 'R-4' });
  const changed = rec({ key: 'R-5' });
  const cache = {
    'R-4': { hash: E.contentHash(cached), days: 2, routine: false, v: 5 },
    'R-5': { hash: 'stale-hash', days: 2, routine: false, v: 5 },
  };
  const keys = E.selectTargets([recent, old, open, cached, changed], cache, 6, NOW).map((r) => r.key);
  assert.ok(keys.includes('R-1'), 'recent done selected');
  assert.ok(!keys.includes('R-2'), 'outside the window');
  assert.ok(keys.includes('R-3'), 'open always selected');
  assert.ok(!keys.includes('R-4'), 'cache hit skipped');
  assert.ok(keys.includes('R-5'), 'content change re-estimates');
  // window 0 = everything
  assert.ok(E.selectTargets([old], {}, 0, NOW).length === 1);
});

test('parseBatch: lenient extraction, validation, clamping', () => {
  const text = 'Sure! Here are the estimates:\n[{"key":"A-1","days":2.5,"routine":false},{"key":"A-2","days":900},{"key":"NOPE","days":1},{"key":"A-3","days":-1}]\nDone.';
  const out = E.parseBatch(text, ['A-1', 'A-2', 'A-3']);
  assert.equal(out['A-1'].days, 2.5);
  assert.equal(out['A-2'].days, 120, 'clamped to 120');
  assert.equal(out['A-3'], undefined, 'non-positive rejected');
  assert.equal(out.NOPE, undefined, 'unexpected key rejected');
  assert.deepEqual(E.parseBatch('no json here', ['A-1']), {});
});

test('runEstimation: caches results, splits batches across workers, falls back on worker failure', async () => {
  const records = [];
  for (let i = 0; i < 35; i++) records.push(rec({ key: `B-${i}`, summary: `task ${i}` }));
  const cache = {};
  const calls = [];
  const agentRun = async (prompt, worker) => {
    calls.push(worker.provider);
    if (worker.provider === 'codex') throw new Error('codex down'); // lane 2 fails -> retried on worker 0
    const keys = [...prompt.matchAll(/key=([A-Z]+-\d+)/g)].map((m) => m[1]);
    return JSON.stringify(keys.map((k) => ({ key: k, days: 1.5, routine: k.endsWith('0') })));
  };
  const res = await E.runEstimation({
    records,
    cache,
    windowMonths: 6,
    maxBatches: 10,
    workers: [{ provider: 'claude', model: '' }, { provider: 'codex', model: '' }],
    agentRun,
    nowMs: NOW,
  });
  assert.equal(res.estimated, 35, 'all records estimated (codex batches retried on claude)');
  assert.ok(calls.includes('codex'), 'codex lane attempted');
  assert.equal(Object.keys(cache).length, 35);
  assert.equal(cache['B-0'].routine, true);
  assert.equal(cache['B-1'].days, 1.5);

  // Second run: everything cached -> no agent calls.
  const before = calls.length;
  const res2 = await E.runEstimation({ records, cache, windowMonths: 6, maxBatches: 10, workers: [{ provider: 'claude', model: '' }], agentRun, nowMs: NOW });
  assert.equal(res2.estimated, 0);
  assert.equal(calls.length, before, 'no calls when fully cached');
});

test('changeFingerprint invalidates the cache as the diff grows; prompt embeds evidence', () => {
  const base = rec({ key: 'C-1', git_change: { commits: 1, files: 1, insertions: 5, deletions: 0 } });
  const grown = rec({ key: 'C-1', git_change: { commits: 6, files: 18, insertions: 900, deletions: 1300 } });
  const cache = {};
  // First estimate caches at the small fingerprint.
  cache['C-1'] = { hash: E.contentHash(base), days: 1, routine: false, v: 5 };
  assert.equal(E.selectTargets([base], cache, 6, NOW).length, 0, 'unchanged small diff stays cached');
  assert.equal(E.selectTargets([grown], cache, 6, NOW).length, 1, 'a much larger diff re-estimates');

  const p = E.batchPrompt([grown]);
  assert.ok(p.includes('change={commits:6, files:18, +900/-1300'), 'diff evidence embedded');
  assert.ok(p.includes('Calibration rubric'), 'rubric present');
  assert.ok(p.includes('Reverse integration'), 'default rubric line present');
});

test('custom rubric overrides the default lines', () => {
  const p = E.batchPrompt([rec({ key: 'R-1' })], ['Everything is exactly 2 days.']);
  assert.ok(p.includes('Everything is exactly 2 days.'));
  assert.ok(!p.includes('Reverse integration'), 'default lines replaced');
});

test('older prompt-version estimates are re-selected', () => {
  const r = rec({ key: 'V-1' });
  const cache = { 'V-1': { hash: E.contentHash(r), days: 3, routine: false, v: 4 } };
  assert.equal(E.selectTargets([r], cache, 6, NOW).length, 1, 'an older-version entry re-estimates under the current prompt');
});
