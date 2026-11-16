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

test('V1: adding or removing one folder of a named project saves the project on the server', async () => {
  const posts: { url: string; body: { folders: string[] } }[] = [];
  const api = { post: async (url: string, body: { folders: string[] }) => { posts.push({ url, body }); return {}; } };
  const projects = loadSource(ideUrl('shell/ideProjectsManager.ts'), {
    svelte: { tick: async () => {}, untrack: (f: () => unknown) => f() },
    '../../../lib/api/client': { api, ApiError: class extends Error {} },
    '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({}) },
    '../../../lib/confirm.svelte': { confirmer: {} },
    '../../../lib/toast.svelte': { toasts: {} },
    '../../../lib/toastError': { toastError: () => {} },
    '../../../lib/storage': { lsGet: () => null, lsSet: () => {}, lsRemove: () => {} },
    '../../../lib/ideProjectsModel': { loadNamedIdeProjectsLocal: () => [], saveNamedIdeProjectsLocal: () => {}, fetchNamedIdeProjectsFromServer: async () => [], parseNamedIdeProject: () => null },
    '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: { setLastRoot: () => {} } },
    '../../../lib/components/cm-ide-lsp': { disposePooledLspClients: async () => {} },
  }, {}, { relative: true });
  const proj = { id: 'p1', name: 'Hunt Multi', primary_root: '/w/currency', folders: ['/w/currency', '/w/cfg', '/w/club'], pinned: true, updated_at: 0 };
  const host = {
    namedProjects: [proj], activeProjectId: 'p1',
    target: { kind: 'local', root: '/w/currency' }, additionalRoots: ['/w/cfg'],
  };
  await projects.syncCurrentProjectFolders(host);
  assert.equal(posts.length, 1, 'Remove club-manager reaches the server, so a reload keeps it removed');
  assert.equal(posts[0].url, '/ide/projects');
  assert.deepEqual([...posts[0].body.folders], ['/w/currency', '/w/cfg']);
  assert.deepEqual([...host.namedProjects[0].folders], ['/w/currency', '/w/cfg']);
  await projects.syncCurrentProjectFolders(host);
  assert.equal(posts.length, 1, 'nothing to save when the folders already match');
});

test('V1: connecting to an SSH host leaves the named project, so its tabs are not overwritten', async () => {
  const fs = await import('node:fs');
  const store = fs.readFileSync(ideUrl('ideStore.svelte.ts'), 'utf8');
  const start = store.indexOf('async setSshTarget(');
  const body = store.slice(start, store.indexOf('\n  }', start));
  assert.match(body, /this\.activeProjectId = null/);
  assert.match(body, /saveLastSshTarget\(\{/, 'and a reload reopens the SSH host');
  assert.ok(body.indexOf('snapshotProjectTabs') < body.indexOf('this.activeProjectId = null'), 'the project keeps its own local tabs');
});
