// Regression tests for Wave 1 IDE MUST fixes (compare edit debouncer, collab room_closed,
// stale target guards across debug/env/dbwatch/git, per-path collab edit timers, multi-repo
// setLocalTarget guard, and split-pane path-targeted formatting).

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { CompareEditDebouncer } from '../src/modules/ide/history/ideCompareEngine.ts';
import { deferred, loadSource } from './sourceHarness.ts';

class ApiError extends Error {
  status: number;
  constructor(status: number) {
    super(`HTTP ${status}`);
    this.status = status;
  }
}

function loadDebugAndEnv(api: Record<string, unknown>, extra: {
  streamIdeSse?: (...args: unknown[]) => Promise<void>;
  pollWhileVisible?: (fn: (signal: AbortSignal) => void | Promise<void>) => { stop: () => void };
} = {}) {
  const toastLog: string[] = [];
  let sseCalls = 0;
  let pollCalls = 0;
  let lastSseCb: ((raw: string) => void) | null = null;
  let lastSseSignal: AbortSignal | null = null;
  let lastPollFn: ((signal: AbortSignal) => void | Promise<void>) | null = null;
  let lastPollStopped = false;
  const cache = new Map();
  const imports = {
    svelte: { tick: async () => {} },
    '../../../lib/api/client': { api, ApiError, getToken: () => '', laneFetch: async () => new Response('{}') },
    '../../../lib/idePreferences.svelte': { idePreferences: { current: { moduleSdkMap: {} } } },
    '../../../lib/loadError': { loadErrorText: (e: unknown) => (e instanceof Error ? e.message : String(e)) },
    '../../../lib/poll': {
      pollWhileVisible: extra.pollWhileVisible ?? ((fn: (signal: AbortSignal) => void | Promise<void>) => {
        pollCalls += 1;
        lastPollFn = fn;
        lastPollStopped = false;
        return { stop: () => { lastPollStopped = true; } };
      }),
    },
    '../../../lib/stores/auth.svelte': { auth: { isRoot: true } },
    '../../../lib/toast.svelte': {
      toasts: {
        success: (t: string) => toastLog.push(`ok:${t}`),
        error: (t: string) => toastLog.push(`err:${t}`),
        info: (t: string) => toastLog.push(`info:${t}`),
        push: () => {},
      },
    },
    '../../../lib/toastError': { toastError: (t: string) => toastLog.push(`err:${t}`) },
    '../../../lib/storage': { lsGet: () => null, lsSet: () => {}, lsRemove: () => {} },
    '../../../lib/confirm.svelte': { confirmer: { ask: async () => true, choose: async () => ({ value: null }) } },
    '../../../lib/ideProjectsModel': {
      loadNamedIdeProjectsLocal: () => [],
      saveNamedIdeProjectsLocal: () => {},
      fetchNamedIdeProjectsFromServer: async () => [],
      parseNamedIdeProject: () => null,
    },
    '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: { rootsForLocal: () => [] } },
    '../../../lib/components/cm-ide-drafts': { readIdeUndoDraft: () => null, removeIdeUndoDraft: () => {}, writeIdeUndoDraftImmediate: () => {} },
    '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({ autoSave: false }) },
    './ideTypes': (() => {
      const real = loadSource(new URL('../src/modules/ide/store/ideTypes.ts', import.meta.url), {
        '../../../lib/api/client': { getToken: () => '', laneFetch: async () => new Response('{}') },
      });
      return {
        ...real,
        streamIdeSse: extra.streamIdeSse ?? ((_p: string, _h: unknown, sig: AbortSignal, cb: (raw: string) => void) => {
          sseCalls += 1;
          lastSseSignal = sig;
          lastSseCb = cb;
          return new Promise<void>(() => {});
        }),
      };
    })(),
  };
  const ctl = loadSource(
    new URL('../src/modules/ide/store/ideDebugAndEnvController.ts', import.meta.url),
    imports,
    {},
    { relative: true },
    cache,
  );
  const sync = loadSource(
    new URL('../src/modules/ide/store/ideDebugSnapshotSync.ts', import.meta.url),
    imports,
    {},
    { relative: true },
    cache,
  );
  return {
    ctl,
    sync,
    toastLog,
    get sseCalls() { return sseCalls; },
    get pollCalls() { return pollCalls; },
    get lastSseCb() { return lastSseCb; },
    get lastSseSignal() { return lastSseSignal; },
    get lastPollFn() { return lastPollFn; },
    get lastPollStopped() { return lastPollStopped; },
  };
}

