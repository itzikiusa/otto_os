// SEC6-1 (UI half): a cell stuck on a Jupyter kernel can be stopped. Restart
// and Restart & Run All stay enabled while a cell runs on a Jupyter kernel
// (the daemon ends the waiting cell; the view confirms first); the built-in
// kernel keeps them off while busy, and a read-only notebook never restarts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { canRestartKernel } from '../src/modules/ide/editor/ideNotebookModel.ts';

test('Restart is enabled while a Jupyter cell runs, not while a built-in one does', () => {
  assert.equal(canRestartKernel({ readOnly: false, busy: true, jupyter: true }), true, 'busy Jupyter: the way out');
  assert.equal(canRestartKernel({ readOnly: false, busy: true, jupyter: false }), false, 'busy built-in kernel');
  assert.equal(canRestartKernel({ readOnly: false, busy: false, jupyter: false }), true, 'idle');
  assert.equal(canRestartKernel({ readOnly: true, busy: false, jupyter: true }), false, 'read-only');
});

test('the notebook view wires Restart and Restart & Run All to that rule and confirms while busy', () => {
  const src = readFileSync(new URL('../src/modules/ide/editor/IdeNotebookView.svelte', import.meta.url), 'utf8');
  const restart = src.slice(src.indexOf('onclick={() => void restartKernel()}') - 120, src.indexOf('onclick={() => void restartKernel()}'));
  assert.match(restart, /disabled=\{!canRestart\}/, 'the Restart button follows canRestartKernel');
  assert.match(src, /label: 'Restart & Run All'[^\n]*disabled: !canRestart/);
  assert.match(src, /if \(busy\) \{\s*const ok = await confirmer\.ask\('Restart the kernel\? The running cell stops and variables are lost\.'/);
});
