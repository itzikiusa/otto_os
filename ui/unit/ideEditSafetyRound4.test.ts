// The IDE must never lose or corrupt a person's edits (round 4 review G1–G12) and its
// desktop shortcuts must not fire from text fields (G6/G7). Production sources run in a
// vm with deterministic transports; each test fails on the pre-fix code.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createHash } from 'node:crypto';
import { deferred, loadSource } from './sourceHarness.ts';

class ApiError extends Error {
  status: number;
  constructor(status: number) { super(`HTTP ${status}`); this.status = status; }
}

const sha = (s: string): string => createHash('sha256').update(s, 'utf8').digest('hex');

function bufModules(api: Record<string, unknown>, opts: { autoSave?: boolean } = {}) {
  const draftWrites: string[] = [];
  const draftRemovals: string[] = [];
  const toastLog: string[] = [];
  const asked: string[] = [];
  const prefs = { autoSave: opts.autoSave ?? false, localHistoryEnabled: false, localHistoryRetentionDays: 7, maxOpenTabs: 20 };
  const imports = {
    svelte: { tick: async () => {} },
    '../../../lib/api/client': { api, ApiError, getToken: () => '', laneFetch: () => { throw new Error('unused'); } },
    '../../../lib/components/cm-ide-drafts': {
      readIdeUndoDraft: () => null,
      removeIdeUndoDraft: (p: string) => { draftRemovals.push(p); },
      writeIdeUndoDraftImmediate: (p: string, saved: string, draft: string) => { if (saved !== draft) draftWrites.push(p); },
    },
    '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => prefs },
    '../../../lib/confirm.svelte': { confirmer: { ask: async (m: string) => { asked.push(m); return true; }, choose: async () => ({ value: null }) } },
    '../../../lib/toast.svelte': { toasts: { success: (t: string) => toastLog.push(`ok:${t}`), error: (t: string) => toastLog.push(`err:${t}`), info: (t: string) => toastLog.push(`info:${t}`) } },
    '../../../lib/toastError': { toastError: (t: string) => toastLog.push(`err:${t}`) },
    '../../../lib/ideProjectsModel': { loadNamedIdeProjectsLocal: () => [], saveNamedIdeProjectsLocal: () => {}, fetchNamedIdeProjectsFromServer: async () => [], parseNamedIdeProject: () => null },
    '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: { rootsForLocal: () => [] } },
    '../../../lib/storage': { lsGet: () => null, lsSet: () => {}, lsRemove: () => {} },
    '../../../lib/components/cm-ide-lsp': { disposePooledLspClients: async () => {} },
  };
  const cache = new Map();
  const g = { TextEncoder };
  const buf = loadSource(new URL('../src/modules/ide/store/ideBufferPerf.ts', import.meta.url), imports, g, { relative: true }, cache);
  const helpers = loadSource(new URL('../src/modules/ide/shell/ideProjectHelpers.ts', import.meta.url), imports, g, { relative: true }, cache);
  const lsp = loadSource(new URL('../src/modules/ide/shell/ideLspAndSearchHelpers.ts', import.meta.url), imports, g, { relative: true }, cache);
  return { buf, helpers, lsp, draftWrites, draftRemovals, toastLog, asked };
}

function tab(path: string, content: string, savedContent = content, extra: Record<string, unknown> = {}) {
  return {
    path, name: path.split('/').pop(), content, savedContent, dirty: content !== savedContent, eol: '\n',
    sha256: 'sha-0', readOnly: false, truncated: false, loading: false, error: null, ...extra,
  };
}

function host(tabs: ReturnType<typeof tab>[]) {
  const target = { kind: 'local', workspace_id: null, root: '/repo', connection_id: null, connection_name: null };
  return {
    target, targetPayload: { ...target, project_folders: ['/repo'] }, tabs, activePath: tabs[0]?.path ?? null, splitPath: null,
    canEditCollab: true, collabIsGuest: false, resolvePath: (p: string) => p,
    targetQuery: (extra: Record<string, string> = {}) => new URLSearchParams({ root: '/repo', ...extra }).toString(),
    gutterByPath: {}, symbolsByPath: {}, problems: [], symbolsLoading: false,
    loadGutter: async () => {}, loadSymbols: async () => {},
  };
}

// ── G1 ──────────────────────────────────────────────────────────────────────

test('G1: ⌘S on a tab that is still loading writes nothing', async () => {
  const puts: unknown[] = [];
  const { buf } = bufModules({ put: async (_p: string, body: unknown) => { puts.push(body); return { sha256: 'x' }; } });
  const perf = new buf.IdeBufferPerfController();
  const h = host([tab('/repo/a.ts', '', '', { loading: true, sha256: '' })]);
  assert.equal(await buf.performSaveFile(h, perf, '/repo/a.ts'), false);
  assert.deepEqual(puts, [], 'no PUT: the placeholder is not the file');
});

