// Jupyter Notebook (.ipynb) & ML workflow unit tests.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

import {
  addNotebookCell,
  applyNotebookExecution,
  applyNotebookExecutionById,
  changeNotebookCellType,
  clearNotebookOutputs,
  deleteNotebookCell,
  moveNotebookCell,
  ansiSegments,
  applyNotebookStreamOutput,
  normalizeNotebookOutputs,
  parseIpynb,
  pickNotebookKernel,
  withKernelspecMetadata,
  serializeIpynb,
  updateNotebookCellSource,
} from '../src/modules/ide/editor/ideNotebookModel.ts';
import {
  defaultPreviewMode,
  filePreviewKind,
  isNotebookPreviewKind,
} from '../src/lib/components/filePreviewKind.ts';
import { langKeyFor } from '../src/lib/components/langForPath.ts';
import { ideLanguageLabel, ideLanguageSettingsSection } from '../src/modules/ide/editor/ideStatusModel.ts';

test('file preview, raw lang key, and status bar classify .ipynb as Jupyter Notebook', () => {
  assert.equal(filePreviewKind('/repo/notebooks/train.ipynb'), 'notebook');
  assert.equal(isNotebookPreviewKind('notebook'), true);
  assert.equal(isNotebookPreviewKind('csv'), false);
  assert.equal(defaultPreviewMode('notebook'), 'preview');
  assert.equal(langKeyFor('/repo/notebooks/train.ipynb'), 'json');
  assert.equal(ideLanguageLabel('/repo/train.ipynb', 'notebook'), 'Jupyter Notebook');
  assert.equal(ideLanguageSettingsSection('/repo/train.ipynb', 'notebook'), 'lang.python');
});

test('parseIpynb + serializeIpynb round-trips nbformat v4.5 cells, metadata, and outputs', () => {
  const sample = JSON.stringify({
    nbformat: 4,
    nbformat_minor: 5,
    metadata: {
      kernelspec: { name: 'python3', display_name: 'Python 3', language: 'python' },
      custom_key: 123,
    },
    cells: [
      {
        id: 'c1',
        cell_type: 'markdown',
        metadata: {},
        source: ['# Title\n', 'Hello world'],
      },
      {
        id: 'c2',
        cell_type: 'code',
        execution_count: 3,
        metadata: {},
        source: ['x = 42\n', 'print(x)\n'],
        outputs: [
          { output_type: 'stream', name: 'stdout', text: ['42\n'] },
        ],
      },
    ],
  });

  const { doc, parseError } = parseIpynb(sample);
  assert.equal(parseError, null);
  assert.equal(doc.cells.length, 2);
  assert.equal(doc.cells[0].cell_type, 'markdown');
  assert.equal(doc.cells[0].source, '# Title\nHello world');
  assert.equal(doc.cells[1].cell_type, 'code');
  assert.equal(doc.cells[1].execution_count, 3);
  assert.equal(doc.cells[1].source, 'x = 42\nprint(x)\n');

  const serialized = serializeIpynb(doc);
  const reparsed = JSON.parse(serialized);
  assert.equal(reparsed.nbformat, 4);
  assert.equal(reparsed.metadata.custom_key, 123);
  assert.deepEqual(reparsed.cells[1].source, ['x = 42\n', 'print(x)\n']);
  assert.equal(reparsed.cells[1].execution_count, 3);
});

test('cell CRUD, reorder, type switch, execution apply, and clear outputs', () => {
  let { doc } = parseIpynb('');
  assert.equal(doc.cells.length, 1);
  assert.equal(doc.cells[0].cell_type, 'code');

  doc = updateNotebookCellSource(doc, 0, 'a = 1\na + 2');
  doc = addNotebookCell(doc, 0, 'markdown');
  assert.equal(doc.cells.length, 2);
  assert.equal(doc.cells[1].cell_type, 'markdown');

  doc = moveNotebookCell(doc, 1, -1);
  assert.equal(doc.cells[0].cell_type, 'markdown');
  assert.equal(doc.cells[1].source, 'a = 1\na + 2');

  doc = applyNotebookExecution(doc, 1, 7, [
    { output_type: 'execute_result', execution_count: 7, data: { 'text/plain': '3' } },
  ]);
  assert.equal(doc.cells[1].execution_count, 7);
  assert.equal(doc.cells[1].outputs.length, 1);

  doc = clearNotebookOutputs(doc);
  assert.equal(doc.cells[1].execution_count, null);
  assert.equal(doc.cells[1].outputs.length, 0);

  doc = changeNotebookCellType(doc, 0, 'raw');
  assert.equal(doc.cells[0].cell_type, 'raw');

  doc = deleteNotebookCell(doc, 0);
  assert.equal(doc.cells.length, 1);
  assert.equal(doc.cells[0].cell_type, 'code');
});

