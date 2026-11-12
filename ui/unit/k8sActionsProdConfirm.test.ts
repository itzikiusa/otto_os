import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function load(env: string, resp: any = { ok: true, message: 'done' }) {
  const writes: any[] = [], warnings: any[] = [];
  const actions = loadSource(new URL('../src/modules/kubernetes/actions.ts', import.meta.url), {
    '../../lib/api/k8s': { k8sApi: { action: async (...args: any[]) => { writes.push(args); return resp; } } },
    '../../lib/confirmProd': { isProdEnv: (e: string) => e === 'prod', confirmProd: async () => true },
    '../../lib/toast.svelte': { toasts: { success() {}, warn: (...a: any[]) => warnings.push(a) } },
    '../../lib/stores/k8s.svelte': { k8s: { clusters: [{ id: 'c', name: 'Cluster', environment: env }] } },
    './k8s-util': { clusterLabel: (c: any) => c.name, kindDef: () => ({ singular: 'Deployment' }) },
    '../../lib/k8s/actions': {},
    '../../lib/toastError': { toastError: assert.fail },
  });
  return { actions, writes, warnings };
}

test('a non-destructive action on a prod cluster sends the typed name the daemon requires', async () => {
  const { actions, writes } = load('prod');
  await actions.runAction('c', 'deployments', { name: 'web', namespace: 'live' }, { id: 'restart', label: 'Restart' });
  assert.equal(writes.at(-1)[1].params.confirm_name, 'web');
});

test('off prod a non-destructive action carries no confirm', async () => {
  const { actions, writes } = load('dev');
  await actions.runAction('c', 'deployments', { name: 'web', namespace: 'live' }, { id: 'restart', label: 'Restart' });
  assert.equal(writes.at(-1)[1].params, undefined);
});

test('a partially applied multi-step action names every step in the warning', async () => {
  const resp = {
    ok: false,
    message: 'rollout web promoted — FAILED at step 2 of 3',
    steps: [
      { step: 'patch rollouts.argoproj.io/web (status)', status: 'done' },
      { step: 'patch rollouts.argoproj.io/web (status)', status: 'failed', error: 'forbidden' },
      { step: 'patch rollouts.argoproj.io/web', status: 'skipped' },
    ],
  };
  const { actions, warnings } = load('dev', resp);
  await actions.runAction('c', 'rollouts', { name: 'web', namespace: 'live' }, { id: 'rollout_promote', label: 'Promote' });
  const detail = warnings.at(-1)[1] as string;
  assert.match(detail, /FAILED at step 2 of 3/);
  assert.match(detail, /: done; .*: failed; .*: skipped$/);
  assert.equal(actions.partialDetail({ ok: false, message: 'm' }), 'm');
});
