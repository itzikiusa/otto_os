// The shell's lazy page map (shell/pages.svelte.ts, r3-09-01): every sidebar
// module has a page chunk, and nothing re-couples the heavy code the split took
// out of the boot. The page map uses runes, so it is checked as source text.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { SIDEBAR_MODULES } from '../src/lib/sidebar.ts';
import { chainTo, staticClosure } from './staticClosure.ts';

const SRC = join(import.meta.dirname, '..', 'src');
const read = (p: string) => readFileSync(join(SRC, p), 'utf8');

/** Keys of the LOADERS map in shell/pages.svelte.ts. */
function loaderKeys(): Set<string> {
  const src = read('shell/pages.svelte.ts');
  const body = src.slice(src.indexOf('const LOADERS'), src.indexOf('};', src.indexOf('const LOADERS')));
  return new Set([...body.matchAll(/^\s*'?([a-z][a-z-]*)'?:\s*\(\)/gm)].map((m) => m[1]));
}

test('every sidebar module routes to its own page chunk', () => {
  const keys = loaderKeys();
  // Connections is the Database page (an alias in pageKeyOf).
  const missing = SIDEBAR_MODULES.map((m) => (m.id === 'connections' ? 'database' : m.id)).filter((id) => !keys.has(id));
  assert.deepEqual(missing, [], 'add a loader in shell/pages.svelte.ts — an unmapped module renders the Agents page');
  for (const id of ['settings', 'walkthroughs', 'plugin', 'snip', 'canvas']) assert.ok(keys.has(id), id);
});

test('the shell imports no module page statically', () => {
  const shell = read('shell/App.svelte');
  const statics = [...shell.matchAll(/^\s*import\s+[^;]*from\s+'\.\.\/modules\/[^']+\.svelte'/gm)].map((m) => m[0].trim());
  assert.deepEqual(statics, [], 'load pages through shell/pages.svelte.ts');
  assert.doesNotMatch(read('shell/App.svelte'), /import\s+RightPanel\s+from/);
});

test('mermaid, sql-formatter and qrcode are never imported statically', () => {
  const offenders: string[] = [];
  const walk = (dir: string): void => {
    for (const name of readdirSync(dir)) {
      const p = join(dir, name);
      if (statSync(p).isDirectory()) walk(p);
      else if (/\.(ts|svelte)$/.test(name)) {
        const src = readFileSync(p, 'utf8');
        if (/^\s*import\s+[^;'(]*from\s+'(mermaid|sql-formatter)'/m.test(src)) offenders.push(p.slice(SRC.length + 1));
      }
    }
  };
  walk(SRC);
  assert.deepEqual(offenders, [], 'import these heavy libraries with `await import(...)` (canvas/mermaid.ts for mermaid)');
  for (const f of ['shell/Navigator.svelte', 'shell/TabBar.svelte', 'modules/agents/SessionView.svelte']) {
    assert.doesNotMatch(read(f), /import\s+ShareModal\s+from/, `${f}: ShareModal bundles qrcode — load it on open`);
  }
});

test('the shell chunk evaluates no page-owned store, handler file or the ui-commands catalog (perf F2)', () => {
  const entry = join(SRC, 'shell/App.svelte');
  const closure = staticClosure(entry);
  const heavy = [
    'lib/stores/database.svelte.ts',
    'lib/stores/apiClient.svelte.ts',
    'lib/stores/k8s.svelte.ts',
    'lib/stores/aws.svelte.ts',
    'lib/stores/product.svelte.ts',
    'lib/stores/swarm.svelte.ts',
    'lib/stores/canvas.svelte.ts',
    'lib/uiCommands/database.ts',
    'lib/uiCommands/catalog.ts',
    'lib/components/CodeEditor.svelte',
    'modules/vault/vault.svelte.ts',
    'modules/home/home.svelte.ts',
  ];
  const reached = heavy
    .filter((f) => closure.has(join(SRC, f)))
    .map((f) => chainTo(entry, join(SRC, f), SRC).join(' → '));
  assert.deepEqual(reached, [], 'route through lazyModule() / registerLazyUiCommands() instead of a static import');
});

test('the entry chunk never carries the mock-API fixtures (perf F10)', () => {
  const entry = join(SRC, 'main.ts');
  const mock = join(SRC, 'lib/api/mock.ts');
  assert.ok(!staticClosure(entry).has(mock), chainTo(entry, mock, SRC).join(' → '));
});

test('the right activity panel loads each tab on demand (perf G1)', () => {
  const entry = join(SRC, 'shell/RightPanel.svelte');
  const closure = staticClosure(entry);
  const heavy = [
    'lib/components/CodeEditor.svelte',
    'lib/components/Terminal.svelte',
    'modules/git/GitPanel.svelte',
    'modules/api/ApiPanel.svelte',
    'modules/panels/FilesPanel.svelte',
    'modules/panels/BrowserPanel.svelte',
    'modules/panels/BrowserPanelV2.svelte',
    'modules/panels/CanvasPanel.svelte',
    'modules/panels/ActivityPanel.svelte',
    'modules/panels/OutputsPanel.svelte',
    'modules/panels/InfoPanel.svelte',
  ];
  const reached = heavy
    .filter((f) => closure.has(join(SRC, f)))
    .map((f) => chainTo(entry, join(SRC, f), SRC).join(' → '));
  assert.deepEqual(reached, [], 'load the tab through PANEL_LOADERS in RightPanel.svelte');

  // Every tab but Notes (inline) has a loader.
  const src = read('shell/RightPanel.svelte');
  // The tab list lives in lib/rightTabs.ts (shared with the ⌘K "Open … panel" commands).
  const tabsSrc = read('lib/rightTabs.ts');
  const tabs = [...tabsSrc.slice(tabsSrc.indexOf('RIGHT_TABS')).matchAll(/\{ id: '([a-z]+)'/g)].map((m) => m[1]);
  const loaders = src.slice(src.indexOf('PANEL_LOADERS'), src.indexOf('};', src.indexOf('PANEL_LOADERS')));
  const keys = new Set([...loaders.matchAll(/^\s*([a-zA-Z0-9]+):\s*\(\)/gm)].map((m) => m[1]));
  assert.ok(tabs.length >= 9, `parsed the tab list (${tabs.join()})`);
  const missing = tabs.filter((t) => t !== 'notes' && t !== 'browser' && !keys.has(t));
  assert.deepEqual(missing, [], 'add the tab to PANEL_LOADERS');
  for (const k of ['browserV1', 'browserV2']) assert.ok(keys.has(k), k);
});