test('normalizeNotebookOutputs handles stream, HTML tables, PNG/SVG images, and ANSI errors', () => {
  const outputs = normalizeNotebookOutputs([
    { output_type: 'stream', name: 'stdout', text: ['\x1b[32mok\x1b[0m\n'] },
    {
      output_type: 'display_data',
      data: { 'image/png': 'iVBORw0KGgo=', 'text/plain': '<Figure>' },
    },
    {
      output_type: 'execute_result',
      data: { 'text/html': ['<table><tr><td>1</td></tr></table>'], 'text/plain': 'df' },
    },
    {
      output_type: 'error',
      ename: 'ZeroDivisionError',
      evalue: 'division by zero',
      traceback: ['\x1b[31mTraceback\x1b[0m', 'ZeroDivisionError: division by zero'],
    },
  ]);

  assert.equal(outputs.length, 4);
  assert.equal(outputs[0].kind, 'stream');
  assert.equal(outputs[0].text, 'ok\n');
  assert.equal(outputs[1].kind, 'image');
  assert.equal(outputs[1].mime, 'image/png');
  assert.equal(outputs[1].dataUri, 'data:image/png;base64,iVBORw0KGgo=');
  assert.equal(outputs[2].kind, 'html');
  assert.equal(outputs[2].text, '<table><tr><td>1</td></tr></table>');
  assert.equal(outputs[3].kind, 'error');
  assert.equal(outputs[3].ename, 'ZeroDivisionError');
  assert.ok(!outputs[3].text.includes('\x1b['), 'ANSI escape sequences should be stripped from tracebacks');
});

test('DATA-4: Markdown cell attachments round-trip across parse, edit, and serialize', () => {
  const sample = JSON.stringify({
    nbformat: 4,
    nbformat_minor: 5,
    metadata: {},
    cells: [
      {
        id: 'md-img',
        cell_type: 'markdown',
        metadata: {},
        attachments: {
          'plot.png': { 'image/png': 'iVBORw0KGgoAAAANSUhEUg==' },
        },
        source: ['![diagram](attachment:plot.png)\n'],
      },
      {
        id: 'code-1',
        cell_type: 'code',
        metadata: {},
        execution_count: null,
        outputs: [],
        source: ['x = 1'],
      },
    ],
  });
  const { doc, parseError } = parseIpynb(sample);
  assert.equal(parseError, null);
  const edited = updateNotebookCellSource(doc, 1, 'x = 2');
  const reparsed = JSON.parse(serializeIpynb(edited));
  assert.deepEqual(reparsed.cells[0].attachments, {
    'plot.png': { 'image/png': 'iVBORw0KGgoAAAANSUhEUg==' },
  });

  // DATA2-2: Converting an attached Markdown cell to Code must NOT emit invalid top-level
  // `attachments` on a Code cell, and converting back to Markdown restores them losslessly.
  const asCode = changeNotebookCellType(edited, 0, 'code');
  const codeJson = JSON.parse(serializeIpynb(asCode));
  assert.equal(codeJson.cells[0].cell_type, 'code');
  assert.equal('attachments' in codeJson.cells[0], false, 'nbformat 4.5 forbids attachments on code cells');

  const backToMd = changeNotebookCellType(asCode, 0, 'markdown');
  const mdJson = JSON.parse(serializeIpynb(backToMd));
  assert.equal(mdJson.cells[0].cell_type, 'markdown');
  assert.deepEqual(mdJson.cells[0].attachments, {
    'plot.png': { 'image/png': 'iVBORw0KGgoAAAANSUhEUg==' },
  });
  assert.equal('otto_preserved_attachments' in (mdJson.cells[0].metadata ?? {}), false);
});