function makeDebugHost(root = '/repo-a') {
  return {
    target: { kind: 'local' as const, workspace_id: null, root, connection_id: null, connection_name: null },
    get targetPayload() { return { ...this.target, project_folders: [this.target.root] }; },
    get projectRoots() { return [{ path: this.target.root, name: 'repo', isPrimary: true }]; },
    targetOs: '',
    targetArch: '',
    activePath: `${root}/main.go`,
    activeTab: { path: `${root}/main.go`, name: 'main.go', content: 'package main', savedContent: 'package main', language: 'go', readOnly: false, truncated: false, loading: false, error: null, preview: false },
    toolchains: [],
    detectedSdks: [],
    toolchainsLoading: false,
    toolchainsError: null,
    installJobs: [],
    projectEnv: null,
    effectiveEnv: {},
    shellEnv: null,
    shellEnvLoading: false,
    shellEnvAttempted: false,
    shellEnvError: null,
    pythonVenvs: [],
    dotenvFiles: [],
    runConfigs: [],
    detectedConfigs: [],
    selectedConfigId: null,
    envState: { gutterTarget: null },
    envLoading: false,
    envError: null,
    activeRun: null,
    runsHistory: [],
    allBreakpoints: [],
    effectiveBreakpoints: [],
    breakOnExceptions: false,
    activeDebug: null,
    expandedVariables: {},
    watches: [],
    replEntries: [],
    debugLoading: false,
    debugStreamActive: false,
    lastDebugLaunch: null,
    activeMavenProfiles: [],
    activeSpringProfiles: [],
    buildSkipTests: false,
    buildOffline: false,
    collabIsGuest: false,
    collabRoom: null,
    collabPeerId: null,
    collabPeerSecret: null,
    collabGuestToken: null,
    targetQuery: () => `kind=local&root=${encodeURIComponent(root)}`,
    openBottomTab: () => {},
    openFile: async () => {},
  };
}

// ---------------------------------------------------------------------------
// Fix 1: CompareEditDebouncer captures exact destPath and never overwrites across files
// ---------------------------------------------------------------------------

test('Fix 1: CompareEditDebouncer flushes previous file to its own destPath when switching files', (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const applied: Array<{ text: string; reason: string; destPath: string }> = [];
  const debouncer = new CompareEditDebouncer((text, reason, destPath) => {
    applied.push({ text, reason, destPath });
  }, 40);

  debouncer.schedule('/repo/a.ts', 'edited A');
  assert.equal(debouncer.hasPending(), true);

  // Switch to file B before the timer fires: A must flush to /repo/a.ts, never /repo/b.ts.
  debouncer.schedule('/repo/b.ts', 'edited B');
  assert.deepEqual(applied, [{ text: 'edited A', reason: 'Edited Current', destPath: '/repo/a.ts' }]);

  t.mock.timers.tick(40);
  assert.deepEqual(applied, [
    { text: 'edited A', reason: 'Edited Current', destPath: '/repo/a.ts' },
    { text: 'edited B', reason: 'Edited Current', destPath: '/repo/b.ts' },
  ]);
  assert.equal(debouncer.hasPending(), false);
});

