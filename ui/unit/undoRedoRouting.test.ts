// Edit ▸ Undo / Redo in the desktop app: the menu items are custom (not
// AppKit's predefined undo:/redo:, whose native list never held CodeMirror's
// edits), so the SPA routes them by focus — lib/undoredo.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { loadSource } from './sourceHarness.ts';

/** A focused element whose ancestors match `inside` selectors. */
function el(tagName: string, inside: string[] = []) {
  const root = { tagName: 'DIV', marker: 'root' };
  return {
    tagName,
    closest: (sel: string) => (inside.includes(sel) ? root : null),
    root,
  };
}

function load() {
  const mod = loadSource(new URL('../src/lib/undoredo.ts', import.meta.url), {});
  mod.resetUndoGuard();
  const calls: string[] = [];
  const deps = {
    editor: (cm: { marker: string }, action: string) => calls.push(`editor:${action}:${cm.marker}`),
    exec: (action: string) => calls.push(`exec:${action}`),
  };
  return { mod, calls, deps };
}

type KeyInit = { key?: string; metaKey?: boolean; ctrlKey?: boolean; altKey?: boolean; shiftKey?: boolean };
function key(over: KeyInit = {}) {
  const e = {
    key: 'z', metaKey: true, ctrlKey: false, altKey: false, shiftKey: false,
    defaultPrevented: false, stopped: false,
    preventDefault() { this.defaultPrevented = true; },
    stopImmediatePropagation() { this.stopped = true; },
    ...over,
  };
  return e;
}

test('focus in a CodeMirror editor runs that editor\'s own history', () => {
  const { mod, calls, deps } = load();
  assert.equal(mod.undoRedoInFocus('undo', el('DIV', ['.cm-editor']), deps, 1000), 'editor');
  assert.equal(mod.undoRedoInFocus('redo', el('DIV', ['.cm-editor']), deps, 5000), 'editor');
  assert.deepEqual(calls, ['editor:undo:root', 'editor:redo:root']);
});

test('a plain input keeps the native undo command', () => {
  const { mod, calls, deps } = load();
  assert.equal(mod.undoRedoInFocus('undo', el('INPUT'), deps, 1000), 'native');
  assert.equal(mod.undoRedoInFocus('redo', el('TEXTAREA'), deps, 5000), 'native');
  assert.equal(mod.undoRedoInFocus('undo', null, deps, 9000), 'native');
  assert.deepEqual(calls, ['exec:undo', 'exec:redo', 'exec:undo']);
});

test('the terminal and another pane\'s iframe are left alone', () => {
  const { mod, calls, deps } = load();
  assert.equal(mod.undoRedoInFocus('undo', el('TEXTAREA', ['.xterm']), deps, 1000), 'terminal');
  assert.equal(mod.undoRedoInFocus('undo', el('IFRAME'), deps, 5000), 'elsewhere');
  assert.deepEqual(calls, []);
});

test('a keypress the page already handled is not undone a second time by the menu', () => {
  const { mod, calls, deps } = load();
  const e = key();
  mod.onHistoryKeydown(e, 1000);
  e.preventDefault(); // CodeMirror's keymap ran undo
  assert.equal(mod.undoRedoInFocus('undo', el('DIV', ['.cm-editor']), deps, 1050), 'handled');
  assert.deepEqual(calls, []);
});

test('an unhandled keydown (plain input) still lets the menu run the command', () => {
  const { mod, calls, deps } = load();
  mod.onHistoryKeydown(key(), 1000);
  assert.equal(mod.undoRedoInFocus('undo', el('INPUT'), deps, 1050), 'native');
  assert.deepEqual(calls, ['exec:undo']);
});

test('a keydown arriving after the menu already ran is swallowed', () => {
  const { mod, deps } = load();
  mod.undoRedoInFocus('redo', el('DIV', ['.cm-editor']), deps, 1000);
  const e = key({ shiftKey: true });
  mod.onHistoryKeydown(e, 1100);
  assert.equal(e.defaultPrevented, true);
  assert.equal(e.stopped, true);
  // A later, separate press goes through untouched.
  const later = key({ shiftKey: true });
  mod.onHistoryKeydown(later, 3000);
  assert.equal(later.defaultPrevented, false);
});

test('only ⌘Z / ⌘⇧Z count as history keys', () => {
  const { mod } = load();
  assert.equal(mod.historyKeyAction(key()), 'undo');
  assert.equal(mod.historyKeyAction(key({ key: 'Z', shiftKey: true })), 'redo');
  assert.equal(mod.historyKeyAction(key({ metaKey: false, ctrlKey: true })), null);
  assert.equal(mod.historyKeyAction(key({ key: 'a' })), null);
});

test('the desktop Edit menu emits custom undo/redo ids, not the predefined items', () => {
  const rs = readFileSync(new URL('../../apps/desktop/src-tauri/src/main.rs', import.meta.url), 'utf8');
  assert.match(rs, /"undo",\s*"Undo",\s*true,\s*Some\("Cmd\+Z"\)/);
  assert.match(rs, /"redo",\s*"Redo",\s*true,\s*Some\("Cmd\+Shift\+Z"\)/);
  assert.doesNotMatch(rs, /PredefinedMenuItem::(undo|redo)\(/);
});

test('menu.ts routes the undo/redo ids', () => {
  const calls: string[] = [];
  const { handleMenu } = loadSource(new URL('../src/lib/menu.ts', import.meta.url), {
    './stores/ui.svelte': { ui: { modalCount: 0 } },
    './stores/workspace.svelte': { ws: {} },
    './router.svelte': { router: { go() {} } },
    './snip': { startSnip() {} },
    './selectall': { selectAllInFocus() {} },
    './undoredo': { undoRedoInFocus: (a: string) => calls.push(a), installUndoKeyGuard: () => () => {} },
    './stores/sidePane.svelte': { sidePane: { handleMenu: () => false, nativeState: null, focused: false } },
    './nativePane': { nativePane: { focus: async () => { calls.push('focus'); } } },
    './desktop': { isEmbedded: false, closePopoutWindow: async () => true },
    './keys': { dismissTopDialog() {}, modalKeyVerdict: () => 'run' },
    './stores/sessionScope': { sessionVerbsApply: () => true },
  });
  handleMenu('undo');
  handleMenu('redo');
  assert.deepEqual(calls, ['undo', 'redo']);
});
