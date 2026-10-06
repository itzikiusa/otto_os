import { test } from 'node:test';
import assert from 'node:assert/strict';
import { truncatedNote, unwrapListing } from '../src/lib/listingPage.ts';

test('a meta listing carries its truncation flag', () => {
  assert.deepEqual(unwrapListing({ items: [1, 2], truncated: true }), { items: [1, 2], truncated: true });
  assert.deepEqual(unwrapListing({ items: [1], truncated: false }), { items: [1], truncated: false });
});

test('a bare array (an older daemon) reads as complete', () => {
  assert.deepEqual(unwrapListing([1, 2, 3]), { items: [1, 2, 3], truncated: false });
  assert.deepEqual(unwrapListing(null), { items: [], truncated: false });
});

test('the note names how many are shown', () => {
  assert.match(truncatedNote('projects', 2000), /first 2000 projects/);
});
