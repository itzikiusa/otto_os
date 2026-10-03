import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  cmExtFor,
  detectLanguage,
  effectiveLanguage,
  extensionFor,
  WB_LANGUAGES,
} from '../src/modules/workbench/lib/detect.ts';

test('the file extension wins over the content', () => {
  assert.equal(detectLanguage('data.json', 'SELECT 1'), 'json');
  assert.equal(detectLanguage('diagram.d2', ''), 'd2');
  assert.equal(detectLanguage('flow.mmd', ''), 'mermaid');
  assert.equal(detectLanguage('q.SQL', ''), 'sql');
  assert.equal(detectLanguage('notes.md', '{}'), 'md');
  assert.equal(detectLanguage('x.yml', ''), 'yaml');
  assert.equal(detectLanguage('shot.png', ''), 'image');
  assert.equal(detectLanguage('table.tsv', ''), 'tsv');
});

test('content sniffing per language', () => {
  const cases: [string, string][] = [
    ['{"a": [1, 2]}', 'json'],
    ['[{"id": 1}]', 'json'],
    ['<!DOCTYPE html><html><body>x</body></html>', 'html'],
    ['<div class="a">hi</div>', 'html'],
    ['<?xml version="1.0"?><root/>', 'xml'],
    ['<svg xmlns="http://www.w3.org/2000/svg"></svg>', 'svg'],
    ['graph TD\n  A --> B', 'mermaid'],
    ['%% title\nsequenceDiagram\n  A->>B: hi', 'mermaid'],
    ['erDiagram\n  A ||--o{ B : has', 'mermaid'],
    ['server -> db: query\ndb: {\n  shape: cylinder\n}', 'd2'],
    ['a -> b\nb -> c', 'd2'],
    ['select * from brands where id = :id', 'sql'],
    ['WITH x AS (SELECT 1) SELECT * FROM x', 'sql'],
    ["curl -X POST https://api.example.com -d '{}'", 'http'],
    ['GET https://example.com/v1/items', 'http'],
    ['# Title\n\nSome *text* and a [link](http://x).', 'md'],
    ['id,name,brand\n1,a,b\n2,c,d', 'csv'],
    ['id\tname\n1\ta', 'tsv'],
    ['name: otto\nversion: 1\nitems:\n  - a\n  - b', 'yaml'],
    ['[server]\nport = 8080\nhost = "x"', 'toml'],
    ['#!/usr/bin/env python3\nprint(1)', 'py'],
    ['#!/bin/bash\necho hi', 'sh'],
    ['diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b', 'diff'],
    ['just some words', 'txt'],
    ['', 'txt'],
  ];
  for (const [content, want] of cases) assert.equal(detectLanguage('Untitled', content), want, content);
});

test('effectiveLanguage: auto resolves, an explicit override sticks', () => {
  assert.equal(effectiveLanguage('auto', 'a.json', ''), 'json');
  assert.equal(effectiveLanguage('', 'Untitled', 'select 1'), 'sql');
  assert.equal(effectiveLanguage('md', 'a.json', '{}'), 'md');
  assert.equal(effectiveLanguage('bogus', 'a.sql', ''), 'sql');
});

test('every language maps to a CodeEditor key and an extension', () => {
  const cmKeys = new Set(['', 'sql', 'md', 'json', 'html', 'xml', 'css', 'js', 'ts', 'py', 'go', 'rs', 'java']);
  const ids = new Set<string>();
  for (const l of WB_LANGUAGES) {
    assert.ok(!ids.has(l.id), `duplicate ${l.id}`);
    ids.add(l.id);
    assert.ok(cmKeys.has(cmExtFor(l.id)), `${l.id} → ${cmExtFor(l.id)}`);
    assert.ok(extensionFor(l.id).length > 0);
  }
  for (const id of ['auto', 'txt', 'md', 'json', 'yaml', 'toml', 'csv', 'tsv', 'sql', 'html', 'xml', 'svg', 'css', 'js', 'ts', 'py', 'sh', 'go', 'rs', 'java', 'mermaid', 'd2', 'http', 'diff', 'image']) {
    assert.ok(ids.has(id), id);
  }
  assert.equal(extensionFor('mermaid'), 'mmd');
  assert.equal(cmExtFor('unknown'), '');
});

test('svg behind an xml prologue and comments is still svg, in linear time', () => {
  assert.equal(detectLanguage('untitled', '<?xml version="1.0"?>\n<!-- a -->\n<!-- b --> <svg width="1"></svg>'), 'svg');
  assert.equal(detectLanguage('untitled', '<!-- c --><div>x</div>'), 'html');
  // CodeQL js/redos: `<!--` + `--><!--`×n backtracked exponentially before.
  const started = performance.now();
  assert.equal(detectLanguage('untitled', '<!--' + '--><!--'.repeat(50_000)), 'xml');
  assert.ok(performance.now() - started < 500);
});
