// IDE round 4, second pass: mixed line endings survive a save (G18), the delete prompt
// names unsaved tabs (G19), applied agent proposals survive leaving the page (G11), a
// failed folder load is an inline row with Retry (G17), the tree's keyboard model (G15)
// and its one-pass filter (G16).
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { loadSource } from './sourceHarness.ts';
import { treeFilterMatches, treeNavAction, type TreeNavRow } from '../src/modules/ide/history/ideTreeNav.ts';

const types = loadSource(new URL('../src/modules/ide/store/ideTypes.ts', import.meta.url), {
  '../../../lib/api/client': { getToken: () => '', laneFetch: () => { throw new Error('unused'); } },
});

// ── G18 ─────────────────────────────────────────────────────────────────────

test('G18: a one-line edit in a mixed-ending file changes only that line', () => {
  const disk = 'a\r\nb\nc\r\nd\n';
  const tab = { eol: types.detectTextEol(disk), diskEols: types.diskEolsOf(disk), savedContent: types.normalizeEditorLf(disk) };
  assert.ok(tab.diskEols, 'mixed endings are remembered');
  assert.equal(types.serializeTabDiskContent(tab, 'a\nB\nc\nd\n'), 'a\r\nB\nc\r\nd\n');
  assert.equal(types.serializeTabDiskContent(tab, 'a\nb\nnew\nc\nd\n'), `a\r\nb\nnew${tab.eol}c\r\nd\n`, 'an inserted line takes the dominant style, the rest keep theirs');
  assert.equal(types.serializeTabDiskContent(tab, 'a\nc\nd\n'), 'a\r\nc\r\nd\n', 'a deleted line drops only its own ending');
});

test('G18: lone CR endings are kept, and one-style files behave as before', () => {
  const disk = 'x\ry\nz';
  const tab = { eol: types.detectTextEol(disk), diskEols: types.diskEolsOf(disk), savedContent: types.normalizeEditorLf(disk) };
  assert.equal(types.serializeTabDiskContent(tab, 'x\ny\nZ'), 'x\ry\nZ');
  assert.equal(types.diskEolsOf('a\r\nb\r\n'), undefined);
  assert.equal(types.serializeTabDiskContent({ eol: '\r\n' }, 'a\nb\n'), 'a\r\nb\r\n');
  assert.equal(types.serializeTabDiskContent({ eol: '\n', diskEols: ['\r\n', ''], savedContent: 'stale\nmore\nlines' }, 'a\nb'), 'a\nb', 'stale endings fall back to the dominant style');
});

// ── G19 ─────────────────────────────────────────────────────────────────────

test('G19: the delete prompt names open files with unsaved changes', () => {
  const menus = loadSource(new URL('../src/modules/ide/history/ideTreeNodeMenus.ts', import.meta.url), {
    '../../../lib/clipboard': { copyWithToast: () => {} },
    '../../../lib/confirm.svelte': { confirmer: {} },
    '../../../lib/contextmenu.svelte': {},
    '../git/ideGitQuickMenu': { buildGitSubmenuItems: () => [], captureMenuAnchor: () => null, showIdeMenu: () => {} },
    '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({}) },
    '../ideStore.svelte': { ide: {}, ideBasename: (p: string) => p },
    '../store/ideAgentController.svelte': { ideAgent: {} },
    '../store/ideRepoTestAndBuildTags': { resolveRepoRootForPath: () => null },
    './ideComparisonLoader': { promptPutLocalHistoryLabel: async () => false },
    './ideManifestActions': { buildManifestMenuItems: () => [] },
  }, {}, { relative: false });
  assert.equal(menus.deleteConfirmText('src', 'disk', []), 'Permanently delete “src” from disk?');
  assert.match(menus.deleteConfirmText('src', 'disk', ['a.ts']), /“a\.ts” has unsaved changes that will be lost/);
  assert.match(menus.deleteConfirmText('src', 'disk', ['a', 'b', 'c', 'd', 'e']), /“a”, “b”, “c” and 2 more have unsaved changes/);
});

// ── G17 ─────────────────────────────────────────────────────────────────────

test('G17: a folder load failure is recorded for an inline row and cleared when it loads', () => {
  const mod = loadSource(new URL('../src/modules/ide/store/ideTreeDirErrors.svelte.ts', import.meta.url), {});
  const errs = mod.ideTreeDirErrors;
  errs.set('/repo/src', 'HTTP 502');
  assert.equal(errs.byPath['/repo/src'], 'HTTP 502');
  errs.clear('/repo/src');
  assert.equal(errs.byPath['/repo/src'], undefined);
  errs.set('/repo/a', 'x');
  errs.reset();
  assert.deepEqual(Object.keys(errs.byPath), []);
});

