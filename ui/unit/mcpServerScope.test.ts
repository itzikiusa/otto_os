import { test } from 'node:test';
import assert from 'node:assert/strict';
import { scopeBadge, scopeBody, scopeConfirm, scopeMode, scopeOf, widens } from '../src/modules/mcp/serverScope.ts';

const names = (id: string) => ({ a: 'Alpha', b: 'Beta', c: 'Gamma' })[id] ?? id;
const server = (over: Partial<{ workspace_id: string; all_workspaces: boolean; shared_workspace_ids: string[] }> = {}) => ({
  workspace_id: 'a',
  all_workspaces: false,
  shared_workspace_ids: [] as string[],
  ...over,
});

test('the mode follows the stored scope; an empty selection is home only', () => {
  assert.equal(scopeMode(scopeOf(server())), 'this');
  assert.equal(scopeMode(scopeOf(server({ shared_workspace_ids: ['b'] }))), 'selected');
  assert.equal(scopeMode(scopeOf(server({ all_workspaces: true }))), 'all');
  assert.deepEqual(scopeBody({ all: true, shared: ['b'] }), { all_workspaces: true, shared_workspace_ids: [] });
  assert.deepEqual(scopeBody({ all: false, shared: ['b'] }), { all_workspaces: false, shared_workspace_ids: ['b'] });
});

test('only a wider scope asks first', () => {
  const home = { all: false, shared: [] };
  assert.ok(widens(home, { all: false, shared: ['b'] }));
  assert.ok(widens({ all: false, shared: ['b'] }, { all: true, shared: [] }));
  assert.ok(widens({ all: false, shared: ['b'] }, { all: false, shared: ['b', 'c'] }));
  assert.ok(!widens({ all: false, shared: ['b', 'c'] }, { all: false, shared: ['b'] }));
  assert.ok(!widens({ all: true, shared: [] }, { all: false, shared: ['b'] }));
  assert.ok(!widens(home, home));
});

test('the confirm says where it goes, who can use it, and what stays private', () => {
  const all = scopeConfirm('gh', { all: true, shared: [] }, names);
  assert.match(all.message, /every workspace except Scratch — also ones added later —/);
  assert.match(all.message, /tools/);
  assert.match(all.message, /credentials and sign-in it holds/);
  assert.match(all.message, /can’t read or change them, or its settings/);
  assert.match(all.message, /access rules still decide/);
  assert.match(scopeConfirm('gh', { all: false, shared: ['b', 'c'] }, names).message, /“Beta”, “Gamma”/);
});

test('the badge names the home of a shared-in server and the reach of an own one', () => {
  assert.equal(scopeBadge(server(), 'a', names), null);
  assert.equal(scopeBadge(server(), 'b', names), 'Shared from Alpha');
  assert.equal(scopeBadge(server({ all_workspaces: true }), 'a', names), 'All workspaces');
  assert.equal(scopeBadge(server({ shared_workspace_ids: ['b'] }), 'a', names), 'Shared with Beta');
  assert.equal(scopeBadge(server({ shared_workspace_ids: ['b', 'c'] }), 'a', names), 'Shared with 2 workspaces');
});
