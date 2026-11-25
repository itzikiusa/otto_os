// Restoring a project's saved tab strip (owner report 0.5.1: "a lot of files open … the
// loading just seems slow"). The strip shows at once, the active tab is read first on the
// interactive lane, hidden tabs are read afterwards on the background lane, and nothing a
// person typed is lost meanwhile. Production sources run in a vm; each test fails on the
// sequential restore it replaces.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { deferred, loadSource } from './sourceHarness.ts';

type Draft = { savedContent: string; draftContent: string };

function modules(api: Record<string, unknown>, drafts: Record<string, Draft> = {}) {
  const draftCalls: string[] = [];
  const prefs = { autoSave: false, localHistoryEnabled: false, localHistoryRetentionDays: 7, maxOpenTabs: 20 };
  const imports = {
    svelte: { tick: async () => {} },
    '../../../lib/api/client': { api, ApiError: Error, getToken: () => '', laneFetch: () => { throw new Error('unused'); } },
    '../../../lib/components/cm-ide-drafts': {
      readIdeUndoDraft: (p: string) => drafts[p] ?? null,
      removeIdeUndoDraft: (p: string) => { draftCalls.push(`remove:${p}`); },
      writeIdeUndoDraftImmediate: (p: string) => { draftCalls.push(`write:${p}`); },
    },
    '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => prefs },
    '../../../lib/confirm.svelte': { confirmer: { ask: async () => true, choose: async () => ({ value: null }) } },
    '../../../lib/toast.svelte': { toasts: { success: () => {}, error: () => {}, info: () => {} } },
    '../../../lib/toastError': { toastError: () => {} },
    '../../../lib/ideProjectsModel': { loadNamedIdeProjectsLocal: () => [], saveNamedIdeProjectsLocal: () => {}, fetchNamedIdeProjectsFromServer: async () => [], parseNamedIdeProject: () => null },
    '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: { rootsForLocal: () => [] } },
    '../../../lib/storage': { lsGet: () => null, lsSet: () => {}, lsRemove: () => {} },
    '../../../lib/components/cm-ide-lsp': { disposePooledLspClients: async () => {} },
  };
  const cache = new Map();
  const g = { TextEncoder };
  const buf = loadSource(new URL('../src/modules/ide/store/ideBufferPerf.ts', import.meta.url), imports, g, { relative: true }, cache);
  const helpers = loadSource(new URL('../src/modules/ide/shell/ideProjectHelpers.ts', import.meta.url), imports, g, { relative: true }, cache);
  return { buf, helpers, draftCalls };
}

/** A store-shaped host whose openFile is the real performOpenFile. */
function storeHost(buf: ReturnType<typeof modules>['buf']) {
  const perf = new buf.IdeBufferPerfController();
  const target = { kind: 'local', workspace_id: null, root: '/repo', connection_id: null, connection_name: null };
  const h = {
    target, targetPayload: { ...target, project_folders: ['/repo'] }, projectRoots: [{ path: '/repo', name: 'repo', isPrimary: true }],
    tabs: [] as Array<Record<string, unknown> & { path: string }>, activePath: null as string | null, splitPath: null as string | null,
    splitDirection: null, sharedBuffers: {}, recentPaths: [] as string[], hasClosedAllTabs: false,
    canEditCollab: true, collabIsGuest: false, resolvePath: (p: string) => p,
    targetQuery: (extra: Record<string, string> = {}) => new URLSearchParams({ root: '/repo', ...extra }).toString(),
    gutterByPath: {}, symbolsByPath: {}, problems: [], symbolsLoading: false,
    cursorInfo: { line: 1, col: 1, cursors: 1, selections: [] },
    treeRootEntries: [], childrenByDir: {}, toggleDir: async () => {},
    loadGutter: async () => {}, loadSymbols: async () => {}, jumpTo: () => {}, pushNavLocation: () => {},
    openInSplit: (p: string) => { h.splitPath = p; },
    openFile: (p: string, o?: Record<string, unknown>) => buf.performOpenFile(h, perf, false, p, o ?? {}),
  };
  return { h, perf };
}

const fileResp = (content: string) => ({ content, sha256: `sha-${content}`, language: 'typescript' });
const pathOf = (url: string) => new URLSearchParams(url.slice(url.indexOf('?') + 1)).get('path');

test('restore shows the whole strip at once and reads the active tab first, alone, on the interactive lane', async () => {
  const interactive: string[] = [];
  const background: string[] = [];
  const active = deferred<ReturnType<typeof fileResp>>();
  const { buf, helpers } = modules({
    get: (url: string) => { interactive.push(pathOf(url)!); return pathOf(url) === '/repo/c.ts' ? active.promise : Promise.resolve(fileResp('?')); },
    bg: { get: async (url: string) => { background.push(pathOf(url)!); return fileResp(pathOf(url)!); } },
  });
  buf.saveWorkspaceOpenTabs('/repo', ['/repo/a.ts', '/repo/b.ts', '/repo/c.ts'], '/repo/c.ts', null);
  const { h } = storeHost(buf);
  const done = helpers.restoreWorkspaceTabsOrInitial(h);
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual([...h.tabs.map((t) => t.path)], ['/repo/a.ts', '/repo/b.ts', '/repo/c.ts'], 'every tab, in its saved order');
  assert.equal(h.activePath, '/repo/c.ts', 'the saved active tab, not each tab in turn');
  assert.deepEqual(interactive, ['/repo/c.ts'], 'only the active tab is read before it is on screen');
  assert.deepEqual(background, []);
  active.resolve(fileResp('C'));
  await done;
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(h.tabs[2].content, 'C');
  assert.deepEqual(background.sort(), ['/repo/a.ts', '/repo/b.ts'], 'hidden tabs follow on the background lane');
  assert.deepEqual(interactive, ['/repo/c.ts']);
  assert.ok(h.tabs.every((t) => !t.loading && !t.deferred));
  assert.equal(h.activePath, '/repo/c.ts');
});

test('a restored tab read later still gets the unsaved draft a reload left behind', async () => {
  const { buf, helpers } = modules(
    { get: async () => fileResp('disk'), bg: { get: async () => fileResp('disk') } },
    { '/repo/a.ts': { savedContent: 'disk', draftContent: 'disk + my edit' } },
  );
  buf.saveWorkspaceOpenTabs('/repo', ['/repo/a.ts', '/repo/b.ts'], '/repo/b.ts', null);
  const { h } = storeHost(buf);
  await helpers.restoreWorkspaceTabsOrInitial(h);
  await new Promise((r) => setTimeout(r, 0));
  const a = h.tabs.find((t) => t.path === '/repo/a.ts')!;
  assert.equal(a.content, 'disk + my edit');
  assert.equal(a.savedContent, 'disk');
  assert.equal(a.dirty, true);
});

test('leaving the page before a tab is read keeps its saved draft', () => {
  const { buf, draftCalls } = modules({});
  const { h, perf } = storeHost(buf);
  h.tabs = [
    { path: '/repo/a.ts', name: 'a.ts', content: '', savedContent: '', loading: true, deferred: true, error: null, readOnly: false, truncated: false },
    { path: '/repo/b.ts', name: 'b.ts', content: 'b2', savedContent: 'b', loading: false, error: null, readOnly: false, truncated: false },
  ];
  perf.flushAndSnapshotAll(h);
  assert.ok(!draftCalls.some((c) => c.endsWith('/repo/a.ts')), 'the placeholder must not overwrite or remove its draft');
  assert.ok(draftCalls.includes('write:/repo/b.ts'));
});