test('G17: loadAttachedRootEntries records the failure instead of toasting', async () => {
  const toasts: string[] = [];
  const cache = new Map();
  const imports = {
    '../../../lib/api/client': { api: { get: async () => { throw new Error('HTTP 502'); } }, getToken: () => '', laneFetch: () => { throw new Error('unused'); } },
    '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({}) },
    '../../../lib/confirm.svelte': { confirmer: {} },
    '../../../lib/ideProjectsModel': { loadNamedIdeProjectsLocal: () => [], saveNamedIdeProjectsLocal: () => {}, fetchNamedIdeProjectsFromServer: async () => [], parseNamedIdeProject: () => null },
    '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: {} },
    '../../../lib/storage': { lsGet: () => null, lsSet: () => {}, lsRemove: () => {} },
    '../../../lib/toast.svelte': { toasts: { success() {}, error: (t: string) => toasts.push(t), info() {} } },
    '../../../lib/toastError': { toastError: (t: string) => toasts.push(t) },
    '../../../lib/components/cm-ide-drafts': { readIdeUndoDraft: () => null, removeIdeUndoDraft: () => {}, writeIdeUndoDraftImmediate: () => {} },
    svelte: { tick: async () => {} },
  };
  const helpers = loadSource(new URL('../src/modules/ide/shell/ideProjectHelpers.ts', import.meta.url), imports, {}, { relative: true }, cache);
  const errs = loadSource(new URL('../src/modules/ide/store/ideTreeDirErrors.svelte.ts', import.meta.url), imports, {}, { relative: true }, cache).ideTreeDirErrors;
  const host = { target: { kind: 'local', root: '/repo', connection_id: null }, showHidden: false, projectRoots: [], childrenByDir: {}, targetQuery: () => '' };
  await helpers.loadAttachedRootEntries(host, '/other');
  assert.match(String(errs.byPath['/other']), /HTTP 502/);
  assert.deepEqual(toasts, []);
});

// ── G15 ─────────────────────────────────────────────────────────────────────

const rows: TreeNavRow[] = [
  { depth: 0, isDir: true, expanded: true },   // 0 project root
  { depth: 1, isDir: true, expanded: false },  // 1 src (closed)
  { depth: 1, isDir: true, expanded: true },   // 2 lib (open)
  { depth: 2, isDir: false, expanded: false, loadError: 'HTTP 502' }, // 3 inline error row
  { depth: 2, isDir: false, expanded: false }, // 4 lib/a.ts
  { depth: 1, isDir: false, expanded: false }, // 5 README
];

test('G15: arrow keys move like a native tree and skip error rows', () => {
  assert.deepEqual(treeNavAction(rows, 0, 'ArrowDown'), { kind: 'focus', index: 1 });
  assert.deepEqual(treeNavAction(rows, 2, 'ArrowDown'), { kind: 'focus', index: 4 }, 'skips the error row');
  assert.deepEqual(treeNavAction(rows, 4, 'ArrowUp'), { kind: 'focus', index: 2 });
  assert.deepEqual(treeNavAction(rows, 3, 'Home'), { kind: 'focus', index: 0 });
  assert.deepEqual(treeNavAction(rows, 0, 'End'), { kind: 'focus', index: 5 });
  assert.deepEqual(treeNavAction(rows, 5, 'ArrowDown'), { kind: 'focus', index: 5 }, 'stays on the last row');
});

test('G15: → expands / enters a folder, ← collapses / goes to the parent, ⇧F10 opens the menu', () => {
  assert.deepEqual(treeNavAction(rows, 1, 'ArrowRight'), { kind: 'expand', index: 1 });
  assert.deepEqual(treeNavAction(rows, 2, 'ArrowRight'), { kind: 'focus', index: 4 });
  assert.equal(treeNavAction(rows, 5, 'ArrowRight'), null, 'a file has nothing to expand');
  assert.deepEqual(treeNavAction(rows, 2, 'ArrowLeft'), { kind: 'collapse', index: 2 });
  assert.deepEqual(treeNavAction(rows, 4, 'ArrowLeft'), { kind: 'focus', index: 2 });
  assert.deepEqual(treeNavAction(rows, 4, 'F10', true), { kind: 'menu', index: 4 });
  assert.equal(treeNavAction(rows, 4, 'F10', false), null);
  assert.equal(treeNavAction(rows, 4, 'Enter'), null, 'Enter stays with the row button');
});

// ── G16 ─────────────────────────────────────────────────────────────────────