test('Fix 1: CompareEditDebouncer flush on unmount applies once and cancel prevents stale fire', (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const applied: Array<{ text: string; destPath: string }> = [];
  const debouncer = new CompareEditDebouncer((text, _reason, destPath) => {
    applied.push({ text, destPath });
  }, 40);

  debouncer.schedule('/repo/a.ts', 'draft A');
  debouncer.flush();
  debouncer.flush(); // idempotent
  assert.deepEqual(applied, [{ text: 'draft A', destPath: '/repo/a.ts' }]);

  debouncer.schedule('/repo/a.ts', 'cancelled draft');
  debouncer.cancel();
  t.mock.timers.tick(60);
  assert.deepEqual(applied, [{ text: 'draft A', destPath: '/repo/a.ts' }]);
});

// ---------------------------------------------------------------------------
// Fix 2: Collab room_closed clears cursors/following and aborts stream
// ---------------------------------------------------------------------------

test('Fix 2: room_closed clears collabRoom, peerCursors, followingPeerId, sets error, and aborts stream', () => {
  let onEvent: ((raw: string) => void) | null = null;
  const ctl = loadSource(new URL('../src/modules/ide/store/ideCollabController.ts', import.meta.url), {
    '../../../lib/api/client': { api: {}, laneFetch: async () => new Response('{}') },
    '../../../lib/loadError': { loadErrorText: (e: unknown) => String(e) },
    '../../../lib/toast.svelte': { toasts: { success() {}, error() {}, info() {} } },
    '../../../lib/toastError': { toastError() {} },
    './ideCollabNotices.svelte': { ideCollabNotices: { reset() {} } },
    './ideCollabSync': { mergeText: (_b: string, _m: string, t: string) => t },
    './ideTypes': {
      collabGuestPost: async () => ({}),
      ideBasename: (p: string) => p,
      streamIdeSse: (_p: string, _h: unknown, _s: unknown, cb: (raw: string) => void) => {
        onEvent = cb;
        return Promise.resolve();
      },
    },
  });
  const store = {
    collabIsGuest: true,
    collabRoom: { id: 'r1', title: 'Pairing', target: { kind: 'local', root: '/repo' }, default_role: 'editor', peers: [] },
    peerCursors: [{ peer_id: 'host', path: '/repo/a.ts', active_line: 4 }],
    followingPeerId: 'host',
    collabError: null,
    tabs: [],
    sharedBuffers: {},
  } as Record<string, unknown>;
  const abortCtl = ctl.attachCollabEventStream(store, 'r1', 'tok', 'guest-1', null) as AbortController;
  onEvent!(JSON.stringify({ type: 'room_closed' }));
  assert.equal(store.collabRoom, null);
  assert.equal((store.peerCursors as unknown[]).length, 0);
  assert.equal(store.followingPeerId, null);
  assert.equal(store.collabError, 'The host ended this shared session.');
  assert.equal(abortCtl.signal.aborted, true);
});

// ---------------------------------------------------------------------------
// Fix 3a: Stale target guards in ideDebugAndEnvController & ideDebugSnapshotSync
// ---------------------------------------------------------------------------

test('Fix 3a: fetchToolchains and fetchShellEnv discard stale responses after target switch', async () => {
  const tcDef = deferred<{ target_os: string; target_arch: string; ecosystems: unknown[]; jobs: unknown[] }>();
  const envDef = deferred<{ shell: string; vars: Record<string, string> }>();
  const mod = loadDebugAndEnv({
    long: { get: () => tcDef.promise },
    get: () => envDef.promise,
  });
  const host = makeDebugHost('/repo-a');
  let ensuredJobs = 0;

  const pTc = mod.ctl.fetchToolchains(host, () => { ensuredJobs += 1; });
  const pEnv = mod.ctl.fetchShellEnv(host);

  // Switch target before responses arrive.
  host.target = { kind: 'local', workspace_id: null, root: '/repo-b', connection_id: null, connection_name: null };
  host.toolchainsLoading = false;
  host.shellEnvLoading = false;

  tcDef.resolve({ target_os: 'darwin', target_arch: 'arm64', ecosystems: [{ ecosystem: 'go', installed: [], detected: [] }], jobs: [] });
  envDef.resolve({ shell: '/bin/zsh', vars: { FOO: 'from-a' } });

  await Promise.all([pTc, pEnv]);
  assert.deepEqual(host.toolchains, []);
  assert.equal(host.targetOs, '');
  assert.equal(ensuredJobs, 0);
  assert.equal(host.shellEnv, null);
});