test('CONC-4: executeNotebookCellGuarded drops late results on tab switch or raw-source disposal', async () => {
  const { executeNotebookCellGuarded } = await import('../src/modules/ide/editor/ideNotebookRunner.ts');
  const { doc } = parseIpynb(JSON.stringify({
    nbformat: 4,
    nbformat_minor: 5,
    metadata: {},
    cells: [
      { id: 'c-first', cell_type: 'code', metadata: {}, execution_count: null, outputs: [], source: ['1'] },
      { id: 'c-second', cell_type: 'code', metadata: {}, execution_count: null, outputs: [], source: ['2'] },
    ],
  }));
  // Move c-first from index 0 to index 1 while execution is in flight.
  const moved = moveNotebookCell(doc, 0, 1);
  const out = [{ output_type: 'execute_result', execution_count: 5, data: { 'text/plain': '1' } }];
  const res = applyNotebookExecutionById(moved, 'c-first', 5, out);
  assert.equal(res.applied, true);
  assert.equal(res.doc.cells[1].id, 'c-first');
  assert.equal(res.doc.cells[1].execution_count, 5);

  const deleted = deleteNotebookCell(moved, 1);
  assert.equal(applyNotebookExecutionById(deleted, 'c-first', 6, out).applied, false);

  // Scenario 1: switching to another tab mid-run never calls commitDoc/onchange.
  let currentPath = '/ws/a.ipynb';
  let disposed = false;
  let viewToken = 1;
  let runningId: string | null = null;
  const commits: string[] = [];
  let resolveReq: (v: unknown) => void = () => {};
  const pTabSwitch = executeNotebookCellGuarded('c-first', {
    getDoc: () => doc,
    getPath: () => currentPath,
    isDisposed: () => disposed,
    getViewToken: () => viewToken,
    getRunningCellId: () => runningId,
    setRunningCellId: (id) => { runningId = id; },
    setSessionAndVars: () => {},
    commitDoc: (_next, targetPath) => { commits.push(targetPath); },
    onError: () => { commits.push('ERROR'); },
    executeRequest: () => new Promise<unknown>((r) => { resolveReq = r; }) as ReturnType<Parameters<typeof executeNotebookCellGuarded>[1]['executeRequest']>,
  });
  currentPath = '/ws/b.ipynb';
  viewToken += 1;
  resolveReq({ session_id: 's1', status: 'ok', execution_count: 1, outputs: out, variables: [] });
  assert.equal(await pTabSwitch, false);
  assert.equal(commits.length, 0, 'delayed completion after tab switch must not commit');

  // Scenario 2: switching the SAME notebook to raw JSON view (disposing IdeNotebookView) mid-run
  // must NOT write stale notebook JSON over the raw text buffer.
  currentPath = '/ws/a.ipynb';
  disposed = false;
  runningId = null;
  const rawCommits: string[] = [];
  const pRawSwitch = executeNotebookCellGuarded('c-first', {
    getDoc: () => doc,
    getPath: () => currentPath,
    isDisposed: () => disposed,
    getViewToken: () => viewToken,
    getRunningCellId: () => runningId,
    setRunningCellId: (id) => { runningId = id; },
    setSessionAndVars: () => {},
    commitDoc: (_next, targetPath) => { rawCommits.push(targetPath); },
    onError: () => { rawCommits.push('ERROR'); },
    executeRequest: () => new Promise<unknown>((r) => { resolveReq = r; }) as ReturnType<Parameters<typeof executeNotebookCellGuarded>[1]['executeRequest']>,
  });
  disposed = true;
  resolveReq({ session_id: 's1', status: 'ok', execution_count: 2, outputs: out, variables: [] });
  assert.equal(await pRawSwitch, false);
  assert.equal(rawCommits.length, 0, 'delayed completion after raw-source view switch must not commit');
});

test('OBS-4: runManifestDepsAction reports error toast when backend returns ok: false', async () => {
  const errCalls: [string, string][] = [];
  const okCalls: [string, string][] = [];
  let postResult: { ok: boolean; output?: string } = {
    ok: false,
    output: 'uv pip install exited with status 1\nmissing wheel',
  };
  const mod = loadSource(
    new URL('../src/modules/ide/history/ideManifestActions.ts', import.meta.url),
    {
      '../../../lib/api/client': {
        api: { post: async () => postResult },
      },
      '../../../lib/toast.svelte': {
        toasts: {
          error: (title: string, detail: string) => { errCalls.push([title, detail]); },
          success: (title: string, detail: string) => { okCalls.push([title, detail]); },
        },
      },
      '../../../lib/toastError': { toastError: () => {} },
      '../git/ideGitController.svelte': {
        ideGit: {
          rootContainingPath: () => ({ path: '/ws' }),
          reloadIdeTabIfOpen: async () => {},
          refreshActive: async () => {},
        },
      },
      '../ideStore.svelte': {
        ide: {
          target: { root: '/ws' },
          targetPayload: { kind: 'local', root: '/ws' },
          loadTree: async () => {},
        },
        ideBasename: (p: string) => p.split('/').pop() ?? p,
      },
    },
  );
  await mod.runManifestDepsAction('/ws/requirements.txt', 'sync', 'Sync Python Dependencies');
  assert.equal(errCalls.length, 1, 'real runManifestDepsAction must call toasts.error on ok: false');
  assert.equal(okCalls.length, 0, 'real runManifestDepsAction must NOT call toasts.success on ok: false');
  assert.match(errCalls[0][0], /Couldn’t run Sync Python Dependencies/);
  assert.match(errCalls[0][1], /exited with status 1/);

  postResult = { ok: true, output: 'Resolved 12 packages' };
  await mod.runManifestDepsAction('/ws/requirements.txt', 'sync', 'Sync Python Dependencies');
  assert.equal(okCalls.length, 1);
  assert.equal(okCalls[0][0], 'Sync Python Dependencies');
});

