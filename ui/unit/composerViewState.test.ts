import { test } from 'node:test';
import assert from 'node:assert/strict';
import { composerViewState, saveComposerCursorState } from '../src/lib/stores/runtimeDraftCursor.ts';

// lib/stores/runtimeDraftCursor.ts `composerViewState` on a textarea that also
// has Svelte's `bind:value` (every composer: Threads, the New-thread stage,
// terminal threads, the IDE agent).

class FakeTextarea extends EventTarget {
  value = '';
  selectionStart = 0;
  selectionEnd = 0;
  selectionDirection: 'forward' | 'backward' | 'none' = 'none';
  scrollTop = 0;
  clientHeight = 44;
  isConnected = true;
  setSelectionRange(start: number, end: number): void {
    this.selectionStart = start;
    this.selectionEnd = end;
  }
}

type Action = ReturnType<typeof composerViewState>;

/** One key, in the order a browser delivers it: `beforeinput`, the text
 *  changes, then `input` — whose first listener is `bind:value`'s (it is
 *  registered before the action's), and the microtask checkpoint after it
 *  runs Svelte's effects, i.e. the action's `update()`, before the action's
 *  own `input` listener (observed in WebKit, e2e/threads-composer-mobile.spec.ts). */
async function key(node: FakeTextarea, action: Action, k: string, ch: string): Promise<void> {
  node.dispatchEvent(new Event('beforeinput'));
  const at = node.selectionStart;
  node.value = node.value.slice(0, at) + ch + node.value.slice(node.selectionEnd);
  node.setSelectionRange(at + ch.length, at + ch.length);
  action.update({ key: k, hasText: node.value.length > 0 });
  await Promise.resolve();
  node.dispatchEvent(new Event('input'));
}

test('typing into a composer that took focus without a tap keeps every letter in order', async () => {
  const k = 'rt:unit-first-letter';
  // The last send emptied this composer: its saved caret is {0,0}.
  saveComposerCursorState(k, { selectionStart: 0, selectionEnd: 0, scrollTop: 0 });
  // The thread opens again: a new composer, focused by the page, no pointer on it yet.
  const node = new FakeTextarea();
  const action = composerViewState(node as unknown as HTMLTextAreaElement, { key: k, hasText: false });
  for (const ch of 'i also saw') await key(node, action, k, ch);
  assert.equal(node.value, 'i also saw', 'the first letter stays first');
  assert.equal(node.selectionStart, 'i also saw'.length, 'the caret follows the typing');
  action.destroy();
});

test('a draft that arrives after the composer mounted still gets its saved caret back', async () => {
  const k = 'rt:unit-late-draft';
  saveComposerCursorState(k, { selectionStart: 3, selectionEnd: 3, scrollTop: 0 });
  const node = new FakeTextarea();
  const action = composerViewState(node as unknown as HTMLTextAreaElement, { key: k, hasText: false });
  // The saved draft loads (bind:value writes it, no typing).
  node.value = 'hello world';
  node.setSelectionRange(11, 11);
  action.update({ key: k, hasText: true });
  await Promise.resolve();
  assert.equal(node.selectionStart, 3, 'the caret is back where it was');
  action.destroy();
});