test('Fix 3a: launchIdeRun aborts stale response after target switch and guards live SSE callbacks', async () => {
  const postDef = deferred<{ run: { id: string; status: string; output: string[] } }>();
  const mod = loadDebugAndEnv({ post: () => postDef.promise });
  const host = makeDebugHost('/repo-a');
  let prevStopped = false;
  const prevSse = new AbortController();

  const pRun = mod.ctl.launchIdeRun(
    host,
    { adhoc: { id: 'adhoc', name: 'Run A', runner: 'go_run', args: [] } },
    { stop: () => { prevStopped = true; } },
    prevSse,
  );
  // Switch target while POST /ide/runs is in flight.
  host.target = { kind: 'local', workspace_id: null, root: '/repo-b', connection_id: null, connection_name: null };
  postDef.resolve({ run: { id: 'run-a', status: 'running', output: ['hi'] } });
  const res = await pRun;

  assert.equal(prevStopped, true);
  assert.equal(prevSse.signal.aborted, true);
  assert.equal(res.poller, null);
  assert.equal(res.sseCtl, null);
  assert.equal(host.activeRun, null);
  assert.equal(mod.sseCalls, 0);
  assert.equal(mod.pollCalls, 0);

  // Now launch on /repo-b, then switch to /repo-c and fire an SSE message from /repo-b.
  const postDef2 = deferred<{ run: { id: string; status: string; output: string[] } }>();
  const mod2 = loadDebugAndEnv({ post: () => postDef2.promise });
  const pRun2 = mod2.ctl.launchIdeRun(
    host,
    { adhoc: { id: 'adhoc', name: 'Run B', runner: 'go_run', args: [] } },
    null,
    null,
  );
  postDef2.resolve({ run: { id: 'run-b', status: 'running', output: ['from-b'] } });
  const res2 = await pRun2;
  assert.ok(res2.sseCtl);
  assert.equal((host.activeRun as { id: string } | null)?.id, 'run-b');

  // Switch to /repo-c and clear activeRun as target reset would.
  host.target = { kind: 'local', workspace_id: null, root: '/repo-c', connection_id: null, connection_name: null };
  host.activeRun = null;

  mod2.lastSseCb!(JSON.stringify({ run: { id: 'run-b', status: 'running', output: ['late-from-b'] } }));
  assert.equal(host.activeRun, null, 'stale SSE callback must not overwrite new target activeRun');
  assert.equal(mod2.lastSseSignal?.aborted, true, 'stale SSE callback must abort stream');
  assert.equal(mod2.lastPollStopped, true, 'stale SSE callback must stop poller');
});

test('Fix 3a: launchDebugSession and attachDebugSessionStream guard against target switches', async () => {
  const postDef = deferred<{ session: { id: string; state: string; output: string[] } }>();
  const mod = loadDebugAndEnv({ post: () => postDef.promise });
  const host = makeDebugHost('/repo-a');
  const prevCtl = new AbortController();
  let snapshots = 0;

  const pDbg = mod.ctl.launchDebugSession(
    host,
    { language: 'go', launch: { program: '/repo-a/main.go' } },
    prevCtl,
    () => { snapshots += 1; },
  );
  host.target = { kind: 'local', workspace_id: null, root: '/repo-b', connection_id: null, connection_name: null };
  postDef.resolve({ session: { id: 'dbg-a', state: 'running', output: [] } });
  const ctl = await pDbg;

  assert.equal(prevCtl.signal.aborted, true);
  assert.equal(ctl, null);
  assert.equal(snapshots, 0);
  assert.equal(host.lastDebugLaunch, null);
  assert.equal(mod.sseCalls, 0);

  // Also test attachDebugSessionStream aborting when target switches mid-stream.
  const streamCtl = mod.sync.attachDebugSessionStream(host, 'dbg-b', null, () => { snapshots += 1; }) as AbortController;
  host.target = { kind: 'local', workspace_id: null, root: '/repo-c', connection_id: null, connection_name: null };
  mod.lastSseCb!(JSON.stringify({ session: { id: 'dbg-b', state: 'paused', output: [] } }));
  assert.equal(snapshots, 0);
  assert.equal(streamCtl.signal.aborted, true);
});

