import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  canPreview,
  lineColAt,
  parseJsonWithPos,
  previewKind,
  svgDataUrl,
} from '../src/modules/ide/filetools/preview/kinds.ts';

test('previewKind maps languages to renderers', () => {
  assert.equal(previewKind('md'), 'markdown');
  assert.equal(previewKind('markdown'), 'markdown');
  assert.equal(previewKind('html'), 'html');
  assert.equal(previewKind('svg'), 'svg');
  assert.equal(previewKind('mermaid'), 'mermaid');
  assert.equal(previewKind('d2'), 'd2');
  assert.equal(previewKind('JSON'), 'json');
  assert.equal(previewKind('csv'), null, 'CSV opens in the IDE table view');
  assert.equal(previewKind('mmd'), 'mermaid');
  assert.equal(previewKind('sql'), null);
  assert.equal(previewKind(''), null);
  assert.equal(canPreview('py'), false);
  assert.equal(canPreview('mermaid'), true);
});

test('lineColAt is 1-based and counts newlines', () => {
  assert.deepEqual(lineColAt('abc', 0), { line: 1, col: 1 });
  assert.deepEqual(lineColAt('ab\ncd', 4), { line: 2, col: 2 });
  assert.deepEqual(lineColAt('ab\n', 99), { line: 2, col: 1 });
});

test('parseJsonWithPos parses or reports a position', () => {
  const ok = parseJsonWithPos('{"a":[1,2]}');
  assert.equal(ok.ok, true);
  assert.deepEqual(ok.value, { a: [1, 2] });
  const bad = parseJsonWithPos('{\n  "a": 1,\n  oops\n}');
  assert.equal(bad.ok, false);
  assert.ok(bad.error);
  // V8 reports a position or line/column → resolved onto line 3.
  if (bad.line !== undefined) assert.equal(bad.line, 3);
  assert.equal(parseJsonWithPos('   ').ok, false);
});

test('svgDataUrl encodes markup', () => {
  const u = svgDataUrl('<svg xmlns="http://www.w3.org/2000/svg"/>');
  assert.ok(u.startsWith('data:image/svg+xml;charset=utf-8,'));
  assert.ok(!u.includes('<'));
});
