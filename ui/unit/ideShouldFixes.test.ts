// Regression tests for Wave 2 IDE SHOULD fixes (S-38):
// - Breadcrumb ancestor path resolution
// - DB Watch stacked phone visibility gate & loadErrorText error handling
// - Build Tools recursive module/goal/plugin/dependency filter
// - Workspace search cancel + latestOnly() guards on runFindReferences & runPreviewRename
// - Safe-delete non-identifier filename guard (no stale ide.usages leak)
// - Per-path savesInFlight serialization for format + autosave
// - Split-pane separator keyboard resize with RTL arrow inversion & clamping

import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  moduleMatchesBuildFilter,
  moduleSelfMatchesBuildFilter,
} from '../src/modules/ide/buildtools/ideBuildToolFilter.ts';
import { isDbWatchPanelVisible } from '../src/modules/ide/dbwatch/ideDbWatchModel.ts';
import { collapseIdeCrumbs, ideCrumbAbsPath } from '../src/modules/ide/editor/ideCrumbs.ts';
import {
  computeSplitRatioFromKey,
  extractSafeDeleteSymbol,
  formatSafeDeleteMessage,
} from '../src/modules/ide/editor/ideEditorHelpers.ts';
import type { IdeBuildToolModule } from '../src/modules/ide/store/ideTypes.ts';
import { deferred, loadSource } from './sourceHarness.ts';

class ApiError extends Error {
  status: number;
  constructor(status: number, message = `HTTP ${status}`) {
    super(message);
    this.status = status;
  }
}

// ---------------------------------------------------------------------------
// 1. Breadcrumb ancestor folder path resolution
// ---------------------------------------------------------------------------

test('S-38 #1: folder breadcrumb resolves ancestor directory path while file crumb resolves file path', () => {
  const absPath = '/repo/services/payments/internal/handler.go';
  const segments = ['services', 'payments', 'internal', 'handler.go'];
  const crumbs = collapseIdeCrumbs(segments);
  const segCrumbs = crumbs.filter((c) => c.kind === 'seg');

  const resolved = segCrumbs.map((c) => ({
    label: c.label,
    path: c.last ? absPath : ideCrumbAbsPath(absPath, segments.length, c.index),
  }));

  assert.deepEqual(resolved, [
    { label: 'services', path: '/repo/services' },
    { label: 'payments', path: '/repo/services/payments' },
    { label: 'internal', path: '/repo/services/payments/internal' },
    { label: 'handler.go', path: '/repo/services/payments/internal/handler.go' },
  ]);
});

// ---------------------------------------------------------------------------
// 2. DB Watch phone stacked visibility & loadErrorText error messages
// ---------------------------------------------------------------------------

test('S-38 #3: isDbWatchPanelVisible enables live polling in phone stacked Tools pane even when bottomDockOpen is false', () => {
  assert.equal(
    isDbWatchPanelVisible({ stack: true, stackPane: 'tools', bottomDockOpen: false, activeBottomTab: 'dbwatch' }),
    true,
    'stacked phone layout on Tools pane + dbwatch tab must poll',
  );
  assert.equal(
    isDbWatchPanelVisible({ stack: true, stackPane: 'editor', bottomDockOpen: true, activeBottomTab: 'dbwatch' }),
    false,
    'stacked phone layout on Editor pane must not poll even if bottomDockOpen is true',
  );
  assert.equal(
    isDbWatchPanelVisible({ stack: true, stackPane: 'tools', bottomDockOpen: false, activeBottomTab: 'terminal' }),
    false,
    'stacked phone layout on another bottom tab must not poll',
  );
  assert.equal(
    isDbWatchPanelVisible({ stack: false, stackPane: 'editor', bottomDockOpen: true, activeBottomTab: 'dbwatch' }),
    true,
    'desktop workbench with dock open + dbwatch tab must poll',
  );
  assert.equal(
    isDbWatchPanelVisible({ stack: false, stackPane: 'tools', bottomDockOpen: false, activeBottomTab: 'dbwatch' }),
    false,
    'desktop workbench with dock closed must not poll',
  );
});

