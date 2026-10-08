import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';

for (const file of ['ArtifactView', 'brand/BrandEditor']) {
  test(`${file} approves the version shown in the confirmation despite live updates`, async () => {
    const confirm = deferred<boolean>();
    const calls: unknown[][] = [];
    const state = componentFunctions(new URL(`../src/modules/design-hall/${file}.svelte`, import.meta.url), ['approve'], {
      id: 'A', artifact: { id: 'A', title: 'Design A' }, head: { id: 'v1', seq: 1 }, dirty: false,
      detail: {}, split: { usedIn: [] }, usage: { consumers: [] }, approved: null,
      confirmer: { ask: () => confirm.promise },
      api: { approveArtifact: async (...args: unknown[]) => { calls.push(args); return { id: 'A' }; } },
      toasts: { success() {}, error() {} }, toastError: (what: string, err: unknown) => { throw err; },
      loadUsage() {}, plural: (n: number) => String(n), errText: String,
    });
    const pending = state.approve();
    state.head = { id: 'v2', seq: 2 };
    confirm.resolve(true); await pending;
    assert.deepEqual(calls, [['A', 'v1']]);
    assert.equal((file === 'ArtifactView' ? state.detail.approved : state.approved).id, 'v1');
  });
}

for (const action of ['renameProject', 'archiveProject']) {
  test(`project ${action} keeps the confirmed project across route changes`, async () => {
    const confirm = deferred<any>(); const calls: unknown[][] = []; const navigation: string[] = [];
    const state = componentFunctions(new URL('../src/modules/design-hall/CollectionView.svelte', import.meta.url), [action], {
      project: { id: 'A', name: 'Project A' },
      confirmer: { ask: () => confirm.promise, promptText: () => confirm.promise },
      updateProject: async (...args: unknown[]) => { calls.push(args); },
      library: { load() {} }, toasts: { success() {} }, toastError: (_: string, err: unknown) => { throw err; },
      router: { go: (route: string) => navigation.push(route) },
    });
    const pending = state[action](); state.project = { id: 'B', name: 'Project B' };
    confirm.resolve(action === 'renameProject' ? 'Renamed A' : true); await pending;
    assert.equal(calls[0][0], 'A');
    assert.deepEqual(navigation, [], 'completion must not navigate away from B');
  });
}

for (const action of ['sendToSwarm', 'regenerate']) {
  test(`Product ${action} does not act after its confirmed story is replaced`, async () => {
    const confirm = deferred<boolean>(); const calls: unknown[] = []; let current = true;
    const state = componentFunctions(new URL('../src/modules/product/PlanTab.svelte', import.meta.url), [action], {
      sendingToSwarm: false, ws: { currentId: 'ws-A' }, swarmLink: null, targetSwarmId: 'team-A',
      product: { detail: { story: { stage: 'approved' } }, captureSelection: () => () => current, sendToSwarm: async () => { calls.push('sent'); } },
      swarm: { swarms: [{ id: 'team-A', name: 'Team A' }] },
      confirmer: { ask: () => confirm.promise }, generate: async () => calls.push('generated'),
      toastError() {}, toasts: { success() {} },
    });
    const pending = state[action](); current = false; confirm.resolve(true); await pending;
    assert.deepEqual(calls, []);
  });
}

test('Product question posting does not follow a changed story after consent', async () => {
  const confirm = deferred<boolean>(); const calls: unknown[] = []; let current = true;
  const state = componentFunctions(new URL('../src/modules/product/QuestionsTab.svelte', import.meta.url), ['postSelected'], {
    selectedIds: new Set(['q-A']), postingIds: false, postTarget: 'JIRA-A', isJira: true,
    story: { source_key: 'JIRA-A', title: 'Story A' },
    product: { questions: [{ id: 'q-A', text: 'Question A' }], captureSelection: () => () => current, postQuestions: async () => calls.push('posted') },
    confirmOutward: () => confirm.promise, plural: (n: number) => String(n),
    toasts: { success() {} }, toastError() {},
  });
  const pending = state.postSelected(); current = false; confirm.resolve(true); await pending;
  assert.deepEqual(calls, []);
});
