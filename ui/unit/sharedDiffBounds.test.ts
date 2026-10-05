import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

const component = readFileSync(new URL('../src/lib/components/DiffView.svelte', import.meta.url), 'utf8');
const script = component.split('<script lang="ts">')[1].split('</script>')[0];
const file = ts.createSourceFile('DiffView.ts', script, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
const declarations = file.statements.filter(ts.isVariableStatement).flatMap(n => [...n.declarationList.declarations]);
const names = declarations.filter(n => ts.isIdentifier(n.name)).map(n => n.name.getText(file));
const functionNames = file.statements.filter(ts.isFunctionDeclaration).map(n => n.name!.text);
const renderedExpressions = [...component.matchAll(/\{#each ([A-Za-z_$][\w$]*) as row,/g)].map(match => match[1]);

function diff(before: string, after: string, contextLines: number | undefined = 4) {
  const allocations: number[] = [];
  class MeasuredInt32Array extends Int32Array {
    constructor(length: number) { allocations.push(length); super(length); }
  }
  const context: Record<string, any> = {
    $props: () => ({ before, after, mode: 'split', contextLines }),
    $state: (value: unknown) => value,
    $derived: Object.assign((value: unknown) => value, { by: (fn: () => unknown) => fn() }),
    $effect: () => {}, Int32Array: MeasuredInt32Array,
  };
  const renderExpressions = renderedExpressions.map(name => {
    const declaration = declarations.find(n => n.name.getText(file) === name);
    return declaration?.initializer?.getText(file) ?? name;
  });
  const downloads: Blob[] = [];
  Object.assign(context, { Blob, URL: { createObjectURL: (blob: Blob) => { downloads.push(blob); return 'blob:fixture'; }, revokeObjectURL() {} },
    document: { createElement: () => ({ href: '', download: '', click() {} }) }, setTimeout: (fn: () => void) => fn() });
  const code = `${script}\nglobalThis.result = { ${names.join(',')}, ${functionNames.join(',')}, before, after,
    renderAgain: () => [${renderExpressions.join(',')}], currentPage: () => pageIndex };`;
  runInNewContext(ts.transpileModule(code, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, context, { timeout: 5000 });
  return { ...context.result, allocations, downloads, rendered: renderedExpressions.map(name => context.result[name]) };
}

test('pagination reaches the final omitted change and downloads retain complete source bytes', async () => {
  const before = lines(10_000).join('\n');
  const after = lines(10_000).map(line => `changed ${line}`).join('\n');
  const result = diff(before, after);
  const visited = new Set<number>();
  for (let page = 0; page < result.pageCount; page++) {
    const rendered = result.renderAgain()[0];
    assert.ok(rendered.length <= 1000);
    for (const row of rendered) {
      if (row.right) visited.add(row.right.no);
    }
    result.changePage(1);
  }
  assert.equal(visited.size, 10_000, 'every changed source line can be reached through the real page handler');
  assert.ok(visited.has(10_000));
  assert.equal(result.currentPage(), result.pageCount - 1);
  result.downloadSource('before'); result.downloadSource('after');
  assert.equal(await result.downloads[0].text(), before);
  assert.equal(await result.downloads[1].text(), after);
});

const lines = (n: number) => Array.from({ length: n }, (_, i) => `  "key_${i}": "value_${i}",`);

test('identical 50k-line JSON produces no false changed lines or full-file render', () => {
  const body = lines(50_000).join('\n');
  const result = diff(body, body);
  const ops = result.diffLines(body, body);
  assert.equal(ops.filter((op: { tag: string }) => op.tag !== 'eq').length, 0);
  assert.ok(result.rendered.every((rows: unknown[]) => rows.length <= 1000), 'actual template row inputs must remain bounded');
  assert.equal(result.before, body); assert.equal(result.after, body);
});

test('distant sparse changes retain equal interior lines within a fixed allocation budget', () => {
  const before = lines(50_000), after = [...before];
  after[10] = '  "first_change": true,';
  after[49_980] = '  "last_change": true,';
  const result = diff(before.join('\n'), after.join('\n'));
  const ops = result.diffLines(before.join('\n'), after.join('\n'));
  assert.equal(ops.filter((op: { tag: string }) => op.tag === 'del').length, 2);
  assert.equal(ops.filter((op: { tag: string }) => op.tag === 'add').length, 2);
  assert.equal(ops.filter((op: { tag: string }) => op.tag !== 'add').map((op: { line: string }) => op.line).join('\n'), before.join('\n'));
  assert.equal(ops.filter((op: { tag: string }) => op.tag !== 'del').map((op: { line: string }) => op.line).join('\n'), after.join('\n'));
  assert.ok(result.allocations.reduce((sum: number, n: number) => sum + n, 0) <= 4_100_000,
    'instrumented production DP allocations must not grow with the full line-product');
  assert.ok(result.rendered.every((rows: unknown[]) => rows.length <= 1000));
});

test('distant substitutions inside 50k repeated lines preserve the unchanged common run', () => {
  const before = Array.from({ length: 50_000 }, () => 'repeated line');
  const after = [...before]; after[10] = 'first substitution'; after[49_980] = 'last substitution';
  const result = diff(before.join('\n'), after.join('\n'));
  const ops = result.diffLines(before.join('\n'), after.join('\n'));
  assert.equal(ops.filter((op: { tag: string }) => op.tag === 'del').length, 2);
  assert.equal(ops.filter((op: { tag: string }) => op.tag === 'add').length, 2);
  assert.ok(result.rendered.every((rows: unknown[]) => rows.length <= 1000));
});

for (const change of ['insertions', 'deletions', 'reordering']) {
  test(`bounded repeated-line traceback reconstructs both sources after ${change}`, () => {
    const before = Array.from({ length: 12_000 }, (_, i) => `repeated ${i % 3}`);
    const after = [...before];
    if (change === 'insertions') {
      after.splice(10, 0, 'inserted first'); after.splice(11_980, 0, 'inserted last');
    } else if (change === 'deletions') {
      after.splice(11_980, 1); after.splice(10, 1);
    } else {
      const moved = after.splice(10, 2); after.splice(11_980, 0, ...moved);
    }
    const result = diff(before.join('\n'), after.join('\n'));
    const ops = result.diffLines(before.join('\n'), after.join('\n'));
    assert.equal(ops.filter((op: { tag: string }) => op.tag !== 'add').map((op: { line: string }) => op.line).join('\n'), before.join('\n'));
    assert.equal(ops.filter((op: { tag: string }) => op.tag !== 'del').map((op: { line: string }) => op.line).join('\n'), after.join('\n'));
    assert.ok(ops.filter((op: { tag: string }) => op.tag !== 'eq').length <= 8, 'a few edits must not become a whole-file replacement');
    assert.ok(result.allocations.reduce((sum: number, n: number) => sum + n, 0) <= 8_000_000,
      'two production computations each stay within the4M cell budget');
  });
}

for (const shape of ['disjoint', 'empty-before', 'unlimited-context']) {
  test(`actual template render inputs are bounded for ${shape} large sources`, () => {
    const before = shape === 'empty-before' ? '' : lines(10_000).join('\n');
    const after = shape === 'unlimited-context' ? before : lines(10_000).map(line => `changed ${line}`).join('\n');
    const result = diff(before, after, shape === 'unlimited-context' ? -1 : 4);
    assert.ok(result.rendered.length > 0, 'inspect real template each inputs, not a copied render model');
    assert.ok(result.rendered.every((rows: unknown[]) => rows.length <= 1000),
      `rendered row inputs: ${result.rendered.map((rows: unknown[]) => rows.length)}`);
    assert.equal(result.before, before, 'full original source remains accessible');
    assert.equal(result.after, after, 'full resulting source remains accessible');
  });
}