test('S-38 #3: ideDbWatchState maps detect and snapshot failures through loadErrorText', async () => {
  const mod = loadSource(
    new URL('../src/modules/ide/dbwatch/ideDbWatchState.svelte.ts', import.meta.url),
    {
      '../../../lib/api/client': {
        api: {
          post: async (url: string) => {
            if (url.includes('/detect')) throw new ApiError(404, 'not found');
            throw new ApiError(401, 'unauthorized');
          },
        },
      },
      '../store/ideStateSlices.svelte': { onIdeTargetReset: () => {} },
    },
    {},
    { relative: true },
  );
  const watch = mod.ideDbWatch;

  await watch.detect({ root: '/repo' });
  assert.equal(watch.detectError, 'It no longer exists, or this daemon doesn’t serve it.');

  watch.savedConnections = [
    { target_kind: 'saved', connection_id: 'c1', name: 'MySQL', engine: 'mysql', host: '127.0.0.1', port: 3306, environment: 'dev', database: 'app' },
  ];
  watch.selectedTargetKey = 'saved:c1';
  watch.selectedObjectName = 'users';

  const ok = await watch.captureSnapshot('/repo', 'baseline');
  assert.equal(ok, false);
  assert.equal(watch.snapshotError, 'Your session expired. Sign in again, then retry.');
});

// ---------------------------------------------------------------------------
// 3. Build Tools recursive module/goal/plugin/dependency filtering
// ---------------------------------------------------------------------------

test('S-38 #4: moduleMatchesBuildFilter actually filters out non-matching modules while matching names, goals, plugins, deps, and children', () => {
  const billingChild: IdeBuildToolModule = {
    name: 'billing-worker',
    artifact_id: 'billing-worker',
    group_id: 'com.acme',
    tool: 'maven',
    root_dir: '/repo/billing-worker',
    manifest_path: '/repo/billing-worker/pom.xml',
    lifecycle: ['clean', 'compile', 'test'],
    plugins: [{ name: 'spring-boot', goals: ['spring-boot:run'] }],
    dependencies: ['org.postgresql:postgresql:42.7.0'],
    repositories: [{ id: 'central', url: 'https://repo.maven.apache.org/maven2' }],
    modules: [],
  };
  const rootMod: IdeBuildToolModule = {
    name: 'platform-parent',
    artifact_id: 'platform-parent',
    group_id: 'com.acme',
    tool: 'maven',
    root_dir: '/repo',
    manifest_path: '/repo/pom.xml',
    lifecycle: ['clean', 'verify'],
    plugins: [{ name: 'compiler', goals: ['compiler:compile'] }],
    dependencies: [],
    repositories: [{ id: 'central', url: 'https://repo.maven.apache.org/maven2' }],
    modules: [billingChild],
  };
  const unrelatedMod: IdeBuildToolModule = {
    name: 'docs-site',
    artifact_id: 'docs-site',
    group_id: 'com.acme',
    tool: 'gradle',
    root_dir: '/docs',
    manifest_path: '/docs/build.gradle.kts',
    lifecycle: ['clean', 'build', 'bootRun'],
    plugins: [],
    dependencies: [],
    repositories: [],
    modules: [],
  };

  assert.equal(moduleMatchesBuildFilter(unrelatedMod, ''), true);
  assert.equal(moduleMatchesBuildFilter(unrelatedMod, 'billing'), false, 'non-matching module must be hidden');
  assert.equal(moduleMatchesBuildFilter(billingChild, 'billing'), true);
  assert.equal(moduleMatchesBuildFilter(rootMod, 'billing'), true, 'parent stays visible when child module matches');
  assert.equal(moduleSelfMatchesBuildFilter(rootMod, 'billing'), false);
  assert.equal(moduleMatchesBuildFilter(rootMod, 'spring-boot:run'), true, 'matches plugin goal');
  assert.equal(moduleMatchesBuildFilter(rootMod, 'postgresql'), true, 'matches dependency');
  assert.equal(moduleMatchesBuildFilter(unrelatedMod, 'bootrun'), true, 'matches gradle lifecycle phase case-insensitively');
  assert.equal(moduleMatchesBuildFilter(rootMod, 'nonexistent-xyz'), false);
});

