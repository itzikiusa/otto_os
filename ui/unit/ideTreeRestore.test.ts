// The project tree comes back as it was, fast: a root load reads the open folders in the same
// request and lands in one paint, live re-reads asked for together go out as one request, and a
// project switched back to paints the listings this page holds at its old scroll offset at once.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { loadSource } from './sourceHarness.ts';

function slicesWith(get: (url: string) => Promise<unknown>) {
  const imports = {
    svelte: { tick: async () => {} },
    '../../../lib/api/client': { api: { get }, ApiError: class extends Error { status = 0; }, getToken: () => '', laneFetch: () => { throw new Error('unused'); } },
    '../../../lib/components/cm-ide-drafts': { readIdeUndoDraft: () => null, removeIdeUndoDraft: () => {}, writeIdeUndoDraftImmediate: () => {} },
    '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({}) },
    '../../../lib/confirm.svelte': { confirmer: {} },
    '../../../lib/toast.svelte': { toasts: {} },
    '../../../lib/toastError': { toastError: () => {} },
    '../../../lib/ideProjectsModel': { loadNamedIdeProjectsLocal: () => [], saveNamedIdeProjectsLocal: () => {}, fetchNamedIdeProjectsFromServer: async () => [], parseNamedIdeProject: () => null },
    '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: {} },
    '../../../lib/storage': { lsGet: () => null, lsSet: () => {}, lsRemove: () => {} },
    '../../../lib/components/cm-ide-lsp': { disposePooledLspClients: async () => {} },
  };
  return loadSource(new URL('../src/modules/ide/store/ideStateSlices.svelte.ts', import.meta.url), imports, {}, { relative: true }, new Map());
}

const target = { kind: 'local', workspace_id: null, root: '/repo', connection_id: null, connection_name: null };
const file = (path: string) => ({ name: path.split('/').pop(), path, kind: 'file' });
const dir = (path: string) => ({ name: path.split('/').pop(), path, kind: 'dir' });
const listing: Record<string, unknown[]> = {
  '': [dir('/repo/src')],
  '/repo/src': [dir('/repo/src/deep'), file('/repo/src/a.ts')],
  '/repo/src/deep': [file('/repo/src/deep/b.ts')],
};

function treeHost(expanded: string[]) {
  let assigned = 0;
  let children: Record<string, unknown[]> = {};
  return {
    target, collabIsGuest: false, additionalRoots: [], projectRoots: [], showHidden: false,
    treeLoading: false, treeError: null, treeRootEntries: [] as unknown[], runConfigs: [{ id: 'x' }],
    expandedDirs: Object.fromEntries(['/repo', ...expanded].map((d) => [d, true])) as Record<string, boolean>,
    get childrenByDir() { return children; },
    set childrenByDir(v) { assigned += 1; children = v; },
    get assigned() { return assigned; },
    targetQuery: (extra: Record<string, unknown>) => new URLSearchParams({ root: '/repo', path: String(extra.path ?? '') }).toString(),
    syncWorkspacePort: () => {},
  };
}

function batchedGet(urls: string[]) {
  return async (url: string) => {
    urls.push(url);
    const q = new URLSearchParams(url.split('?')[1]);
    const paths = JSON.parse(q.get('paths') ?? '[]') as string[];
    return { root: '/repo', path: q.get('path'), entries: listing[q.get('path') ?? ''], listings: paths.map((p) => ({ path: p, entries: listing[p] })) };
  };
}

test('a root load reads the open folders in the same request and commits them in one paint', async () => {
  const urls: string[] = [];
  const slices = slicesWith(batchedGet(urls));
  const host = treeHost(['/repo/src', '/repo/src/deep']);
  await slices.fetchIdeWorkspaceTree(host, undefined, () => {}, { deferTabRestore: true });
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(urls.length, 1, 'one request, not one per folder level');
  assert.deepEqual(JSON.parse(new URLSearchParams(urls[0].split('?')[1]).get('paths')!).sort(), ['/repo/src', '/repo/src/deep']);
  assert.equal(host.assigned, 1, 'the whole tree lands at once');
  assert.equal(host.childrenByDir['/repo/src/deep'].length, 1);
  // A refresh that finds nothing new keeps every listing as it is: nothing repaints.
  const before = host.childrenByDir;
  await slices.fetchIdeWorkspaceTree(host, undefined, () => {}, { deferTabRestore: true });
  for (const d of Object.keys(before)) assert.equal(host.childrenByDir[d], before[d], d);
});

