// Several projects in the IDE (review I3): every project you opened can be found
// again, same-name folders are told apart, what follows you back is per project
// (bounded), and a run left running in another project is re-adopted, not lost.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { loadSource } from './sourceHarness.ts';

function memStorage() {
  const m = new Map<string, string>();
  return {
    m,
    lsGet: (k: string) => m.get(k) ?? null,
    lsSet: (k: string, v: string) => void m.set(k, v),
    lsRemove: (k: string) => void m.delete(k),
    ssGet: (k: string) => m.get(`ss:${k}`) ?? null,
    ssSet: (k: string, v: string) => void m.set(`ss:${k}`, v),
    ssRemove: (k: string) => void m.delete(`ss:${k}`),
  };
}

function loadRecents(storage = memStorage()) {
  const mod = loadSource(new URL('../src/modules/ide/shell/ideProjectRecents.svelte.ts', import.meta.url), {
    '../../../lib/storage': storage,
  });
  return { mod, storage };
}

// ── I3-05: recents ──────────────────────────────────────────────────────────

test('I3-05: a folder opened by path is recorded and listed in the switcher after leaving it', () => {
  const { mod, storage } = loadRecents();
  const r = mod.ideProjectRecents;
  r.record({ kind: 'local', root: '/w/loose-folder/', workspace_id: null });
  r.record({ kind: 'local', root: '/w/alpha', workspace_id: 'ws_alpha' });
  assert.deepEqual([...r.list.map((x: { root: string }) => x.root)], ['/w/alpha', '/w/loose-folder'], 'MRU first, trailing slash dropped');
  const rows = mod.ideSwitcherFolders([{ id: 'ws_alpha', name: 'alpha', root_path: '/w/alpha' }], r.list);
  assert.deepEqual([...rows.map((x: { name: string }) => x.name)], ['alpha', 'loose-folder']);
  assert.equal(mod.realWorkspaceId(rows[1].id), null, 'a recent folder has no shell workspace behind it');
  assert.equal(mod.realWorkspaceId('ws_alpha'), 'ws_alpha');
  // Survives a reload: a fresh module reads the same storage.
  const again = loadRecents(storage).mod;
  assert.equal(again.ideProjectRecents.list.length, 2);
});

test('I3-05: an SSH target is recorded with what reopens it and never a secret', () => {
  const { mod, storage } = loadRecents();
  mod.ideProjectRecents.record({ kind: 'ssh', root: '/srv/app', connection_id: 'conn_1', connection_name: 'prod-box', password: 'hunter2' } as never);
  mod.ideProjectRecents.record({ kind: 'ssh', root: '/srv/app', connection_id: null } as never);
  const saved = storage.m.get(mod.IDE_RECENTS_KEY) ?? '';
  assert.doesNotMatch(saved, /hunter2|password/);
  const [entry] = JSON.parse(saved);
  assert.deepEqual(Object.keys(entry).sort(), ['connection_id', 'connection_name', 'kind', 'lastOpenedAt', 'root', 'workspace_id']);
  assert.equal(entry.connection_id, 'conn_1');
  assert.equal(JSON.parse(saved).length, 1, 'an SSH target without a connection id is not recorded');
  const rows = mod.ideKnownProjects([], [], mod.ideProjectRecents.list);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].title, 'prod-box:app');
  const calls: unknown[] = [];
  rows[0].open({ activateProject: async () => {}, setLocalTarget: async () => {}, setSshTarget: async (...a: unknown[]) => { calls.push(a); } });
  assert.deepEqual(calls, [['conn_1', '/srv/app', 'prod-box']]);
});

test('I3-05: the list is bounded and de-duplicated across /var ≡ /private/var', () => {
  const { mod } = loadRecents();
  for (let i = 0; i < 30; i++) mod.ideProjectRecents.record({ kind: 'local', root: `/w/p${i}` });
  assert.equal(mod.ideProjectRecents.list.length, mod.MAX_RECENTS);
  mod.ideProjectRecents.record({ kind: 'local', root: '/private/var/folders/x/api' });
  mod.ideProjectRecents.record({ kind: 'local', root: '/var/folders/x/api' });
  assert.equal(mod.ideProjectRecents.list.filter((r: { root: string }) => r.root.endsWith('/x/api')).length, 1);
  mod.ideProjectRecents.remove({ kind: 'local', root: '/var/folders/x/api' });
  assert.equal(mod.ideProjectRecents.has({ kind: 'local', root: '/private/var/folders/x/api' }), false);
});

