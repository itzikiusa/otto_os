// IDE launch life: Run/Debug act on the file being looked at (I2-01), a second
// debug launch never leaves an invisible session (I2-03), start failures are
// kept for an inline card (I2-10), copy for failed commands (I2-22), per-runner
// configuration fields (I2-15), and the session-scoped resets (I2-19, I2-23).
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { loadSource } from './sourceHarness.ts';

const plain = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

class ApiError extends Error {
  status: number;
  constructor(status: number, message = `HTTP ${status}`) { super(message); this.status = status; }
}

function modules(api: Record<string, unknown> = {}, storage = new Map<string, string>()) {
  const toastLog: string[] = [];
  const imports = {
    svelte: { tick: async () => {} },
    '../../../lib/api/client': { api, ApiError, getToken: () => '', laneFetch: () => { throw new Error('unused'); } },
    '../../../lib/storage': {
      lsGet: (k: string) => storage.get(k) ?? null,
      lsSet: (k: string, v: string) => { storage.set(k, v); },
      lsRemove: (k: string) => { storage.delete(k); },
    },
    '../../../lib/confirm.svelte': { confirmer: { ask: async () => true, choose: async () => ({ value: null }) } },
    '../../../lib/toast.svelte': {
      toasts: {
        success: (t: string) => toastLog.push(`ok:${t}`), error: (t: string) => toastLog.push(`err:${t}`),
        info: (t: string, b?: string) => toastLog.push(`info:${t}:${b ?? ''}`), warn: () => {},
      },
    },
    '../../../lib/toastError': { toastError: (t: string) => toastLog.push(`err:${t}`) },
    '../../../lib/idePreferences.svelte': { idePreferences: { current: { moduleSdkMap: {} } } },
    '../../../lib/stores/auth.svelte': { auth: { isRoot: true } },
    '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: { rootsForLocal: () => [], saveRootsForLocal() {} } },
    '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({ autoSave: false, maxOpenTabs: 20 }) },
    '../../../lib/components/cm-ide-drafts': { readIdeUndoDraft: () => null, removeIdeUndoDraft() {}, writeIdeUndoDraftImmediate() {} },
  };
  const opts = { relative: true };
  const cache = new Map();
  const load = (rel: string) => loadSource(new URL(`../src/modules/ide/${rel}`, import.meta.url), imports, {}, opts, cache);
  return {
    target: load('store/ideLaunchTarget.ts'),
    launch: load('store/ideDebugLaunch.svelte.ts'),
    sync: load('store/ideDebugSnapshotSync.ts'),
    ctl: load('store/ideDebugAndEnvController.ts'),
    form: load('shell/ideRunConfigForm.ts'),
    toastLog,
    storage,
  };
}

const pyTab = { path: '/ws/app/review.py', name: 'review.py', language: 'python', content: 'print(1)\n' };
const readme = { path: '/ws/app/README.md', name: 'README.md', language: 'markdown', content: '# x\n' };
const saved = { id: 'cfg_pipeline', name: 'Python Settlement pipeline', runner: 'python', args: [], env_overrides: {}, target_path: '/ws/app/pipeline.py' };
const autoGo = { id: 'auto:go', name: 'Go Run app', runner: 'go_run', args: [], env_overrides: {}, auto_detected: true };
const autoPy = { id: 'auto:py', name: 'pytest', runner: 'pytest', args: [], env_overrides: {}, auto_detected: true };