test('G1: ⌘S on a tab whose read failed writes nothing', async () => {
  const puts: unknown[] = [];
  const { buf } = bufModules({ put: async (_p: string, body: unknown) => { puts.push(body); return { sha256: 'x' }; } });
  const perf = new buf.IdeBufferPerfController();
  const h = host([tab('/repo/a.ts', '', '', { error: 'HTTP 502', sha256: '' })]);
  assert.equal(await buf.performSaveFile(h, perf), false);
  assert.deepEqual(puts, []);
});

test('G1: a loaded tab whose sha was cleared still sends a precondition (hash of the saved text, in its line endings)', async () => {
  const sent: (string | undefined)[] = [];
  const { buf } = bufModules({ put: async (_p: string, body: { expected_sha256?: string }) => { sent.push(body.expected_sha256); return { sha256: 'new' }; } });
  const perf = new buf.IdeBufferPerfController();
  const h = host([tab('/repo/a.ts', 'a\nB\n', 'a\nb\n', { sha256: undefined, eol: '\r\n' })]);
  assert.equal(await buf.performSaveFile(h, perf, '/repo/a.ts'), true);
  assert.deepEqual(sent, [sha('a\r\nb\r\n')]);
});

// ── G4 ──────────────────────────────────────────────────────────────────────

test('G4: a Replace/Rename reload keeps a tab that still holds unsaved edits and reports it', async () => {
  const gets: string[] = [];
  const { buf } = bufModules({ get: async (url: string) => { gets.push(url); return { content: 'disk', sha256: 'sha-d', read_only: false, truncated: false }; } });
  const perf = new buf.IdeBufferPerfController();
  const h = host([tab('/repo/dirty.ts', 'mine', 'old'), tab('/repo/clean.ts', 'old', 'old')]);
  const kept = await buf.reloadTabsFromDisk(h, perf, new Set(['/repo/dirty.ts', '/repo/clean.ts']));
  assert.deepEqual([...kept], ['/repo/dirty.ts']);
  assert.equal(h.tabs[0].content, 'mine', 'unsaved edits survive');
  assert.equal(h.tabs[0].dirty, true, 'still dirty, not marked clean');
  assert.equal(h.tabs[0].sha256, 'sha-0', 'old sha kept: the next save asks about the disk change');
  assert.equal(h.tabs[1].content, 'disk');
  assert.equal(gets.length, 1);
});

test('G3/G4: keystrokes typed while the reload read is in flight win over the disk copy', async () => {
  const read = deferred<{ content: string; sha256: string; read_only: boolean; truncated: boolean }>();
  const { buf } = bufModules({ get: () => read.promise });
  const perf = new buf.IdeBufferPerfController();
  const h = host([tab('/repo/a.ts', 'v1', 'v1')]);
  const done = buf.reloadTabsFromDisk(h, perf, new Set(['/repo/a.ts']));
  await new Promise((r) => setTimeout(r, 0));
  perf.recordEdit(h, '/repo/a.ts', 'v1 typed');
  read.resolve({ content: 'disk', sha256: 'sha-d', read_only: false, truncated: false });
  assert.deepEqual([...(await done)], ['/repo/a.ts']);
  assert.equal(perf.getBuffer(h, '/repo/a.ts'), 'v1 typed');
  assert.equal(perf.isTabDirty(h.tabs[0]), true);
});

test('G4: Rename stops when a touched tab could not be saved', async () => {
  const posts: unknown[] = [];
  const { buf, lsp } = bufModules({ post: async (_u: string, b: unknown) => { posts.push(b); return { applied: true, edits: [] }; }, put: async () => { throw new ApiError(409); } });
  const perf = new buf.IdeBufferPerfController();
  const h = {
    ...host([tab('/repo/a.ts', 'mine', 'old')]),
    renameState: { open: true, path: '/repo/b.ts', line: 1, column: 1, symbol: 'x', newName: 'y', edits: [{ path: '/repo/a.ts' }], loading: false, error: null },
    saveFile: (p: string, f?: boolean, o?: { silent?: boolean }) => buf.performSaveFile(h, perf, p, f, o),
  };
  h.tabs.push(tab('/repo/b.ts', 'b', 'b'));
  assert.equal(await lsp.runApplyRename(h, perf), false);
  assert.deepEqual(posts, [], 'nothing sent to the daemon');
  assert.match(String(h.renameState.error), /“a\.ts” has unsaved changes/);
  assert.equal(h.tabs[0].content, 'mine');
});

