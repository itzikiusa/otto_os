// Global key map: chords a focused editor owns must never reach the global
// dispatcher (node:test, Node's built-in type stripping).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { editorOwnsChord, type FocusLike } from '../src/lib/keys.ts';

const key = (k: string, shiftKey = false, altKey = false) => ({ key: k, shiftKey, altKey });
const cm: FocusLike = { tagName: 'DIV', closest: (s) => (s === '.cm-editor' ? {} : null) };
const input: FocusLike = { tagName: 'INPUT', closest: () => null };
const body: FocusLike = { tagName: 'BODY', closest: () => null };

test('CodeMirror keeps ⌘D / ⌘[ / ⌘] / ⌘U / ⌘I', () => {
  for (const k of ['d', 'D', '[', ']', 'u', 'i']) assert.equal(editorOwnsChord(key(k), cm), true, k);
  assert.equal(editorOwnsChord(key('u', true), cm), true, '⌘⇧U is redoSelection');
});

test('global chords still work inside CodeMirror', () => {
  for (const k of ['k', 'w', 't', 'j', '1', 'f', '=', '-', '0']) {
    assert.equal(editorOwnsChord(key(k), cm), false, k);
  }
  assert.equal(editorOwnsChord(key('d', true), cm), false, '⌘⇧D horizontal split');
  assert.equal(editorOwnsChord(key('d', false, true), cm), false, '⌥ combos are not ours to judge');
});

test('⌘U never fires from a plain text field; ⌘D still splits there', () => {
  assert.equal(editorOwnsChord(key('u'), input), true);
  assert.equal(editorOwnsChord(key('u'), { tagName: 'DIV', isContentEditable: true }), true);
  assert.equal(editorOwnsChord(key('d'), input), false);
});

test('nothing focused / page body → the global map handles everything', () => {
  for (const k of ['d', '[', 'u', 'i']) {
    assert.equal(editorOwnsChord(key(k), body), false, k);
    assert.equal(editorOwnsChord(key(k), null), false, k);
  }
});