// ── I3-07: labels ───────────────────────────────────────────────────────────

test('I3-07: same-name folders get the shortest distinguishing parent; unique names stay', () => {
  const { mod } = loadRecents();
  const labels = mod.disambiguateLabels([
    { name: 'api', path: '/w/team-a/api' },
    { name: 'api', path: '/w/team-b/api' },
    { name: 'web', path: '/w/web' },
  ]);
  assert.deepEqual([...labels], ['api — team-a', 'api — team-b', 'web']);
  const deep = mod.disambiguateLabels([
    { name: 'api', path: '/a/x/svc/api' },
    { name: 'api', path: '/b/x/svc/api' },
  ]);
  assert.deepEqual([...deep], ['api — a/x/svc', 'api — b/x/svc']);
  const sameFolder = mod.disambiguateLabels([
    { name: 'api', path: '/private/var/f/api' },
    { name: 'api', path: '/var/f/api' },
  ]);
  assert.deepEqual([...sameFolder], ['api', 'api'], 'two spellings of one folder are not "different"');
  assert.equal(mod.ideFolderLabel('/w/team-b/api', [{ id: '1', name: 'api', root_path: '/w/team-a/api' }, { id: '2', name: 'api', root_path: '/w/team-b/api' }]), 'api — team-b');
});

test('I3-21: one canonical root spelling', () => {
  const { mod } = loadRecents();
  assert.equal(mod.sameRootKey('/private/var/folders/a/'), '/var/folders/a');
  assert.equal(mod.sameRootKey('/private/tmp/x'), '/tmp/x');
  assert.equal(mod.sameRootKey('/Users/me/code/'), '/Users/me/code');
});

test('I3-06: one palette row per project you can reopen', () => {
  const { mod } = loadRecents();
  mod.ideProjectRecents.record({ kind: 'local', root: '/w/loose' });
  const rows = mod.ideKnownProjects(
    [{ id: 'p1', name: 'Payments', primary_root: '/w/core', folders: ['/w/core', '/w/api'], pinned: true }],
    [{ id: 'ws1', name: 'core', root_path: '/w/core' }],
    mod.ideProjectRecents.list,
  );
  assert.deepEqual([...rows.map((r: { title: string }) => r.title)], ['Payments', 'core', 'loose']);
  const cmds = mod.ideProjectPaletteCommands(rows, async () => ({}));
  assert.equal(cmds[0].title, 'IDE: Open project Payments');
  assert.match(cmds[0].keywords, /\/w\/api/);
  assert.equal(new Set(cmds.map((c: { id: string }) => c.id)).size, cmds.length, 'ids are unique');
});

// ── I3-08 / I3-17: per-project memory ───────────────────────────────────────

function loadMemory(storage = memStorage()) {
  return loadSource(new URL('../src/modules/ide/store/ideProjectMemory.ts', import.meta.url), { '../../../lib/storage': storage });
}

test('I3-08: per-project memory is keyed by target and bounded (LRU 12)', () => {
  const storage = memStorage();
  const mem = loadMemory(storage);
  for (let i = 0; i < 15; i++) mem.rememberDurable(`local::/w/p${i}`, { expanded: [`/w/p${i}/src`] });
  const saved = JSON.parse(storage.m.get(mem.IDE_PROJECT_MEMORY_KEY)!);
  assert.equal(Object.keys(saved).length, mem.MAX_PROJECTS);
  assert.equal(mem.recallDurable('local::/w/p0'), null, 'oldest dropped');
  assert.deepEqual([...mem.recallDurable('local::/w/p14').expanded], ['/w/p14/src']);
  assert.equal(mem.memoryKeyOf({ kind: 'ssh', root: '/srv', connection_id: 'c' }), 'ssh:c:/srv');
  assert.equal(mem.memoryKeyOf({ kind: 'local', root: '' }), '');
});

test('I3-08: expansion kept is open, inside the project, not a root, parents first, capped', () => {
  const mem = loadMemory();
  const kept = mem.expandedToRemember({ '/w/a': true, '/w/a/src/deep': true, '/w/a/src': true, '/w/a/docs': false, '/elsewhere/x': true }, ['/w/a']);
  assert.deepEqual([...kept], ['/w/a/src', '/w/a/src/deep']);
  const many: Record<string, boolean> = {};
  for (let i = 0; i < 300; i++) many[`/w/a/d${i}`] = true;
  assert.equal(mem.expandedToRemember(many, ['/w/a']).length, mem.MAX_EXPANDED_DIRS);
});