test('BUG3-12: idePreviewMode retains large-file manual render snapshot across Editor <-> Preview switches', () => {
  const mod = loadSource(
    new URL('../src/modules/ide/editor/idePreviewMode.svelte.ts', import.meta.url),
    {
      '../../../lib/components/filePreviewKind': {
        availablePreviewModes: (k: string | null) => (k ? ['source', 'split', 'preview'] : []),
        effectivePreviewMode: (k: string | null, saved?: string) => (k ? (saved ?? 'source') : 'source'),
        filePreviewKind: (p: string) => (p.endsWith('.md') ? 'markdown' : null),
        isNotebookPreviewKind: () => false,
        isTablePreviewKind: () => false,
        parsePreviewModes: () => ({}),
      },
      '../../../lib/components/cm-ide-manifest': { isIdeManifestPath: () => false },
      '../shell/ideLayout.svelte': { ideLayout: { stack: false } },
    },
  );
  const store = mod.idePreviewMode;
  const bigPath = '/ws/docs/huge-spec.md';
  const bigBody = '# Huge\n' + 'x'.repeat(1_000_050);

  store.set(bigPath, 'preview');
  assert.equal(store.layoutFor(bigPath), 'only');
  assert.equal(store.getRendered(bigPath), null);

  // User clicks "Render preview", switches back to Editor (unmounting lazy IdePreviewPane),
  // then returns to Preview without editing — snapshot must still be present.
  store.setRendered(bigPath, bigBody);
  store.set(bigPath, 'source');
  assert.equal(store.layoutFor(bigPath), 'none');
  store.set(bigPath, 'preview');
  assert.equal(store.layoutFor(bigPath), 'only');
  assert.equal(store.getRendered(bigPath), bigBody);

  // Evicting/closing the tab frees the cached large-file snapshot.
  store.forgetRendered([bigPath]);
  assert.equal(store.getRendered(bigPath), null);
});

test('ansiSegments maps SGR colours onto token classes and drops other escapes', () => {
  const segs = ansiSegments('\x1b[0;31mZeroDivisionError\x1b[0m: \x1b[1;32mdivision\x1b[39m by zero\x1b[2K!');
  assert.deepEqual(segs, [
    { text: 'ZeroDivisionError', color: 'danger' },
    { text: ': ' },
    { text: 'division', color: 'success', bold: true },
    { text: ' by zero!', bold: true },
  ]);
  // 256-colour codes of the base 16 map too; others stay plain.
  assert.deepEqual(ansiSegments('\x1b[38;5;12mx\x1b[38;5;200my'), [{ text: 'x', color: 'info' }, { text: 'y' }]);
  assert.deepEqual(ansiSegments('plain'), [{ text: 'plain' }]);
});

test('normalizeNotebookOutputs keeps ANSI tracebacks as coloured segments and notes widgets', () => {
  const [err] = normalizeNotebookOutputs([
    { output_type: 'error', ename: 'ValueError', evalue: 'bad', traceback: ['\x1b[31mValueError\x1b[0m: bad'] },
  ]);
  assert.equal(err.text, 'ValueError: bad');
  assert.deepEqual(err.segments, [{ text: 'ValueError', color: 'danger' }, { text: ': bad' }]);
  const [widget] = normalizeNotebookOutputs([
    { output_type: 'display_data', data: { 'application/vnd.jupyter.widget-view+json': { model_id: 'm' }, 'text/plain': 'IntSlider(value=3)' } },
  ]);
  assert.equal(widget.text, 'IntSlider(value=3)');
  assert.equal(widget.note, 'Widgets aren’t supported in Otto notebooks.');
  const [js] = normalizeNotebookOutputs([{ output_type: 'display_data', data: { 'application/javascript': 'alert(1)' } }]);
  assert.equal(js.text, '');
  assert.match(js.note ?? '', /JavaScript outputs don’t run/);
  const [htmlOverJs] = normalizeNotebookOutputs([
    { output_type: 'display_data', data: { 'application/javascript': 'alert(1)', 'text/html': '<b>x</b>' } },
  ]);
  assert.equal(htmlOverJs.kind, 'html', 'the richest non-JS MIME type wins');
});

