import { test } from 'node:test';
import assert from 'node:assert/strict';
import { canFormat, formatContent, jsonErrorPosition, validateContent } from '../src/modules/workbench/lib/format.ts';
import { parseCsv, stringifyCsv } from '../src/modules/workbench/lib/csv.ts';
import { formatMarkup } from '../src/modules/workbench/lib/xml.ts';

async function fmt(lang: string, text: string, opts?: { indent?: number; sqlDialect?: string }): Promise<string> {
  const r = await formatContent(lang, text, opts);
  if (!r.ok) throw new Error(`format failed: ${r.error}`);
  return r.text;
}

async function idempotent(lang: string, text: string): Promise<string> {
  const once = await fmt(lang, text);
  const twice = await formatContent(lang, once);
  assert.ok(twice.ok);
  assert.equal(twice.text, once, `${lang} formatting is not idempotent`);
  assert.equal(twice.changed, false);
  return once;
}

test('json: pretty prints and keeps the trailing-newline policy', async () => {
  assert.equal(await fmt('json', '{"a":1,"b":[1,2]}'), '{\n  "a": 1,\n  "b": [\n    1,\n    2\n  ]\n}');
  assert.equal(await fmt('json', '{"a":1}\n'), '{\n  "a": 1\n}\n');
  assert.equal(await fmt('json', '[1]', { indent: 4 }), '[\n    1\n]');
  await idempotent('json', '{"x":{"y":"{{brand}}"}}\n');
});

test('json: an error reports a line and column and changes nothing', async () => {
  const r = await formatContent('json', '{\n  "a": 1,\n  "b": }\n');
  assert.equal(r.ok, false);
  if (!r.ok) {
    assert.equal(r.line, 3);
    assert.ok((r.col ?? 0) >= 7, `col ${r.col}`);
  }
  const v = await validateContent('json', '{"a":');
  assert.ok(v && v.line === 1);
  assert.equal(await validateContent('json', '{"a":1}'), null);
});

test('json: error position parsing across engine message formats', () => {
  const text = 'ab\ncdef';
  assert.deepEqual(jsonErrorPosition('Unexpected token } in JSON at position 5', text), { line: 2, col: 3 });
  assert.deepEqual(jsonErrorPosition("Expected ',' at position 9 (line 4 column 2)", text), { line: 4, col: 2 });
  assert.deepEqual(jsonErrorPosition('JSON.parse: expected property name at line 1 column 3 of the JSON data', text), { line: 1, col: 3 });
});

test('yaml: normalises and reports parse errors with a line', async () => {
  const out = await idempotent('yaml', 'a:   1\nb:\n    - x\n    - y\n');
  assert.equal(out, 'a: 1\nb:\n  - x\n  - y\n');
  const r = await formatContent('yaml', 'a: [1, 2\nb: 3\n');
  assert.equal(r.ok, false);
  if (!r.ok) assert.ok(typeof r.line === 'number');
  assert.equal(await validateContent('yaml', 'a: 1\n'), null);
});

test('sql: formats and keeps :name, {name} and {{name}} placeholders intact', async () => {
  const src = 'select id, name from brands where id = :brand_id and region = {region} and env = {{env}}';
  const out = await idempotent('sql', src);
  assert.match(out, /^SELECT/);
  assert.ok(out.includes(':brand_id'));
  assert.ok(out.includes('{region}'));
  assert.ok(out.includes('{{env}}'));
  assert.ok(!/ottoph/i.test(out));
  const my = await fmt('sql', 'select 1', { sqlDialect: 'postgres' });
  assert.match(my, /SELECT\s+1/);
});

test('html: indents block elements, keeps <pre>/<script> verbatim, idempotent', async () => {
  const src = '<!doctype html><html><head><title>T</title><script>if (a<b) {  x()  }</script></head><body><div><p>Hello <b>world</b>!</p><pre>  keep\n    this  </pre><br><img src="a.png"></div></body></html>';
  const out = await idempotent('html', src);
  assert.ok(out.includes('<p>Hello <b>world</b>!</p>'));
  assert.ok(out.includes('<pre>  keep\n    this  </pre>'));
  assert.ok(out.includes('<script>if (a<b) {  x()  }</script>'));
  assert.match(out, /\n {4}<div>\n/);
  assert.ok(out.startsWith('<!doctype html>\n<html>'));
});