// ---------------------------------------------------------------------------
// Fix 3c: ideDbWatchState reset() + generation guard + onIdeTargetReset hook
// ---------------------------------------------------------------------------

test('Fix 3c: ideDbWatch registers onIdeTargetReset, clears state, and discards in-flight calls on reset', async () => {
  const resetHooks: Array<() => void> = [];
  const detectDef = deferred<{ detected_objects: unknown[]; saved_connections: unknown[]; ephemeral_targets: unknown[] }>();
  const snapDef = deferred<{ snapshot: { captured_at_ms: number; columns: string[]; rows: unknown[][] }; diff: null }>();
  const mod = loadSource(
    new URL('../src/modules/ide/dbwatch/ideDbWatchState.svelte.ts', import.meta.url),
    {
      '../../../lib/api/client': {
        api: {
          post: (url: string) => (url.includes('/detect') ? detectDef.promise : snapDef.promise),
        },
      },
      '../store/ideStateSlices.svelte': {
        onIdeTargetReset: (fn: () => void) => resetHooks.push(fn),
      },
    },
    {},
    { relative: true },
  );
  const watch = mod.ideDbWatch;
  assert.equal(resetHooks.length, 1, 'must register reset hook with onIdeTargetReset');

  watch.selectedObjectName = 'users';
  watch.liveWatch = true;
  watch.savedConnections = [{ target_kind: 'saved', connection_id: 'c1', name: 'MySQL', engine: 'mysql', host: '127.0.0.1', port: 3306, environment: 'dev', database: 'app' }];
  watch.selectedTargetKey = 'saved:c1';
  watch.baselineSnapshot = { captured_at_ms: 1, columns: ['id'], rows: [[1]] };

  const pDetect = watch.detect({ root: '/repo-a' });
  const pSnap = watch.captureSnapshot('/repo-a', 'diff');

  // Target switches: reset hook fires while detect & snapshot are in flight.
  resetHooks[0]();
  assert.equal(watch.selectedObjectName, '');
  assert.equal(watch.baselineSnapshot, null);
  assert.equal(watch.latestResp, null);
  assert.equal(watch.liveWatch, false);
  assert.equal(watch.detectLoading, false);
  assert.equal(watch.snapshotLoading, false);

  detectDef.resolve({
    detected_objects: [{ engine: 'mysql', object_name: 'stale_table' }],
    saved_connections: [],
    ephemeral_targets: [],
  });
  snapDef.resolve({ snapshot: { captured_at_ms: 2, columns: ['id'], rows: [[2]] }, diff: null });

  await pDetect;
  const snapOk = await pSnap;
  assert.equal(snapOk, false);
  assert.equal(watch.detectedObjects.length, 0, 'stale detect must not write after reset');
  assert.equal(watch.selectedObjectName, '', 'stale detect must not re-select object after reset');
  assert.equal(watch.latestResp, null, 'stale snapshot must not write after reset');
});

// ---------------------------------------------------------------------------
// Fix 3e: ideGitController generation counter discards stale probes on reset()
// ---------------------------------------------------------------------------

