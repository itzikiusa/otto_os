import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { loadSource } from './sourceHarness.ts';

// The harness's opt-in relative-import resolution, which a store split into
// per-concern files (lib/stores/database/*) needs: the façade's parts load for
// real and share one instance; everything else stays an explicit stub.

test('without `relative`, an unmapped import still throws (the default stays strict)', () => {
  assert.throws(
    () => loadSource(new URL('../src/lib/jsonTree/json-find.ts', import.meta.url), {}),
    /Missing fixture import: \.\/bson/,
  );
});

test('`relative` loads a real sibling, and a src-relative key stubs it from any importer', () => {
  const real = loadSource(new URL('../src/lib/jsonTree/json-find.ts', import.meta.url), {}, {}, { relative: true });
  assert.equal(typeof real.revealSteps, 'function');
  let stubbed = 0;
  const plan = loadSource(new URL('../src/lib/jsonTree/expansion-plan.ts', import.meta.url), {
    'lib/jsonTree/bson': { bsonScalar: () => { stubbed++; return 'STUB'; } },
  }, {}, { relative: true });
  // The stub calls every object a BSON scalar, so even the root is one leaf
  // (the real ./bson would flatten it to `a.b`).
  assert.deepEqual([...plan.flattenPaths({ a: { b: 1 } }).keys()], ['']);
  assert.ok(stubbed > 0, 'the src-keyed stub replaced the real ./bson');
});

test('`relative` shares one instance per file and survives an import cycle', () => {
  const dir = mkdtempSync(join(tmpdir(), 'otto-harness-'));
  writeFileSync(join(dir, 'state.svelte.ts'), 'export const box = { n: 0 };\n');
  writeFileSync(join(dir, 'a.ts'), "import { box } from './state.svelte';\nimport { fromB } from './b';\nexport function bump() { box.n++; return fromB(); }\nexport const name = 'a';\n");
  writeFileSync(join(dir, 'b.ts'), "import { box } from './state.svelte';\nimport * as a from './a';\nexport function fromB() { return `${a.name}:${box.n}`; }\n");
  const a = loadSource(pathToFileURL(join(dir, 'a.ts')), {}, {}, { relative: true });
  assert.equal(a.bump(), 'a:1');
  assert.equal(a.bump(), 'a:2');
});
