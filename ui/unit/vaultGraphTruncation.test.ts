import { test } from 'node:test';
import assert from 'node:assert/strict';
import { truncationNote } from '../src/modules/vault/graphTruncation.ts';

const payload = (over: Record<string, unknown>) => ({ truncated: true, paths: new Array(300).fill('n.md'), edges: new Array(3000).fill(0), ...over });

test('a whole graph has no note', () => {
  assert.equal(truncationNote(payload({ truncated: false })), null);
});

test('a capped local graph says how many notes of at least how many', () => {
  const note = truncationNote(payload({ total_nodes: 1204, total_edges: 5000 }));
  assert.match(note!.label, /^Showing 300 of 1,204\+ notes$/);
});

test('a full graph over its edge budget counts links', () => {
  const note = truncationNote(payload({ total_edges: 2_500_000 }));
  assert.match(note!.label, /^Showing 1,500 of 2,500,000 links$/);
});

test('an older daemon without totals still says it is truncated', () => {
  assert.equal(truncationNote(payload({}))!.label, 'Truncated');
});
