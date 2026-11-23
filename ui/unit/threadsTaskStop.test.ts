// Stopping a runtime thread's background work and telling the truth about it
// (review C1–C4, D-06/D-07, F5–F7, K3): what each running row says through a
// stop, the calm "background task running" state, the Ask-another-agent
// effort reset, the timeline's model/effort/access change row, and releasing
// queued attachments once the daemon delivered them.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { loadSource } from './sourceHarness.ts';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));

const ts = await import('../src/modules/threads/runtime/taskStop.ts');
const aa = await import('../src/modules/threads/runtime/askAgents.ts');
const sc = await import('../src/modules/threads/runtime/settingsChange.ts');
const tl = await import('../src/modules/threads/runtime/timeline.ts');
const at = await import('../src/modules/threads/runtime/agentTasks.ts');
const ct = await import('../src/modules/threads/runtime/childThreads.ts');

class Http extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

test('a stop error: older daemon hides Stop, an ended task is gone, anything else is a reason with Retry', () => {
  assert.deepEqual(ts.stopFailure(new Http(404, 'no such route: /api/v1/runtime-sessions/a/tasks/b/stop')), { unsupported: true });
  assert.deepEqual(ts.stopFailure(new Http(404, 'unknown task b1')), { unsupported: false, state: { phase: 'gone' } });
  assert.deepEqual(ts.stopFailure(new Http(409, 'already ended')), { unsupported: false, state: { phase: 'gone' } });
  assert.deepEqual(ts.stopFailure(new Http(500, 'driver crashed')), { unsupported: false, state: { phase: 'error', error: 'driver crashed' } });
  assert.equal(ts.stopStateOf('native').phase, 'stopping');
  assert.equal(ts.stopStateOf('signal').phase, 'stopping');
  assert.equal(ts.stopStateOf('asked').phase, 'asked');
});

test('a running row: Stop, then Stopping…, Asked, or Couldn’t stop with Retry; nothing on an old daemon', () => {
  assert.deepEqual(ts.rowStop(null, null, false), { canStop: true, busy: false, note: null, failed: false });
  assert.equal(ts.rowStop(null, null, true).canStop, false, 'no dead button on a daemon without the route');
  assert.equal(ts.rowStop(null, { phase: 'stopping' }, false).note, 'Stopping…');
  assert.equal(ts.rowStop({ stopping: true }, null, false).busy, true, 'the daemon’s own stopping flag shows too');
  const asked = ts.rowStop(null, { phase: 'asked' }, false);
  assert.equal(asked.note, 'Asked the agent to stop it');
  assert.equal(asked.canStop, false);
  const failed = ts.rowStop(null, { phase: 'error', error: 'busy' }, false);
  assert.equal(failed.note, 'Couldn’t stop: busy');
  assert.equal(failed.canStop && failed.failed, true, 'Retry shows');
  assert.equal(ts.rowStop({ stop_error: 'timed out' }, { phase: 'stopping' }, false).note, 'Couldn’t stop: timed out');
  assert.equal(ts.rowStop(null, { phase: 'gone' }, false).canStop, false);
});

test('the stop summary says what happened, never a silent no-op', () => {
  assert.deepEqual(ts.stopSummary({ stopping: 2, asked: 0, failed: 0 }), { ok: true, title: 'Stopping 2 background tasks' });
  assert.equal(ts.stopSummary({ stopping: 0, asked: 1, failed: 0, turn: true }).title, 'Stopped the turn · Asked the agent to stop 1 task');
  assert.equal(ts.stopSummary({ stopping: 0, asked: 0, failed: 0 }).title, 'Nothing was running');
  const bad = ts.stopSummary({ stopping: 1, asked: 0, failed: 2 });
  assert.equal(bad.ok, false);
  assert.equal(bad.title, 'Couldn’t stop 2 background tasks');
});

