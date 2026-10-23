import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { compile, compileModule } from 'svelte/compiler';

// architecture-1-09 (structure): GraphView.svelte is a thin shell over
// git/graph/*. Every piece of the split must compile as Svelte 5 (runes in
// the .svelte.ts modules included), and the shell must stay within its size
// target — GraphView ≤ 1,800 lines with a script block ≤ 700, every split-out
// file under 800 — so the god component cannot quietly grow back.

const git = (name: string) => new URL(`../src/modules/git/${name}`, import.meta.url);
const COMPONENTS = [
  'GraphView.svelte',
  'graph/RefSidebar.svelte',
  'graph/RefSideLists.svelte',
  'graph/RefPopover.svelte',
  'graph/RefChip.svelte',
  'graph/CommitDetailPane.svelte',
];
const MODULES = [
  'graph/graphData.svelte.ts',
  'graph/graphSync.svelte.ts',
  'graph/graphRefIndex.svelte.ts',
  'graph/graphViewport.svelte.ts',
  'graph/graphSelection.svelte.ts',
  'graph/refPopoverState.svelte.ts',
  'graph/refDrag.svelte.ts',
];

test('every split-out graph component compiles (structure)', () => {
  for (const name of COMPONENTS) {
    const src = readFileSync(git(name), 'utf8');
    assert.doesNotThrow(() => compile(src, { filename: name, generate: 'client' }), name);
  }
});

test('every graph rune module compiles (structure)', () => {
  for (const name of MODULES) {
    const js = ts.transpileModule(readFileSync(git(name), 'utf8'), {
      compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
    }).outputText;
    assert.doesNotThrow(() => compileModule(js, { filename: name, generate: 'client' }), name);
  }
});

test('GraphView stays a thin shell and no split file reaches 800 lines (structure)', () => {
  const [shell] = COMPONENTS;
  const view = readFileSync(git(shell), 'utf8');
  const lines = view.split('\n').length;
  const script = view.split('</script>')[0].split('\n').length;
  assert.ok(lines <= 1800, `GraphView.svelte is ${lines} lines`);
  assert.ok(script <= 700, `GraphView.svelte script is ${script} lines`);
  for (const name of [...COMPONENTS.slice(1), ...MODULES, 'graph/graphActions.svelte.ts', 'graph/graphMenus.ts']) {
    const n = readFileSync(git(name), 'utf8').split('\n').length;
    assert.ok(n < 800, `${name} is ${n} lines`);
  }
  // No refs-sidebar, popover or detail-pane markup is left inline.
  for (const hook of ['class="refs-panel"', 'class="ref-popover"', 'class="detail-panel"']) {
    assert.ok(!view.includes(hook), `GraphView.svelte still renders ${hook}`);
  }
});
