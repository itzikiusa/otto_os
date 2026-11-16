// IDE editor chrome (round 5, I1): one path crumbs row with a collapsed middle, a status
// strip model (problems, language, line endings, language-server state), editor actions
// that respect the file kind, the ⋯ menu built from them, and the tab strip's reveal math.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { loadSource } from './sourceHarness.ts';
import {
  collapseIdeCrumbs,
  ideCrumbAbsPath,
  ideParentFolderHint,
  ideRevealDelta,
  ideSymbolChainAt,
  ideSymbolSiblings,
} from '../src/modules/ide/editor/ideCrumbs.ts';
import {
  applyIdeLspLink,
  ideEditorActionsFor,
  ideEolLabel,
  ideFileKind,
  ideLanguageLabel,
  ideLanguageSettingsSection,
  ideLspItem,
  ideLspLangFor,
  ideLspRecordFor,
  ideProblemCounts,
  type IdeLspRecord,
} from '../src/modules/ide/editor/ideStatusModel.ts';

const DEEP = 'services/payments/internal/settlement/reconciliation/adapters/very_long_descriptive_settlement_reconciliation_adapter_test.go'.split('/');

test('I1-19: a deep path keeps first folder, parent folder and the full file name; the middle is one "…"', () => {
  const crumbs = collapseIdeCrumbs(DEEP);
  assert.deepEqual(crumbs.map((c) => (c.kind === 'seg' ? c.label : '…')), ['services', '…', 'adapters', DEEP[6]]);
  const more = crumbs[1];
  assert.equal(more.kind, 'more');
  if (more.kind === 'more') assert.deepEqual(more.hidden.map((h) => h.label), ['payments', 'internal', 'settlement', 'reconciliation']);
  const last = crumbs.at(-1)!;
  assert.ok(last.kind === 'seg' && last.last && last.label === DEEP[6], 'the file name is the last crumb, whole');
  // Short paths are not collapsed.
  assert.deepEqual(collapseIdeCrumbs(['app', 'main.go']).map((c) => c.kind), ['seg', 'seg']);
  assert.equal(collapseIdeCrumbs(['a', 'b', 'c', 'd.go']).some((c) => c.kind === 'more'), false);
  assert.equal(collapseIdeCrumbs(['a', 'b', 'c', 'd', 'e.go']).some((c) => c.kind === 'more'), true);
});

test('I1-19: a hidden folder crumb resolves to its absolute folder', () => {
  const abs = `/repo/${DEEP.join('/')}`;
  assert.equal(ideCrumbAbsPath(abs, DEEP.length, 1), '/repo/services/payments');
  assert.equal(ideCrumbAbsPath(abs, DEEP.length, DEEP.length - 1), abs);
});

test('I1-06: the overflow menu hint is the parent folder, not the whole relative path', () => {
  assert.equal(ideParentFolderHint('notes/note-01-meeting-summary.md'), 'notes');
  assert.equal(ideParentFolderHint('a/b/c.go'), 'b');
  assert.equal(ideParentFolderHint('README.md'), undefined);
});

test('I1-01: the symbol trail follows the cursor and each level lists its siblings', () => {
  const syms = [
    { name: 'A', line: 1, end_line: 10, children: [{ name: 'a1', line: 2, end_line: 4 }, { name: 'a2', line: 5, end_line: 9 }] },
    { name: 'B', line: 12, end_line: 20 },
  ];
  const chain = ideSymbolChainAt(syms, 6);
  assert.deepEqual(chain.map((s) => s.name), ['A', 'a2']);
  assert.deepEqual(ideSymbolSiblings(syms, chain, 0).map((s) => s.name), ['A', 'B']);
  assert.deepEqual(ideSymbolSiblings(syms, chain, 1).map((s) => s.name), ['a1', 'a2']);
  assert.deepEqual(ideSymbolChainAt(syms, 11), []);
});