test('G16: the filter keeps a folder that holds a loaded match, in one pass', () => {
  const root = [
    { name: 'src', path: '/r/src', kind: 'dir' },
    { name: 'docs', path: '/r/docs', kind: 'dir' },
    { name: 'README.md', path: '/r/README.md', kind: 'file' },
  ];
  const children = {
    '/r/src': [{ name: 'deep', path: '/r/src/deep', kind: 'dir' }],
    '/r/src/deep': [{ name: 'needle.ts', path: '/r/src/deep/needle.ts', kind: 'file' }],
    '/r/docs': [{ name: 'guide.md', path: '/r/docs/guide.md', kind: 'file' }],
  };
  const hits = treeFilterMatches('NEEDLE', [root as never], children as never)!;
  assert.deepEqual([...hits].sort(), ['/r/src', '/r/src/deep', '/r/src/deep/needle.ts']);
  assert.equal(treeFilterMatches('  ', [root as never], children as never), null);
});

// ── G11 ─────────────────────────────────────────────────────────────────────

test('G11: leaving the IDE page keeps which proposals were applied; a project switch clears it', () => {
  const mod = loadSource(new URL('../src/modules/ide/agent/ideAgentState.svelte.ts', import.meta.url), {
    '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: { agentDockOpen: false } },
    '../../../lib/providers': { defaultAgentProvider: () => 'claude', isRuntimeProvider: () => true },
    '../../../lib/runtimeThreadPort': {
      getIdeRuntimeComposerUi: () => null, getIdeRuntimeSessionPort: () => null,
      loadIdeRuntimeComposerUi: async () => null, loadIdeRuntimeSessionPort: async () => null,
      IDE_OPEN_THREAD_EVENT: 'otto:ide-open-thread',
    },
    '../../../lib/storage': { lsGet: () => null, lsSet: () => {}, lsRemove: () => {} },
    '../ideStore.svelte': { ide: {}, ideBasename: (p: string) => p },
    '../store/ideDiffApplier': { detectProposalLanguage: () => '', lineRangeFromOffsets: () => null, offsetsForLineRange: () => null },
    './ideSymbolChip': { upsertIdeContextChipList: (l: unknown) => l },
  });
  const st = mod.ideAgentState;
  st.appliedUndoByProposalId = { p1: 'before' };
  st.appliedPathByProposalId.set('p1', '/repo/a.ts');
  st.disposeState();
  assert.equal(st.appliedUndoByProposalId.p1, 'before', 'Undo survives leaving the page');
  assert.equal(st.appliedPathByProposalId.get('p1'), '/repo/a.ts', 'the card still shows Applied');
  st.resetTargetState();
  assert.equal(st.appliedPathByProposalId.size, 0);
  assert.deepEqual(Object.keys(st.appliedUndoByProposalId), []);
});

// ── phone layout: tree row height, compact editor ───────────────────────────

test('tree rows: one height constant the virtualiser reads — 40 px on touch, 26 px otherwise', async () => {
  const { treeRowHeight, TREE_ROW_H_COARSE, TREE_ROW_H_FINE } = await import('../src/modules/ide/history/ideTreeNav.ts');
  assert.equal(treeRowHeight(true), 40);
  assert.equal(treeRowHeight(false), 26);
  assert.ok(TREE_ROW_H_COARSE >= 40 && TREE_ROW_H_FINE === 26);
});

test('compact (stacked) editor: soft wrap and the narrow breakpoint lane follow the layout', async () => {
  const { EditorState } = await import('@codemirror/state');
  const { EditorView } = await import('@codemirror/view');
  const compactMod = await import('../src/lib/components/cm-ide-compact.ts');
  const wraps = (st: ReturnType<typeof EditorState.create>) =>
    st.facet(EditorView.contentAttributes).some((a: unknown) => typeof a === 'object' && a !== null && String((a as { class?: string }).class ?? '').includes('cm-lineWrapping'));
  assert.equal(wraps(EditorState.create({ extensions: compactMod.ideCompactExtension() })), false, 'desktop: no soft wrap');
  compactMod.setIdeCompactLayout(true);
  assert.equal(compactMod.isIdeCompactLayout(), true);
  assert.equal(wraps(EditorState.create({ extensions: compactMod.ideCompactExtension() })), true, 'stacked: soft wrap on');
  const src = (await import('node:fs')).readFileSync(new URL('../src/lib/components/cm-ide-compact.ts', import.meta.url), 'utf8');
  assert.match(src, /\.cm-ide-bp-gutter[^']*': \{ inlineSize: '20px'/, 'a 20 px breakpoint lane');
  compactMod.setIdeCompactLayout(false);
});