test('an idle thread with background work is its own calm state, not Working', () => {
  const running = { key: 'running', label: 'Running', tone: 'info', hint: 'Running', live: false };
  const st = ts.withBackgroundState(running, 2);
  assert.equal(st.label, 'Background task running');
  assert.equal(st.live, false);
  assert.match(st.hint!, /2 background tasks running/);
  const working = { key: 'working', label: 'Working', tone: 'success', live: true };
  assert.equal(ts.withBackgroundState(working, 2), working, 'a running turn stays Working');
});

test('asThreadSession: idle + background work maps to `running`, a turn to `working`', () => {
  const { asThreadSession } = loadSource(new URL('../src/modules/threads/runtime/runtimeThreads.svelte.ts', import.meta.url), harness(async () => []));
  const base = { id: 'A', workspace_id: 'ws', provider: 'claude', title: 'A', status: 'idle', created_at: 'x', last_active_at: 'x', meta: {} };
  const bg = [{ id: 'b1', kind: 'command', label: 'npm run dev', started_at: 'x' }];
  assert.equal(asThreadSession({ ...base, background_tasks: bg }).status, 'running');
  assert.equal(asThreadSession({ ...base, status: 'working', background_tasks: bg }).status, 'working');
  assert.equal(asThreadSession({ ...base, status: 'closed', background_tasks: bg }).status, 'idle');
  // A turn waiting on an approval is in flight, not Idle (A2).
  assert.equal(asThreadSession({ ...base, status: 'awaiting_permission' }).status, 'working');
  assert.equal(asThreadSession(base).status, 'idle');
});

test('a child busy only on background work says so (its Stop stops that work)', () => {
  const row = { id: 'k', status: 'idle', background_tasks: [{ id: 'b', kind: 'command', label: 'x', started_at: 'x' }] } as never;
  const st = ct.childState(row, null);
  assert.equal(st.tone, 'background');
  assert.equal(st.label, 'Working', 'one vocabulary for subagents');
  assert.equal(ct.rowTone('background'), 'live');
  assert.equal(ct.childState({ id: 'k', status: 'working', background_tasks: [] } as never, null).tone, 'working');
});

test('background rows carry the task id to stop and the daemon’s stop fields', () => {
  const t = at.agentTasks([], [
    { id: 'b1', kind: 'command', label: 'npm run dev', started_at: '2026-11-16T10:00:00Z', stopping: true },
    { id: 'b2', kind: 'command', label: 'tail -f log', started_at: '2026-11-16T10:00:01Z', stop_error: 'busy' },
  ]);
  assert.deepEqual(t.running.map((r) => [r.stopId, r.stopping, r.stopError]), [['b1', true, null], ['b2', false, 'busy']]);
  const sw = at.withSwarm({ running: [], finished: [] }, [{ name: 'scout', killed: false }], []);
  assert.equal(sw.running[0].stopId, undefined, 'a swarm member has no per-task stop');
});

test('Ask another agent never carries an effort the new provider refuses', () => {
  assert.equal(aa.clampEffort('max', []), null, 'Gemini takes none');
  assert.equal(aa.clampEffort('max', ['minimal', 'low', 'medium', 'high', 'xhigh']), null, 'Codex has no max');
  assert.equal(aa.clampEffort('high', ['low', 'high']), 'high');
  assert.equal(aa.clampEffort(null, ['low']), null);
  assert.match(aa.answerHandoff('Codex', '  Use a queue.  '), /^Codex answered the question I asked it:\n\nUse a queue\.$/);
});

test('a model / effort / access switch (a `selection:` notice) is one quiet timeline row, never a work row too', () => {
  const base = { turn_id: 't1', ordinal: 2, status: 'completed', started_at: '2026-11-16T10:00:00Z', completed_at: null, updated_at: '2026-11-16T10:00:00Z' };
  const change = { ...base, id: 'selection:7', type: 'system_notice', title: 'Model', message: 'Model: opus → sonnet' };
  assert.deepEqual(sc.settingsChangeOf(change), { setting: 'Model', text: 'Model: opus → sonnet', refused: false });
  const refused = sc.settingsChangeOf({ ...base, id: 'selection:8', type: 'system_notice', title: 'Model', message: 'Model: bad was refused — still on sonnet' });
  assert.equal(refused?.refused, true);
  assert.equal(sc.settingsChangeOf({ ...base, id: 'n1', type: 'system_notice', message: 'Background command finished: x' }), null, 'other notices stay notices');
  assert.equal(sc.settingsChangeOf({ ...base, id: 'selection:9', type: 'assistant_message', text: 'x' }), null);
  const rows = tl.timelineRows([change as never, { ...base, id: 'w1', type: 'reasoning', text: 'x', streaming: false } as never]);
  assert.deepEqual(rows.map((r) => r.kind), ['setting', 'work']);
});

