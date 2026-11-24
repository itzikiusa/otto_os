import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { IdeSymbolNode } from '../src/lib/api/types';
import { confSymbols, propertiesSymbols, tomlSymbols, yamlProblems, yamlSymbols } from '../src/lib/components/configModel.ts';

// The Structure tab for config files: keys (dotted .properties keys grouped), YAML's top two
// levels (by document), TOML tables and INI sections, read from the buffer.

const outline = (nodes: IdeSymbolNode[]): unknown => nodes.map((n) => (n.children ? [n.kind + ' ' + n.name + ':' + n.line, outline(n.children)] : n.kind + ' ' + n.name + ':' + n.line));

test('.properties: keys in order, dotted keys grouped by their first segment, continuations and comments skipped', () => {
  const text = [
    '# comment',
    'spring.datasource.url=jdbc:x',
    'spring.datasource.username=games',
    'server.port=8080',
    'app.description=one \\',
    '  two=not a key',
    '! another comment',
    'spring.kafka.topic=bets',
    'name\\ with\\ space = v',
  ].join('\n');
  assert.deepEqual(outline(propertiesSymbols(text)), [
    ['section spring:2', ['key spring.datasource.url:2', 'key spring.datasource.username:3', 'key spring.kafka.topic:8']],
    'key server.port:4',
    'key app.description:5',
    'key name with space:9',
  ]);
  const spring = propertiesSymbols(text)[0];
  assert.equal(spring.end_line, 8);
});

test('YAML: two levels, block scalars skipped, list items not keys', () => {
  const text = [
    'apiVersion: apps/v1',
    'kind: Deployment',
    'metadata:',
    '  name: games',
    '  labels:',
    '    app: games',
    'spec:',
    '  template:',
    '    spec:',
    '      containers:',
    '        - name: app',
    '  script: |',
    '    key: inside a block',
    '  replicas: 3',
  ].join('\n');
  assert.deepEqual(outline(yamlSymbols(text)), [
    'key apiVersion:1',
    'key kind:2',
    ['key metadata:3', ['key name:4', 'key labels:5']],
    ['key spec:7', ['key template:8', 'key script:12', 'key replicas:14']],
  ]);
});

test('YAML: a multi-document file is listed by document', () => {
  const text = 'a: 1\nb:\n  c: 2\n---\nd: 3\n---\n# empty-ish\ne: 4\n';
  assert.deepEqual(outline(yamlSymbols(text)), [
    ['document Document 1:1', ['key a:1', ['key b:2', ['key c:3']]]],
    ['document Document 2:4', ['key d:5']],
    ['document Document 3:6', ['key e:8']],
  ]);
  // A leading --- alone is one document.
  assert.deepEqual(outline(yamlSymbols('---\nx: 1\ny: 2\n')), ['key x:2', 'key y:3']);
});

test('TOML: tables with their keys, top-level keys first, multi-line strings skipped', () => {
  const text = 'title = "x"\n[package]\nname = "otto"\ndesc = """\nfake = 1\n"""\n[dependencies]\nserde = "1"\nsite."a.b" = 2\n[[bin]]\nname = "ottod"\n';
  assert.deepEqual(outline(tomlSymbols(text)), [
    'key title:1',
    ['section package:2', ['key name:3', 'key desc:4']],
    ['section dependencies:7', ['key serde:8', 'key site."a.b":9']],
    ['section bin:10', ['key name:11']],
  ]);
});

test('INI: sections and keys', () => {
  assert.deepEqual(outline(confSymbols('; c\nroot = true\n[*]\nindent_style = space\n[*.md]\ntrim_trailing_whitespace = false\n')), [
    'key root:2',
    ['section *:3', ['key indent_style:4']],
    ['section *.md:5', ['key trim_trailing_whitespace:6']],
  ]);
});

test('YAML parse errors become Problems entries at their line', async () => {
  assert.deepEqual(await yamlProblems('/x/a.yml', 'a: 1\nb: 2\n'), []);
  const errs = await yamlProblems('/x/a.yml', 'a: 1\nb: [1, 2\nc: 3\n');
  assert.ok(errs.length >= 1, JSON.stringify(errs));
  assert.equal(errs[0].path, '/x/a.yml');
  assert.equal(errs[0].severity, 'error');
  assert.equal(errs[0].source, 'yaml');
  assert.ok(errs[0].line >= 2, JSON.stringify(errs[0]));
});