test('applyNotebookStreamOutput sets outputs by index and clears on clear_output', () => {
  let outs = applyNotebookStreamOutput([], { kind: 'output', index: 0, output: { output_type: 'stream', name: 'stdout', text: '0\n' } });
  outs = applyNotebookStreamOutput(outs, { kind: 'output', index: 0, output: { output_type: 'stream', name: 'stdout', text: '0\n1\n' } });
  outs = applyNotebookStreamOutput(outs, { kind: 'output', index: 1, output: { output_type: 'display_data', data: { 'text/plain': 'x' } } });
  assert.equal(outs.length, 2);
  assert.equal(outs[0].text, '0\n1\n');
  assert.deepEqual(applyNotebookStreamOutput(outs, { kind: 'clear_output' }), []);
});

test('pickNotebookKernel follows metadata.kernelspec name, then language, then the active kernel', () => {
  const kernels = [
    { id: 'builtin:/p', kind: 'builtin', language: 'python', active: true },
    { id: 'jupyter:py', kind: 'jupyter', language: 'python', spec_name: 'python3', active: false },
    { id: 'jupyter:bash', kind: 'jupyter', language: 'bash', spec_name: 'bash', active: false },
  ];
  assert.equal(pickNotebookKernel(kernels, { kernelspec: { name: 'bash', language: 'bash' } }), 'jupyter:bash');
  assert.equal(pickNotebookKernel(kernels, { kernelspec: { name: 'gone', language: 'python' } }), 'jupyter:py');
  assert.equal(pickNotebookKernel(kernels, {}), 'builtin:/p');
  assert.equal(pickNotebookKernel([], {}), '');
});

test('withKernelspecMetadata writes the chosen Jupyter kernel and leaves built-in choices alone', () => {
  const { doc } = parseIpynb('');
  const next = withKernelspecMetadata(doc, { kind: 'jupyter', spec_name: 'bash', display_name: 'Bash', language: 'bash' });
  assert.deepEqual(next.metadata.kernelspec, { display_name: 'Bash', language: 'bash', name: 'bash' });
  assert.equal(withKernelspecMetadata(next, { kind: 'jupyter', spec_name: 'bash', display_name: 'Bash', language: 'bash' }), next);
  assert.equal(withKernelspecMetadata(doc, { kind: 'builtin', display_name: 'Py', language: 'python' }), doc);
});

test('executeNotebookCellGuarded tags each run with a run id and clears it when the run ends', async () => {
  const { executeNotebookCellGuarded } = await import('../src/modules/ide/editor/ideNotebookRunner.ts');
  const { doc } = parseIpynb(JSON.stringify({
    nbformat: 4, nbformat_minor: 5, metadata: {},
    cells: [{ id: 'c1', cell_type: 'code', metadata: {}, execution_count: null, outputs: [], source: ['1'] }],
  }));
  const runIds: (string | null)[] = [];
  let sent = '';
  let running: string | null = null;
  const hooks = {
    getDoc: () => doc,
    getPath: () => '/ws/a.ipynb',
    isDisposed: () => false,
    getViewToken: () => 1,
    getRunningCellId: () => running,
    setRunningCellId: (id: string | null) => { running = id; },
    setRunId: (id: string | null) => { runIds.push(id); },
    setSessionAndVars: () => {},
    commitDoc: () => {},
    onError: () => {},
    executeRequest: async (_p: string, _c: string, runId: string) => {
      sent = runId;
      return { session_id: 's', interpreter: 'py', status: 'dead', execution_count: 0, elapsed_ms: 1, outputs: [], variables: [] };
    },
  };
  assert.equal(await executeNotebookCellGuarded('c1', hooks), false, 'a dead kernel stops Run All');
  assert.match(sent, /^c1:\d+$/);
  assert.deepEqual(runIds, [sent, null]);
});