// ---------------------------------------------------------------------------
// 4. Workspace search cancel & latestOnly() guards on findReferences / previewRename
// ---------------------------------------------------------------------------

function loadLspModule(apiImpl: Record<string, unknown>) {
  const toastErrors: string[] = [];
  const mod = loadSource(
    new URL('../src/modules/ide/shell/ideLspAndSearchHelpers.ts', import.meta.url),
    {
      svelte: { tick: async () => {} },
      '../../../lib/api/client': { api: apiImpl, ApiError, getToken: () => '', laneFetch: async () => new Response('{}') },
      '../../../lib/components/cm-ide-drafts': { readIdeUndoDraft: () => null, removeIdeUndoDraft: () => {}, writeIdeUndoDraftImmediate: () => {} },
      '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({ autoSave: false }) },
      '../../../lib/confirm.svelte': { confirmer: { ask: async () => true, choose: async () => ({ value: null }) } },
      '../../../lib/toast.svelte': { toasts: { success() {}, error() {}, info() {} } },
      '../../../lib/toastError': { toastError: (t: string) => toastErrors.push(t) },
      '../../../lib/ideProjectsModel': { loadNamedIdeProjectsLocal: () => [], saveNamedIdeProjectsLocal: () => {}, fetchNamedIdeProjectsFromServer: async () => [], parseNamedIdeProject: () => null },
      '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: { rootsForLocal: () => [] } },
      '../../../lib/storage': { lsGet: () => null, lsSet: () => {}, lsRemove: () => {} },
    },
    {},
    { relative: true },
  );
  return { mod, toastErrors };
}

