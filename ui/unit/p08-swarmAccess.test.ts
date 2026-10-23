import { test } from 'node:test';
import assert from 'node:assert/strict';
import { canEditSwarmIdentity, canOpenRunSession } from '../src/modules/swarm/access.ts';

const editor = { isRoot: false, myRole: 'editor', me: 'bob' };

test('only the swarm creator, a ws admin or root edits an agent identity', () => {
  assert.equal(canEditSwarmIdentity(editor, 'alice'), false, 'a non-creator editor');
  assert.equal(canEditSwarmIdentity(editor, 'bob'), true, 'the creator');
  assert.equal(canEditSwarmIdentity({ ...editor, myRole: 'admin' }, 'alice'), true, 'a ws admin');
  assert.equal(canEditSwarmIdentity({ ...editor, isRoot: true }, 'alice'), true, 'root');
  assert.equal(canEditSwarmIdentity({ ...editor, me: null }, null), false, 'unknown caller and creator');
  assert.equal(canEditSwarmIdentity({ ...editor, myRole: 'viewer' }, 'alice'), false);
});

test('a run session opens only from the caller store, or for root / ws admin', () => {
  assert.equal(canOpenRunSession(editor, true), true, 'my own session');
  assert.equal(canOpenRunSession(editor, false), false, 'someone else’s session');
  assert.equal(canOpenRunSession({ ...editor, myRole: 'admin' }, false), true);
  assert.equal(canOpenRunSession({ ...editor, isRoot: true }, false), true);
});