test('I3-17: the agent draft is volatile and per project; participants run in order', () => {
  const storage = memStorage();
  const mem = loadMemory(storage);
  let draft = 'about project A';
  mem.registerProjectMemoryParticipant('agent', {
    save: (k: string) => mem.stashVolatile(k, 'draft', draft),
    restore: (k: string) => { draft = mem.recallVolatile(k, 'draft') ?? ''; },
  });
  mem.leaveProject('local::/w/a');
  mem.enterProject('local::/w/b');
  assert.equal(draft, '', 'B starts with an empty composer');
  draft = 'about B';
  mem.leaveProject('local::/w/b');
  mem.enterProject('local::/w/a');
  assert.equal(draft, 'about project A', 'A gets its own draft back');
  assert.ok(![...storage.m.values()].some((v) => v.includes('about project A')), 'a draft never reaches storage');
});

// ── I3-03: runs are per project ─────────────────────────────────────────────

function loadRuns(get: (path: string) => Promise<unknown>) {
  const toastsLog: { level: string; title: string; action?: { label: string; run: () => void } }[] = [];
  class ApiError extends Error { status: number; constructor(status: number) { super(String(status)); this.status = status; } }
  const storage = memStorage();
  const mod = loadSource(new URL('../src/modules/ide/store/ideParkedRuns.ts', import.meta.url), {
    '../../../lib/api/client': { api: { bg: { get } }, ApiError },
    '../../../lib/storage': storage,
    '../../../lib/poll': { pollWhileVisible: () => ({ stop: () => {}, now: () => {} }) },
    '../../../lib/toast.svelte': { toasts: { push: (level: string, title: string, _b: unknown, _t: unknown, o: { action?: { label: string; run: () => void } }) => toastsLog.push({ level, title, action: o?.action }) } },
  });
  return { mod, toastsLog, storage, ApiError };
}

const running = { id: 'run_1', title: 'api-server', runner: 'python', status: 'running', output: ['tick'], tests: [] };

test('I3-03: a run left in A is parked, then re-adopted with its live state when you return', async () => {
  let status = 'running';
  const { mod, storage } = loadRuns(async () => ({ run: { ...running, status, output: ['tick', 'tock'] } }));
  const hostA = { target: { kind: 'local', root: '/w/a', connection_id: null }, envState: { activeRun: running, runsHistory: [] as unknown[] } };
  const parked = mod.parkRun(hostA, 'local::/w/a', 'alpha', { kind: 'local', root: '/w/a' });
  assert.equal(parked.runId, 'run_1');
  assert.match(storage.m.get(`ss:${mod.PARKED_RUNS_KEY}`)!, /run_1/, 'survives a reload (sessionStorage)');
  assert.doesNotMatch(storage.m.get(`ss:${mod.PARKED_RUNS_KEY}`)!, /tick/, 'output never goes to storage');
  // Switched to B: nothing to adopt there.
  const hostB = { target: { kind: 'local', root: '/w/b', connection_id: null }, envState: { activeRun: null, runsHistory: [] } };
  assert.equal(await mod.adoptParkedRun(hostB, 'local::/w/b'), null);
  // Back in A: the same run, live.
  const back = { target: { kind: 'local', root: '/w/a', connection_id: null }, envState: { activeRun: null as unknown, runsHistory: [] as unknown[] } };
  const adopted = await mod.adoptParkedRun(back, 'local::/w/a');
  assert.equal(adopted.id, 'run_1');
  assert.deepEqual(back.envState.activeRun, { ...running, output: ['tick', 'tock'] });
  assert.equal(mod.parkedRuns().length, 0, 'adopted runs leave the parking list');
  mod.stopFollowing();
  status = 'stopped';
  mod.resetParkedRunsForTests();
});

test('I3-03: a parked run that ends while you are elsewhere gets one toast naming the project, with Show', async () => {
  const { mod, toastsLog } = loadRuns(async () => ({ run: { ...running, status: 'failed', exit_code: 2 } }));
  const reopened: unknown[] = [];
  mod.setParkedRunReopener((t: unknown) => reopened.push(t));
  mod.setParkedRunCurrentKey(() => 'local::/w/b');
  mod.parkRun({ target: { kind: 'local', root: '/w/a' }, envState: { activeRun: running, runsHistory: [] } }, 'local::/w/a', 'alpha', { kind: 'local', root: '/w/a', project_id: null });
  await mod.watchOnce();
  await mod.watchOnce();
  assert.equal(toastsLog.length, 1, 'announced once');
  assert.equal(toastsLog[0].level, 'error');
  assert.equal(toastsLog[0].title, 'api-server failed (exit 2) in alpha');
  assert.equal(toastsLog[0].action?.label, 'Show');
  toastsLog[0].action!.run();
  assert.deepEqual(reopened, [{ kind: 'local', root: '/w/a', project_id: null }]);
  // On return the finished run is shown (from memory, no fetch needed).
  const back = { target: { kind: 'local', root: '/w/a', connection_id: null }, envState: { activeRun: null as { status: string } | null, runsHistory: [] as unknown[] } };
  await mod.adoptParkedRun(back, 'local::/w/a');
  assert.equal(back.envState.activeRun?.status, 'failed');
  assert.equal(back.envState.runsHistory.length, 1);
  mod.resetParkedRunsForTests();
});