test('S-38 #5a: runFindReferences and runPreviewRename ignore stale out-of-order responses and cancel on cancelLspAndSearch', async () => {
  const refDefs: Array<ReturnType<typeof deferred<{ symbol: string; locations: Array<{ path: string; line: number; column: number; preview: string }> }>>> = [];
  const refSignals: AbortSignal[] = [];
  const renDefs: Array<ReturnType<typeof deferred<{ symbol: string; source: 'lsp' | 'text'; edits: Array<{ path: string; line: number; preview: string }> }>>> = [];
  const renSignals: AbortSignal[] = [];
  const searchSignals: AbortSignal[] = [];
  const searchDef = deferred<{ items: Array<{ kind: string; path: string }>; truncated: boolean }>();

  const { mod, toastErrors } = loadLspModule({
    post: (url: string, _body: unknown, signal?: AbortSignal) => {
      if (url === '/ide/lsp/references') {
        if (signal) refSignals.push(signal);
        const d = deferred<{ symbol: string; locations: Array<{ path: string; line: number; column: number; preview: string }> }>();
        refDefs.push(d);
        return d.promise;
      }
      if (url === '/ide/lsp/rename') {
        if (signal) renSignals.push(signal);
        const d = deferred<{ symbol: string; source: 'lsp' | 'text'; edits: Array<{ path: string; line: number; preview: string }> }>();
        renDefs.push(d);
        return d.promise;
      }
      return Promise.resolve({});
    },
    long: {
      post: (_url: string, _body: unknown, signal?: AbortSignal) => {
        if (signal) searchSignals.push(signal);
        return searchDef.promise;
      },
    },
  });

  const host = {
    target: { kind: 'local', root: '/repo' },
    targetPayload: { kind: 'local', root: '/repo', project_folders: ['/repo'] },
    tabs: [],
    canEditCollab: true,
    collabIsGuest: false,
    usages: [] as Array<{ path: string; line: number }>,
    usagesSymbol: '',
    usagesSource: 'text' as 'lsp' | 'text',
    usagesLoading: false,
    searchScopePath: null,
    searchResults: [] as Array<{ kind: string; path: string }>,
    searchTruncated: false,
    searchLoading: false,
    searchError: null as string | null,
    renameState: {
      open: true,
      path: '/repo/a.ts',
      line: 1,
      column: 1,
      symbol: 'oldName',
      newName: '',
      edits: [] as Array<{ path: string; line: number; preview: string }>,
      source: 'text' as 'lsp' | 'text',
      previewedName: '',
      loading: false,
      error: null as string | null,
    },
    resolvePath: (p: string) => p,
    getBuffer: () => 'const oldName = 1;',
    editorHandleForPath: () => null,
    lspRenameEdits: async () => null,
    openBottomTab: () => {},
  };

  const flushMicrotasks = async (): Promise<void> => {
    for (let i = 0; i < 6; i++) await Promise.resolve();
  };

  // Start two overlapping findReferences calls: #1 must be aborted & ignored when #2 finishes first.
  const pRef1 = mod.runFindReferences(host, '/repo/a.ts', 1, 1, 'SymA');
  await flushMicrotasks();
  const pRef2 = mod.runFindReferences(host, '/repo/b.ts', 2, 1, 'SymB');
  await flushMicrotasks();
  assert.equal(refSignals[0].aborted, true, 'first findReferences signal aborted by second');
  assert.equal(refSignals[1].aborted, false);

  refDefs[1].resolve({ symbol: 'SymB', locations: [{ path: '/repo/b.ts', line: 2, column: 1, preview: 'SymB' }] });
  await pRef2;
  assert.equal(host.usagesSymbol, 'SymB');
  assert.equal(host.usagesLoading, false);

  refDefs[0].resolve({ symbol: 'SymA', locations: [{ path: '/repo/a.ts', line: 1, column: 1, preview: 'SymA' }] });
  await pRef1;
  assert.equal(host.usagesSymbol, 'SymB', 'stale findReferences response must not overwrite newer usages');
  assert.equal(host.usages.length, 1);
  assert.equal(host.usages[0].path, '/repo/b.ts');

  // Start two overlapping previewRename calls: #1 is superseded by #2.
  const pRen1 = mod.runPreviewRename(host, 'firstNew');
  await flushMicrotasks();
  const pRen2 = mod.runPreviewRename(host, 'secondNew');
  await flushMicrotasks();
  assert.equal(renSignals[0].aborted, true, 'first previewRename signal aborted by second');

  renDefs[1].resolve({ symbol: 'oldName', source: 'lsp', edits: [{ path: '/repo/a.ts', line: 1, preview: 'secondNew' }] });
  await pRen2;
  assert.equal(host.renameState.previewedName, 'secondNew');
  assert.equal(host.renameState.loading, false);

  renDefs[0].reject(new Error('stale error'));
  await pRen1;
  assert.equal(host.renameState.previewedName, 'secondNew');
  assert.equal(host.renameState.error, null, 'stale previewRename error must not overwrite current preview');

  // Start workspace search + third findReferences + third previewRename, then cancelLspAndSearch().
  const pSearch = mod.runSearchWorkspace(host, 'hello', 'all');
  const pRef3 = mod.runFindReferences(host, '/repo/c.ts', 3, 1, 'SymC');
  const pRen3 = mod.runPreviewRename(host, 'thirdNew');
  await flushMicrotasks();
  mod.cancelLspAndSearch();

  assert.equal(searchSignals[0].aborted, true);
  assert.equal(refSignals[2].aborted, true);
  assert.equal(renSignals[2].aborted, true);

  searchDef.resolve({ items: [{ kind: 'file', path: '/repo/stale.ts' }], truncated: false });
  refDefs[2].resolve({ symbol: 'SymC', locations: [{ path: '/repo/c.ts', line: 3, column: 1, preview: 'SymC' }] });
  renDefs[2].resolve({ symbol: 'oldName', source: 'text', edits: [] });
  await Promise.all([pSearch, pRef3, pRen3]);

  assert.deepEqual(host.searchResults, [], 'cancelled workspace search must not write results');
  assert.equal(host.usagesSymbol, 'SymB', 'cancelled findReferences must not overwrite usages');
  assert.equal(host.renameState.previewedName, 'secondNew', 'cancelled previewRename must not overwrite preview');
  assert.deepEqual(toastErrors, []);
});

