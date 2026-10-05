// Shared design primitives (design iteration 1): the lazy/loading/error state
// components, overlay scrims, chart palette and shared motion. Svelte
// components can't be mounted under `node --test`, so the contracts that
// regressed are checked as source text, and the pure palette helper directly.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { CHART_COLORS, chartColor } from '../src/lib/components/chartPalette.ts';

const UI = join(import.meta.dirname, '..');
const read = (p: string) => readFileSync(join(UI, p), 'utf8');

test('LazyMount hands LoadState `empty`, or the skeleton / error never render', () => {
  // LoadState shows its skeleton only on loading && empty, and the full error
  // only on error && empty; without `empty` a loading chunk was a blank pane.
  const src = read('src/lib/components/LazyMount.svelte');
  assert.match(src, /<LoadState[^>]*\bempty\b[^>]*loading=\{!lazy\.error\}/);
});

test('LoadState announces loading and errors', () => {
  const src = read('src/lib/components/LoadState.svelte');
  assert.match(src, /class="ls-loading"[^>]*role="status"/);
  assert.match(src, /class="ls-error"[^>]*role="alert"/);
  assert.match(src, /--danger-soft/);
});

test('Skeleton is a labelled status region unless a parent owns the announcement', () => {
  const src = read('src/lib/components/Skeleton.svelte');
  assert.match(src, /role=\{announce \? 'status' : undefined\}/);
  assert.match(src, /sr-only/);
});

test('every overlay backdrop uses the scrim tokens, defined for each scheme', () => {
  for (const f of ['src/lib/components/Modal.svelte', 'src/shell/Drawer.svelte']) {
    assert.match(read(f), /background: var\(--scrim\)/, f);
    assert.doesNotMatch(read(f), /rgba\(0, 0, 0, 0\.\d+\)\s*;[^}]*z-index/, f);
  }
  assert.match(read('src/shell/Palette.svelte'), /var\(--scrim-soft\)/);
  const tokens = read('src/lib/tokens.css');
  assert.equal([...tokens.matchAll(/--scrim:/g)].length, 2, 'light + dark');
  assert.equal([...tokens.matchAll(/--scrim-soft:/g)].length, 2, 'light + dark');
});

test('Modal: only a press that began on the backdrop closes; dismissable gates every close path', () => {
  const src = read('src/lib/components/Modal.svelte');
  assert.match(src, /onpointerdown=/);
  assert.match(src, /downOnBackdrop && e\.target === e\.currentTarget/);
  assert.match(src, /dismissable = true/);
  assert.match(src, /if \(dismissable\) onclose\(\)/);
});

test('BottomNav "More" sheet is a real dialog', () => {
  // The sheet is the shared shell Drawer, which owns the dialog semantics
  // (role=dialog, pushModal, dialogFocus, Esc, scrim) — asserted for Drawer above.
  const src = read('src/shell/BottomNav.svelte');
  assert.match(src, /<Drawer\b[^>]*title="More"/);
  assert.match(src, /aria-expanded=\{moreOpen\}/);
  assert.doesNotMatch(src, /svelte-ignore/);
  const drawer = read('src/shell/Drawer.svelte');
  assert.match(drawer, /dialogFocus/);
  assert.match(drawer, /ui\.pushModal\(\)/);
});

test('motion: one global spin/pulse, and a reduced-motion alternative for .spinner', () => {
  const css = read('src/app.css');
  assert.match(css, /@keyframes otto-spin/);
  assert.match(css, /@keyframes otto-pulse/);
  assert.match(css, /prefers-reduced-motion: reduce\)[\s\S]*?\.spinner \{[^}]*animation: none !important;[^}]*border-style: dotted/);
  for (const f of ['src/shell/StatusBar.svelte', 'src/lib/components/StatusDot.svelte', 'src/lib/components/StatusBadge.svelte']) {
    assert.doesNotMatch(read(f), /@keyframes (pulse|sbadge-pulse)\b/, f);
  }
});

test('chart palette is the scheme-aware --cat-* set and wraps', () => {
  assert.deepEqual([...CHART_COLORS], [1, 2, 3, 4, 5, 6].map((n) => `var(--cat-${n})`));
  assert.equal(chartColor(0), 'var(--cat-1)');
  assert.equal(chartColor(6), 'var(--cat-1)');
  assert.equal(chartColor(-1), 'var(--cat-6)');
  for (const f of ['src/modules/database/Chart.svelte', 'src/lib/components/MetricChart.svelte']) {
    assert.doesNotMatch(read(f), /#[0-9a-fA-F]{6}/, `${f} has a hex colour`);
  }
});

test('status-* tokens are not used for text in shared components', () => {
  for (const f of ['ProofBadge', 'ProofStatusChip', 'DoneContractMeter', 'ResourceAccess', 'DiffView']) {
    assert.doesNotMatch(read(`src/lib/components/${f}.svelte`), /(^|\s)color:\s*var\(--status-/m, f);
  }
});
