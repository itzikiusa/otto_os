import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { loadSource } from './sourceHarness.ts';

// A user gutter-ran TestX, set breakpoints and pressed the toolbar Debug: it
// debugged the auto-selected first config (`go run ./cmd/…`, with that Run
// argv as the debuggee's args) and ignored the breakpoints. A gutter launch
// now becomes the toolbar's selected target, and a debugged run config never
// passes its tool command line as the program's argv.
function load(api: Record<string, unknown>) {
  return loadSource(
    new URL('../src/modules/ide/store/ideDebugAndEnvController.ts', import.meta.url),
    {
      '../../../lib/api/client': { api, ApiError: class extends Error {} },
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
}

function host(extra: Record<string, unknown> = {}): any {
  return {
    target: { kind: 'local', root: '/ws/app' },
    targetPayload: { kind: 'local', root: '/ws/app' },
    projectRoots: [{ path: '/ws/app', name: 'app', isPrimary: true }],
    activePath: '/ws/app/pkg/x_test.go',
    activeTab: null,
    projectEnv: null,
    runConfigs: [],
    selectedConfigId: null,
    gutterTarget: null,
    activeSpringProfiles: [],
    activeMavenProfiles: [],
    effectiveBreakpoints: [],
    openBottomTab() {},
    targetQuery: () => 'root=%2Fws%2Fapp',
    ...extra,
  };
}

const goTest = {
  line: 9, kind: 'test', label: 'Run TestX', language: 'go',
  run_spec: { runner: 'go_test', args: ['test', '-v', '-run', '^TestX$', '.'], cwd: '/ws/app/pkg', target_path: '/ws/app/pkg/x_test.go', debug_program: '/ws/app/pkg', debug_test_name: '^TestX$' },
};

describe('IDE toolbar target after a gutter launch', () => {
  it('a gutter Run or Debug becomes the selected toolbar target', () => {
    const mod = load({});
    for (const mode of ['run', 'debug'] as const) {
      const h = host({ selectedConfigId: 'auto:go_run:cmd' });
      mod.launchGutterAction(h, goTest, mode, () => {}, () => {});
      assert.equal(h.selectedConfigId, 'gutter', mode);
      assert.equal(h.gutterTarget, goTest, mode);
    }
  });

  it('reloading the run configs keeps the gutter target selected', async () => {
    const auto = { id: 'auto:go_run:cmd', name: 'Go Run cmd', runner: 'go_run', args: ['run', './cmd/x'], auto_detected: true };
    const mod = load({ get: async () => ({ env: {}, configs: [], detected_configs: [auto] }) });
    const h = host({ selectedConfigId: 'gutter', gutterTarget: goTest });
    await mod.fetchEnvironments(h);
    assert.equal(h.selectedConfigId, 'gutter');
  });

  it('debugging a tool-runner config does not pass its Run argv to the program', async () => {
    let body: any = null;
    const mod = load({ post: async (_url: string, b: unknown) => { body = b; throw new Error('stop'); } });
    const cfgs = [
      { id: 'auto:go_run:cmd', name: 'Go Run cmd', runner: 'go_run', args: ['run', './cmd/x'], target_path: './cmd/x', auto_detected: true, env_overrides: {} },
      { id: 'py', name: 'script', runner: 'python', args: ['--verbose'], target_path: '/ws/app/s.py', env_overrides: {} },
    ];
    const h = host({ runConfigs: cfgs });
    await mod.launchDebugSession(h, { configId: 'auto:go_run:cmd' }, null, () => {});
    assert.deepEqual([...body.launch.args], []);
    await mod.launchDebugSession(h, { configId: 'py' }, null, () => {});
    assert.deepEqual([...body.launch.args], ['--verbose']);
  });
});