describe('I2-01 launch target: the file being looked at', () => {
  const { target } = modules();
  const none = { selected: null, lastUsed: null };

  it('defaults to Current file, never the first config by position', () => {
    assert.equal(target.resolveLaunchSelection(none, [saved, autoGo]), 'current-file');
    assert.equal(target.resolveLaunchSelection({ selected: 'cfg_pipeline', lastUsed: null }, [saved]), 'cfg_pipeline');
    // a remembered pick that no longer exists falls back to Current file
    assert.equal(target.resolveLaunchSelection({ selected: 'cfg_gone', lastUsed: null }, [saved]), 'current-file');
    // a saved config picked in the editor this session survives a reload; a fast-detected one does not
    assert.equal(target.resolveLaunchSelection(none, [saved], 'cfg_pipeline'), 'cfg_pipeline');
    assert.equal(target.resolveLaunchSelection(none, [saved, autoGo], 'auto:go'), 'current-file');
  });

  it('plans: current file when runnable, the last used config for a README, else nothing', () => {
    assert.deepEqual(plain(target.planLaunch('current-file', pyTab, none, [saved])), { kind: 'current-file' });
    assert.deepEqual(plain(target.planLaunch(null, pyTab, none, [saved])), { kind: 'current-file' });
    assert.deepEqual(plain(target.planLaunch('cfg_pipeline', readme, none, [saved])), { kind: 'config', configId: 'cfg_pipeline' });
    assert.deepEqual(
      plain(target.planLaunch('current-file', readme, { selected: null, lastUsed: 'cfg_pipeline' }, [saved, autoGo])),
      { kind: 'config', configId: 'cfg_pipeline' },
    );
    const nothing = plain(target.planLaunch('current-file', readme, none, [saved, autoGo]));
    assert.equal(nothing.kind, 'none');
    assert.match(nothing.reason, /README\.md can’t be run/);
  });

  it('marks a non-runnable tab with a reason and labels the selector', () => {
    assert.deepEqual(plain(target.currentFileRunnable(pyTab)), { ok: true, reason: null });
    assert.equal(target.currentFileRunnable(readme).ok, false);
    assert.equal(target.currentFileRunnable(readme).reason, 'Can’t run this file type');
    assert.equal(target.launchSelectorLabel(null, pyTab), 'Current file (review.py)');
    assert.equal(target.launchSelectorLabel(saved, pyTab), 'Python Settlement pipeline');
  });

  it('lists the active file’s ecosystem first among auto-detected targets', () => {
    const order = target.orderDetectedForLanguage([autoGo, autoPy], 'python').map((c: { id: string }) => c.id);
    assert.deepEqual([...order], ['auto:py', 'auto:go']);
    assert.deepEqual([...target.orderDetectedForLanguage([autoGo, autoPy], 'markdown').map((c: { id: string }) => c.id)], ['auto:go', 'auto:py']);
  });

  it('remembers the pick and the last used config per project', () => {
    const m = modules();
    const a = { kind: 'local', root: '/ws/a', connection_id: null };
    const b = { kind: 'local', root: '/ws/b', connection_id: null };
    m.target.saveLaunchMemory(a, { selected: 'cfg_pipeline' });
    m.target.saveLaunchMemory(a, { lastUsed: 'cfg_pipeline' });
    assert.deepEqual(plain(m.target.loadLaunchMemory(a)), { selected: 'cfg_pipeline', lastUsed: 'cfg_pipeline' });
    assert.deepEqual(plain(m.target.loadLaunchMemory(b)), { selected: null, lastUsed: null });
  });
});

function debugHost(over: Record<string, unknown> = {}) {
  return {
    target: { kind: 'local', root: '/ws/app', connection_id: null },
    targetPayload: { kind: 'local', root: '/ws/app' },
    projectRoots: [{ path: '/ws/app', name: 'app', isPrimary: true }],
    activePath: pyTab.path, activeTab: pyTab,
    projectEnv: { env_vars: [] }, runConfigs: [saved], detectedConfigs: [], selectedConfigId: 'current-file',
    activeMavenProfiles: [], activeSpringProfiles: [], buildSkipTests: false, buildOffline: false,
    effectiveBreakpoints: [], keepTestcontainersAlive: false, lastDebugLaunch: null, expandedVariables: {},
    openBottomTab() {}, resolvePath: (p: string) => p, targetQuery: () => 'kind=local&root=%2Fws%2Fapp',
    ...over,
  };
}