// ---------------------------------------------------------------------------
// 5. Safe-delete skips stale ide.usages for non-identifier filenames
// ---------------------------------------------------------------------------

test('S-38 #5b: extractSafeDeleteSymbol & formatSafeDeleteMessage skip usage lookup for hyphen/dot filenames', () => {
  assert.equal(extractSafeDeleteSymbol('/repo/src/my-comp.svelte'), null);
  assert.equal(extractSafeDeleteSymbol('/repo/src/foo.test.ts'), null);
  assert.equal(extractSafeDeleteSymbol('/repo/src/123bad.go'), null);
  assert.equal(extractSafeDeleteSymbol('/repo/src/PaymentService.ts'), 'PaymentService');

  const skipped = formatSafeDeleteMessage({
    relPath: 'src/my-comp.svelte',
    symbol: null,
    checked: false,
    externalRefs: 99,
  });
  assert.match(skipped, /skipped/i);
  assert.doesNotMatch(skipped, /99/, 'must never report stale usages when search was skipped');

  const foundOne = formatSafeDeleteMessage({
    relPath: 'src/PaymentService.ts',
    symbol: 'PaymentService',
    checked: true,
    externalRefs: 1,
  });
  assert.match(foundOne, /1 workspace usage referencing “PaymentService”/);

  const foundNone = formatSafeDeleteMessage({
    relPath: 'src/PaymentService.ts',
    symbol: 'PaymentService',
    checked: true,
    externalRefs: 0,
  });
  assert.match(foundNone, /No external symbol usages found for “PaymentService”/);
});

// ---------------------------------------------------------------------------
// 6. Per-path savesInFlight serialization (formatActiveFile + autosave)
// ---------------------------------------------------------------------------

