import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { loadSource } from './sourceHarness.ts';

// Kotest (rc.6 B4): a gutter Run or Debug of a Kotest container or test carries its path
// of names, so the daemon selects that test (`--tests 'pkg.Spec.a -- b'`), not the spec.
describe('IDE gutter Kotest launch', () => {
  it('passes the Kotest test path to Run and to Debug', () => {
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
      envState: { gutterTarget: null },
    };
    const item = {
      line: 5, kind: 'test', label: 'a user -- logs in', language: 'kotlin',
      run_spec: {
        runner: 'kotest', cwd: '/ws/app', target_path: '/ws/app/src/test/kotlin/LoginSpec.kt',
        test_filter: 'com.acme.LoginSpec', debug_program: 'com.acme.LoginSpec', test_path: ['a user', 'logs in'],
      },
    };
    let ran: any = null;
    mod.launchGutterAction(host, item, 'run', (adhoc: any) => { ran = adhoc; }, () => {});
    assert.equal(ran.runner, 'kotest');
    assert.equal(ran.test_filter, 'com.acme.LoginSpec');
    assert.deepEqual([...ran.test_path], ['a user', 'logs in']);
    let debugged: any = null;
    mod.launchGutterAction(host, item, 'debug', () => {}, (o: any) => { debugged = o; });
    assert.equal(debugged.launch.program, 'com.acme.LoginSpec');
    assert.deepEqual([...debugged.launch.test_path], ['a user', 'logs in']);
    // The spec item: an empty path still marks a Kotest debug (the whole spec).
    mod.launchGutterAction(host, { ...item, run_spec: { ...item.run_spec, test_path: [] } }, 'debug', () => {}, (o: any) => { debugged = o; });
    assert.deepEqual([...debugged.launch.test_path], []);
  });
});
