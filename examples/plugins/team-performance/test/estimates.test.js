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

test('contentHash: stable — title change re-estimates, description/points edits do not', () => {
  const a = E.contentHash(rec({}));
  assert.equal(a, E.contentHash(rec({})));
  assert.notEqual(a, E.contentHash(rec({ summary: 'Implement the OTHER thing' })));
  assert.equal(a, E.contentHash(rec({ points: 5 })), 'points ignored');
  assert.equal(a, E.contentHash(rec({ description_snippet: 'a much longer rewritten spec' })), 'description edits ignored');
});

test('ruler change re-estimates everything; first estimate is kept', () => {
  const r = rec({ key: 'RU-1' });
  const ruler = E.rulerId(['x'], '');
  const cache = { 'RU-1': { hash: E.contentHash(r), days: 3, v: 6, ruler } };
  assert.equal(E.selectTargets([r], cache, 6, NOW, 0, ruler).length, 0);
  assert.equal(E.selectTargets([r], cache, 6, NOW, 0, E.rulerId(['y'], '')).length, 1);
});

test('selectTargets: window filter, cache hits skipped, hash mismatch re-selected', () => {
  const recent = rec({ key: 'R-1' });
  const old = rec({ key: 'R-2', done_at: T('2025-01-01T00:00:00Z'), eff_done_at: T('2025-01-01T00:00:00Z') });
  const open = rec({ key: 'R-3', done_at: null, eff_done_at: null });
  const cached = rec({ key: 'R-4' });
  const changed = rec({ key: 'R-5' });
  const cache = {
    'R-4': { hash: E.contentHash(cached), days: 2, routine: false, v: 6 },
    'R-5': { hash: 'stale-hash', days: 2, routine: false, v: 6 },
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

test('medianReconcile: median days + majority routine across workers', () => {
  const per = [
    { 'K-1': { days: 2, routine: true }, 'K-2': { days: 5, routine: false } },
    { 'K-1': { days: 4, routine: true }, 'K-2': { days: 9, routine: true } },
    { 'K-1': { days: 6, routine: false }, 'K-2': { days: 7, routine: false } },
  ];
  const out = E.medianReconcile(['K-1', 'K-2'], per);
  assert.equal(out['K-1'].days, 4, 'median of 2,4,6');
  assert.equal(out['K-1'].routine, true, '2/3 routine → true');
  assert.equal(out['K-2'].days, 7, 'median of 5,7,9');
  assert.equal(out['K-2'].routine, false, '1/3 routine → false');
});

test('runEstimation consensus: every worker sizes each task; summarizer reconciles', async () => {
  const records = [rec({ key: 'X-1', summary: 'a' }), rec({ key: 'X-2', summary: 'b' })];
  const seen = { workers: [], summarizer: 0 };
  const workers = [{ provider: 'claude', model: '' }, { provider: 'grok', model: '' }];
  const summarizer = { provider: 'codex', model: '' };
  const agentRun = async (prompt, who) => {
    const keys = [...prompt.matchAll(/key=([A-Z]+-\d+)/g)].map((m) => m[1]);
    if (who.provider === 'codex') {
      // Summarizer prompt lists per-task votes ("- X-1: agent1=..."). Reconcile → 3d.
      seen.summarizer++;
      const sumKeys = [...prompt.matchAll(/- ([A-Z]+-\d+):/g)].map((m) => m[1]);
      return JSON.stringify(sumKeys.map((k) => ({ key: k, days: 3, routine: false })));
    }
    seen.workers.push(who.provider);
    const d = who.provider === 'claude' ? 2 : 6;
    return JSON.stringify(keys.map((k) => ({ key: k, days: d, routine: false })));
  };
  const cache = {};
  const res = await E.runEstimation({
    records, cache, windowMonths: 6, maxBatches: 10,
    mode: 'consensus', workers, summarizer, agentRun, nowMs: NOW,
  });
  assert.equal(res.estimated, 2);
  // BOTH workers sized the batch (consensus, not round-robin split).
  assert.ok(seen.workers.includes('claude') && seen.workers.includes('grok'));
  assert.ok(seen.summarizer >= 1, 'summarizer ran');
  assert.equal(cache['X-1'].days, 3, 'summarizer value wins');
  assert.equal(cache['X-2'].days, 3);
});

test('runEstimation consensus: no summarizer → deterministic median of workers', async () => {
  const records = [rec({ key: 'Y-1', summary: 'a' })];
  const workers = [{ provider: 'claude', model: '' }, { provider: 'grok', model: '' }];
  const agentRun = async (prompt, who) => {
    const keys = [...prompt.matchAll(/key=([A-Z]+-\d+)/g)].map((m) => m[1]);
    const d = who.provider === 'claude' ? 4 : 8;
    return JSON.stringify(keys.map((k) => ({ key: k, days: d, routine: false })));
  };
  const cache = {};
  const res = await E.runEstimation({
    records, cache, windowMonths: 6, maxBatches: 10, mode: 'consensus', workers, agentRun, nowMs: NOW,
  });
  assert.equal(res.estimated, 1);
  assert.equal(cache['Y-1'].days, 6, 'median of 4 and 8');
});

test('changeFingerprint invalidates the cache as the diff grows; prompt embeds evidence', () => {
  const base = rec({ key: 'C-1', git_change: { commits: 1, files: 1, insertions: 5, deletions: 0 } });
  const grown = rec({ key: 'C-1', git_change: { commits: 6, files: 18, insertions: 900, deletions: 1300 } });
  const cache = {};
  // First estimate caches at the small fingerprint.
  cache['C-1'] = { hash: E.contentHash(base), days: 1, routine: false, v: 6 };
  assert.equal(E.selectTargets([base], cache, 6, NOW).length, 0, 'unchanged small diff stays cached');
  assert.equal(E.selectTargets([grown], cache, 6, NOW).length, 1, 'a much larger diff re-estimates');

  const p = E.batchPrompt([grown]);
  assert.ok(p.includes('change={commits:6, files:18, +900/-1300'), 'diff evidence embedded');
  assert.ok(p.includes('Calibration rubric'), 'rubric present');
  assert.ok(p.includes('Scaffolding a new component'), 'default rubric line present');
});

test('custom rubric overrides the default lines', () => {
  const p = E.batchPrompt([rec({ key: 'R-1' })], ['Everything is exactly 2 days.']);
  assert.ok(p.includes('Everything is exactly 2 days.'));
  assert.ok(!p.includes('Scaffolding a new component'), 'default lines replaced');
});

test('older prompt-version estimates are re-selected', () => {
  const r = rec({ key: 'V-1' });
  const cache = { 'V-1': { hash: E.contentHash(r), days: 3, routine: false, v: 4 } };
  assert.equal(E.selectTargets([r], cache, 6, NOW).length, 1, 'an older-version entry re-estimates under the current prompt');
});

// ── Feature (epic-level) estimation ─────────────────────────────────────────
const fs = require('fs'); const os = require('os'); const path = require('path');

const epicABC = { key: 'ABC-1', type: 'Epic', summary: 'Checkout revamp' };
const kidsABC = () => [
  rec({ key: 'ABC-2', summary: 'Cart API' }),
  rec({ key: 'ABC-3', summary: 'Payment step' }),
  rec({ key: 'ABC-4', summary: 'Receipt "email"\nflow' }),
];

test('featurePrompt lists every child and asks for the epic key', () => {
  const p = E.featurePrompt(epicABC, kidsABC());
  for (const k of ['ABC-2', 'ABC-3', 'ABC-4']) assert.ok(p.includes(`- ${k} Story`), `${k} listed`);
  assert.ok(p.includes('Feature ABC-1'));
  assert.ok(p.includes('{"key":"ABC-1","days":<number>}'));
  assert.ok(p.includes('"Receipt \\"email\\"\\nflow"'), 'child summaries JSON-escaped');
});

test('runFeatureEstimation: median consensus, stable key, re-ask on new child, cached not re-asked', async () => {
  const cache = {};
  let calls = 0;
  const answers = { a: 3, b: 10, c: 5 };
  const agentRun = async (_p, w) => { calls++; return `{"key":"ABC-1","days":${answers[w.provider]}}`; };
  const workers = [{ provider: 'a' }, { provider: 'b' }, { provider: 'c' }];
  const r1 = await E.runFeatureEstimation({ epics: [{ epic: epicABC, kids: kidsABC() }], cache, agentRun, workers, paceMs: 0 });
  assert.equal(r1.estimated, 1);
  assert.equal(cache['ABC-1'].days, 5, 'median of 3/10/5');
  assert.deepEqual(cache['ABC-1'].samples, [3, 5, 10]);
  const hash = cache['ABC-1'].hash;

  // Reordered children → same key → no calls.
  const r2 = await E.runFeatureEstimation({ epics: [{ epic: epicABC, kids: kidsABC().reverse() }], cache, agentRun, workers, paceMs: 0 });
  assert.equal(r2.estimated, 0); assert.equal(calls, 3); assert.equal(cache['ABC-1'].hash, hash);

  // A new child → new key → re-asked.
  const more = [...kidsABC(), rec({ key: 'ABC-5', summary: 'Refunds' })];
  const r3 = await E.runFeatureEstimation({ epics: [{ epic: epicABC, kids: more }], cache, agentRun, workers, paceMs: 0 });
  assert.equal(r3.estimated, 1); assert.equal(calls, 6);
  assert.notEqual(cache['ABC-1'].hash, hash);
  assert.ok(cache['ABC-1'].kids.includes('ABC-5'));
});

test('runFeatureEstimation: failing worker falls back to the others; all failing leaves cache untouched', async () => {
  const cache = {};
  const agentRun = async (_p, w) => { if (w.provider === 'bad') throw new Error('down'); return 'noise {"key":"ABC-1","days":8} noise'; };
  await E.runFeatureEstimation({ epics: [{ epic: epicABC, kids: kidsABC() }], cache, agentRun, workers: [{ provider: 'bad' }, { provider: 'ok' }], paceMs: 0 });
  assert.equal(cache['ABC-1'].days, 8);
  assert.deepEqual(cache['ABC-1'].samples, [8]);

  const empty = {};
  const r = await E.runFeatureEstimation({ epics: [{ epic: epicABC, kids: kidsABC() }], cache: empty, agentRun: async () => { throw new Error('x'); }, workers: [{ provider: 'bad' }], paceMs: 0 });
  assert.equal(r.estimated, 0); assert.deepEqual(empty, {});
});

// ── Corrections block in the batch prompt ───────────────────────────────────
test('batchPrompt corrections: original → corrected, capped, JSON-escaped, absent when none', () => {
  const none = E.batchPrompt([rec({})], null, [], '');
  assert.ok(!none.includes('CORRECTED'), 'no block without corrections');
  assert.ok(!E.batchPrompt([rec({})], null, undefined, '').includes('CORRECTED'));

  const corr = [{ key: 'ABC-9', summary: 'Say "hi"\nthere', original: 2, corrected: 5, reason: 'had "hidden"\nmigration' }];
  for (let i = 0; i < 30; i++) corr.push({ key: `ABC-${100 + i}`, summary: `filler ${i}`, original: 1, corrected: 2 });
  const p = E.batchPrompt([rec({})], null, corr, '');
  assert.ok(p.includes('CORRECTED'));
  assert.ok(p.includes('"Say \\"hi\\"\\nthere": 2d → 5d'), 'original → corrected, escaped summary');
  assert.ok(p.includes('("had \\"hidden\\"\\nmigration")'), 'escaped reason');
  const block = p.slice(p.indexOf('CORRECTED'), p.indexOf('Also flag'));
  assert.equal((block.match(/^ {2}• /gm) || []).length, E.MAX_CORRECTIONS, 'capped at N');
  assert.ok(!block.includes('\nthere'), 'no raw newline injected');
  // Missing original: corrected only.
  assert.ok(E.correctionsBlock([{ key: 'ABC-7', corrected: 3 }]).includes('"ABC-7": 3d'));
});

// ── Debounced persistence ───────────────────────────────────────────────────
test('createDebouncedSaver: coalesces touches, flush writes once at the end', async () => {
  const writes = [];
  const obj = { a: 1 };
  const s = E.createDebouncedSaver('/x.json', obj, { intervalMs: 20, write: async (f, o) => writes.push([f, { ...o }]) });
  s.touch(); s.touch(); s.touch();
  await new Promise((r) => setTimeout(r, 50));
  assert.equal(writes.length, 1, 'one write per interval');
  obj.b = 2; s.touch();
  await s.flush();
  assert.equal(writes.length, 2);
  assert.deepEqual(writes[1][1], { a: 1, b: 2 });
  await s.flush();
  assert.equal(writes.length, 2, 'nothing pending → no write');
});

test('runEstimation persistTo writes the cache atomically at job end', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'est-'));
  const file = path.join(dir, 'estimates.json');
  const cache = {};
  const agentRun = async (prompt) => JSON.stringify([...prompt.matchAll(/key=([A-Z]+-\d+)/g)].map((m) => ({ key: m[1], days: 2 })));
  await E.runEstimation({ records: [rec({ key: 'ABC-2' }), rec({ key: 'ABC-3' })], cache, agentRun, nowMs: NOW, persistTo: file, saveEveryMs: 60000 });
  const saved = JSON.parse(fs.readFileSync(file, 'utf8'));
  assert.equal(saved['ABC-2'].days, 2); assert.equal(saved['ABC-3'].days, 2);
  fs.rmSync(dir, { recursive: true, force: true });
});