test('Fix 3e: ideGitController discards in-flight and queued root probes after reset()', async () => {
  const resetHooks: Array<() => void> = [];
  const detectDefs = new Map<string, ReturnType<typeof deferred<unknown>>>();
  const postCalls: string[] = [];
  const upserted: string[] = [];
  const mod = loadSource(
    new URL('../src/modules/ide/git/ideGitController.svelte.ts', import.meta.url),
    {
      '../../../lib/api/client': {
        api: {
          bg: {
            post: (_url: string, body: { path: string }) => {
              postCalls.push(body.path);
              const d = deferred();
              detectDefs.set(body.path, d);
              return d.promise;
            },
            get: async () => { throw new ApiError(404); },
          },
        },
        ApiError,
      },
      '../../../lib/api/git': { gitApi: {} },
      '../../../lib/fileUri': { isBroadNonProjectRootClient: () => false, normalizeMacPrivatePath: (p: string) => p },
      '../../../lib/loadError': { loadErrorText: (e: unknown) => String(e) },
      '../../../lib/stores/git.svelte': {
        git: {
          repos: [],
          allRepos: [],
          statusById: {},
          liveRev: {},
          refsRev: {},
          ensureStatus: () => {},
          upsertRepo: (r: { id: string }) => upserted.push(r.id),
          refreshStatus: async () => {},
        },
      },
      '../../../lib/stores/workspace.svelte': {
        SCRATCH_WORKSPACE_ID: '__scratch__',
        ws: { current: { id: 'ws-1' }, workspaces: [{ id: 'ws-1' }] },
      },
      '../../../lib/toastError': { toastError: () => {} },
      '../ideStore.svelte': {
        ide: {
          target: { kind: 'local', workspace_id: 'ws-1', root: '/repo-a' },
          projectRoots: [{ path: '/repo-a', name: 'repo-a', isPrimary: true }],
          tabs: [],
          activePath: null,
          expandedDirs: {},
          treeRootEntries: [],
          childrenByDir: {},
          resolvePath: (p: string) => p,
        },
        ideBasename: (p: string) => p.split('/').pop() ?? p,
      },
      '../store/ideStateSlices.svelte': { onIdeTargetReset: (fn: () => void) => resetHooks.push(fn) },
      '../store/ideTypes': { detectTextEol: () => '\n', diskEolsOf: () => undefined, normalizeEditorLf: (s: string) => s },
      './ideGitTreeOverlay': { buildIdeTreeGitOverlay: () => ({}) },
    },
  );
  const gitCtl = mod.ideGit;
  assert.equal(resetHooks.length, 1);

  // Start 4 probes so 3 are active and the 4th sits in `probeQueue`.
  const p1 = gitCtl.ensureRootDetected('/repo-a/r1');
  const p2 = gitCtl.ensureRootDetected('/repo-a/r2');
  const p3 = gitCtl.ensureRootDetected('/repo-a/r3');
  const p4 = gitCtl.ensureRootDetected('/repo-a/r4');
  assert.deepEqual(postCalls, ['/repo-a/r1', '/repo-a/r2', '/repo-a/r3']);

  // Switch project -> reset() bumps targetGen and clears inFlightRoots.
  gitCtl.reset();

  for (const [path, d] of detectDefs.entries()) {
    d.resolve({ id: `id:${path}`, workspace_id: 'ws-1', name: path, path, remote_url: null, provider: 'git', git_account_id: null, created_at: '' });
  }
  const results = await Promise.all([p1, p2, p3, p4]);
  assert.deepEqual(results, [null, null, null, null]);
  assert.equal(Object.keys(gitCtl.repoByRoot).length, 0, 'stale probes must not write into repoByRoot');
  assert.deepEqual(upserted, [], 'stale probes must not upsert into git store');
  assert.deepEqual(postCalls, ['/repo-a/r1', '/repo-a/r2', '/repo-a/r3'], 'queued probe r4 must not fire network call after reset');
});

// ---------------------------------------------------------------------------
// Fix 4: Per-path collab edit timers & multi-repo setLocalTarget guard
// ---------------------------------------------------------------------------

