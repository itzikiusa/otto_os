// ↑ / ↓ recall sent prompts in the runtime composer (promptHistory.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { recallPrompt, sentPrompts, type HistoryState } from '../src/modules/threads/runtime/promptHistory.ts';

const user = (id: string, text: string, extra = {}) => ({ id, type: 'user_message', text, ordinal: 0, status: 'completed', turn_id: null, title: null, started_at: null, completed_at: null, updated_at: '', ...extra }) as never;
const key = (k: string, o = {}) => ({ key: k, shiftKey: false, metaKey: false, ctrlKey: false, altKey: false, isComposing: false, ...o });
const box = (value: string, at = value.length) => ({ value, selectionStart: at, selectionEnd: at });

test('sent prompts: newest first, own messages only, context cut, repeats once', () => {
  const items = [
    user('1', 'first'),
    { id: 'a', type: 'assistant_message', text: 'ok' } as never,
    user('2', 'fix it\n\n--- Diff context: a.ts ---\n+x'),
    user('3', '[Message from thread "Other" (codex, id x). Answer it with otto.reply]\n\nhi'),
    user('4', 'again'),
    user('5', 'again'),
    user('6', 'sub', { parent_item_id: 'p' }),
  ];
  assert.deepEqual(sentPrompts(items), ['again', 'fix it', 'first']);
});

test('↑ from an empty composer walks back, ↓ forward and back to the empty draft', () => {
  const p = ['three', 'two', 'one'];
  const st: HistoryState = { at: null, shown: '' };
  assert.equal(recallPrompt(key('ArrowDown'), box(''), p, st), null, '↓ alone is not history');
  assert.equal(recallPrompt(key('ArrowUp'), box(''), p, st), 'three');
  assert.equal(recallPrompt(key('ArrowUp'), box('three'), p, st), 'two');
  assert.equal(recallPrompt(key('ArrowUp'), box('two'), p, st), 'one');
  assert.equal(recallPrompt(key('ArrowUp'), box('one'), p, st), null, 'the oldest stays');
  assert.equal(recallPrompt(key('ArrowDown'), box('one'), p, st), 'two');
  assert.equal(recallPrompt(key('ArrowDown'), box('two'), p, st), 'three');
  assert.equal(recallPrompt(key('ArrowDown'), box('three'), p, st), '');
  assert.equal(st.at, null);
});

test('never hijacks ↑: a draft, an edit, modifiers, IME, a multi-line recall off its edge line', () => {
  const p = ['line a\nline b', 'older'];
  const st: HistoryState = { at: null, shown: '' };
  assert.equal(recallPrompt(key('ArrowUp'), box('my draft'), p, st), null);
  assert.equal(recallPrompt(key('ArrowUp', { isComposing: true }), box(''), p, st), null);
  assert.equal(recallPrompt(key('ArrowUp', { shiftKey: true }), box(''), p, st), null);
  assert.equal(recallPrompt(key('ArrowUp'), box('', 0), [], st), null);
  assert.equal(recallPrompt(key('ArrowUp'), box(''), p, st), 'line a\nline b');
  // Caret on the second line: ↑ moves it up, not through history.
  assert.equal(recallPrompt(key('ArrowUp'), box('line a\nline b'), p, st), null);
  assert.equal(recallPrompt(key('ArrowUp'), box('line a\nline b', 3), p, st), 'older');
  // Edited: history mode is over; ↑ in a non-empty box stays the caret's.
  assert.equal(recallPrompt(key('ArrowDown'), box('older!'), p, st), null);
  assert.equal(st.at, null);
});
