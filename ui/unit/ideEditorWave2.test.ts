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

test('E3: ⌘⌥L says when nothing changed, and when the buffer moved during the request', async () => {
  let answer = { formatted: 'x', changed: false, formatter: 'builtin' };
  const { load, notes } = ideModules({ post: async () => answer });
  const lsp = load('shell/ideLspAndSearchHelpers.ts');
  const tab = { path: '/repo/a.ts', name: 'a.ts', language: 'typescript', readOnly: false, truncated: false };
  let buffer = 'x';
  const host = { activeTab: tab, tabs: [tab], targetPayload: {}, applyAgentEdit: async () => true };
  const perf = { flush: () => buffer, getBuffer: () => buffer };
  await lsp.runFormatActiveFile(host, perf);
  answer = { formatted: 'x', changed: false, formatter: 'prettier' };
  await lsp.runFormatActiveFile(host, perf);
  answer = { formatted: 'y', changed: true, formatter: 'prettier' };
  const moving = { flush: () => buffer, getBuffer: () => `${buffer}!` };
  await lsp.runFormatActiveFile(host, moving);
  assert.deepEqual(notes, [
    'info:No changes: no formatter is installed for this file type',
    'info:Already formatted (prettier)',
    'info:The file changed while formatting. Format it again',
  ]);
});

test('E4: an opened tab carries the read flags, so a truncated or binary file is labelled', async () => {
  const reads: Record<string, unknown> = {
    big: { content: 'head', sha256: 's', read_only: true, truncated: true, binary: false },
    blob: { content: '', sha256: 's', read_only: true, truncated: true, binary: true },
  };
  const { load } = ideModules({ get: async (url: string) => reads[/big/.test(url) ? 'big' : 'blob'] });
  const buf = load('store/ideBufferPerf.ts');
  const perf = new buf.IdeBufferPerfController();
  const host = {
    target, targetPayload: target, tabs: [] as { path: string; truncated: boolean; binary?: boolean }[], activePath: null, splitPath: null,
    sharedBuffers: {}, collabIsGuest: false, canEditCollab: true, recentPaths: [], projectRoots: [], cursorInfo: { line: 1, col: 1 }, splitDirection: null,
    resolvePath: (p: string) => p, targetQuery: () => 'root=/repo', jumpTo: () => {}, pushNavLocation: () => {},
    openInSplit: () => {}, loadGutter: async () => {}, loadSymbols: async () => {}, saveFile: async () => true, closeSplit: () => {},
  };
  await buf.performOpenFile(host, perf, false, '/repo/big.log', {});
  await buf.performOpenFile(host, perf, false, '/repo/blob.bin', {});
  const big = host.tabs.find((t) => t.path === '/repo/big.log');
  const blob = host.tabs.find((t) => t.path === '/repo/blob.bin');
  assert.deepEqual([big?.truncated, big?.binary], [true, false]);
  assert.deepEqual([blob?.truncated, blob?.binary], [true, true]);
});

function shell(over: Record<string, unknown> = {}) {
  const calls: string[] = [];
  let promptOpts: { validate?: (v: string) => string | null } = {};
  const ide = {
    activePath: '/repo/a.ts', activeTab: { path: '/repo/a.ts', pinned: false } as { path: string; pinned: boolean } | null,
    cursorInfo: { line: 1, col: 1 }, searchEverywhereOpen: false, activeDebug: null, goToLineOpen: false,
    jumpTo: () => {}, openSettings: () => {}, saveFile: async () => true,
    closeTab: async (p: string) => { calls.push(`close:${p}`); },
    openSearchEverywhere: (t: string) => { calls.push(`search:${t}`); },
    ...over,
  };
  const page = { tagName: 'DIV', isContentEditable: false, matches: () => false, closest: () => null, querySelector: () => null };
  const mod = loadSource(new URL('../src/modules/ide/shell/ideShellCommands.ts', import.meta.url), {
    '../../../lib/confirm.svelte': { confirmer: { promptText: async (_m: string, o: typeof promptOpts) => { promptOpts = o; return null; } } },
    '../../../lib/contextmenu.svelte': { ctxMenu: { showAt: () => {} } },
    '../../../lib/idePreferences.svelte': { readIdePreferences: () => ({}), writeIdePreferences: () => {} },
    '../ideStore.svelte': { ide, ideBasename: (p: string) => p },
    '../store/ideAgentController.svelte': { ideAgent: { markSelectionFromEditor: () => null, openInlineEdit: () => {} } },
  }, { Element: class {}, window: { location: { reload() {} }, dispatchEvent() {} }, CustomEvent: class {} });
  const key = (k: string, code: string, mods: { meta?: boolean; shift?: boolean } = {}) => ({
    target: page, key: k, code, ctrlKey: false, metaKey: Boolean(mods.meta), shiftKey: Boolean(mods.shift), altKey: false,
    defaultPrevented: false, preventDefault() { this.defaultPrevented = true; },
  });
  return { mod, calls, ide, key, prompt: () => promptOpts };
}

