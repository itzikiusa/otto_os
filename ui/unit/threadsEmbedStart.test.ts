// An embed starts and finds its thread through the Threads store
// (runtimeThreads.svelte.ts `start` / `findByOrigin`), never the raw routes:
// the create body carries the access mode and the embed's source / origin
// (with a plain-thread fallback for a daemon that refuses them), and the
// lookup prefers an open thread over a closed one. Plus the start composer's
// pure rules (startComposer.ts): which agents an embed offers and starts on.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function loadStore(handler: (method: string, path: string, body?: unknown) => unknown) {
  const calls: { method: string; path: string; body?: unknown }[] = [];
  const call = async (method: string, path: string, body?: unknown) => {
    calls.push({ method, path, body: body === undefined ? undefined : JSON.parse(JSON.stringify(body)) });
    return handler(method, path, body);
  };
  const { runtimeThreads } = loadSource(new URL('../src/modules/threads/runtime/runtimeThreads.svelte.ts', import.meta.url), {
    '../../../lib/api/client': { api: { get: (p: string) => call('GET', p), post: (p: string, b?: unknown) => call('POST', p, b) } },
    '../../../lib/stores/runtimeDraftRecovery.svelte': { installRuntimeDraftRecovery: () => {} },
    '../../../lib/live': { onLive: () => () => {}, appLive: { onResync: () => () => {} } },
    '../../../lib/router.svelte': { router: {} },
    '../../../lib/stores/runtimeAttention.svelte': { noteRuntimeWaiting: () => {}, syncRuntimeWaiting: () => {} },
    '../../../lib/stores/uiControl.svelte': { uiControl: { addSource: () => {} } },
    '../threadsStore.svelte': { threads: {} },
    '../../../lib/stores/runtimeDrafts.svelte': { runtimeDrafts: { forget: () => {} } },
    '../../../lib/loadError': { loadErrorText: (e: unknown) => String(e) },
    '../../../lib/providers': { isRuntimeProvider: () => true, RUNTIME_PROVIDERS: ['claude'], sessionProviderSlug: (r: { provider: string }) => r.provider },
  });
  return { runtimeThreads, calls };
}

const row = (id: string, last: string, extra: Record<string, unknown> = {}) => ({
  id, workspace_id: 'w', provider: 'claude', title: id, cwd: '/r', status: 'idle', access_mode: 'approval-required',
  created_by: 'u', created_at: last, last_active_at: last, last_sequence: 0, meta: null, ...extra,
});
const tagged = (id: string, last: string, origin: string, extra: Record<string, unknown> = {}) =>
  row(id, last, { meta: { source: 'pr-agent', origin: { kind: 'pr', id: origin } }, ...extra });

class HttpError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

test('start carries the access mode, source and origin, then sends the first message', async () => {
  const { runtimeThreads, calls } = loadStore((method, path) => (path.endsWith('/prompt') ? { turn_id: 't', sequence: 3 } : row('t1', 'now')));
  const created = await runtimeThreads.start('w', {
    provider: 'codex', cwd: '/r', title: 'PR #5 · x', model: 'gpt-5', effort: 'high', prompt: 'Explain',
    accessMode: 'approval-required', source: 'pr-agent', origin: { kind: 'pr', id: 'rep#5', label: 'otto #5 x' },
  });
  assert.equal(created.id, 't1');
  assert.deepEqual(calls[0], {
    method: 'POST', path: '/workspaces/w/runtime-sessions',
    body: { provider: 'codex', cwd: '/r', title: 'PR #5 · x', model: 'gpt-5', access_mode: 'approval-required', effort: 'high', source: 'pr-agent', origin: { kind: 'pr', id: 'rep#5', label: 'otto #5 x' } },
  });
  assert.deepEqual(calls[1], { method: 'POST', path: '/runtime-sessions/t1/prompt', body: { text: 'Explain', effort: 'high' } });
  assert.equal(runtimeThreads.byId.t1.id, 't1');
});

