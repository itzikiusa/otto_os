import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import { componentFunctions } from './componentFunctions.ts';

function k9s(typedOk: boolean) {
  const menus: any[] = [], confirms: any[] = [];
  const mod = loadSource(new URL('../src/modules/kubernetes/k9sLaunch.ts', import.meta.url), {
    '../../lib/confirmProd': {
      isProdEnv: (e: string) => e === 'prod',
      confirmProd: async (o: any) => { confirms.push(o); return typedOk; },
    },
    '../../lib/contextmenu.svelte': { ctxMenu: { show: (_e: any, items: any[]) => menus.push(items) } },
    './k8s-util': { clusterLabel: (c: any) => c.name },
  });
  return { mod, menus, confirms };
}

test('k9s off prod opens straight away with no write flag', () => {
  const { mod, menus } = k9s(true);
  const opened: any[] = [];
  mod.chooseK9sMode({}, { name: 'dev', environment: 'dev' }, (m: any) => opened.push(JSON.parse(JSON.stringify(m))));
  assert.deepEqual(opened, [{}]);
  assert.equal(menus.length, 0);
});

test('k9s on prod offers read-only first; write access only after the typed cluster name', async () => {
  const { mod, menus, confirms } = k9s(true);
  const opened: any[] = [];
  mod.chooseK9sMode({}, { name: 'payments', environment: 'prod' }, (m: any) => opened.push(JSON.parse(JSON.stringify(m))));
  const [readOnly, write] = menus[0];
  assert.match(readOnly.label, /read-only/);
  readOnly.action();
  assert.deepEqual(opened, [{}]);
  write.action();
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(confirms[0].typed, 'payments');
  assert.deepEqual(opened[1], { write: true, confirm_name: 'payments' });
});

test('a declined write confirm opens nothing', async () => {
  const { mod, menus } = k9s(false);
  const opened: any[] = [];
  mod.chooseK9sMode({}, { name: 'payments', environment: 'prod' }, (m: any) => opened.push(m));
  menus[0][1].action();
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(opened.length, 0);
});

test('a prod shell sends the pod name the daemon requires', async () => {
  const posted: any[] = [];
  const c = componentFunctions(new URL('../src/modules/kubernetes/ExecView.svelte', import.meta.url), ['open'], {
    canExec: true, opening: false, isProd: true, error: '', sessionId: null, status: null,
    container: '', clusterId: 'c', ns: 'shop', pod: 'web-1', alive: true, cluster: { name: 'prod' },
    ws: { currentId: 'w1' }, NO_WORKSPACE: 'x', clusterLabel: (x: any) => x.name,
    confirmProd: async () => true,
    k8sApi: { exec: async (_id: string, body: any) => { posted.push(body); return { id: 's1' }; } },
    api: { del: async () => {} },
  });
  await c.open();
  assert.equal(posted[0].confirm_name, 'web-1');
});
