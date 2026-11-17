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
  assert.match(body, /leaveProjectForSsh\(this, \{ connection_id: connectionId/);
  const mgr = fs.readFileSync(ideUrl('shell/ideProjectsManager.ts'), 'utf8');
  const leave = mgr.slice(mgr.indexOf('export function leaveProjectForSsh('), mgr.indexOf('\n}\n', mgr.indexOf('export function leaveProjectForSsh(')));
  assert.match(leave, /host\.activeProjectId = null/);
  assert.match(leave, /saveLastSshTarget\(ssh\)/, 'and a reload reopens the SSH host');
  assert.ok(leave.indexOf('snapshotProjectTabs') < leave.indexOf('host.activeProjectId = null'), 'the project keeps its own local tabs');
});

test('V6: after a dependency action the daemon wrote, the manifest tab reloads from disk', async () => {
  const fs = await import('node:fs');
  const read = (rel: string) => fs.readFileSync(ideUrl(rel), 'utf8');
  for (const rel of ['editor/IdeDependencyAnalyzer.svelte', 'editor/IdeIntentionPopover.svelte']) {
    assert.doesNotMatch(read(rel), /onapplycontent\(res\.updated_content\)/, `${rel}: no edit over the old sha`);
    assert.match(read(rel), /onfilewritten\(\)/);
  }
  assert.match(read('IdeEditorArea.svelte'), /reloadIdeTabIfOpen\(path\)/);
});

test('V9: a plugin switch in Settings waits for Apply, and Cancel drops it', async () => {
  const toggles: [string, boolean][] = [];
  const ide = {
    plugins: [{ slug: 'kotlin', enabled: true }],
    projectEnv: null, projectRoots: [], target: { root: '' }, hasTarget: false, activeProject: null, settingsOpen: true,
    setKeymapPreset: () => {}, setFormatOnSave: () => {},
    togglePlugin: async (slug: string, enabled: boolean) => {
      toggles.push([slug, enabled]);
      ide.plugins = ide.plugins.map((p) => (p.slug === slug ? { ...p, enabled } : p));
      return true;
    },
  };
  const prefs = { theme: 'x', moduleSdkMap: {} };
  const load = () => loadSource(ideUrl('settings/ideSettingsStore.svelte.ts'), {
    '../../../lib/idePreferences.svelte': {
      cloneIdePreferences: (p: object) => ({ ...p }), DEFAULT_IDE_PREFERENCES: prefs, IDE_COLOR_SCHEME_OPTIONS: [],
      IDE_PREFS_STORAGE_KEY: 'k', readIdePreferences: () => prefs, writeIdePreferences: (p: object) => p, loadIdePreferences: () => prefs,
    },
    '../ideStore.svelte': { ide, ideBasename: (p: string) => p },
  });
  const { ideSettings } = load();
  ideSettings.stagePlugin('kotlin', false);
  assert.deepEqual(toggles, [], 'switching it off posts nothing yet');
  assert.equal(ideSettings.pluginDraft.kotlin, false, 'staged');
  ideSettings.cancel();
  assert.deepEqual(toggles, [], 'Cancel leaves the plugin on');
  ideSettings.stagePlugin('kotlin', false);
  await ideSettings.apply();
  assert.deepEqual(toggles, [['kotlin', false]], 'Apply turns it off');
  assert.equal(Object.keys(ideSettings.pluginDraft).length, 0);
});

test('V8: a host prepend and a guest append both survive; overlapping edits keep the peer\'s', async () => {
  const sync = loadSource(ideUrl('store/ideCollabSync.ts'), {
    '../../../lib/api/client': { laneFetch: async () => ({ ok: false }) },
    './ideCollabNotices.svelte': { ideCollabNotices: {} },
    './ideTypes': { IdeCollabHttpError: class extends Error {} },
  });
  const base = 'line1\nline2\n';
  assert.equal(sync.mergeText(base, `HOSTHOST${base}`, `${base}guestguest`), `HOSTHOST${base}guestguest`);
  assert.equal(sync.mergeText(base, base, 'x'), 'x', 'nothing local: take theirs');
  assert.equal(sync.mergeText(base, 'mine', base), 'mine', 'nothing remote: keep mine');
  assert.equal(sync.mergeText(base, 'line1\nMINE\n', 'line1\nTHEIRS\n'), null, 'same line: conflict');

  // The event handler: a guest's typing not yet in the room is merged and sent, not overwritten.
  let onEvent: (raw: string) => void = () => {};
  const sent: [string, string][] = [];
  const ctl = loadSource(ideUrl('store/ideCollabController.ts'), {
    '../../../lib/api/client': { api: {}, laneFetch: async () => ({ ok: false }) },
    '../../../lib/loadError': { loadErrorText: String },
    '../../../lib/toast.svelte': { toasts: { info: () => {} } },
    '../../../lib/toastError': { toastError: () => {} },
    './ideCollabNotices.svelte': { ideCollabNotices: {} },
    './ideCollabSync': sync,
    './ideTypes': { collabGuestPost: async () => ({}), ideBasename: (p: string) => p, streamIdeSse: async (_p: string, _h: unknown, _s: unknown, cb: (raw: string) => void) => { onEvent = cb; } },
  });
  const tab = { path: '/r/shared.txt', content: base, savedContent: base, dirty: false };
  const store = {
    tabs: [tab], collabPeerId: 'me', collabIsGuest: true, sharedBuffers: { '/r/shared.txt': { content: base, version: 3 } },
    getBuffer: () => `${base}guestguest`, clearLiveBuffer: () => {},
    sendCollabEdit: async (p: string, c: string) => { sent.push([p, c]); return true; },
  };
  ctl.attachCollabEventStream(store, 'room', 'tok', 'me', null);
  onEvent(JSON.stringify({ type: 'buffer_updated', path: '/r/shared.txt', content: `HOSTHOST${base}`, version: 4, peer_id: 'host' }));
  assert.equal(tab.content, `HOSTHOST${base}guestguest`);
  assert.deepEqual(sent.map((s) => s[1]), [`HOSTHOST${base}guestguest`], 'the merge goes back to the room');
  assert.equal(store.sharedBuffers['/r/shared.txt'].version, 4);
});

test('V-TC: "Delve is too old" becomes an Update Delve action', () => {
  const mod = loadSource(ideUrl('store/ideDebugAndEnvController.ts'), {
    '../../../lib/api/client': { api: {}, ApiError: class extends Error {} },
    '../../../lib/idePreferences.svelte': { idePreferences: { current: {} } },
    '../../../lib/stores/auth.svelte': { auth: {} },
    '../../../lib/toast.svelte': { toasts: {} },
    '../../../lib/toastError': { toastError() {} },
    '../../../lib/confirm.svelte': { confirmer: {} },
    '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: {} },
    '../../../lib/storage': { lsGet: () => null, lsSet() {} },
    '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({}) },
    '../../../lib/components/cm-ide-drafts': {},
    svelte: { tick: async () => {} },
  }, {}, { relative: true });
  const n = mod.toolOutdatedNotice(new Error('invalid: tool_outdated:go:dap — Delve is too old for Go 1.25. Update Delve.'));
  assert.equal(n.action, 'Update Delve');
  assert.equal(n.body, 'Delve is too old for Go 1.25. Update Delve.');
  assert.equal(mod.toolOutdatedNotice(new Error('exec: dlv not found')), null);
});
