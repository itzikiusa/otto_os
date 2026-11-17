import assert from 'node:assert/strict';
import { test } from 'node:test';
import { CodeVisionWidget } from '../src/lib/components/cm-ide-semantic-widgets.ts';

// P10: the code-vision count is a same-file word count. Labelled "N usages",
// "0 usages" on code used from other files read as dead code.
test('code vision labels its count as in-file', () => {
  const g = globalThis as Record<string, unknown>;
  g.HTMLButtonElement = class {};
  g.document = { createElement: () => ({ dataset: {}, addEventListener() {} }) };
  try {
    const el = new CodeVisionWidget(0, 3, 'transfer').toDOM() as unknown as { textContent: string; title: string };
    assert.equal(el.textContent, '0 in file');
    assert.match(el.title, /in this file/);
  } finally {
    delete g.HTMLButtonElement;
    delete g.document;
  }
});
