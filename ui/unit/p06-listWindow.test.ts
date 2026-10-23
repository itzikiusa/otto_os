import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { loadSource } from './sourceHarness.ts';

// architecture-6-05: one windowing primitive. `TableWindow` is `ListWindow` in
// self-scrolling mode (shared range + pitch rule), and both register a ⌘F
// provider over EVERY row, so a windowed WIP list, file tree or diff nav finds
// rows that aren't mounted.

function load() {
  const providers: any[] = [];
  const imports = {
    svelte: { tick: async () => {}, untrack: (fn: () => unknown) => fn() },
    './findProviders': { registerFindProvider: (p: unknown) => { providers.push(p); return () => providers.splice(providers.indexOf(p), 1); } },
  };
  const globals = { requestAnimationFrame: (fn: () => void) => { fn(); return 1; }, cancelAnimationFrame() {} };
  const lib = loadSource(new URL('../src/lib/listWindow.svelte.ts', import.meta.url), imports, globals);
  const table = loadSource(new URL('../src/lib/tableWindow.svelte.ts', import.meta.url), { ...imports, './listWindow.svelte': lib }, globals);
  const out: Record<string, any> = { ...lib, TableWindow: table.TableWindow, providers };
  return out;
}

test('TableWindow is the shared ListWindow in self-scrolling mode', () => {
  const { ListWindow, TableWindow } = load();
  const tw = new TableWindow(400, 15);
  assert.ok(tw instanceof ListWindow);
  tw.rowH = 30;
  tw.viewH = 0; // before layout: a pane's worth, not one row
  const r = tw.range(10_000);
  assert.ok(r.end >= 600 / 30, `mounted ${r.end} rows`);
  tw.scrollTop = 3000;
  assert.equal(tw.rel, 3000, 'scrollTop is the window offset');
});

test('the pitch rule ignores tiny and sub-pixel rows', () => {
  const { nextPitch } = load();
  assert.equal(nextPitch(3, 26), null);
  assert.equal(nextPitch(26.2, 26), null);
  assert.equal(nextPitch(22, 26), 22);
});

test('findRows covers every row and sits out while the list is not windowed', async () => {
  const { ListWindow, providers } = load();
  const lw = new ListWindow('.row', { min: 300 });
  const rows = Array.from({ length: 5000 }, (_, i) => ({ name: `file-${i}.ts` }));
  const mounted = Array.from({ length: 60 }, (_, i) => ({ i }));
  const el = { querySelectorAll: () => mounted };
  const off = lw.findRows(() => el, () => rows, (r: { name: string }) => r.name);
  const p = providers[0];
  assert.equal(p.count(), 5000);
  assert.equal(p.text(4321), 'file-4321.ts');
  assert.equal(p.active(), true);
  assert.equal(p.root(), el);
  assert.equal(p.rowElement(4321), null, 'an unmounted row has no element');
  assert.equal(p.rowElement(10), mounted[10]);
  rows.length = 200;
  assert.equal(p.active(), false, 'everything is mounted: the DOM walk is exact');
  off();
  assert.equal(providers.length, 0);
});

test('the long lists that window through ListWindow register the provider', () => {
  const src = (p: string) => readFileSync(new URL(`../src/${p}`, import.meta.url), 'utf8');
  assert.match(src('modules/git/WipPanel.svelte'), /unstagedWin\.findRows\(/);
  assert.match(src('modules/git/WipPanel.svelte'), /stagedWin\.findRows\(/);
  assert.match(src('modules/panels/FileTree.svelte'), /treeWin\.findRows\(/);
  assert.match(src('modules/git/DiffViewer.svelte'), /navWin\.findRows\(/);
  assert.match(src('modules/git/BlamePanel.svelte'), /tw\.findRows\(/);
});