test('Fix 4: IdeBufferPerfController debounces collab edits independently per path and clears on eviction/reset', (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const bufMod = loadSource(
    new URL('../src/modules/ide/store/ideBufferPerf.ts', import.meta.url),
    {
      svelte: { tick: async () => {} },
      '../../../lib/api/client': { api: {}, ApiError, getToken: () => '', laneFetch: async () => new Response('{}') },
      '../../../lib/components/cm-ide-drafts': { readIdeUndoDraft: () => null, removeIdeUndoDraft: () => {}, writeIdeUndoDraftImmediate: () => {} },
      '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({ autoSave: false }) },
      '../../../lib/confirm.svelte': { confirmer: { ask: async () => true, choose: async () => ({ value: null }) } },
      '../../../lib/toast.svelte': { toasts: { success() {}, error() {}, info() {} } },
      '../../../lib/toastError': { toastError() {} },
      '../../../lib/ideProjectsModel': { loadNamedIdeProjectsLocal: () => [], saveNamedIdeProjectsLocal: () => {}, fetchNamedIdeProjectsFromServer: async () => [], parseNamedIdeProject: () => null },
      '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: { rootsForLocal: () => [] } },
      '../../../lib/storage': { lsGet: () => null, lsSet: () => {}, lsRemove: () => {} },
    },
    { setTimeout: globalThis.setTimeout, clearTimeout: globalThis.clearTimeout },
    { relative: true },
  );
  const perf = new bufMod.IdeBufferPerfController();
  const sent: string[] = [];

  // Rapid edits to A and B must BOTH fire (no cross-file drop), while repeated edits to A coalesce.
  perf.scheduleCollabEdit('/repo/a.ts', () => sent.push('a-1'), 25);
  perf.scheduleCollabEdit('/repo/b.ts', () => sent.push('b-1'), 25);
  perf.scheduleCollabEdit('/repo/a.ts', () => sent.push('a-2'), 25);

  t.mock.timers.tick(25);
  assert.deepEqual(sent.sort(), ['a-2', 'b-1']);

  // Evicting A cancels A's pending timer while B still fires.
  sent.length = 0;
  const fakeHost = { gutterByPath: {}, symbolsByPath: {}, problems: [] };
  perf.scheduleCollabEdit('/repo/a.ts', () => sent.push('a-evicted'), 25);
  perf.scheduleCollabEdit('/repo/b.ts', () => sent.push('b-kept'), 25);
  perf.evictPaths(fakeHost, ['/repo/a.ts']);
  t.mock.timers.tick(25);
  assert.deepEqual(sent, ['b-kept']);

  // clearAllCollabEditTimers cancels all pending timers.
  sent.length = 0;
  perf.scheduleCollabEdit('/repo/c.ts', () => sent.push('c-cleared'), 25);
  perf.clearAllCollabEditTimers();
  t.mock.timers.tick(50);
  assert.deepEqual(sent, []);
});

test('Fix 4: shouldSkipLocalTargetSwitch allows switching from multi-repo/named project to standalone primary root', () => {
  const helpers = loadSource(
    new URL('../src/modules/ide/shell/ideProjectHelpers.ts', import.meta.url),
    {
      svelte: { tick: async () => {} },
      '../../../lib/api/client': { api: {}, ApiError, getToken: () => '', laneFetch: async () => new Response('{}') },
      '../../../lib/components/cm-ide-drafts': { readIdeUndoDraft: () => null, removeIdeUndoDraft: () => {}, writeIdeUndoDraftImmediate: () => {} },
      '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({ autoSave: false }) },
      '../../../lib/confirm.svelte': { confirmer: { ask: async () => true, choose: async () => ({ value: null }) } },
      '../../../lib/toast.svelte': { toasts: { success() {}, error() {}, info() {} } },
      '../../../lib/toastError': { toastError() {} },
      '../../../lib/ideProjectsModel': { loadNamedIdeProjectsLocal: () => [], saveNamedIdeProjectsLocal: () => {}, fetchNamedIdeProjectsFromServer: async () => [], parseNamedIdeProject: () => null },
      '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: { rootsForLocal: () => [] } },
      '../../../lib/storage': { lsGet: () => null, lsSet: () => {}, lsRemove: () => {} },
    },
    {},
    { relative: true },
  );

  assert.equal(
    helpers.shouldSkipLocalTargetSwitch(
      { target: { kind: 'local', root: '/repo-a' }, activeProjectId: null, additionalRoots: [] },
      '/repo-a',
      {},
    ),
    true,
    'standalone same root can skip',
  );
  assert.equal(
    helpers.shouldSkipLocalTargetSwitch(
      { target: { kind: 'local', root: '/repo-a' }, activeProjectId: 'proj-1', additionalRoots: [] },
      '/repo-a',
      {},
    ),
    false,
    'named project with same primary root must NOT skip',
  );
  assert.equal(
    helpers.shouldSkipLocalTargetSwitch(
      { target: { kind: 'local', root: '/repo-a' }, activeProjectId: null, additionalRoots: ['/repo-b'] },
      '/repo-a',
      {},
    ),
    false,
    'multi-root project with additionalRoots must NOT skip',
  );
});

