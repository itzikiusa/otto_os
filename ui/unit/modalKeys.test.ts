// Modal (lib/components/Modal.svelte): the window-level Esc handler must leave
// a key alone once something inside the sheet handled it (CodeMirror closing
// its completion popup / search panel preventDefaults it), and Tab must treat
// a focused contenteditable (CodeMirror's content node) as inside the trap.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { componentFunctions } from './componentFunctions.ts';

const MODAL = new URL('../src/lib/components/Modal.svelte', import.meta.url);

function harness(opts: { dismissable?: boolean } = {}) {
  let closed = 0;
  const sheetEl = { contains: () => true };
  const state: Record<string, any> = {
    sheetEl,
    dismissable: opts.dismissable ?? true,
    onclose: () => (closed += 1),
    document: { querySelectorAll: () => [sheetEl], activeElement: null },
    focusables: () => [],
  };
  const fns = componentFunctions(MODAL, ['onKeydown'], state);
  return { onKeydown: fns.onKeydown as (e: any) => void, closed: () => closed };
}

const key = (k: string, defaultPrevented = false) => ({
  key: k,
  shiftKey: false,
  defaultPrevented,
  stopPropagation() {},
  preventDefault() {},
});

test('Esc closes the top sheet', () => {
  const h = harness();
  h.onKeydown(key('Escape'));
  assert.equal(h.closed(), 1);
});

test('Esc already handled inside the sheet (CodeMirror popup) keeps the modal open', () => {
  const h = harness();
  h.onKeydown(key('Escape', true));
  assert.equal(h.closed(), 0, 'a defaultPrevented Esc must not close the sheet and drop the edit');
});

test('a busy (non-dismissable) sheet ignores Esc', () => {
  const h = harness({ dismissable: false });
  h.onKeydown(key('Escape'));
  assert.equal(h.closed(), 0);
});

test('the Tab trap counts contenteditable editors as focusable', () => {
  const src = readFileSync(MODAL, 'utf8');
  const focusable = /const FOCUSABLE =\s*'([^']+)'/.exec(src.replace(/'\s*\n\s*'/g, ''))?.[1] ?? '';
  assert.match(focusable, /\[contenteditable="true"\]/);
});