test('Threads’ own start names no access mode (the daemon default) and no embed fields', async () => {
  const { runtimeThreads, calls } = loadStore(() => row('t2', 'now'));
  await runtimeThreads.start('w', { provider: 'claude', cwd: null, title: 'Claude thread', model: null, prompt: '' });
  assert.deepEqual(calls[0].body, { provider: 'claude', cwd: null, title: 'Claude thread', model: null });
  assert.equal(calls.length, 1); // no prompt: an empty thread
});

test('a daemon that refuses the embed fields gets a plain thread; any other failure surfaces', async () => {
  let refuse = true;
  const { runtimeThreads, calls } = loadStore((_m, path, body) => {
    if (path.endsWith('/prompt')) return { turn_id: 't', sequence: 1 };
    if (refuse && (body as { source?: string }).source) throw new HttpError(400, 'unknown field `source`');
    return row('t3', 'now');
  });
  const req = { provider: 'claude', cwd: '/r', title: 't', model: null, prompt: '', accessMode: 'approval-required', source: 'connection-diagnose', origin: { kind: 'connection', id: 'c1' } };
  assert.equal((await runtimeThreads.start('w', req)).id, 't3');
  assert.deepEqual(calls[1].body, { provider: 'claude', cwd: '/r', title: 't', model: null, access_mode: 'approval-required' });
  // Effort alone refused (a daemon from before it was stored): the prompt still carries it.
  const { runtimeThreads: noEffort, calls: c2 } = loadStore((_m, path, body) => {
    if (path.endsWith('/prompt')) return { turn_id: 't', sequence: 1 };
    if ((body as { effort?: string }).effort) throw new HttpError(422, 'unknown field `effort`');
    return row('t4', 'now');
  });
  await noEffort.start('w', { provider: 'codex', cwd: '/r', title: 't', model: null, effort: 'low', prompt: 'hi' });
  assert.deepEqual(c2.map((c) => (c.body as { effort?: string }).effort), ['low', undefined, 'low']);
  refuse = false;
  const { runtimeThreads: other } = loadStore(() => { throw new HttpError(400, 'the agent could not start: signed out'); });
  await assert.rejects(other.start('w', req), /signed out/);
});

test('findByOrigin: this origin only, never a linked thread, an open one before a closed one', async () => {
  const rows = [
    tagged('old-open', '2026-11-10T00:00:00Z', 'r#1'),
    tagged('new-closed', '2026-11-12T00:00:00Z', 'r#1', { status: 'closed' }),
    tagged('other', '2026-11-13T00:00:00Z', 'r#2'),
    tagged('linked', '2026-11-14T00:00:00Z', 'r#1', { parent_id: 'old-open' }),
    row('plain', '2026-11-15T00:00:00Z'),
  ];
  const { runtimeThreads, calls } = loadStore(() => rows);
  assert.equal((await runtimeThreads.findByOrigin('w', 'pr-agent', 'r#1')).id, 'old-open');
  assert.equal(calls[0].path, '/workspaces/w/runtime-sessions?source=pr-agent&origin_id=r%231');
  assert.equal(runtimeThreads.byId['old-open'].id, 'old-open');
  assert.equal((await runtimeThreads.findByOrigin('w', 'pr-agent', 'r#2')).id, 'other');
  assert.equal(await runtimeThreads.findByOrigin('w', 'pr-agent', 'r#3'), null);
  // An older daemon stored no meta: the id the host remembered is a match.
  assert.equal((await runtimeThreads.findByOrigin('w', 'pr-agent', 'r#3', 'plain')).id, 'plain');
  // Two closed ones: the newest.
  const { runtimeThreads: closed } = loadStore(() => [
    tagged('a', '2026-11-10T00:00:00Z', 'c', { status: 'closed' }),
    tagged('b', '2026-11-11T00:00:00Z', 'c', { status: 'closed' }),
  ]);
  assert.equal((await closed.findByOrigin('w', 'pr-agent', 'c')).id, 'b');
});