test('I3-03: a finished or idle run is not parked; copy for stopped/finished', () => {
  const { mod } = loadRuns(async () => ({}));
  assert.equal(mod.parkRun({ target: { kind: 'local', root: '/w/a' }, envState: { activeRun: { ...running, status: 'passed' }, runsHistory: [] } }, 'local::/w/a', 'a', { kind: 'local', root: '/w/a' }), null);
  assert.equal(mod.parkRun({ target: { kind: 'local', root: '/w/a' }, envState: { activeRun: null, runsHistory: [] } }, 'local::/w/a', 'a', { kind: 'local', root: '/w/a' }), null);
  assert.equal(mod.runEndedCopy({ title: 'srv', label: 'alpha' }, { status: 'stopped' }).title, 'srv stopped in alpha');
  assert.equal(mod.runEndedCopy({ title: 'srv', label: 'alpha' }, { status: 'passed' }).level, 'info');
  mod.resetParkedRunsForTests();
});

// ── I3-12: deleted folder / host down ───────────────────────────────────────

test('I3-12: a missing local root and an unreachable SSH host are told apart from ordinary errors', () => {
  class ApiError extends Error { status: number; constructor(status: number) { super(String(status)); this.status = status; } }
  const mod = loadSource(new URL('../src/modules/ide/store/ideProjectHealth.svelte.ts', import.meta.url), {
    '../../../lib/api/client': { ApiError },
  });
  assert.equal(mod.classifyRootError({ kind: 'local' }, 404, 'x'), 'missing');
  assert.equal(mod.classifyRootError({ kind: 'local' }, 400, "cannot resolve workspace root '/w/gone': No such file or directory"), 'missing');
  assert.equal(mod.classifyRootError({ kind: 'local' }, 500, 'boom'), null);
  assert.equal(mod.classifyRootError({ kind: 'ssh' }, 502, 'timeout'), 'unreachable');
  assert.equal(mod.classifyRootError({ kind: 'ssh' }, 403, 'no'), null);
  const h = mod.ideProjectHealth;
  h.noteRootError({ kind: 'local', root: '/w/gone', connection_id: null }, new ApiError(404), 'not found');
  assert.deepEqual({ ...h.issue }, { kind: 'missing', key: 'local::/w/gone', root: '/w/gone', message: 'not found' });
  h.clear();
  assert.equal(h.issue, null);
});

// ── switch wiring ───────────────────────────────────────────────────────────

test('I3-08: only remembered folders whose parent chain is open are re-listed', () => {
  const mod = loadSource(new URL('../src/modules/ide/store/ideProjectSwitch.ts', import.meta.url), {
    '../shell/ideProjectRecents.svelte': { ideProjectRecents: { record: () => {} }, sameRootKey: (p: string) => p },
    './ideProjectHealth.svelte': { ideProjectHealth: { clear: () => {} } },
    './ideWindowTitle.svelte': { ideWindowTitle: { project: '' } },
    './ideParkedRuns': { adoptParkedRun: async () => null, parkRun: () => null, resumeParkedRunWatch: () => {}, setParkedRunCurrentKey: () => {}, setParkedRunReopener: () => {} },
    './ideProjectMemory': { enterProject: () => {}, expandedToRemember: () => [], leaveProject: () => {}, recallDurable: () => null, rememberDurable: () => {} },
    './ideTypes': { ideBasename: (p: string) => p.split('/').pop(), ideTargetKey: (t: { kind: string; root: string }) => `${t.kind}::${t.root}` },
  });
  const kept = mod.reachableExpanded(['/w/a/src/deep', '/w/a/src', '/w/a/x/y'], ['/w/a']);
  assert.deepEqual([...kept], ['/w/a/src', '/w/a/src/deep'], '/w/a/x/y sits under a collapsed /w/a/x');
});
