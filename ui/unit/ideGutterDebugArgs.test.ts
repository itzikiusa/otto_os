import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { loadSource } from './sourceHarness.ts';

// Gutter Debug: the item's `run_spec.args` are the Run command line
// (`cargo run`, `. script.sh && fn`, the pytest node id). Every debug
// adapter rebuilds its own argv from `debug_program` / `test_name`, so
// passing them on made them the debuggee's arguments (R3 / P3).
describe('IDE gutter debug launch', () => {
  it('never passes the Run command line as the debuggee argv', () => {
    const mod = loadSource(
      new URL('../src/modules/ide/store/ideDebugAndEnvController.ts', import.meta.url),
      {
        '../../../lib/api/client': { api: {}, ApiError: class extends Error {} },
        '../../../lib/idePreferences.svelte': { idePreferences: { current: {} } },
        '../../../lib/stores/auth.svelte': { auth: {} },
        '../../../lib/toast.svelte': { toasts: {} },
        '../../../lib/toastError': { toastError() {} },
        '../../../lib/confirm.svelte': { confirmer: {} },
        '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: {} },
        '../../../lib/storage': { lsGet: () => null, lsSet() {} },
        '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({}) },
        '../../../lib/components/cm-ide-drafts': {},
        svelte: { tick: async () => {} },
      },
      {},
      { relative: true },
    );
    const host: any = {
      target: { kind: 'local', root: '/ws/app' },
      projectRoots: [{ path: '/ws/app', name: 'app', isPrimary: true }],
      activePath: null,
      activeTab: null,
      projectEnv: null,
    };
    const items = [
      { language: 'rust', kind: 'run', run_spec: { runner: 'cargo', args: ['run'], target_path: '/ws/app/src/main.rs' } },
      { language: 'shellscript', kind: 'run', run_spec: { runner: 'shell', args: ['-c', '. ./x.sh && build'], target_path: '/ws/app/x.sh' } },
      { language: 'python', kind: 'test', run_spec: { runner: 'pytest', args: ['tests/test_a.py::test_one'], test_name: 'tests/test_a.py::test_one', target_path: '/ws/app/tests/test_a.py' } },
    ];
    for (const item of items) {
      let launched: any = null;
      mod.launchGutterAction(host, { line: 1, label: 'x', ...item }, 'debug', () => {}, (o: any) => { launched = o; });
      assert.ok(launched, `${item.language}: debug launched`);
      assert.deepEqual([...launched.launch.args], [], `${item.language}: empty debuggee argv`);
    }
  });
});
