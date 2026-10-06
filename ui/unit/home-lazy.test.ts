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
  for (const k of ['SessionsBox', 'MissionControlBox', 'DbDashboardBox', 'K8sBox', 'InsightsBox', 'UsageBox', 'ClassroomsBox']) {
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

test('service worker never caches the proxy, plugin UIs or credentialed URLs', () => {
  const sw = read('public/sw.js');
  const firstRespond = sw.indexOf('event.respondWith(');
  for (const gate of ['(browser|plugins)', '(token|ticket|access_token)=', 'url.origin !== self.location.origin']) {
    const at = sw.indexOf(gate);
    assert.ok(at > 0 && at < firstRespond, `${gate} must bail out before any respondWith`);
  }
  // Navigations store only the shell, under '/', never the per-URL response.
  assert.match(sw, /c\.put\('\/', clone\)/);
  assert.doesNotMatch(sw, /c\.put\(event\.request, clone\)\);\n\s*const ct = resp\.headers/);
  assert.doesNotMatch(sw, /CACHE_NAME = 'otto-shell-v4'/, 'bump CACHE_NAME on a policy change');
});

test('Classrooms keeps three.js behind a dynamic import', () => {
  for (const f of ['src/modules/home/boxes/ClassroomsBox.svelte', 'src/modules/home/classrooms/scene.ts', 'src/modules/home/classrooms/model.ts']) {
    const imports = staticImports(read(f));
    assert.ok(!imports.some((s) => s === 'three' || s.startsWith('three/')), `${f} must not statically import three`);
  }
  assert.match(read('src/modules/home/classrooms/scene.ts'), /import\('three'\)/);
  assert.match(read('scripts/bundle-budget.mjs'), /three: \['home'/);
});

test('host pages load the Needs-you link lazily (it pulls the whole Today store)', () => {
  // NeedsYouLink → today.svelte.ts joins assistant, MCP, scheduled-tasks,
  // design and mission-control sources; a static import put all of that into
  // each host page's chunk (Mission Control +17 % gzip vs main).
  for (const host of [
    'src/modules/mission-control/MissionControlPage.svelte',
    'src/modules/mcp/ApprovalsTab.svelte',
    'src/modules/assistant/NeedsYouRail.svelte',
    'src/modules/agents/WorkQueue.svelte',
  ]) {
    const imports = staticImports(read(host));
    assert.ok(!imports.some((s) => s.endsWith('/home/NeedsYouLink.svelte')), `${host}: use LazyNeedsYouLink`);
    assert.ok(imports.some((s) => s.endsWith('/home/LazyNeedsYouLink.svelte')), host);
  }
  const lazy = read('src/modules/home/LazyNeedsYouLink.svelte');
  assert.deepEqual(staticImports(lazy), []);
  assert.match(lazy, /import\('\.\/NeedsYouLink\.svelte'\)/);
});
