// IDE bug hunt, wave 2 (environment layer): SSH files skip Local History, a `~`
// remote root resolves to the user's home (V2), and the first ⌘I inline edit on a
// session that is still starting is not refused as "still answering" (V7).
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { loadSource } from './sourceHarness.ts';

const ideUrl = (rel: string) => new URL(`../src/modules/ide/${rel}`, import.meta.url);

test('V2: editing an SSH file posts no Local History snapshot', () => {
  const posts: string[] = [];
  const api = { bg: { post: (url: string) => { posts.push(url); return Promise.resolve({}); } } };
  const buf = loadSource(ideUrl('store/ideBufferPerf.ts'), {
    svelte: { tick: async () => {} },
    '../../../lib/api/client': { api, ApiError: class extends Error {} },
    '../../../lib/components/cm-ide-drafts': { readIdeUndoDraft: () => null, removeIdeUndoDraft: () => {}, writeIdeUndoDraftImmediate: () => {} },
    '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({ localHistoryEnabled: true }) },
    '../../../lib/confirm.svelte': { confirmer: {} },
    '../../../lib/toast.svelte': { toasts: {} },
    '../../../lib/toastError': { toastError: () => {} },
  }, {}, { relative: true });
  const perf = new buf.IdeBufferPerfController();
  const host = (kind: string) => ({
    collabIsGuest: false,
    target: { kind, root: '/home/dev/rproj' },
    targetPayload: { kind, root: '/home/dev/rproj', project_folders: [] },
  });
  perf.pushHistorySnapshot(host('ssh'), '/home/dev/rproj/main.py', 'x = 1', 'Save');
  assert.deepEqual(posts, [], 'the daemon refuses Local History for an SSH target');
  perf.pushHistorySnapshot(host('local'), '/home/dev/rproj/main.py', 'x = 1', 'Save');
  assert.deepEqual(posts, ['/ide/history/snapshot']);
});

test('V2: an SSH project root of "~" or "~/x" is the user\'s home, never a literal ~', () => {
  const guard = loadSource(ideUrl('shell/networkBridgeGuard.svelte.ts'), {
    '../../../lib/api/client': { api: {} },
    '../../../lib/confirm.svelte': { confirmer: { ask: async () => true } },
    '../../../lib/stores/workspace.svelte': { SCRATCH_WORKSPACE_ID: 'scratch', ws: {} },
    './networkBridgeModel': { describeProdBridgeTarget: () => null, isProdEnvironment: () => false },
  });
  const dev = { params: { user: 'dev' } };
  assert.equal(guard.absoluteRemoteRoot(dev, '~'), '/home/dev');
  assert.equal(guard.absoluteRemoteRoot(dev, ''), '/home/dev');
  assert.equal(guard.absoluteRemoteRoot(dev, '~/rproj'), '/home/dev/rproj');
  assert.equal(guard.absoluteRemoteRoot({ params: { user: 'root' } }, '~'), '/root');
  assert.equal(guard.absoluteRemoteRoot(dev, ' /srv/app '), '/srv/app');
});

test('V7: a session that is still starting has no chat turn, but a prompt to it is still queued', () => {
  const ops = loadSource(ideUrl('agent/ideAgentRuntimeOps.svelte.ts'), {
    '../ideStore.svelte': { ide: {}, ideBasename: (p: string) => p },
    '../store/ideDiffApplier': {},
    './ideAgentState.svelte': { ideAgentState: {} },
    './ideContextBuilders': {},
  });
  const bridge = (status: string, turns: { status: string }[] = []) => ({
    runtimeThreads: { get: () => ({ status }) },
    runtimeViews: { projections: { s1: { turns, items: [] } } },
  });
  // The first ⌘I creates the session (status `starting`): that is not "the agent is still answering".
  assert.equal(ops.isChatTurnRunning(bridge('starting'), 's1'), false);
  assert.equal(ops.isRuntimeTurnBusy(bridge('starting'), 's1'), true, 'dispatch still queues behind the start');
  assert.equal(ops.isChatTurnRunning(bridge('working'), 's1'), true);
  assert.equal(ops.isChatTurnRunning(bridge('idle', [{ status: 'running' }]), 's1'), true);
  assert.equal(ops.isChatTurnRunning(bridge('idle'), 's1'), false);
});
