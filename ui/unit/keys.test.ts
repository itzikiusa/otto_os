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

import { appModifier, isMacPlatform } from '../src/lib/keys.ts';

test('⌃ never stands in for ⌘ on a Mac — ⌃K/⌃T/⌃W are text editing there', () => {
  const ctrl = { metaKey: false, ctrlKey: true };
  assert.equal(appModifier(ctrl, false, true), false, 'Mac text field: ⌃ is not the app modifier');
  assert.equal(appModifier(ctrl, false, false), true, 'non-Mac remote client: ⌃ stands in for ⌘');
  assert.equal(appModifier(ctrl, true, false), false, 'terminal owns ⌃ everywhere');
  for (const mac of [true, false]) {
    for (const term of [true, false]) {
      assert.equal(appModifier({ metaKey: true, ctrlKey: false }, term, mac), true, 'real ⌘ always counts');
    }
  }
});

test('isMacPlatform recognises Mac / iOS clients only', () => {
  assert.equal(isMacPlatform({ platform: 'MacIntel' }), true);
  assert.equal(isMacPlatform({ platform: 'iPad' }), true);
  assert.equal(isMacPlatform({ platform: '', userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)' }), true);
  assert.equal(isMacPlatform({ platform: 'Win32' }), false);
  assert.equal(isMacPlatform({ platform: 'Linux x86_64' }), false);
});