test('I1-02: editor actions respect the file kind — no function navigation on CSV or Markdown', () => {
  assert.equal(ideFileKind('data.csv'), 'data');
  assert.equal(ideFileKind('README.md'), 'markup');
  assert.equal(ideFileKind('app/main.go'), 'code');
  assert.equal(ideFileKind('Makefile'), 'code');
  const csv = ideEditorActionsFor('data.csv', 'csv', true);
  assert.equal(csv.jumpFunction, false);
  assert.equal(csv.quickFix, false);
  assert.equal(csv.reformat, false);
  const md = ideEditorActionsFor('README.md', 'markdown', true);
  assert.equal(md.jumpFunction, false);
  assert.equal(md.structure, true, 'headings still give a structure');
  assert.equal(ideEditorActionsFor('notes.txt', null, false).structure, false);
  const go = ideEditorActionsFor('main.go', 'go', true);
  assert.deepEqual(go, { structure: true, jumpFunction: true, quickFix: true, reformat: true, optimizeImports: true });
  assert.equal(ideEditorActionsFor('site.css', 'css', true).jumpFunction, false);
  assert.equal(ideEditorActionsFor('cfg.json', 'json', true).reformat, true);
});

test('I1-02: status strip items — problems, language, line ending (unknown is left out)', () => {
  assert.deepEqual(ideProblemCounts([{ severity: 'error' }, { severity: 'warning' }, { severity: 'error' }, { severity: 'info' }]), { errors: 2, warnings: 1 });
  assert.equal(ideLanguageLabel('main.go', 'go'), 'Go');
  assert.equal(ideLanguageLabel('x.ts', ''), 'TypeScript');
  assert.equal(ideLanguageLabel('LICENSE', null), 'Plain text');
  assert.equal(ideLanguageSettingsSection('main.go', 'go'), 'lang.go');
  assert.equal(ideLanguageSettingsSection('a.tsx', 'typescript'), 'lang.node');
  assert.equal(ideLanguageSettingsSection('README.md', 'markdown'), null);
  assert.equal(ideEolLabel({ eol: '\n' }), 'LF');
  assert.equal(ideEolLabel({ eol: '\r\n' }), 'CRLF');
  assert.equal(ideEolLabel({ eol: '\n', diskEols: ['\r\n', '\n'] }), 'Mixed');
  assert.equal(ideEolLabel({}), null);
});

test('I1-13: language-server state — starting → ready, a dropped link restarts, a server that never started is unavailable', () => {
  assert.equal(ideLspLangFor('main.go', 'go'), 'go');
  assert.equal(ideLspLangFor('README.md', 'markdown'), null, 'no item for a file Otto runs no server for');
  let recs: IdeLspRecord[] = [];
  assert.equal(ideLspItem('go', ideLspRecordFor(recs, 'go', '/repo/app/main.go')), null, 'nothing asked for yet: no item');
  recs = applyIdeLspLink(recs, { rootPath: '/repo', state: 'starting', attempt: 0, lang: 'go' });
  assert.equal(ideLspItem('go', ideLspRecordFor(recs, 'go', '/repo/app/main.go'))?.label, 'Starting gopls…');
  recs = applyIdeLspLink(recs, { rootPath: '/repo', state: 'ready', attempt: 0, lang: 'go' });
  const ready = ideLspItem('go', ideLspRecordFor(recs, 'go', '/repo/app/main.go'));
  assert.equal(ready?.label, 'gopls');
  assert.equal(ready?.tone, 'ok');
  recs = applyIdeLspLink(recs, { rootPath: '/repo/app', state: 'reconnecting', attempt: 1 });
  assert.equal(ideLspItem('go', ideLspRecordFor(recs, 'go', '/repo/app/main.go'))?.tone, 'busy', 'a related root restarts');
  recs = applyIdeLspLink(recs, { rootPath: '/repo', state: 'lost', attempt: 5 });
  assert.equal(ideLspItem('go', ideLspRecordFor(recs, 'go', '/repo/app/main.go'))?.action, 'retry');
  recs = applyIdeLspLink(recs, { rootPath: '/other', state: 'unavailable', attempt: 0, lang: 'python', reason: 'no_session' });
  const py = ideLspItem('python', ideLspRecordFor(recs, 'python', '/other/x.py'));
  assert.equal(py?.label, 'pyright unavailable');
  assert.equal(py?.action, 'settings');
  assert.equal(ideLspRecordFor(recs, 'python', '/repo/x.py'), null, 'a record serves only files under its root');
});