test('S-38 #5c: IdeBufferPerfController.enqueuePathOp serializes format & save per path without blocking other paths', async () => {
  const bufMod = loadSource(
    new URL('../src/modules/ide/store/ideBufferPerf.ts', import.meta.url),
    {
      svelte: { tick: async () => {} },
      '../../../lib/api/client': { api: {}, ApiError, getToken: () => '', laneFetch: async () => new Response('{}') },
      '../../../lib/components/cm-ide-drafts': { readIdeUndoDraft: () => null, removeIdeUndoDraft: () => {}, writeIdeUndoDraftImmediate: () => {} },
      '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({ autoSave: false }) },
      '../../../lib/confirm.svelte': { confirmer: { ask: async () => true, choose: async () => ({ value: null }) } },
      '../../../lib/toast.svelte': { toasts: { success() {}, error() {}, info() {} } },
      '../../../lib/toastError': { toastError() {} },
      '../../../lib/ideProjectsModel': { loadNamedIdeProjectsLocal: () => [], saveNamedIdeProjectsLocal: () => {}, fetchNamedIdeProjectsFromServer: async () => [], parseNamedIdeProject: () => null },
      '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: { rootsForLocal: () => [] } },
      '../../../lib/storage': { lsGet: () => null, lsSet: () => {}, lsRemove: () => {} },
    },
    {},
    { relative: true },
  );
  const perf = new bufMod.IdeBufferPerfController();
  const order: string[] = [];
  const dFormat = deferred<void>();
  const dOther = deferred<void>();

  // Start format on /repo/a.go, queue autosave on /repo/a.go, and start concurrent op on /repo/b.go.
  const pFormat = perf.enqueuePathOp('/repo/a.go', async () => {
    order.push('a:format:start');
    await dFormat.promise;
    order.push('a:format:end');
  });
  const pSave = perf.enqueuePathOp('/repo/a.go', async () => {
    order.push('a:save:start');
    return true;
  });
  const pOther = perf.enqueuePathOp('/repo/b.go', async () => {
    order.push('b:save:start');
    await dOther.promise;
    order.push('b:save:end');
  });

  await Promise.resolve();
  await Promise.resolve();
  assert.deepEqual(order, ['a:format:start', 'b:save:start'], 'same-path save must wait for format; different path runs concurrently');

  dOther.resolve();
  await pOther;
  dFormat.resolve();
  await Promise.all([pFormat, pSave]);
  assert.deepEqual(order, ['a:format:start', 'b:save:start', 'b:save:end', 'a:format:end', 'a:save:start']);
  assert.equal(perf.savesInFlight.size, 0, 'queue cleans up after completion');

  // An error in one op must not wedge subsequent ops on the same path.
  await assert.rejects(() => perf.enqueuePathOp('/repo/a.go', async () => { throw new Error('boom'); }), /boom/);
  const recovered = await perf.enqueuePathOp('/repo/a.go', async () => 'ok');
  assert.equal(recovered, 'ok');
  assert.equal(perf.savesInFlight.size, 0);
});

// ---------------------------------------------------------------------------
// 7. Split-pane keyboard resize with RTL arrow inversion & clamping
// ---------------------------------------------------------------------------

test('S-38 #6: computeSplitRatioFromKey flips Left/Right in RTL, handles vertical split, clamps, and resets', () => {
  // LTR side-by-side
  assert.equal(computeSplitRatioFromKey(0.5, 'ArrowLeft', { down: false, rtl: false }), 0.45);
  assert.equal(computeSplitRatioFromKey(0.5, 'ArrowRight', { down: false, rtl: false }), 0.55);
  assert.equal(computeSplitRatioFromKey(0.5, 'ArrowRight', { down: false, rtl: false, shiftKey: true }), 0.6);

  // RTL side-by-side: ArrowLeft grows primary (inline-start is right), ArrowRight shrinks it.
  assert.equal(computeSplitRatioFromKey(0.5, 'ArrowLeft', { down: false, rtl: true }), 0.55);
  assert.equal(computeSplitRatioFromKey(0.5, 'ArrowRight', { down: false, rtl: true }), 0.45);

  // Split-down (horizontal separator): Up/Down only
  assert.equal(computeSplitRatioFromKey(0.5, 'ArrowUp', { down: true, rtl: true }), 0.45);
  assert.equal(computeSplitRatioFromKey(0.5, 'ArrowDown', { down: true, rtl: true }), 0.55);
  assert.equal(computeSplitRatioFromKey(0.5, 'ArrowLeft', { down: true, rtl: false }), null);

  // Clamping & jump/reset keys
  assert.equal(computeSplitRatioFromKey(0.2, 'ArrowLeft', { down: false, rtl: false }), 0.2);
  assert.equal(computeSplitRatioFromKey(0.8, 'ArrowRight', { down: false, rtl: false }), 0.8);
  assert.equal(computeSplitRatioFromKey(0.6, 'Home', { down: false, rtl: false }), 0.2);
  assert.equal(computeSplitRatioFromKey(0.4, 'End', { down: false, rtl: false }), 0.8);
  assert.equal(computeSplitRatioFromKey(0.7, 'Enter', { down: false, rtl: false }), 0.5);
  assert.equal(computeSplitRatioFromKey(0.5, 'Tab', { down: false, rtl: false }), null);
});