// ---------------------------------------------------------------------------
// Fix 6: runFormatActiveFile formats explicit split-pane path instead of activeTab
// ---------------------------------------------------------------------------

test('Fix 6: runFormatActiveFile with explicit path formats the split pane tab rather than activeTab', async () => {
  const formattedPaths: string[] = [];
  const appliedEdits: Array<{ path: string; text: string }> = [];
  const cache = new Map();
  const lsp = loadSource(
    new URL('../src/modules/ide/shell/ideLspAndSearchHelpers.ts', import.meta.url),
    {
      svelte: { tick: async () => {} },
      '../../../lib/api/client': {
        api: {
          post: async (_url: string, body: { path: string; content: string }) => {
            formattedPaths.push(body.path);
            return { formatted: `${body.content}\n// formatted`, changed: true, formatter: 'gofmt' };
          },
        },
        ApiError,
        getToken: () => '',
        laneFetch: async () => new Response('{}'),
      },
      '../../../lib/components/cm-ide-drafts': { readIdeUndoDraft: () => null, removeIdeUndoDraft: () => {}, writeIdeUndoDraftImmediate: () => {} },
      '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({ autoSave: false }) },
      '../../../lib/confirm.svelte': { confirmer: { ask: async () => true, choose: async () => ({ value: null }) } },
      '../../../lib/toast.svelte': { toasts: { success() {}, error() {}, info() {} } },
      '../../../lib/toastError': { toastError() {} },
      '../../../lib/ideProjectsModel': { loadNamedIdeProjectsLocal: () => [], saveNamedIdeProjectsLocal: () => {}, fetchNamedIdeProjectsFromServer: async () => [], parseNamedIdeProject: () => null },
      '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: { rootsForLocal: () => [] } },
      '../../../lib/storage': { lsGet: () => null, lsSet: () => {}, lsRemove: () => {} },
    },
    {},
    { relative: true },
    cache,
  );
  const tabA = { path: '/repo/a.go', name: 'a.go', content: 'package a', savedContent: 'package a', language: 'go', readOnly: false, truncated: false, loading: false, error: null, preview: false };
  const tabB = { path: '/repo/b.go', name: 'b.go', content: 'package b', savedContent: 'package b', language: 'go', readOnly: false, truncated: false, loading: false, error: null, preview: false };
  const host = {
    targetPayload: { kind: 'local', root: '/repo' },
    tabs: [tabA, tabB],
    activeTab: tabA,
    applyAgentEdit: async (path: string, text: string) => {
      appliedEdits.push({ path, text });
      return true;
    },
  };
  const bufPerf = {
    flush: (_h: unknown, p: string) => (p === '/repo/b.go' ? tabB.content : tabA.content),
    getBuffer: (_h: unknown, p: string) => (p === '/repo/b.go' ? tabB.content : tabA.content),
  };

  await lsp.runFormatActiveFile(host, bufPerf, { path: '/repo/b.go' });
  assert.deepEqual(formattedPaths, ['/repo/b.go']);
  assert.deepEqual(appliedEdits, [{ path: '/repo/b.go', text: 'package b\n// formatted' }]);
});