test('I1-13: the reconnect module reports server states and replays them to a late listener', () => {
  const events: { type: string; detail: { state: string; lang?: string } }[] = [];
  const mod = loadSource(new URL('../src/lib/components/cm-ide-lsp-reconnect.ts', import.meta.url), {}, {
    window: { dispatchEvent: (e: { type: string; detail: never }) => { events.push({ type: e.type, detail: e.detail }); return true; } },
    CustomEvent: class { type: string; detail: unknown; constructor(type: string, init: { detail: unknown }) { this.type = type; this.detail = init.detail; } },
    setTimeout: () => 1,
    clearTimeout: () => {},
  });
  mod.noteLspStatus('/repo/', 'starting', 'go');
  mod.noteLspStatus('/repo', 'unavailable', 'go', 'no_session');
  assert.deepEqual(events.map((e) => [e.type, e.detail.state, e.detail.lang]), [
    ['otto:ide-lsp-link', 'starting', 'go'],
    ['otto:ide-lsp-link', 'unavailable', 'go'],
  ]);
  assert.deepEqual(JSON.parse(JSON.stringify(mod.recentLspLinks().map((d: { rootPath: string; state: string }) => [d.rootPath, d.state]))), [['/repo', 'starting'], ['/repo', 'unavailable']]);
});

test('I1-05: the strip scrolls just enough to show the active tab', () => {
  assert.equal(ideRevealDelta(100, 200, 0, 500), 0);
  assert.equal(ideRevealDelta(450, 600, 0, 500), 100, 'past the end: scroll forward by the overflow');
  assert.equal(ideRevealDelta(-80, 20, 0, 500), -80, 'before the start: scroll back');
  assert.equal(ideRevealDelta(100, 900, 0, 500), 100, 'wider than the strip: align its start');
});

function moreMenuModule() {
  const calls: string[] = [];
  const ide = {
    splitSelectionToLines: () => calls.push('split'),
    formatActiveFile: async () => { calls.push('format'); },
    organizeImportsActiveFile: async () => { calls.push('imports'); },
    openSettings: () => calls.push('settings'),
  };
  const mod = loadSource(new URL('../src/modules/ide/editor/ideEditorMoreMenu.ts', import.meta.url), {
    '../../../lib/contextmenu.svelte': { ctxMenu: { showAt() {} } },
    '../ideStore.svelte': { ide },
    './ideEditorStoreBridge.svelte': { flushAutoLocalHistorySnapshot() {}, toggleFormatOnSave() {} },
  }, {}, { relative: true });
  return { mod, calls };
}

function ctx(path: string, language: string, extra: Record<string, unknown> = {}) {
  const noop = () => {};
  return {
    tab: { path, name: path.split('/').pop(), language, content: 'x', readOnly: false },
    hasSymbols: true, hasSelection: false, blameActive: false, coarse: false, isGo: language === 'go',
    onjumpsymbol: noop, onopenstructure: noop, ontoggleblame: noop, onquickfix: noop, onsetgotags: noop, onsendsession: noop,
    ...extra,
  };
}

test('I1-02: the ⋯ menu offers function navigation for code only, and always Send to + Appearance', () => {
  const { mod } = moreMenuModule();
  const labels = (rows: { label?: string }[]) => rows.map((r) => r.label).filter(Boolean);
  const csv = labels(mod.ideEditorMoreRows(ctx('/r/data.csv', 'csv'), []));
  assert.equal(csv.includes('Previous function'), false);
  assert.equal(csv.includes('Next function'), false);
  assert.equal(csv.includes('Reformat code'), false);
  assert.ok(csv.includes('Send to') && csv.includes('Appearance') && csv.includes('Git blame'));
  const md = labels(mod.ideEditorMoreRows(ctx('/r/README.md', 'markdown'), []));
  assert.equal(md.includes('Previous function'), false);
  const go = labels(mod.ideEditorMoreRows(ctx('/r/main.go', 'go', { hasSelection: true }), []));
  for (const l of ['Split selection into cursors', 'File structure…', 'Previous function', 'Next function', 'Context actions…', 'Reformat code', 'Optimize imports', 'Go build tags…']) {
    assert.ok(go.includes(l), `Go file offers ${l}`);
  }
  const send = mod.ideEditorMoreRows(ctx('/r/main.go', 'go'), []).find((r: { label?: string }) => r.label === 'Send to');
  assert.deepEqual([...send.children.map((c: { label: string }) => c.label.split(' — ')[0].split(' /')[0])], ['Database', 'API client', 'Terminal', 'Vault']);
});