function harness(get: (p: string) => Promise<unknown>, post = async (_p: string, _b?: unknown): Promise<unknown> => ({ sequence: 1 })) {
  return {
    '../../../lib/api/client': { api: { get, post, del: async () => undefined } },
    '../../../lib/stores/runtimeDraftRecovery.svelte': { installRuntimeDraftRecovery: () => {} },
    '../../../lib/live': { onLive: () => () => {}, appLive: { onResync: () => () => {} } },
    '../../../lib/router.svelte': { router: {} },
    '../../../lib/stores/runtimeAttention.svelte': { noteRuntimeWaiting: () => {} },
    '../../../lib/stores/uiControl.svelte': { uiControl: { addSource: () => {} } },
    '../threadsStore.svelte': { threads: {} },
    '../../../lib/stores/runtimeDrafts.svelte': { runtimeDrafts: { forget: () => {} } },
    '../../../lib/providers': { isRuntimeProvider: () => true, RUNTIME_PROVIDERS: ['claude'], sessionProviderSlug: (r: { provider: string }) => r.provider },
  };
}

test('queued attachments are released when the daemon delivers the queue, and on delete', async () => {
  const { runtimeThreads } = loadSource(
    new URL('../src/modules/threads/runtime/runtimeThreads.svelte.ts', import.meta.url),
    harness(async () => [], async (_p, b) => ({ sequence: 1, queued_id: (b as { text: string }).text })),
  );
  runtimeThreads.byId = { A: { id: 'A', status: 'working' }, B: { id: 'B', status: 'working' } };
  const img = [{ name: 'shot.png', mime: 'image/png', data_base64: 'AAAA' }];
  await runtimeThreads.prompt('A', { text: 'q1', images: img });
  await runtimeThreads.prompt('A', { text: 'q2', images: img });
  await runtimeThreads.prompt('B', { text: 'q3', images: img });
  assert.equal(runtimeThreads.queuedAttachments.size, 3);
  // The turn ended and the daemon sent q1: only q2 is still queued on A.
  runtimeThreads.onEvent({ session_id: 'A', sequence: 5, ts: 'now', type: 'queue.updated', queued_prompts: [{ id: 'q2', text: 'q2', attachments: 1, created_at: 'x' }] });
  assert.deepEqual([...runtimeThreads.queuedAttachments.keys()].sort(), ['q2', 'q3']);
  runtimeThreads.onEvent({ session_id: 'A', sequence: 6, ts: 'now', type: 'queue.updated', queued_prompts: [] });
  assert.deepEqual([...runtimeThreads.queuedAttachments.keys()], ['q3'], 'another thread’s queue is untouched');
  await runtimeThreads.remove('B');
  assert.equal(runtimeThreads.queuedAttachments.size, 0);
});

test('a resync re-reads every loaded workspace and the cross-workspace list', async () => {
  const gets: string[] = [];
  const { runtimeThreads } = loadSource(
    new URL('../src/modules/threads/runtime/runtimeThreads.svelte.ts', import.meta.url),
    harness(async (p) => {
      gets.push(p);
      return p.includes('/workspaces/') ? [{ id: 'A', workspace_id: 'ws', status: 'idle' }] : [];
    }),
  );
  runtimeThreads.byId = { A: { id: 'A', workspace_id: 'ws', status: 'working' } };
  runtimeThreads.loaded = { ws: true };
  await runtimeThreads.resync();
  assert.deepEqual(gets.sort(), ['/runtime-sessions', '/workspaces/ws/runtime-sessions']);
  assert.equal(runtimeThreads.byId.A.status, 'idle', 'a row left "working" by a lost event is corrected');
});

