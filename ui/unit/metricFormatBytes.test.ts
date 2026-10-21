import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formatBytes } from '../src/lib/metric-format.ts';

// The one byte formatter every surface shares (lib/metric-format.ts).

test('formatBytes steps by 1024 past MB (no more "5120.0 MB")', () => {
  assert.equal(formatBytes(512), '512 B');
  assert.equal(formatBytes(1536), '1.5 KB');
  assert.equal(formatBytes(5 * 1024 ** 3), '5 GB');
  assert.equal(formatBytes(1.25 * 1024 ** 3), '1.3 GB');
  assert.equal(formatBytes(2 * 1024 ** 4), '2 TB');
  assert.equal(formatBytes(150 * 1024 ** 2), '150 MB', 'no decimal at 100 and above');
});

test('formatBytes renders a missing size as the surface placeholder', () => {
  for (const v of [null, undefined, Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.equal(formatBytes(v), '—');
    assert.equal(formatBytes(v, { empty: '' }), '');
  }
  assert.equal(formatBytes(0), '0 B', 'zero is a size, not a gap');
});

test('formatBytes iec: kubectl-style suffixes, no space', () => {
  assert.equal(formatBytes(128 * 1024 ** 2, { iec: true }), '128Mi');
  assert.equal(formatBytes(1.5 * 1024 ** 3, { iec: true }), '1.5Gi');
  assert.equal(formatBytes(2048, { iec: true }), '2Ki');
  assert.equal(formatBytes(512, { iec: true }), '512 B');
  assert.equal(formatBytes(null, { iec: true, empty: '' }), '');
});
