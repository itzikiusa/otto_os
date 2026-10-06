// Settings → Users, "By user": removing a user from EVERY workspace is
// irreversible (access revoked everywhere, role history dropped), so
// setRoleEverywhere('none') asks first and does nothing on Cancel; setting a
// role everywhere stays one click.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';

const USERS = new URL('../src/modules/settings/Users.svelte', import.meta.url);

function harness(answer: boolean) {
  const asked: string[] = [];
  const writes: [string, string, string][] = [];
  const roles: Record<string, string> = { w1: 'editor', w2: 'viewer' };
  const state: Record<string, any> = {
    memberUserId: 'u1',
    savingWs: [],
    allMembersLoading: false,
    allMembersError: null,
    users: [{ id: 'u1', username: 'dana' }],
    ws: { workspaces: [{ id: 'w1' }, { id: 'w2' }] },
    roleIn: (w: string) => roles[w],
    setRoleIn: async (w: string, u: string, r: string) => (writes.push([w, u, r]), true),
    plural: (n: number, w: string) => `${n} ${w}${n === 1 ? '' : 's'}`,
    confirmer: { ask: async (msg: string) => (asked.push(msg), answer) },
    toasts: { success() {}, error() {} },
  };
  const fns = componentFunctions(USERS, ['setRoleEverywhere'], state);
  return { run: fns.setRoleEverywhere as (r: string) => Promise<void>, asked, writes };
}

test('Remove from all workspaces confirms, and Cancel writes nothing', async () => {
  const h = harness(false);
  await h.run('none');
  assert.equal(h.asked.length, 1);
  assert.match(h.asked[0], /Remove @dana from 2 workspaces/);
  assert.deepEqual(h.writes, []);
});

test('confirmed removal writes every workspace', async () => {
  const h = harness(true);
  await h.run('none');
  assert.deepEqual(h.writes.map((w) => w[0]).sort(), ['w1', 'w2']);
});

test('setting a role everywhere does not ask', async () => {
  const h = harness(false);
  await h.run('admin');
  assert.equal(h.asked.length, 0);
  assert.equal(h.writes.length, 2);
});
