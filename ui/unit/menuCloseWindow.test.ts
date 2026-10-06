// S13-306: ⌘W (File ▸ Close Tab) off the Agents page used to be a silent
// no-op, so a Git / Vault pop-out could not be closed from the keyboard.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { loadSource } from './sourceHarness.ts';

function menu(closedTab: boolean) {
  const calls: string[] = [];
  const { handleMenu } = loadSource(new URL('../src/lib/menu.ts', import.meta.url), {
    './stores/ui.svelte': { ui: { modalCount: 0 } },
    './stores/workspace.svelte': { ws: { closeActiveTab: () => { calls.push('tab'); return closedTab; } } },
    './router.svelte': { router: { go() {} } },
    './snip': { startSnip() {} },
    './selectall': { selectAllInFocus() {} },
    './stores/sidePane.svelte': { sidePane: { handleMenu: () => false, nativeState: null, focused: false } },
    './nativePane': { nativePane: { focus: async () => {} } },
    './desktop': { isEmbedded: false, closePopoutWindow: async () => { calls.push('window'); return true; } },
    './keys': { dismissTopDialog() {}, modalKeyVerdict: () => 'run' },
    './stores/sessionScope': { sessionVerbsApply: () => true },
  });
  return { handleMenu, calls };
}

test('Close Tab with no tab to close closes the pop-out window', () => {
  const m = menu(false);
  m.handleMenu('close-tab');
  assert.deepEqual(m.calls, ['tab', 'window']);
});

test('Close Tab that closed a session tab leaves the window alone', () => {
  const m = menu(true);
  m.handleMenu('close-tab');
  assert.deepEqual(m.calls, ['tab']);
});

test('closePopoutWindow never closes the main window or a pane', () => {
  const src = readFileSync(new URL('../src/lib/desktop.ts', import.meta.url), 'utf8');
  assert.match(src, /if \(!isPopout \|\| isEmbedded\) return false;/);
  const rs = readFileSync(new URL('../../apps/desktop/src-tauri/src/main.rs', import.meta.url), 'utf8');
  assert.match(rs, /"close-window",\s*"Close Window",\s*true,\s*Some\("Cmd\+Shift\+W"\)/, 'File ▸ Close Window (⌘⇧W)');
});
