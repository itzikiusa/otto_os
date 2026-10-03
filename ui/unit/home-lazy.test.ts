// Lazy edges inside page chunks (perf 02 F4–F6): Home loads each box kind on
// demand, the results grid loads its modal-only children on first open, the
// code editor loads language packs / the LSP client on demand, and the service
// worker only serves hashed /assets/* cache-first. Checked as source text.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const UI = join(import.meta.dirname, '..');
const read = (p: string) => readFileSync(join(UI, p), 'utf8');

/** Static (value) import specifiers of a source file — `import type` excluded. */
function staticImports(src: string): string[] {
  return [...src.matchAll(/^\s*import\s+(?!type\s)[^;'"]*?from\s+'([^']+)'/gm)].map((m) => m[1]);
}

test('HomeBox loads every box kind lazily', () => {
  const src = read('src/modules/home/HomeBox.svelte');
  const boxes = staticImports(src).filter((s) => /\/boxes\/\w+Box\.svelte$/.test(s));
  assert.deepEqual(boxes, [], 'map the kind to () => import(...) in BODY_LOADERS');
  for (const k of ['SessionsBox', 'MissionControlBox', 'DbDashboardBox', 'K8sBox', 'InsightsBox', 'UsageBox']) {
    assert.match(src, new RegExp(`import\\('\\./boxes/${k}\\.svelte'\\)`), k);
  }
});

test('ResultsGrid loads its modal-only children on open', () => {
  const imports = staticImports(read('src/modules/database/ResultsGrid.svelte'));
  for (const name of ['DocEditor', 'RawDocViewer', 'ReviewModal', 'ExportDialog', 'AggregateBuilder', 'RecordDiff', 'ContextPacketDialog']) {
    assert.ok(!imports.some((s) => s.endsWith(`/${name}.svelte`)), `${name} must be {#await import()}-ed`);
  }
});

test('CodeEditor keeps language packs and the LSP client out of its static graph', () => {
  const editor = staticImports(read('src/lib/components/CodeEditor.svelte'));
  assert.deepEqual(editor.filter((s) => s.startsWith('@codemirror/lang-')), []);
  assert.ok(!editor.includes('@marimo-team/codemirror-languageserver'), 'import the LSP client in attachLsp');
  // cm-langs.ts: only SQL stays static (the DB query editor must never flash).
  const langs = staticImports(read('src/lib/components/cm-langs.ts')).filter((s) => s.startsWith('@codemirror/lang-'));
  assert.deepEqual(langs, ['@codemirror/lang-sql']);
});

test('service worker serves only hashed /assets/* cache-first and prunes old builds', () => {
  const sw = read('public/sw.js');
  const cacheFirst = sw.indexOf('caches.match(event.request).then(\n');
  const gate = sw.indexOf("if (url.pathname.startsWith('/assets/'))");
  assert.ok(gate > 0 && cacheFirst > gate, 'the cache-first branch must sit behind the /assets/ gate');
  assert.equal(sw.match(/caches\.match\(event\.request\)\.then\(\n/g)?.length, 1, 'one cache-first branch only');
  assert.match(sw, /async function pruneAssets\(html\)/);
  const devGate = sw.indexOf('@vite|@fs|@id|node_modules');
  assert.ok(devGate > 0 && devGate < sw.indexOf('const isNav'), 'dev-server modules pass through before any respondWith');
  assert.doesNotMatch(sw, /CACHE_NAME = 'otto-shell-v3'/, 'bump CACHE_NAME on a policy change');
});