test('interrupt returns the daemon’s answer (interrupted:false is not swallowed)', async () => {
  const { runtimeThreads } = loadSource(
    new URL('../src/modules/threads/runtime/runtimeThreads.svelte.ts', import.meta.url),
    harness(async () => [], async () => ({ interrupted: false, background_tasks: 2 })),
  );
  assert.deepEqual(await runtimeThreads.interrupt('A'), { interrupted: false, background_tasks: 2 });
});

test('a queued message whose delivery failed shows why, with Send again, Edit and Remove', async () => {
  const qr = await import('../src/modules/threads/runtime/queueRows.ts');
  assert.deepEqual(qr.queuedRow({ error: 'provider refused the image' }, { running: true, canSteer: true }), {
    error: 'Couldn’t send: provider refused the image',
    actions: ['retry', 'edit', 'remove'],
  });
  assert.deepEqual(qr.queuedRow({}, { running: true, canSteer: true }).actions, ['steer', 'edit', 'remove']);
  assert.deepEqual(qr.queuedRow({ error: '  ' }, { running: false, canSteer: true }), { error: null, actions: ['edit', 'remove'] });
  // The composer's words for a refused send.
  assert.match(qr.sendRefusal(413, 'payload too large'), /32 MB send limit/);
  assert.match(qr.sendRefusal(413, 'queue attachments exceed 64 MiB'), /queue is full of attachments/);
  assert.match(qr.sendRefusal(409, 'queue is full (20 messages)'), /20 messages/);
  assert.equal(qr.sendRefusal(409, 'a turn is running'), 'a turn is running');
});

test('a retry keeps the queued attachments cached; steer releases them', async () => {
  const { runtimeThreads } = loadSource(
    new URL('../src/modules/threads/runtime/runtimeThreads.svelte.ts', import.meta.url),
    harness(async () => [], async (p, b) => (p.endsWith('/prompt') ? { sequence: 1, queued_id: (b as { text: string }).text } : { sequence: 2 })),
  );
  runtimeThreads.byId = { A: { id: 'A', status: 'working' } };
  const img = [{ name: 'shot.png', mime: 'image/png', data_base64: 'AAAA' }];
  await runtimeThreads.prompt('A', { text: 'q1', images: img });
  await runtimeThreads.deliverQueued('A', 'q1', 'queue');
  assert.equal(runtimeThreads.queuedAttachments.has('q1'), true);
  await runtimeThreads.deliverQueued('A', 'q1', 'steer');
  assert.equal(runtimeThreads.queuedAttachments.has('q1'), false);
});

test('a thread started in Plan keeps asking for a plan until the person leaves it (A4)', async () => {
  const posts: { path: string; body: Record<string, unknown> }[] = [];
  const { runtimeThreads } = loadSource(
    new URL('../src/modules/threads/runtime/runtimeThreads.svelte.ts', import.meta.url),
    harness(async () => [], async (path, body) => {
      posts.push({ path, body: body as Record<string, unknown> });
      return path.endsWith('/runtime-sessions') ? { id: 'T', status: 'idle', last_sequence: 0 } : { sequence: 1 };
    }),
  );
  await runtimeThreads.start('ws', { provider: 'codex', cwd: null, title: 'T', model: null, plan: true, prompt: 'look around' });
  await runtimeThreads.prompt('T', { text: 'and now?' });
  const prompts = posts.filter((p) => p.path.endsWith('/prompt'));
  assert.deepEqual(prompts.map((p) => p.body.plan), [true, true], 'the second prompt is still a plan');
  runtimeThreads.planIds.T = false;
  await runtimeThreads.prompt('T', { text: 'go' });
  assert.equal(posts.at(-1)!.body.plan, undefined, 'leaving Plan stops asking for one');
});
