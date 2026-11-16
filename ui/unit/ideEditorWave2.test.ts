// IDE editor bug hunt, wave 2: F6 Move to a relative path (E1), a tree refresh rereads the
// disk (E2), ⌘⌥L always answers (E3), truncated/binary tabs say so (E4), and tab/shortcut
// fixes (E6): ⌘F12 is File Structure, ⌘⇧T after a batch close, ⌘W on a pinned tab, Go to Line.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { loadSource } from './sourceHarness.ts';

function ideModules(api: Record<string, unknown>) {
  const notes: string[] = [];
  const toasts = { success: (t: string) => notes.push(`ok:${t}`), error: (t: string) => notes.push(`err:${t}`), info: (t: string) => notes.push(`info:${t}`) };
  const client = { api, ApiError: class extends Error { status = 0; }, getToken: () => '', laneFetch: () => { throw new Error('unused'); } };
  const imports = {
    svelte: { tick: async () => {} },
    '../../../lib/api/client': client,
    '../../../lib/components/cm-ide-drafts': { readIdeUndoDraft: () => null, removeIdeUndoDraft: () => {}, writeIdeUndoDraftImmediate: () => {} },
    '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({ localHistoryEnabled: false, localHistoryRetentionDays: 7 }) },
    '../../../lib/confirm.svelte': { confirmer: {} },
    '../../../lib/toast.svelte': { toasts },
    '../../../lib/toastError': { toastError: (t: string) => notes.push(`err:${t}`) },
    '../../../lib/ideProjectsModel': { loadNamedIdeProjectsLocal: () => [], saveNamedIdeProjectsLocal: () => {}, fetchNamedIdeProjectsFromServer: async () => [], parseNamedIdeProject: () => null },
    '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: {} },
    '../../../lib/storage': { lsGet: () => null, lsSet: () => {}, lsRemove: () => {} },
    '../../../lib/components/cm-ide-lsp': { disposePooledLspClients: async () => {} },
  };
  const opts = { relative: true };
  const cache = new Map();
  const load = (rel: string) => loadSource(new URL(`../src/modules/ide/${rel}`, import.meta.url), imports, {}, opts, cache);
  return { load, notes };
}

const target = { kind: 'local', workspace_id: null, root: '/repo', connection_id: null, connection_name: null };

test('E1: F6 Move to a project-relative path renames to the absolute path and retargets the tab', async () => {
  const posted: { path: string; dest_path?: string }[] = [];
  const { load } = ideModules({ post: async (_u: string, body: { path: string; dest_path?: string }) => { posted.push(body); return { ok: true, path: body.dest_path }; } });
  const projects = load('shell/ideProjectsManager.ts');
  let active: string | null = '/repo/a.txt';
  const host = {
    target, targetPayload: { ...target, project_folders: ['/repo'] }, projectRoots: [], expandedDirs: {},
    tabs: [{ path: '/repo/a.txt', name: 'a.txt' }], activePath: active, splitPath: null,
    loadTree: async () => {}, openFile: async () => {}, closeSplit: () => {},
    resolvePath: (p: string) => (p.startsWith('/') ? p : `/repo/${p}`),
  };
  const ok = await projects.performMutateFs(host, () => '', () => {}, (p: string | null) => { active = p; }, 'rename', '/repo/a.txt', 'src/x.txt');
  assert.equal(ok, true);
  assert.equal(posted[0].dest_path, '/repo/src/x.txt', 'the daemon gets the absolute destination');
  assert.equal(host.tabs[0].path, '/repo/src/x.txt', 'the tab keys on the absolute path, so ⌘S saves it');
  assert.equal(active, '/repo/src/x.txt');
});

test('E2: a tree refresh refetches expanded folders and forgets collapsed ones; every expand reads the disk', async () => {
  const gets: string[] = [];
  const listings: Record<string, { name: string; path: string; kind: string }[]> = {
    '': [{ name: 'src', path: '/repo/src', kind: 'dir' }, { name: 'lib', path: '/repo/lib', kind: 'dir' }],
    '/repo/src': [{ name: 'a.ts', path: '/repo/src/a.ts', kind: 'file' }, { name: 'new.ts', path: '/repo/src/new.ts', kind: 'file' }],
  };
  const { load } = ideModules({ get: async (url: string) => {
    const path = new URLSearchParams(url.split('?')[1]).get('path') ?? '';
    gets.push(path);
    return { root: '/repo', path, entries: listings[path] ?? [] };
  } });
  const slices = load('store/ideStateSlices.svelte.ts');
  const host = {
    target, collabIsGuest: false, additionalRoots: [], projectRoots: [], showHidden: false,
    treeLoading: false, treeError: null, treeRootEntries: [], runConfigs: [{ id: 'x' }], selectedConfigId: 'x',
    expandedDirs: { '/repo': true, '/repo/src': true, '/repo/lib': false } as Record<string, boolean>,
    childrenByDir: {
      '/repo': [], '/repo/src': [{ name: 'a.ts', path: '/repo/src/a.ts', kind: 'file' }], '/repo/lib': [{ name: 'gone.ts', path: '/repo/lib/gone.ts', kind: 'file' }],
    } as Record<string, unknown[]>,
    targetQuery: (extra: Record<string, unknown>) => new URLSearchParams({ root: '/repo', path: String(extra.path ?? '') }).toString(),
    syncWorkspacePort: () => {},
  };
  await slices.fetchIdeWorkspaceTree(host, undefined, () => {}, { deferTabRestore: true });
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(gets, ['', '/repo/src'], 'the expanded folder is reread with the root');
  assert.equal(host.childrenByDir['/repo/src'].length, 2, 'Refresh shows the file created outside Otto');
  assert.equal(host.childrenByDir['/repo/lib'], undefined, 'a collapsed folder drops its stale listing');
});
