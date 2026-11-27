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
  normalizeNotebookOutputs,
  parseIpynb,
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
});

test('CONC-4: applyNotebookExecutionById tracks moved cells and discards deleted cells', () => {
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
  assert.equal(res.doc.cells[0].id, 'c-second');
  assert.equal(res.doc.cells[0].outputs.length, 0);
  assert.equal(res.doc.cells[1].id, 'c-first');
  assert.equal(res.doc.cells[1].execution_count, 5);

  // Deleted cell discards cleanly without touching remaining cells.
  const deleted = deleteNotebookCell(moved, 1);
  const dropped = applyNotebookExecutionById(deleted, 'c-first', 6, out);
  assert.equal(dropped.applied, false);
  assert.equal(dropped.doc.cells.length, 1);
  assert.equal(dropped.doc.cells[0].outputs.length, 0);
});

test('OBS-4: classifyDepsActionOutcome reports error when backend returns ok: false', () => {
  const mod = loadSource(
    new URL('../src/modules/ide/history/ideManifestActions.ts', import.meta.url),
    {
      '../../../lib/api/client': { api: {} },
      '../../../lib/toast.svelte': { toasts: {} },
      '../../../lib/toastError': { toastError: () => {} },
      '../git/ideGitController.svelte': { ideGit: {} },
      '../ideStore.svelte': { ide: {}, ideBasename: (p: string) => p.split('/').pop() ?? p },
    },
  );
  const failed = mod.classifyDepsActionOutcome(
    { ok: false, output: 'uv pip install exited with status 1\nmissing wheel' },
    'Sync Python Dependencies',
    'requirements.txt',
  );
  assert.equal(failed.level, 'error');
  assert.match(failed.title, /Couldn’t run Sync Python Dependencies/);
  assert.match(failed.detail, /exited with status 1/);

  const ok = mod.classifyDepsActionOutcome(
    { ok: true, output: 'Resolved 12 packages' },
    'Sync Python Dependencies',
    'requirements.txt',
  );
  assert.equal(ok.level, 'success');
  assert.equal(ok.title, 'Sync Python Dependencies');
});
