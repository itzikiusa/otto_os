import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  ResponseDiffHistory,
  canonicalizeJson,
  formatForDiff,
  snapshotFromResponse,
  summarizeResponseDelta,
} from '../src/modules/api/responseDiff.ts';

test('canonicalizeJson sorts nested object keys alphabetically while preserving array order', () => {
  const input = { z: 1, a: { c: 3, b: 2 }, list: [{ y: 1, x: 2 }, { b: 9, a: 8 }] };
  assert.equal(
    JSON.stringify(canonicalizeJson(input)),
    '{"a":{"b":2,"c":3},"list":[{"x":2,"y":1},{"a":8,"b":9}],"z":1}',
  );
});

test('formatForDiff canonicalizes valid JSON, preserves raw text, and caps oversized bodies', () => {
  const a = formatForDiff('{"b":2,"a":1}', 'application/json', true);
  assert.equal(a.isJson, true);
  assert.equal(a.truncated, false);
  assert.equal(a.text, '{\n  "a": 1,\n  "b": 2\n}');

  const raw = formatForDiff('{"b":2,"a":1}', 'application/json', false);
  assert.equal(raw.text, '{\n  "b": 2,\n  "a": 1\n}');

  const plain = formatForDiff('hello world', 'text/plain', true);
  assert.equal(plain.isJson, false);
  assert.equal(plain.text, 'hello world');

  const big = formatForDiff('x'.repeat(200), 'text/plain', true, 64);
  assert.equal(big.truncated, true);
  assert.ok(big.text.includes('truncated'));
});

test('summarizeResponseDelta detects identical canonical JSON and status/latency/size deltas', () => {
  const s1 = snapshotFromResponse(
    { status: 200, status_text: 'OK', duration_ms: 120, size_bytes: 20, content_type: 'application/json', body: '{"b":1,"a":2}' },
    'Baseline',
  );
  const s2 = snapshotFromResponse(
    { status: 200, status_text: 'OK', duration_ms: 95, size_bytes: 20, content_type: 'application/json', body: '{"a":2,"b":1}' },
    'Current',
  );
  const same = summarizeResponseDelta(s1, s2, true);
  assert.equal(same.identical, true);
  assert.equal(same.statusChanged, false);
  assert.equal(same.durationDeltaMs, -25);

  const s3 = snapshotFromResponse(
    { status: 500, status_text: 'Error', duration_ms: 300, size_bytes: 40, content_type: 'application/json', body: '{"error":"boom"}' },
    'Current',
  );
  const diff = summarizeResponseDelta(s1, s3, true);
  assert.equal(diff.identical, false);
  assert.equal(diff.statusChanged, true);
  assert.equal(diff.statusFrom, 200);
  assert.equal(diff.statusTo, 500);
  assert.equal(diff.sizeDeltaBytes, 20);
});

test('ResponseDiffHistory tracks per-tab previous & pinned snapshots with bounded eviction', () => {
  const h = new ResponseDiffHistory(2);
  const r1 = snapshotFromResponse({ status: 200, status_text: 'OK', duration_ms: 10, size_bytes: 4, content_type: null, body: 'v1' }, 'r1');
  const r2 = snapshotFromResponse({ status: 200, status_text: 'OK', duration_ms: 12, size_bytes: 4, content_type: null, body: 'v2' }, 'r2');

  h.observe('t1', r1);
  assert.equal(h.get('t1').previous, null);
  h.pin('t1', r1);
  assert.equal(h.get('t1').pinned?.body, 'v1');

  h.observe('t1', r2);
  assert.equal(h.get('t1').previous?.body, 'v1');
  assert.equal(h.get('t1').latest?.body, 'v2');

  h.unpin('t1');
  assert.equal(h.get('t1').pinned, null);

  // Evicts oldest tab beyond maxTabs.
  h.observe('t2', r1);
  h.observe('t3', r2);
  assert.equal(h.get('t1').latest, null);
  assert.equal(h.get('t3').latest?.body, 'v2');
});