test('live re-reads asked for in the same tick go out as one request', async () => {
  const urls: string[] = [];
  const slices = slicesWith(batchedGet(urls));
  const host = treeHost(['/repo/src', '/repo/src/deep']);
  await Promise.all(['/repo', '/repo/src', '/repo/src/deep'].map((d) => slices.relistIdeTreeDir(host, d)));
  assert.equal(urls.length, 1);
  assert.equal(host.childrenByDir['/repo/src'].length, 2);
  assert.equal(host.childrenByDir['/repo/src/deep'].length, 1);
});

test('a project switched back to paints its held listings at the old scroll offset at once', () => {
  const m = new Map<string, string>();
  const storage = { lsGet: (k: string) => m.get(k) ?? null, lsSet: (k: string, v: string) => void m.set(k, v), lsRemove: (k: string) => void m.delete(k) };
  const memory = loadSource(new URL('../src/modules/ide/store/ideProjectMemory.ts', import.meta.url), { '../../../lib/storage': storage });
  const sw = loadSource(new URL('../src/modules/ide/store/ideProjectSwitch.ts', import.meta.url), {
    '../shell/ideProjectRecents.svelte': { ideProjectRecents: { record: () => {} }, sameRootKey: (p: string) => p },
    './ideProjectHealth.svelte': { ideProjectHealth: { clear: () => {} } },
    './ideWindowTitle.svelte': { ideWindowTitle: { project: '' } },
    './ideParkedRuns': { adoptParkedRun: async () => null, parkRun: () => null, resumeParkedRunWatch: () => {}, setParkedRunCurrentKey: () => {} },
    './ideProjectMemory': memory,
    './ideTypes': { ideBasename: (p: string) => p.split('/').pop(), ideTargetKey: (t: { kind: string; root: string }) => `${t.kind}::${t.root}` },
  });
  const host = {
    target: { kind: 'local', root: '/repo', connection_id: null }, projectRoots: [{ path: '/repo' }], envState: { activeRun: null, runsHistory: [] },
    expandedDirs: { '/repo': true, '/repo/src': true } as Record<string, boolean>,
    treeRootEntries: listing[''], childrenByDir: { '/repo': listing[''], '/repo/src': listing['/repo/src'] } as Record<string, unknown[]>,
    loadTree: async () => {},
  };
  sw.treeScroll.top = 640;
  sw.leaveIdeTarget(host);
  // Another project, then back (the store resets the tree in between).
  Object.assign(host, { target: { kind: 'local', root: '/other', connection_id: null }, projectRoots: [{ path: '/other' }], expandedDirs: {}, treeRootEntries: [], childrenByDir: {} });
  sw.enterIdeTarget(host);
  assert.equal(sw.treeScroll.pending, 0, 'a project never opened starts at the top');
  sw.treeScroll.top = 0;
  sw.leaveIdeTarget(host);
  Object.assign(host, { target: { kind: 'local', root: '/repo', connection_id: null }, projectRoots: [{ path: '/repo' }], expandedDirs: { '/repo': true }, treeRootEntries: [], childrenByDir: {} });
  sw.enterIdeTarget(host);
  assert.equal(host.treeRootEntries, listing['']);
  assert.equal(host.childrenByDir['/repo/src'], listing['/repo/src'], 'no request needed to paint it');
  assert.equal(host.expandedDirs['/repo/src'], true);
  assert.equal(sw.treeScroll.pending, 640, 'the tree scrolls back to where it was');
});
