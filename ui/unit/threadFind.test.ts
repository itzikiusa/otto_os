import { test } from 'node:test';
import assert from 'node:assert/strict';
import { capToShow, rowFindText } from '../src/modules/threads/runtime/threadFind.ts';

test('a thread search reads message text only', () => {
  const msg = { kind: 'message', item: { type: 'assistant_message', id: 'a', text: 'Found it' } };
  assert.equal(rowFindText(msg as never), 'Found it');
  assert.equal(rowFindText({ kind: 'day', id: 'd', label: 'Today' }), '');
  assert.equal(rowFindText(undefined), '');
});

test('revealing an old row widens the mounted window just past it', () => {
  assert.equal(capToShow(450, 500, 200), 200, 'already mounted');
  assert.equal(capToShow(10, 500, 200), 510, 'row 10 plus context');
});
