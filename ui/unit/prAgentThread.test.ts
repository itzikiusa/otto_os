// One Threads thread per pull request (pr-agent-thread.ts): which thread a PR
// shows, which provider a new one runs on, the create body (and the fallback
// for a daemon that refuses the embed fields), and how a send reaches a
// closed thread.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

function load(handler: (method: string, path: string, body?: unknown) => unknown) {
  const calls: { method: string; path: string; body?: unknown }[] = [];
  const call = async (method: string, path: string, body?: unknown) => {
    calls.push({ method, path, body: body === undefined ? undefined : JSON.parse(JSON.stringify(body)) });
    return handler(method, path, body);
  };
  const api = { get: (p: string) => call('GET', p), post: (p: string, b?: unknown) => call('POST', p, b) };
  const mod = loadSource(new URL('../src/modules/git/pr-agent-thread.ts', import.meta.url), {
    '../../lib/api/client': { api, ApiError },
  });
  return { mod, calls };
}

const row = (id: string, last: string, meta: Record<string, unknown> | null, extra: Record<string, unknown> = {}) => ({
  id, workspace_id: 'w', provider: 'claude', title: id, cwd: '/r', status: 'idle', access_mode: 'approval-required',
  created_by: 'u', created_at: last, last_active_at: last, last_sequence: 0, meta, ...extra,
});
const tagged = (id: string, last: string, origin: string, extra = {}) =>
  row(id, last, { source: 'pr-agent', origin: { kind: 'pr', id: origin } }, extra);

test('origin id and label (≤ 200 characters)', () => {
  const { mod } = load(() => null);
  assert.equal(mod.prAgentOriginId('rep_otto', 51), 'rep_otto#51');
  assert.equal(mod.prAgentOriginLabel('otto', 51, 'Add  the VIP'), 'otto #51 Add the VIP');
  assert.equal(mod.prAgentOriginLabel('otto', 1, 'x'.repeat(400)).length, 200);
});

test('a PR shows its own newest thread — not another PR’s, not a linked one', () => {
  const { mod } = load(() => null);
  const rows = [
    tagged('old', '2026-11-10T00:00:00Z', 'r#1'),
    tagged('new', '2026-11-12T00:00:00Z', 'r#1', { status: 'closed' }),
    tagged('other', '2026-11-13T00:00:00Z', 'r#2'),
    tagged('linked', '2026-11-14T00:00:00Z', 'r#1', { parent_id: 'new' }),
    row('plain', '2026-11-15T00:00:00Z', null),
  ];
  assert.equal(mod.pickPrThread(rows, 'r#1', null).id, 'new');
  assert.equal(mod.pickPrThread(rows, 'r#2', null).id, 'other');
  assert.equal(mod.pickPrThread(rows, 'r#3', null), null);
  // An older daemon stored no meta: the id this browser remembered wins.
  assert.equal(mod.pickPrThread(rows, 'r#3', 'plain').id, 'plain');
});

test('provider: the default when it can run a thread, else the first ready one, else none', () => {
  const { mod } = load(() => null);
  const ready = (p: string) => p !== 'codex';
  assert.equal(mod.pickRuntimeProvider('claude', ['agy', 'claude', 'codex'], ready), 'claude');
  assert.equal(mod.pickRuntimeProvider('agy', ['agy', 'claude'], ready), 'agy'); // agy runs on ACP
  assert.equal(mod.pickRuntimeProvider('shell', ['shell', 'claude'], ready), 'claude'); // terminal-only
  assert.equal(mod.pickRuntimeProvider('codex', ['codex', 'gemini'], ready), 'gemini');
  assert.equal(mod.pickRuntimeProvider('claude', ['my-cli', 'shell'], ready), null);
});

test('find asks the daemon for this PR only, and falls back to the remembered id', async () => {
  const created = row('t1', '2026-11-12T00:00:00Z', null);
  const { mod, calls } = load((method, path) => {
    if (method === 'POST') return created;
    if (path.startsWith('/workspaces/')) return [row('x', '2026-11-12T00:00:00Z', null)]; // an old daemon: unfiltered, no meta
    return created;
  });
  assert.equal(await mod.findPrThread('w', 'rep#5'), null);
  assert.equal(calls[0].path, '/workspaces/w/runtime-sessions?source=pr-agent&origin_id=rep%235');
  await mod.createPrThread('w', { provider: 'claude', cwd: '/r', title: 't', access_mode: 'approval-required', source: 'pr-agent', origin: { kind: 'pr', id: 'rep#5' } });
  const again = await mod.findPrThread('w', 'rep#5');
  assert.equal(again.id, 't1');
  assert.equal(calls.at(-1)?.path, '/runtime-sessions/t1');
});

test('create sends the embed fields; a daemon that refuses them gets a plain thread', async () => {
  let refuse = true;
  const { mod, calls } = load((_m, _p, body) => {
    if (refuse && (body as { source?: string }).source) throw new ApiError(400, 'unknown field `source`');
    return row('t2', '2026-11-12T00:00:00Z', null);
  });
  const req = { provider: 'claude', cwd: '/r', title: 'PR #5 · x', access_mode: 'approval-required', source: 'pr-agent', origin: { kind: 'pr', id: 'rep#5', label: 'otto #5 x' } };
  const t = await mod.createPrThread('w', req);
  assert.equal(t.id, 't2');
  assert.deepEqual(calls[0].body, req);
  assert.deepEqual(calls[1].body, { provider: 'claude', cwd: '/r', title: 'PR #5 · x', access_mode: 'approval-required' });
  // Any other failure is the person's to see, not silently retried.
  refuse = false;
  const { mod: m2 } = load(() => { throw new ApiError(400, 'the agent could not start: signed out'); });
  await assert.rejects(m2.createPrThread('w', req), /signed out/);
});

test('send: a closed thread is reopened first; the access mode rides on the prompt', async () => {
  const { mod, calls } = load(() => ({ turn_id: 't', sequence: 1 }));
  await mod.sendToPrThread({ id: 'a', status: 'closed' }, 'hi', 'approval-required');
  await mod.sendToPrThread({ id: 'b', status: 'working' }, 'fix', 'auto-accept-edits');
  assert.deepEqual(calls.map((c) => `${c.method} ${c.path}`), [
    'POST /runtime-sessions/a/reopen',
    'POST /runtime-sessions/a/prompt',
    'POST /runtime-sessions/b/prompt',
  ]);
  assert.deepEqual(calls[1].body, { text: 'hi', access_mode: 'approval-required', delivery: 'queue' });
  assert.deepEqual(calls[2].body, { text: 'fix', access_mode: 'auto-accept-edits', delivery: 'queue' });
});