describe('I2-01 the controller launches what the selector shows', () => {
  it('Debug with "Current file" launches the open file, not a saved config', async () => {
    const posted: Array<{ url: string; body: any }> = [];
    const api = { post: async (url: string, body: any) => { posted.push({ url, body }); return { session: { id: 'dbg_1', state: 'starting', output: [], breakpoints: [], stack_frames: [] } }; } };
    const m = modules(api);
    const ctl = await m.ctl.launchDebugSession(debugHost(), {}, null, () => {});
    ctl?.abort();
    assert.equal(posted[0].body.launch.program, '/ws/app/review.py');
    assert.equal(posted[0].body.language, 'python');
    assert.equal(posted[0].body.config_id, undefined);
  });

  it('Run on a README falls back to the last used config, and remembers launched configs', async () => {
    const posted: Array<{ url: string; body: any }> = [];
    const api = {
      post: async (url: string, body: any) => { posted.push({ url, body }); return { run: { id: `run_${posted.length}`, status: 'passed', output: [] } }; },
      bg: { get: async () => ({}) },
    };
    const m = modules(api);
    const host = debugHost({ activeTab: readme, activePath: readme.path });
    // nothing remembered yet: the person is told, nothing is launched
    await m.ctl.launchIdeRun(host, {}, null, null);
    assert.equal(posted.length, 0);
    assert.ok(m.toastLog.some((t) => t.startsWith('info:Nothing to run')));
    // an explicit config launch is remembered …
    const r1 = await m.ctl.launchIdeRun(host, { configId: 'cfg_pipeline' }, null, null);
    r1.poller?.stop(); r1.sseCtl?.abort();
    // … and is what a README launch falls back to
    const r2 = await m.ctl.launchIdeRun(host, {}, null, null);
    r2.poller?.stop(); r2.sseCtl?.abort();
    assert.equal(posted.length, 2);
    assert.equal(posted[1].body.config_id, 'cfg_pipeline');
  });

  it('a refused start is kept for the inline card (no prefix) and cleared by the next launch', async () => {
    let fail = true;
    const api = {
      post: async () => {
        if (fail) throw new ApiError(400, 'invalid: unsupported debug language \'markdown\'');
        return { session: { id: 'dbg_2', state: 'starting', output: [], breakpoints: [], stack_frames: [] } };
      },
    };
    const m = modules(api);
    await m.ctl.launchDebugSession(debugHost(), { language: 'markdown', launch: { program: '/ws/app/README.md' } }, null, () => {});
    assert.deepEqual(plain(m.launch.ideDebugLaunch.startError), {
      message: 'unsupported debug language \'markdown\'', kind: null, ecosystem: null,
    });
    fail = false;
    const ctl = await m.ctl.launchDebugSession(debugHost(), {}, null, () => {});
    ctl?.abort();
    assert.equal(m.launch.ideDebugLaunch.startError, null);
  });

  it('a README debug with nothing to fall back to explains itself inline', async () => {
    const m = modules({ post: async () => { throw new Error('must not launch'); } });
    await m.ctl.launchDebugSession(debugHost({ activeTab: readme, activePath: readme.path }), {}, null, () => {});
    assert.match(m.launch.ideDebugLaunch.startError?.message ?? '', /README\.md can’t be run or debugged/);
  });
});

