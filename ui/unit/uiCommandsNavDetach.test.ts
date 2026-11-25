import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// C1-025: `otto.ui_open {placement: "window"}` when this window's side pane
// already shows the module detaches that pane — even when the window is too
// narrow to show a side pane beside the session (a window of its own needs no
// room there).

class UiCommandError extends Error {
  code: string;
  constructor(code: string, message: string) { super(message); this.code = code; }
}

test('window placement: a narrow window still detaches the side pane that shows the module', async () => {
  const handlers: Record<string, (args: any, ctx: any) => Promise<any>> = {};
  const calls: string[] = [];
  const sidePane = { active: true, visible: true, key: 'ide', fits: false, supported: true, detached: false, primaryKey: 'agents', status: 'ready', open() { calls.push('open'); } };
  loadSource(new URL('../src/lib/uiCommands/nav.ts', import.meta.url), {
    '../stores/auth.svelte': { auth: { can: () => true } },
    '../stores/sidePane.svelte': { sidePane },
    '../router.svelte': { router: { go() {} } },
    '../desktop': { isEmbedded: false, isTauri: true, isNativePane: false, openPopout: async () => 'w2' },
    '../nativePane': { nativePane: { detach: async () => void calls.push('detach') }, nativePaneSnapshot: () => ({ mode: 'attached', visible: true }) },
    '../embedGuest': { hostPrimaryKey: () => null, postToHost() {} },
    '../sidePane': { paneKey: (r: string) => r.replace(/^#?\/?/, '').split('/')[0], restorableRoute: (r: string) => r, routeOf: (h: string) => h },
    '../sidebar': { availableModules: () => [{ id: 'ide' }], moduleLabel: () => 'IDE' },
    './windowPlacement': loadSource(new URL('../src/lib/uiCommands/windowPlacement.ts', import.meta.url), {}),
    '../uiCommands': {
      UiCommandError,
      describeDocument: () => ({ module: 'agents' }),
      ensureUiModuleState: async () => {},
      localModuleState: () => null,
      registerUiCommands: (_m: string, map: Record<string, any>) => Object.assign(handlers, map),
    },
  }, { window: { addEventListener() {} } });
  const ctx = { progress() {}, highlight() {}, signal: new AbortController().signal };
  const r = await handlers.open({ module: 'ide', route: 'ide', placement: 'window' }, ctx);
  assert.equal(r.pane, 'window');
  assert.deepEqual(calls, ['detach'], 'detached as it is, not re-opened through the side-pane width check');
});
