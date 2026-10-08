import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource, deferred } from './sourceHarness.ts';
import { componentFunctions } from './componentFunctions.ts';
import { plural } from '../src/lib/plural.ts';

test('production scaling confirms the replica count actually submitted', async () => {
  const confirmations: any[] = [], writes: any[] = [];
  const actions = loadSource(new URL('../src/modules/kubernetes/actions.ts', import.meta.url), {
    '../../lib/api/k8s': { k8sApi: { action: async (...args: any[]) => { writes.push(args); return { ok: true }; } } },
    '../../lib/confirm.svelte': { confirmer: {} },
    '../../lib/confirmProd': { isProdEnv: () => true, confirmProd: async (details: any) => { confirmations.push(details); return true; } },
    '../../lib/toast.svelte': { toasts: { success() {} } },
    '../../lib/stores/k8s.svelte': { k8s: { clusters: [{ id: 'prod', name: 'Production', environment: 'prod' }] } },
    './k8s-util': { clusterLabel: (c: any) => c.name, kindDef: () => ({ singular: 'Deployment' }) },
    '../../lib/toastError': { toastError: assert.fail },
  });
  for (const replicas of [0, 3]) {
    await actions.runAction('prod', 'deployments', { name: 'payments', namespace: 'live' }, { id: 'scale', label: 'Scale…' }, { replicas });
    assert.match(confirmations.at(-1).what, new RegExp(`to ${replicas} replicas`));
    assert.equal(writes.at(-1)[1].params.replicas, replicas);
  }
});

test('partial replay is a warning with retained evidence and an explicit duplicate-risk retry', async () => {
  const confirmations: any[] = [], successes: any[] = [], warnings: any[] = [];
  const response = { replay_id: 'r1', count: 1, source_topic: 'source', target_topic: 'target',
    evidence: [{ partition: 0, offset: 10, target_partition: 0, target_offset: 20 }],
    error: 'Injected second-send failure', evidence_saved: true };
  const c = componentFunctions(new URL('../src/modules/brokers/ReplayPanel.svelte', import.meta.url), ['runReplay', 'buildSelector'], {
    cluster: { id: 'c1', name: 'Cluster', environment: 'dev', read_only: false }, guarded: false,
    running: false, blockReason: null, sourceTopic: 'source', targetTopic: 'target',
    selectorType: 'latest', count: 3, setKey: '', addHeaderKey: '', addHeaderVal: '', result: null, plural,
    confirmProd: async (details: any) => { confirmations.push(details); return true; },
    api: { post: async () => response }, toastError: assert.fail,
    toasts: { success: (...args: any[]) => successes.push(args), warn: (...args: any[]) => warnings.push(args) },
  });
  await c.runReplay();
  assert.equal(successes.length, 0, 'HTTP success carrying a stopped replay is not full success');
  assert.equal(warnings.length, 1);
  assert.equal(c.result.evidence[0].offset, 10);
  await c.runReplay();
  assert.match(confirmations[1].what, /1.*already.*duplicate/i);
});

test('EC2 approval retains the account and region shown before an asynchronous route change', async () => {
  const approval = deferred<boolean>();
  const writes: any[] = [], confirmations: any[] = [];
  const c = componentFunctions(new URL('../src/modules/aws/Ec2View.svelte', import.meta.url), ['act'], {
    account: { id: 'account-A', name: 'Account A', environment: 'prod' },
    region: 'eu-west-1', resourceAccess: { can: () => true },
    confirmProd: (details: any) => { confirmations.push(details); return approval.promise; },
    busy: {}, awsApi: { ec2Action: async (...args: any[]) => { writes.push(args); return {}; } },
    toasts: { success() {} }, toastError: assert.fail, load: async () => {},
  });
  c.rowRegion = (i: any) => i.region ?? c.region;
  const running = c.act({ instance_id: 'i-123', name: 'payments' }, 'stop');
  assert.match(confirmations[0].where, /Account A · eu-west-1/);
  c.region = 'us-east-1';
  c.account = { id: 'account-B', name: 'Account B', environment: 'prod' };
  approval.resolve(true);
  await running;
  assert.deepEqual(writes, [['account-A', 'i-123', 'stop', 'eu-west-1']]);
});

for (const kind of ['Rds', 'Eks']) {
  test(`${kind} details distinguish the same resource name in different regions`, async () => {
    const old = deferred<any>();
    const c = componentFunctions(new URL(`../src/modules/aws/${kind}View.svelte`, import.meta.url), ['openDetail'], {
      account: { id: 'a' }, detail: null, detailRequest: 0, drawerTab: 'overview',
      rowRegion: (row: any) => row.region,
      awsApi: {
        rdsInstance: (_a: string, _n: string, region: string) => region === 'eu-west-1' ? old.promise : Promise.resolve({ endpoint: 'new' }),
        eksCluster: (_a: string, _n: string, region: string) => region === 'eu-west-1' ? old.promise : Promise.resolve({ endpoint: 'new' }),
      },
    });
    const first = c.openDetail({ name: 'orders', identifier: 'orders', region: 'eu-west-1' });
    await c.openDetail({ name: 'orders', identifier: 'orders', region: 'us-east-1' });
    old.resolve({ endpoint: 'old' }); await first;
    assert.equal((c.detail.inst ?? c.detail.c).region, 'us-east-1');
    assert.equal((c.detail.full ?? c.detail.d).endpoint, 'new');
  });
}

test('SQS deletion stays bound to the approved queue and preserves a newly selected queue', async () => {
  const approval = deferred<boolean>(); const writes: any[] = [];
  const c = componentFunctions(new URL('../src/modules/aws/SqsView.svelte', import.meta.url), ['deleteMessage'], {
    account: { id: 'account-A' }, selected: { name: 'A', url: 'queue-A' }, rq: 'eu-west-1',
    queueWhere: (n: string) => n, confirmProd: () => approval.promise,
    awsApi: { sqsDeleteMessage: async (...args: any[]) => writes.push(args) },
    aws: { loadSqsAttrs: async () => {} }, toasts: { success() {} }, toastError: assert.fail,
    messages: [{ message_id: 'message-A' }],
  });
  const deleting = c.deleteMessage({ message_id: 'message-A', receipt_handle: 'receipt-A' });
  c.selected = { name: 'B', url: 'queue-B' }; c.rq = 'us-east-1';
  c.messages = [{ message_id: 'message-A', body: 'different queue' }];
  approval.resolve(true); await deleting;
  assert.deepEqual(writes, [['account-A', 'queue-A', 'receipt-A', 'eu-west-1']]);
  assert.equal(c.messages.length, 1);
});