test('E6: ⌘F12 is left to the editor (File Structure); ⌘O still opens Symbols', () => {
  const { mod, calls, key } = shell();
  assert.equal(mod.handleIdeChord(key('F12', 'F12', { meta: true }), false, () => {}), false);
  assert.equal(mod.handleIdeChord(key('o', 'KeyO', { meta: true }), false, () => {}), true);
  assert.deepEqual(calls, ['search:symbols']);
});

test('E6: ⌘W on a pinned tab keeps it open (and still keeps the window)', () => {
  const pinned = shell({ activeTab: { path: '/repo/a.ts', pinned: true } });
  const w = pinned.key('w', 'KeyW', { meta: true });
  assert.equal(pinned.mod.handleIdeChord(w, false, () => {}), true);
  assert.equal(w.defaultPrevented, true);
  assert.deepEqual(pinned.calls, []);
  const plain = shell();
  plain.mod.handleIdeChord(plain.key('w', 'KeyW', { meta: true }), false, () => {});
  assert.deepEqual(plain.calls, ['close:/repo/a.ts']);
});

test('E6: Go to Line rejects anything but line or line:column', async () => {
  const { mod, prompt } = shell();
  await mod.promptIdeGoToLine();
  const validate = prompt().validate!;
  for (const ok of ['42', '42:8']) assert.equal(validate(ok), null, ok);
  for (const bad of ['abc', '12x', '4:', ':3', '1:2:3']) assert.ok(validate(bad), bad);
});

test('E6: ⌘⇧T reopens tabs closed by Close others / Close unmodified / Close all', async () => {
  const { load } = ideModules({});
  const slices = load('store/ideStateSlices.svelte.ts');
  const recorded: string[][] = [];
  const tabs = ['/repo/a.ts', '/repo/b.ts', '/repo/c.ts'].map((path) => ({ path, name: path, content: '', savedContent: '' }));
  const host = { target, tabs, activePath: '/repo/a.ts', splitPath: null, closeSplit: () => {}, hasClosedAllTabs: false, syncWorkspacePort: () => {} };
  const perf = { flushAll: () => {}, isTabDirty: () => false };
  slices.performCloseTabBatch(host, perf, (closing: string[]) => recorded.push(closing), '/repo/a.ts');
  assert.deepEqual(recorded, [['/repo/b.ts', '/repo/c.ts']], 'the batch hands its closed paths to the store');
  // The store's callback is forgetClosedTabs, which feeds closedTabPaths (ideStore.svelte.ts).
  const store = await import('node:fs').then((fs) => fs.readFileSync(new URL('../src/modules/ide/ideStore.svelte.ts', import.meta.url), 'utf8'));
  for (const call of ['closeOtherTabs', 'closeUnmodifiedTabs', 'closeAllTabs']) {
    const body = store.slice(store.indexOf(`${call}(`), store.indexOf('\n  }', store.indexOf(`${call}(`)) + 4);
    assert.match(body, /forgetClosedTabs/, `${call} records what it closed for ⌘⇧T`);
  }
});