describe('I2-03 one live debug session, never an orphan', () => {
  const live = { id: 'dbg_old', title: 'Debug review.py (python-bdb-dap)', state: 'paused' };

  it('launches without asking when nothing is live', async () => {
    const { launch } = modules();
    let asked = 0;
    const choose = async () => { asked++; return { value: null }; };
    assert.equal(await launch.guardDebugRelaunch({ activeDebug: null }, undefined, { choose }), true);
    assert.equal(await launch.guardDebugRelaunch({ activeDebug: { ...live, state: 'terminated' } }, undefined, { choose }), true);
    assert.equal(asked, 0);
  });

  it('Cancel keeps the running session and launches nothing', async () => {
    const { launch } = modules();
    let stopped = 0;
    const ok = await launch.guardDebugRelaunch({ activeDebug: live }, undefined, {
      choose: async (msg: string, o: { options: Array<{ label: string }> }) => {
        assert.match(msg, /review\.py is paused/);
        assert.deepEqual([...o.options.map((x) => x.label)], ['Stop and debug again']);
        return { value: null };
      },
      stop: async () => { stopped++; return { ended: true, snap: null }; },
    });
    assert.equal(ok, false);
    assert.equal(stopped, 0);
  });

  it('"Stop and debug again" stops the old session, waits for it, then launches', async () => {
    const { launch } = modules();
    const applied: string[] = [];
    const ok = await launch.guardDebugRelaunch({ activeDebug: live }, (s: { state: string }) => applied.push(s.state), {
      choose: async () => ({ value: 'stop-relaunch' }),
      stop: async (id: string) => { assert.equal(id, 'dbg_old'); return { ended: true, snap: { ...live, state: 'terminated' } }; },
    });
    assert.equal(ok, true);
    assert.deepEqual(applied, ['terminated']);
  });

  it('does not launch a second session while the old one refuses to end', async () => {
    const m = modules();
    const ok = await m.launch.guardDebugRelaunch({ activeDebug: live }, undefined, {
      choose: async () => ({ value: 'stop-relaunch' }),
      stop: async () => ({ ended: false, snap: live }),
    });
    assert.equal(ok, false);
    assert.ok(m.toastLog.some((t) => t.startsWith('err:Couldn’t stop the running debug session')));
  });

  it('stopDebugSessionAndWait polls (bounded) until the daemon reports the end', async () => {
    let gets = 0;
    const api = {
      post: async () => ({ session: { ...live, state: 'running' } }),
      get: async () => { gets++; return { session: { ...live, state: gets >= 2 ? 'terminated' : 'running' } }; },
    };
    const m = modules(api);
    const res = await m.launch.stopDebugSessionAndWait('dbg_old', { pollMs: 1, waitMs: 1000 });
    assert.equal(res.ended, true);
    assert.equal(gets, 2);
    const stuck = modules({ post: async () => ({ session: live }), get: async () => ({ session: live }) });
    const res2 = await stuck.launch.stopDebugSessionAndWait('dbg_old', { pollMs: 1, waitMs: 20 });
    assert.equal(res2.ended, false);
  });

  it('a project switch stops a live session best-effort, never the host’s as a guest', async () => {
    const posted: string[] = [];
    const m = modules({ post: async (url: string) => { posted.push(url); return {}; } });
    m.launch.leaveDebugSessionOnSwitch(live, true);
    m.launch.leaveDebugSessionOnSwitch({ ...live, state: 'terminated' }, false);
    assert.equal(posted.length, 0);
    m.launch.leaveDebugSessionOnSwitch(live, false);
    assert.deepEqual(posted, ['/ide/debug/sessions/dbg_old/command']);
  });
});

describe('I2-22 debugger command copy', () => {
  const { launch } = modules();
  it('uses the person’s verb for every action', () => {
    assert.equal(launch.debugActionVerb('run_to_cursor'), 'run to cursor');
    assert.equal(launch.debugActionVerb('step_over'), 'step over');
    assert.equal(launch.debugActionVerb('select_frame'), 'switch to that frame');
    assert.equal(launch.debugActionVerb('some_new_thing'), 'some new thing');
  });
});

