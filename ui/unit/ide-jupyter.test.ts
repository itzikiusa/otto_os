// Jupyter Notebook (.ipynb) & ML workflow unit tests.
import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  addNotebookCell,
  applyNotebookExecution,
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
