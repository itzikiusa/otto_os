// Workspace-command session scope (node:test, Node's built-in type stripping).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { SCRATCH_WORKSPACE_ID, workspaceCommandScope } from '../src/lib/stores/sessionScope.ts';

const S = (id: string, workspace_id: string) => ({ id, workspace_id });
const store = [S('own-1', 'ws-a'), S('scratch-1', SCRATCH_WORKSPACE_ID), S('own-2', 'ws-a'), S('scratch-2', SCRATCH_WORKSPACE_ID)];
const ids = (xs: { id: string }[]) => xs.map((s) => s.id);

test('in a workspace, scratch sessions the store holds for the sidebar are out of scope', () => {
  // "close all shell sessions" in ws-a must not archive workspace-less sessions.
  assert.deepEqual(ids(workspaceCommandScope(store, 'ws-a', [])), ['own-1', 'own-2']);
});

test('a scratch session opened as a tab here is on screen, so it is in scope', () => {
  assert.deepEqual(ids(workspaceCommandScope(store, 'ws-a', ['scratch-2', 'own-1'])), ['own-1', 'own-2', 'scratch-2']);
});

test('with no workspace selected, scratch sessions are all there is', () => {
  assert.deepEqual(ids(workspaceCommandScope(store, null, [])), ids(store));
});
