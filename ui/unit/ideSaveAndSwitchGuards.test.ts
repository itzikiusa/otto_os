import assert from 'node:assert/strict';
import { test } from 'node:test';
import { deferred, loadSource } from './sourceHarness.ts';

class ApiError extends Error {
  status: number;
  constructor(status: number) { super(`HTTP ${status}`); this.status = status; }
}

function ideModules(api: Record<string, unknown>, confirmAnswers: boolean[] = []) {
  const asked: string[] = [];
  const toastLog: string[] = [];
  const confirmer = { ask: async (msg: string) => { asked.push(msg); return confirmAnswers.shift() ?? false; }, choose: async () => ({ value: null }) };
  const toasts = { success: (t: string) => toastLog.push(`ok:${t}`), error: (t: string) => toastLog.push(`err:${t}`), info: () => {} };
  const prefs = { autoSave: false, localHistoryEnabled: false, localHistoryRetentionDays: 7 };
  const client = { api, ApiError, getToken: () => '', laneFetch: () => { throw new Error('unused'); } };
  const imports = {
    svelte: { tick: async () => {} },
    '../../../lib/api/client': client,
    '../../../lib/components/cm-ide-drafts': { readIdeUndoDraft: () => null, removeIdeUndoDraft: () => {}, writeIdeUndoDraftImmediate: () => {} },
    '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => prefs },
    '../../../lib/confirm.svelte': { confirmer },
    '../../../lib/toast.svelte': { toasts },
    '../../../lib/toastError': { toastError: (t: string) => toastLog.push(`err:${t}`) },
    '../../../lib/ideProjectsModel': { loadNamedIdeProjectsLocal: () => [], saveNamedIdeProjectsLocal: () => {} },
    '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: {} },
    '../../../lib/storage': { lsGet: () => null, lsSet: () => {} },
  };
  const opts = { relative: true };
  const cache = new Map();
  const buf = loadSource(new URL('../src/modules/ide/store/ideBufferPerf.ts', import.meta.url), imports, {}, opts, cache);
  const helpers = loadSource(new URL('../src/modules/ide/shell/ideProjectHelpers.ts', import.meta.url), imports, {}, opts, cache);
  return { buf, helpers, asked, toastLog, prefs };
}

function tab(path: string, content: string, savedContent = content) {
  return { path, name: path.split('/').pop(), content, savedContent, dirty: content !== savedContent, eol: '\n', sha256: 'sha-0', readOnly: false, truncated: false };
}

function host(tabs: ReturnType<typeof tab>[]) {
  const target = { kind: 'local', workspace_id: null, root: '/repo', connection_id: null, connection_name: null };
  return {
    target, targetPayload: { ...target, project_folders: ['/repo'] }, tabs, activePath: tabs[0]?.path ?? null,
    canEditCollab: true, collabIsGuest: false, resolvePath: (p: string) => p,
    loadGutter: async () => {}, loadSymbols: async () => {},
  };
}

test('a second save of the same file waits for the first and sends the sha256 it got back', async () => {
  const sent: (string | undefined)[] = [];
  const first = deferred<{ sha256: string }>();
  let calls = 0;
  const api = { put: async (_p: string, body: { expected_sha256?: string }) => {
    sent.push(body.expected_sha256);
    calls += 1;
    return calls === 1 ? first.promise : { sha256: 'sha-2' };
  } };
  const { buf } = ideModules(api);
  const perf = new buf.IdeBufferPerfController();
  const h = host([tab('/repo/a.ts', 'x = 2', 'x = 1')]);
  const a = buf.performSaveFile(h, perf, '/repo/a.ts', false, { silent: true });
  const b = buf.performSaveFile(h, perf, '/repo/a.ts');
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(sent, ['sha-0'], 'the second PUT must not start while the first is in flight');
  first.resolve({ sha256: 'sha-1' });
  assert.equal(await a, true);
  assert.equal(await b, true);
  assert.deepEqual(sent, ['sha-0', 'sha-1']);
  assert.equal(h.tabs[0].sha256, 'sha-2');
  assert.equal(perf.savesInFlight.size, 0);
});

test('switching target asks before dropping unsaved tabs and stops when declined', async () => {
  const { buf, helpers, asked } = ideModules({}, [false, true]);
  const perf = new buf.IdeBufferPerfController();
  const h = host([tab('/repo/a.ts', 'edited', 'saved'), tab('/repo/b.ts', 'same')]);
  let flushed = 0;
  assert.equal(await helpers.confirmDiscardOnSwitch(h, perf, async () => { flushed += 1; }), false);
  assert.equal(asked.length, 1);
  assert.match(asked[0], /“a\.ts” has unsaved changes/);
  assert.equal(flushed, 0, 'autosave off: nothing to flush');
  assert.equal(await helpers.confirmDiscardOnSwitch(h, perf, async () => {}), true);
  const clean = host([tab('/repo/b.ts', 'same')]);
  assert.equal(await helpers.confirmDiscardOnSwitch(clean, perf, async () => {}), true);
  assert.equal(asked.length, 2, 'clean tabs switch without a prompt');
});

function depsModel(post: () => Promise<unknown>) {
  const toastLog: string[] = [];
  const toasts = { success: (t: string) => toastLog.push(`ok:${t}`), error: (t: string) => toastLog.push(`err:${t}`), info: () => {} };
  const mod = loadSource(new URL('../src/lib/components/cm-ide-deps-model.ts', import.meta.url), {
    '../api/client.ts': { api: { post }, ApiError },
    '../toast.svelte.ts': { toasts },
  }, {}, { relative: true });
  return { mod, toastLog };
}

const GO_MOD = 'module x\n\ngo 1.22\n\nrequire github.com/a/b v1.0.0\n';
const DEP = { name: 'github.com/a/b', current_version: 'v1.0.0', line: 5 };

test('a dependency update the daemon failed to run is reported, not faked with a text edit', async () => {
  const { mod, toastLog } = depsModel(async () => { throw new ApiError(500); });
  const res = await mod.executeIdeDepAction({ root: '/repo', path: '/repo/go.mod', content: GO_MOD, action: 'update_one', dep: DEP, version: 'v1.2.0' });
  assert.equal(res.ok, false);
  assert.equal(res.updated_content, undefined);
  assert.match(toastLog.join(), /^err:Couldn’t run/);
});

test('an action the daemon has no runner for (400) falls back to editing the manifest text', async () => {
  const { mod, toastLog } = depsModel(async () => { throw new ApiError(400); });
  const res = await mod.executeIdeDepAction({ root: '/repo', path: '/repo/go.mod', content: GO_MOD, action: 'update_one', dep: DEP, version: 'v1.2.0' });
  assert.equal(res.ok, true);
  assert.match(res.updated_content, /github\.com\/a\/b v1\.2\.0/);
  assert.match(toastLog.join(), /^ok:Updated github\.com\/a\/b/);
});