test('G4: Replace in Path skips a file whose tab could not be saved, after asking', async () => {
  const bodies: { paths?: string[] }[] = [];
  const { buf, lsp, asked } = bufModules({
    long: { post: async (_u: string, b: { paths?: string[] }) => { bodies.push(b); return { files_changed: 1, replacements: 1, previews: [{ path: '/repo/b.ts' }] }; } },
    put: async () => { throw new ApiError(409); },
    get: async () => ({ content: 'replaced', sha256: 'sha-r', read_only: false, truncated: false }),
  });
  const perf = new buf.IdeBufferPerfController();
  const h = {
    ...host([tab('/repo/a.ts', 'mine', 'old'), tab('/repo/b.ts', 'b', 'b')]),
    replacePreview: { files_changed: 2, replacements: 2, previews: [{ path: '/repo/a.ts' }, { path: '/repo/b.ts' }] },
    searchScopePath: null, replaceLoading: false,
    saveFile: (p: string, f?: boolean, o?: { silent?: boolean }) => buf.performSaveFile(h, perf, p, f, o),
  };
  await lsp.runReplaceInWorkspace(h, perf, 'old', 'new', false, {});
  assert.equal(asked.length, 1);
  assert.deepEqual([...(bodies[0].paths ?? [])], ['/repo/b.ts']);
  assert.equal(h.tabs[0].content, 'mine');
});

// ── G5 ──────────────────────────────────────────────────────────────────────

test('G5: "Discard & switch" does not write the discarded edits back as drafts', () => {
  const { buf, draftWrites, draftRemovals } = bufModules({});
  const perf = new buf.IdeBufferPerfController();
  const h = host([tab('/repo/a.ts', 'unsaved', 'saved')]);
  perf.recordEdit(h, '/repo/a.ts', 'unsaved more');
  perf.resetAll(h, { discardDirty: true });
  assert.deepEqual(draftWrites, [], 'no draft written for discarded text');
  assert.ok(draftRemovals.includes('/repo/a.ts'), 'the earlier draft is removed too');
});

test('G5: a non-interactive reset (no discard) still snapshots drafts', () => {
  const { buf, draftWrites } = bufModules({});
  const perf = new buf.IdeBufferPerfController();
  const h = host([tab('/repo/a.ts', 'unsaved', 'saved')]);
  perf.resetAll(h);
  assert.ok(draftWrites.includes('/repo/a.ts'));
});

// ── G8 ──────────────────────────────────────────────────────────────────────

test('G8: a slow search that answers after a newer one cannot replace its results', async () => {
  const slow = deferred<{ items: unknown[]; truncated: boolean }>();
  const signals: AbortSignal[] = [];
  let n = 0;
  const { lsp } = bufModules({ long: { post: (_u: string, _b: unknown, sig: AbortSignal) => {
    signals.push(sig);
    n += 1;
    return n === 1 ? slow.promise : Promise.resolve({ items: [{ path: 'foo_bar' }], truncated: false });
  } } });
  const h = { ...host([]), searchScopePath: null, searchResults: [] as unknown[], searchTruncated: false, searchLoading: false, searchError: null };
  const first = lsp.runSearchWorkspace(h, 'fo', 'text');
  await lsp.runSearchWorkspace(h, 'foo_bar', 'text');
  assert.equal(signals[0].aborted, true, 'the superseded request is cancelled');
  slow.resolve({ items: [{ path: 'fo' }], truncated: false });
  await first;
  assert.deepEqual(h.searchResults, [{ path: 'foo_bar' }]);
  assert.equal(h.searchLoading, false);
});

// ── G9 ──────────────────────────────────────────────────────────────────────

test('G9: restoring saved tabs stops once the project switched', async () => {
  const { buf, helpers } = bufModules({});
  buf.saveWorkspaceOpenTabs('/old', ['/old/a.ts', '/old/b.ts', '/old/c.ts'], null, null);
  const opened: string[] = [];
  const h = {
    target: { kind: 'local', root: '/old', connection_id: null }, collabIsGuest: false, tabs: [] as unknown[], hasClosedAllTabs: false, activePath: null,
    treeRootEntries: [], childrenByDir: {}, toggleDir: async () => {}, openInSplit: () => {},
    openFile: async (p: string) => { opened.push(p); h.target = { kind: 'local', root: '/new', connection_id: null }; },
  };
  await helpers.restoreWorkspaceTabsOrInitial(h);
  assert.deepEqual(opened, ['/old/a.ts'], 'the old project\'s other tabs are not opened into the new one');
});
