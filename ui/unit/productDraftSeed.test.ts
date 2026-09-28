// Product draft form seeding (r3-02-07): a replaced story detail must not
// overwrite the user's unsaved draft.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { reseedDraft, type DraftSeed } from '../src/modules/product/draftSeed.ts';

const seed = (id: string | null, title: string, body: string): DraftSeed => ({ id, title, body });

test('a new story always seeds the form', () => {
  const out = reseedDraft(seed('a', 'A', 'a-body'), { title: 'edited', body: 'edited' }, seed('b', 'B', 'b-body'));
  assert.deepEqual(out, { title: 'B', body: 'b-body' });
});

test('a detail replacement keeps unsaved edits and refreshes untouched fields', () => {
  const prev = seed('a', 'A', 'body v1');
  // The user edited the body only; the server changed both fields.
  const out = reseedDraft(prev, { title: 'A', body: 'my unsaved text' }, seed('a', 'A2', 'body v2'));
  assert.deepEqual(out, { title: 'A2', body: 'my unsaved text' });
});

test('an unedited form follows the server', () => {
  const out = reseedDraft(seed('a', 'A', 'b1'), { title: 'A', body: 'b1' }, seed('a', 'A', 'b2'));
  assert.deepEqual(out, { title: 'A', body: 'b2' });
});

test('after the user saves, the saved text stays in the form', () => {
  const out = reseedDraft(seed('a', 'A', 'old'), { title: 'A', body: 'saved' }, seed('a', 'A', 'saved'));
  assert.deepEqual(out, { title: 'A', body: 'saved' });
});
