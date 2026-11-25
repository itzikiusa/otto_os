// Restoring a project's saved tab strip (owner report 0.5.1: "a lot of files open … the
// loading just seems slow"). The strip shows at once, the active tab is read first on the
// interactive lane, hidden tabs are read afterwards on the background lane, and nothing a
// person typed is lost meanwhile. Production sources run in a vm; each test fails on the
// sequential restore it replaces.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { deferred, loadSource } from './sourceHarness.ts';

type Draft = { savedContent: string; draftContent: string };

function modules(api: Record<string, unknown>, drafts: Record<string, Draft> = {}, maxOpenTabs = 20) {
  const draftCalls: string[] = [];
  const prefs = { autoSave: false, localHistoryEnabled: false, localHistoryRetentionDays: 7, maxOpenTabs };
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
  const projects = loadSource(new URL('../src/modules/ide/shell/ideProjectsManager.ts', import.meta.url), imports, g, { relative: true }, cache);
  return { buf, helpers, projects, draftCalls, prefs };
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
const strip = (paths: string[], o: { active?: string | null; split?: string | null; pinned?: string[]; closedAll?: boolean } = {}) => ({
  target: { root: '/repo' }, tabs: paths.map((path) => ({ path, pinned: o.pinned?.includes(path) ?? false })),
  activePath: o.active ?? null, splitPath: o.split ?? null, hasClosedAllTabs: o.closedAll ?? false,
});
const pathOf = (url: string) => new URLSearchParams(url.slice(url.indexOf('?') + 1)).get('path');

test('restore shows the whole strip at once and reads the active tab first, alone, on the interactive lane', async () => {
  const interactive: string[] = [];
  const background: string[] = [];
  const active = deferred<ReturnType<typeof fileResp>>();
  const { buf, helpers } = modules({
    get: (url: string) => { interactive.push(pathOf(url)!); return pathOf(url) === '/repo/c.ts' ? active.promise : Promise.resolve(fileResp('?')); },
    bg: { get: async (url: string) => { background.push(pathOf(url)!); return fileResp(pathOf(url)!); } },
  });
  buf.saveWorkspaceOpenTabs(strip(['/repo/a.ts', '/repo/b.ts', '/repo/c.ts'], { active: '/repo/c.ts' }));
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

test('the saved strip keeps its pinned tabs and its split (one copy per project root)', async () => {
  const { buf, helpers } = modules({ get: async () => fileResp('x'), bg: { get: async () => fileResp('x') } });
  buf.saveWorkspaceOpenTabs(strip(['/repo/a.ts', '/repo/b.ts', '/repo/c.ts'], { active: '/repo/c.ts', split: '/repo/a.ts', pinned: ['/repo/b.ts'] }));
  const { h } = storeHost(buf);
  await helpers.restoreWorkspaceTabsOrInitial(h);
  assert.deepEqual([...h.tabs.map((t) => t.path)], ['/repo/a.ts', '/repo/b.ts', '/repo/c.ts']);
  assert.deepEqual([...h.tabs.filter((t) => t.pinned).map((t) => t.path)], ['/repo/b.ts']);
  assert.equal(h.activePath, '/repo/c.ts');
  assert.equal(h.splitPath, '/repo/a.ts');
});

test('a restored tab read later still gets the unsaved draft a reload left behind', async () => {
  const { buf, helpers } = modules(
    { get: async () => fileResp('disk'), bg: { get: async () => fileResp('disk') } },
    { '/repo/a.ts': { savedContent: 'disk', draftContent: 'disk + my edit' } },
  );
  buf.saveWorkspaceOpenTabs(strip(['/repo/a.ts', '/repo/b.ts'], { active: '/repo/b.ts' }));
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

const unread = (path: string) => ({ path, name: path.split('/').pop(), content: '', savedContent: '', loading: true, deferred: true, error: null, readOnly: false, truncated: false });

test('flushing a tab not read yet leaves its saved draft alone (close, rename, Replace in Path all flush first)', () => {
  const { buf, draftCalls } = modules({}, { '/repo/a.ts': { savedContent: 'disk', draftContent: 'mine' } });
  const { h, perf } = storeHost(buf);
  h.tabs = [unread('/repo/a.ts')];
  perf.flush(h, '/repo/a.ts');
  assert.deepEqual([...draftCalls], [], 'the empty placeholder must not be written over (= remove) the draft');
});

test('a tab not read yet that has a saved draft counts as unsaved, so close / Close Others / switch ask first', () => {
  const { buf } = modules({}, { '/repo/a.ts': { savedContent: 'disk', draftContent: 'mine' } });
  const { perf } = storeHost(buf);
  assert.equal(perf.isTabDirty(unread('/repo/a.ts')), true);
  assert.equal(perf.isTabDirty(unread('/repo/b.ts')), false, 'no draft: nothing to lose');
});

test('restoring more saved tabs than the tab cap keeps the active tab and opens nothing twice', async () => {
  const paths = Array.from({ length: 17 }, (_, i) => `/repo/f${i}.ts`);
  const { buf, helpers, prefs } = modules({ get: async () => fileResp('x'), bg: { get: async () => fileResp('x') } }, {}, 20);
  buf.saveWorkspaceOpenTabs(strip(paths, { active: paths[0] }));
  prefs.maxOpenTabs = 15; // lowered since the strip was saved
  const { h } = storeHost(buf);
  await helpers.restoreWorkspaceTabsOrInitial(h);
  assert.equal(h.activePath, paths[0]);
  assert.deepEqual([...h.tabs.map((x) => x.path)], paths, 'no tab (least of all the active one) evicted or reordered by the restore itself');
});

test('Save on a tab not read yet that has a draft reads it first, then writes the draft', async () => {
  const puts: { path: string; content: string }[] = [];
  const { buf } = modules(
    { get: async () => fileResp('disk'), put: async (_u: string, body: { path: string; content: string }) => { puts.push(body); return { sha256: 'sha-new' }; } },
    { '/repo/a.ts': { savedContent: 'disk', draftContent: 'mine' } },
  );
  const { h, perf } = storeHost(buf);
  (h as Record<string, unknown>).saveFile = (p?: string, f?: boolean, o?: { silent?: boolean }) => buf.performSaveFile(h, perf, p, f, o);
  (h as Record<string, unknown>).updateBuffer = () => {};
  await h.openFile('/repo/a.ts', { preview: false, background: true, defer: true });
  assert.equal(h.tabs[0].dirty, true, 'the strip shows the unsaved dot before the read');
  assert.equal(await buf.performSaveFile(h, perf, '/repo/a.ts', false, { silent: true }), true);
  assert.deepEqual(puts.map((p) => p.content), ['mine']);
  assert.equal(h.tabs[0].dirty, false);
});

test('Save on a tab whose background read is in flight waits for the read, then writes the draft', async () => {
  const puts: string[] = [];
  const hidden = deferred<ReturnType<typeof fileResp>>();
  const { buf, helpers } = modules(
    { get: async () => fileResp('A'), bg: { get: () => hidden.promise }, put: async (_u: string, body: { content: string }) => { puts.push(body.content); return { sha256: 's' }; } },
    { '/repo/b.ts': { savedContent: 'disk', draftContent: 'mine' } },
  );
  buf.saveWorkspaceOpenTabs(strip(['/repo/a.ts', '/repo/b.ts'], { active: '/repo/a.ts' }));
  const { h, perf } = storeHost(buf);
  (h as Record<string, unknown>).saveFile = (p?: string, f?: boolean, o?: { silent?: boolean }) => buf.performSaveFile(h, perf, p, f, o);
  (h as Record<string, unknown>).updateBuffer = () => {};
  await helpers.restoreWorkspaceTabsOrInitial(h);
  const b = h.tabs.find((x) => x.path === '/repo/b.ts')!;
  assert.equal(b.deferred, false, 'the prefetch has started reading it');
  assert.equal(b.loading, true);
  const saved = buf.performSaveFile(h, perf, '/repo/b.ts', false, { silent: true });
  hidden.resolve(fileResp('disk'));
  assert.equal(await saved, true);
  assert.deepEqual(puts, ['mine']);
});

// ── One rule for writing the saved strip (review T-001 / T-002 / F-002) ─────────────────────


test('a target reset or page leave before the restore finished does not empty the saved strip; closing every tab does', () => {
  const { buf } = modules({});
  buf.saveWorkspaceOpenTabs(strip(['/repo/a.ts', '/repo/b.ts'], { active: '/repo/b.ts' }));
  const { h, perf } = storeHost(buf); // tabs [] and not closed by the person: the restore has not run yet
  perf.flushAndSnapshotAll(h); // pagehide / leaving the IDE
  perf.resetAll(h, { discardDirty: true }); // a switch to another project
  assert.deepEqual([...(buf.loadWorkspaceOpenTabs('/repo')?.paths ?? [])], ['/repo/a.ts', '/repo/b.ts']);
  h.hasClosedAllTabs = true;
  perf.flushAndSnapshotAll(h);
  assert.equal(buf.loadWorkspaceOpenTabs('/repo'), null, 'the person closed every tab: the empty strip is saved');
});

test('nothing writes the saved strip while a restore is reading it back', () => {
  const { buf } = modules({});
  buf.saveWorkspaceOpenTabs(strip(['/repo/a.ts', '/repo/b.ts', '/repo/c.ts']));
  buf.ideTabRestore.active = true;
  try { buf.saveWorkspaceOpenTabs(strip(['/repo/a.ts'])); } finally { buf.ideTabRestore.active = false; }
  assert.deepEqual([...(buf.loadWorkspaceOpenTabs('/repo')?.paths ?? [])], ['/repo/a.ts', '/repo/b.ts', '/repo/c.ts']);
});

test('past the tab cap the saved strip still keeps the pinned, active and split tabs', () => {
  const { buf } = modules({}, {}, 15);
  const paths = Array.from({ length: 18 }, (_, i) => `/repo/f${i}.ts`);
  buf.saveWorkspaceOpenTabs(strip(paths, { active: paths[17], split: paths[16], pinned: [paths[15]] }));
  const saved = buf.loadWorkspaceOpenTabs('/repo')!;
  assert.deepEqual([...saved.paths], paths, 'the first 15 and the three that must come back');
  assert.deepEqual([...saved.pinned], [paths[15]]);
});

// ── activateNamedProject: one at a time, only the newest counts (review T-003) ────────────────

function projectHost() {
  const calls: string[] = [];
  const confirms: Array<ReturnType<typeof deferred<boolean>>> = [];
  const h = {
    namedProjects: [{ id: 'A', name: 'A', primary_root: '/a', folders: ['/a'] }, { id: 'B', name: 'B', primary_root: '/b', folders: ['/b'] }],
    activeProjectId: null as string | null, target: { kind: 'local', root: '/x' }, additionalRoots: [] as string[], failOn: '',
    /** Each prompt waits until the test answers it (`confirms[i].resolve(true)`). */
    confirmDiscardUnsaved: () => { const d = deferred<boolean>(); confirms.push(d); return d.promise; },
    setLocalTarget: async (root: string, _ws: unknown, o: { projectId?: string }) => {
      if (root === h.failOn) throw new Error('boom');
      calls.push(root); h.target = { kind: 'local', root }; h.activeProjectId = o.projectId ?? null; return true;
    },
  };
  return { h, calls, confirms };
}
const flush = () => new Promise((r) => setTimeout(r, 0));

test('activating the same project again while it loads joins the first activation', async () => {
  const { projects } = modules({});
  const { h, calls, confirms } = projectHost();
  const runs = [projects.activateNamedProject(h, 'A'), projects.activateNamedProject(h, 'A'), projects.activateNamedProject(h, 'A')];
  await flush();
  assert.equal(confirms.length, 1);
  confirms[0].resolve(true);
  await Promise.all(runs);
  assert.deepEqual(calls, ['/a']);
});

test('A then B while A waits: A stops before it switches, and the IDE ends on B', async () => {
  const { projects } = modules({});
  const { h, calls, confirms } = projectHost();
  const a = projects.activateNamedProject(h, 'A');
  await flush();
  const b = projects.activateNamedProject(h, 'B');
  confirms[0].resolve(true); // A's prompt answered after B was picked
  await a; await flush();
  confirms[1]?.resolve(true);
  await b;
  assert.deepEqual(calls, ['/b']);
  assert.equal(h.activeProjectId, 'B');
});

test('A then B then A while the first A waits: A is switched to once, B never', async () => {
  const { projects } = modules({});
  const { h, calls, confirms } = projectHost();
  const runs = [projects.activateNamedProject(h, 'A')];
  await flush();
  runs.push(projects.activateNamedProject(h, 'B'), projects.activateNamedProject(h, 'A'));
  confirms[0].resolve(true);
  await flush(); await flush();
  for (const c of confirms.slice(1)) c.resolve(true);
  await Promise.all(runs);
  assert.deepEqual(calls, ['/a']);
  assert.equal(h.activeProjectId, 'A');
});

test('a failed activation rejects its caller and does not block the next one', async () => {
  const { projects } = modules({});
  const { h, calls, confirms } = projectHost();
  h.failOn = '/a';
  const a = projects.activateNamedProject(h, 'A');
  await flush(); confirms[0].resolve(true);
  await assert.rejects(a, /boom/);
  const b = projects.activateNamedProject(h, 'B');
  await flush(); confirms[1].resolve(true);
  await b;
  assert.deepEqual(calls, ['/b']);
});