describe('I2-15 run configuration fields per runner', () => {
  const { form } = modules();
  const empty = {
    target_path: '', command: '', test_filter: '', argsText: '', cwd: '', buildFlagsText: '', sdk_path: '',
    env_file: '', before_launch: '', inherit_shell_env: true,
  };

  it('a Python script shows script/args/cwd and folds the rest; no test filter or build flags', () => {
    const v = plain(form.visibleRunConfigFields('python', empty));
    assert.deepEqual(v.primary, ['target', 'args', 'cwd']);
    assert.deepEqual(v.advanced, ['sdk', 'env_file', 'before_launch', 'inherit_shell_env']);
    assert.equal(v.advancedOpen, false);
  });

  it('test runners get a filter; Go gets build flags; custom gets a command and no SDK', () => {
    assert.deepEqual(plain(form.runConfigFieldLayout('go_test').primary), ['target', 'test_filter', 'args', 'cwd']);
    assert.ok(form.runConfigFieldLayout('go_test').advanced.includes('build_flags'));
    const custom = plain(form.runConfigFieldLayout('custom'));
    assert.equal(custom.primary[0], 'command');
    assert.ok(!custom.advanced.includes('sdk'));
  });

  it('Advanced opens by itself when a field in it is set, and a runner switch never hides a value', () => {
    assert.equal(form.visibleRunConfigFields('python', { ...empty, env_file: '.env.local' }).advancedOpen, true);
    assert.equal(form.visibleRunConfigFields('python', { ...empty, inherit_shell_env: false }).advancedOpen, true);
    const switched = plain(form.visibleRunConfigFields('python', { ...empty, test_filter: 'TestAdd' }));
    assert.ok(switched.advanced.includes('test_filter'));
    assert.equal(switched.advancedOpen, true);
  });

  it('a new configuration starts from the active tab', () => {
    assert.deepEqual(plain(form.draftDefaultsForTab(pyTab)), { name: 'review.py', runner: 'python', target_path: '/ws/app/review.py' });
    assert.equal(form.defaultRunnerForTab({ name: 'calc_test.go', path: '/x/calc_test.go', language: 'go' }), 'go_test');
    assert.equal(form.defaultRunnerForTab({ name: 'test_api.py', path: '/x/test_api.py', language: 'python' }), 'pytest');
    assert.deepEqual(plain(form.draftDefaultsForTab(readme)), { name: '', runner: 'custom', target_path: '' });
    assert.equal(form.targetFieldCopy('python').label, 'Script');
  });
});

describe('I2-19 / I2-23 / I2-12 session-scoped state', () => {
  const { sync } = modules();

  it('a new session id clears REPL results and watch values, keeping the expressions', () => {
    const host = {
      replEntries: [{ id: 'r1', expression: 'order["meta"]', result: '{}' }],
      watches: [{ id: 'w1', expression: 'total', value: '2', var_type: 'int', error: null }],
    };
    sync.resetForNewDebugSession(host, { id: 'dbg_1' }, { id: 'dbg_1' });
    assert.equal(host.replEntries.length, 1, 'same session: untouched');
    sync.resetForNewDebugSession(host, { id: 'dbg_1' }, { id: 'dbg_2' });
    assert.equal(host.replEntries.length, 0);
    assert.deepEqual(plain(host.watches), [{ id: 'w1', expression: 'total', value: '…', var_type: null, error: null }]);
  });

  it('the editor follows the selected frame, else the stop', () => {
    const paused = { state: 'paused', paused_file: '/a/review.py', paused_line: 11 };
    assert.deepEqual(plain(sync.debugRevealTarget(paused)), { path: '/a/review.py', line: 11, key: '/a/review.py:11' });
    assert.equal(
      sync.debugRevealTarget({ ...paused, selected_frame_file: '/a/caller.py', selected_frame_line: 18 })?.key,
      '/a/caller.py:18',
    );
    assert.equal(sync.debugRevealTarget({ ...paused, state: 'running' }), null);
  });

  it('normalizes the selected frame path like the paused one', () => {
    const host = { target: { kind: 'remote' }, projectRoots: [], resolvePath: (p: string) => `/root/${p}` };
    const { snap } = sync.normalizeIncomingDebugSnapshot(host, {
      id: 'd', state: 'paused', paused_file: 'a.py', paused_line: 1, selected_frame_file: 'b.py', selected_frame_line: 2,
      stack_frames: [], breakpoints: [], output: [],
    });
    assert.equal(snap.selected_frame_file, '/root/b.py');
    assert.equal(snap.paused_file, '/root/a.py');
  });

  it('run output lines are numbered from the daemon’s running count', () => {
    assert.equal(sync.runOutputSeqBase({ output_seq: 2500, output: new Array(2000).fill('x') }), 501);
    assert.equal(sync.runOutputSeqBase({ output: ['a', 'b'] }), 1);
  });
});