test('xml: pretty prints, keeps attributes, CDATA and comments, flags mismatches', async () => {
  const src = '<?xml version="1.0"?><root a="1 > 0"><!-- c --><item id="x">v</item><data><![CDATA[ <raw> ]]></data><empty/></root>';
  const out = await idempotent('xml', src);
  assert.equal(
    out,
    '<?xml version="1.0"?>\n<root a="1 > 0">\n  <!-- c -->\n  <item id="x">v</item>\n  <data><![CDATA[ <raw> ]]></data>\n  <empty/>\n</root>',
  );
  const bad = await formatContent('xml', '<a>\n  <b>\n</a>');
  assert.equal(bad.ok, false);
  if (!bad.ok) assert.equal(bad.line, 3);
  const unclosed = await validateContent('xml', '<a><b></b>');
  assert.ok(unclosed && /unclosed/.test(unclosed.error));
  // SVG goes through the strict XML path.
  await idempotent('svg', '<svg xmlns="http://www.w3.org/2000/svg"><g><rect width="1" height="1"/></g></svg>');
});

test('html: tolerant of stray end tags and implicit closes', () => {
  const out = formatMarkup('<ul><li>a<li>b</ul></span>', 'html');
  assert.ok(out.includes('</span>'));
  assert.ok(out.includes('a'));
  assert.ok(out.includes('b'));
});

test('csv: round-trips quotes, delimiters and embedded newlines', async () => {
  const src = 'id,name,note\n1,"Smith, J","said ""hi"""\n2,Ann,"multi\nline"\n';
  const rows = parseCsv(src);
  assert.deepEqual(rows, [
    ['id', 'name', 'note'],
    ['1', 'Smith, J', 'said "hi"'],
    ['2', 'Ann', 'multi\nline'],
  ]);
  assert.deepEqual(parseCsv(stringifyCsv(rows)), rows);
  const out = await idempotent('csv', '"id","name"\r\n"1","x"\r\n');
  assert.equal(out, 'id,name\n1,x\n');
  assert.deepEqual(parseCsv('a\tb\n1\t2'), [['a', 'b'], ['1', '2']]);
  await idempotent('tsv', 'a\tb\n1\t"x\ty"\n');
  const bad = await formatContent('csv', 'a,b\n1,"open\n');
  assert.equal(bad.ok, false);
  const ragged = await validateContent('csv', 'a,b\n1\n');
  assert.ok(ragged && ragged.line === 2);
});

test('markdown/toml/code: conservative whitespace normalisation, idempotent', async () => {
  const md = await idempotent('md', '# T\t \n\n\n\nline with break  \nnext   \n```\ncode   \n\n\n\nmore\n```\n');
  assert.equal(md, '# T\n\nline with break  \nnext\n```\ncode\n\n\n\nmore\n```\n');
  const toml = await idempotent('toml', 'a = 1   \n[server]\nport = 80\n[db]\nhost = "x"\n');
  assert.equal(toml, 'a = 1\n\n[server]\nport = 80\n\n[db]\nhost = "x"\n');
  const js = await idempotent('js', '\tconst a = 1;   \n\tif (a) {\n\t\tb();\n\t}\n');
  assert.equal(js, '  const a = 1;\n  if (a) {\n    b();\n  }\n');
  await idempotent('mermaid', 'graph TD\n\tA --> B   \n');
  await idempotent('d2', 'a -> b\nb: {\n\tshape: circle\n}\n');
  const toml_bad = await formatContent('toml', 'a = 1\nthis is wrong\n');
  assert.equal(toml_bad.ok, false);
});

test('canFormat covers the text languages but not images', () => {
  for (const l of ['json', 'yaml', 'sql', 'html', 'xml', 'svg', 'csv', 'tsv', 'md', 'toml', 'js', 'ts', 'css', 'mermaid', 'd2']) {
    assert.ok(canFormat(l), l);
  }
  assert.equal(canFormat('image'), false);
});