test('start composer: an embed offers only installed agents that run a thread, and starts on the default', () => {
  const sc = loadSource(new URL('../src/modules/threads/startComposer.ts', import.meta.url), {});
  const runtime = (p: string) => p !== 'shell' && p !== 'aider';
  const ready = (p: string) => p !== 'gemini';
  const all = ['claude', 'claudework', 'codex', 'gemini', 'aider', 'shell'];
  assert.deepEqual(sc.startHarnesses(all, true, runtime, ready), ['claude', 'claudework', 'codex']);
  assert.deepEqual(Array.from(sc.startHarnesses(all, false, runtime, ready)), all); // Threads: every harness
  assert.equal(sc.initialProvider(['claude', 'claudework'], 'claudework', ready), 'claudework'); // a custom default is kept
  assert.equal(sc.initialProvider(['claude', 'codex'], 'gemini', ready), 'claude'); // not offered: first ready
  assert.equal(sc.initialProvider(['gemini', 'codex'], 'gemini', ready), 'codex'); // offered but not installed
  assert.equal(sc.initialProvider([], 'claude', ready), ''); // none: the embed says what to set up
  // Every access mode has words, and they rank from supervised to full.
  assert.deepEqual(Object.keys(sc.ACCESS_HINT).sort(), ['approval-required', 'auto-accept-edits', 'full-access']);
  assert.ok(sc.ACCESS_RANK['full-access'] > sc.ACCESS_RANK['auto-accept-edits']);
  assert.match(sc.ACCESS_HINT['approval-required'], /asks before/);
});

test('the start composer remembers the last agent per scope, only while it is still installed', () => {
  const sc = loadSource(new URL('../src/modules/threads/startComposer.ts', import.meta.url), {});
  const mem = new Map<string, string>();
  const store = { getItem: (k: string) => mem.get(k) ?? null, setItem: (k: string, v: string) => void mem.set(k, v) };
  const ready = (p: string) => p !== 'gemini';
  sc.writeStartPick(store, 'threads:w1', { provider: 'codex', model: 'gpt-5', effort: 'high' });
  sc.writeStartPick(store, 'embed:pr-agent', { provider: 'claudework', model: null, effort: null });
  assert.deepEqual({ ...sc.readStartPick(store, 'threads:w1', ['claude', 'codex'], ready) }, { provider: 'codex', model: 'gpt-5', effort: 'high' });
  // Per scope: another workspace, and Threads vs a PR, keep their own.
  assert.equal(sc.readStartPick(store, 'threads:w2', ['claude', 'codex'], ready), null);
  assert.equal(sc.readStartPick(store, 'embed:pr-agent', ['claude', 'claudework'], ready).provider, 'claudework');
  // No longer offered or installed: the default applies.
  assert.equal(sc.readStartPick(store, 'threads:w1', ['claude'], ready), null);
  sc.writeStartPick(store, 'threads:w3', { provider: 'gemini', model: null, effort: null });
  assert.equal(sc.readStartPick(store, 'threads:w3', ['gemini'], ready), null);
  // Garbage, blocked storage: no pick, never a throw. Access is never part of it.
  mem.set(sc.startPickKey('threads:w4'), '{not json');
  assert.equal(sc.readStartPick(store, 'threads:w4', ['codex'], ready), null);
  const blocked = { getItem: () => { throw new Error('denied'); }, setItem: () => { throw new Error('denied'); } };
  assert.equal(sc.readStartPick(blocked, 'threads:w1', ['codex'], ready), null);
  sc.writeStartPick(blocked, 'threads:w1', { provider: 'codex', model: null, effort: null });
  mem.set(sc.startPickKey('threads:w5'), JSON.stringify({ provider: 'codex', model: 7, effort: '', access_mode: 'full-access' }));
  assert.deepEqual({ ...sc.readStartPick(store, 'threads:w5', ['codex'], ready) }, { provider: 'codex', model: null, effort: null });
});
