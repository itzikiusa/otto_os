// Which thread needs you, at a glance (round 5, T-ATTENTION): a runtime
// thread's pending request and how its last turn ended reach every surface
// through `sessionState` (with or without the newer daemon's fields), the
// sidebar ranks families by their most urgent member, folded groups roll up,
// the child-group header says what the children really do, Agents & tasks
// maps a waiting child to "needs you", and the one-move navigation helpers.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import type { RuntimeSession, Session } from '../src/lib/api/types';
import { loadSource } from './sourceHarness.ts';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));

const { sessionState, turnOutcomeState } = await import('../src/lib/status.ts');
const ta = await import('../src/modules/threads/threadAttention.ts');
const { inboxOrder } = await import('../src/modules/threads/threadFamilies.ts');
const { agentTasks, childThreadStatusOf } = await import('../src/modules/threads/runtime/agentTasks.ts');
const ct = await import('../src/modules/threads/runtime/childThreads.ts');
const { sidebarOrder } = await import('../src/modules/threads/threadSidebarNav.ts');

/** Values from a vm-loaded store live in another realm: compare plain copies. */
const plain = (v: unknown): unknown => JSON.parse(JSON.stringify(v));

const thread = (meta: Record<string, unknown> = {}) => ({ kind: 'agent', status: 'idle', meta: { runtime: 'structured', ...meta } });

test('sessionState: a pending request on the row is needs-you without the live flag (after a reload)', () => {
  const st = sessionState(thread({ runtime_waiting: true }), 'idle', false);
  assert.equal(st.key, 'needs-you');
  assert.equal(st.label, 'Needs you');
  assert.equal(st.tone, 'warning');
});

test('sessionState: a failed last turn is not idle; the error class names it', () => {
  assert.deepEqual(
    [sessionState(thread({ last_turn_status: 'failed' }), 'idle').key, sessionState(thread({ last_turn_status: 'failed' }), 'idle').label, sessionState(thread({ last_turn_status: 'failed' }), 'idle').tone],
    ['failed', 'Failed', 'danger'],
  );
  assert.equal(sessionState(thread({ last_turn_status: 'failed', last_error_class: 'usage_limit' }), 'idle').label, 'Limited');
  assert.equal(sessionState(thread({ last_turn_status: 'failed', last_error_class: 'auth' }), 'idle').label, 'Not signed in');
  assert.equal(sessionState(thread({ last_turn_status: 'failed', last_error_class: 'brand_new' }), 'idle').label, 'Failed');
  const stopped = sessionState(thread({ last_turn_status: 'interrupted' }), 'idle');
  assert.deepEqual([stopped.key, stopped.label, stopped.tone], ['stopped', 'Stopped', 'neutral']);
  // A failed turn leaves the thread usable: not inactive, not resumable.
  assert.equal(sessionState(thread({ last_turn_status: 'failed' }), 'idle').inactive, false);
});

test('sessionState: absent fields (an older daemon) read idle; work and needs-you outrank a failure', () => {
  assert.equal(sessionState(thread(), 'idle').key, 'idle');
  assert.equal(sessionState(thread({ last_turn_status: 'completed' }), 'idle').key, 'idle');
  assert.equal(sessionState(thread({ last_turn_status: 'failed' }), 'working').key, 'working');
  assert.equal(sessionState(thread({ last_turn_status: 'failed' }), 'idle', true).key, 'needs-you');
  // A plain PTY session without meta is unchanged.
  assert.equal(sessionState({ kind: 'agent', status: 'idle' }, 'idle').key, 'idle');
  assert.equal(turnOutcomeState(null), null);
  assert.equal(turnOutcomeState('cancelled')?.key, 'stopped');
});

test('urgency: needs you › working › failed › the rest; roll-ups count and word it', () => {
  const S = (key: string) => ({ key, label: key });
  assert.deepEqual(['needs-you', 'working', 'running', 'failed', 'stopped', 'idle'].map((k) => ta.urgency(S(k))), [0, 1, 1, 2, 3, 3]);
  assert.equal(ta.mostUrgent([S('idle'), S('failed'), S('working')])?.key, 'working');
  assert.equal(ta.mostUrgent([]), null);
  const r = ta.rollup([S('needs-you'), S('working'), S('failed'), S('idle')]);
  assert.deepEqual(r, { needs: 1, working: 1, failed: 1, total: 4 });
  assert.deepEqual(ta.rollupNote(r), { text: '1 needs you', tone: 'warning' });
  assert.deepEqual(ta.rollupNote(ta.rollup([S('needs-you'), S('needs-you')])), { text: '2 need you', tone: 'warning' });
  assert.deepEqual(ta.rollupNote(ta.rollup([S('failed'), S('failed'), S('idle')])), { text: '2 failed', tone: 'danger' });
  assert.equal(ta.rollupNote(ta.rollup([S('idle'), S('working')])), null);
});

test('child group header: tallies the real child states', () => {
  assert.deepEqual(ta.childGroupSummary(['needs-you', 'working']), { text: '1 needs you · 1 working', tone: 'warning' });
  assert.deepEqual(ta.childGroupSummary(['failed', 'failed']), { text: '2 failed', tone: 'danger' });
  assert.deepEqual(ta.childGroupSummary(['done', 'done']), { text: 'all answered', tone: null });
  assert.deepEqual(ta.childGroupSummary(['done', 'failed']), { text: '1 failed · 1 answered', tone: 'danger' });
  assert.deepEqual(ta.childGroupSummary(['background', 'stopped']), { text: '1 working · 1 stopped', tone: null });
});

const sess = (id: string, at: number, parent?: string): Session =>
  ({ id, title: id, last_active_at: new Date(Date.UTC(2026, 10, 16, 10, at)).toISOString(), meta: parent ? { parent_session_id: parent } : {} }) as unknown as Session;

test('inboxOrder: a family ranks by its most urgent member', () => {
  // P is idle and oldest, but its child K waits on you: the family goes first.
  const list = [sess('A', 5), sess('W', 4), sess('P', 1), sess('K', 2, 'P')];
  const score: Record<string, number> = { A: 3, W: 1, P: 3, K: 0 };
  const rows = inboxOrder(list, (s) => score[s.id], (s) => (s.meta?.parent_session_id as string) ?? null);
  assert.deepEqual(rows.map((r) => r.session.id), ['P', 'K', 'W', 'A']);
  assert.deepEqual(rows[0].kids, ['K']);
  // Without an urgent child the order is urgency, then recency.
  score.K = 3;
  assert.deepEqual(inboxOrder(list, (s) => score[s.id], (s) => (s.meta?.parent_session_id as string) ?? null).map((r) => r.session.id), ['W', 'A', 'P', 'K']);
});

test('next that needs you: the next flagged thread after the current one, wrapping, never the current', () => {
  const order = ['a', 'b', 'c', 'd'];
  const needs = (id: string) => id === 'b' || id === 'd';
  assert.equal(ta.nextNeedsYou(order, 'b', needs), 'd');
  assert.equal(ta.nextNeedsYou(order, 'd', needs), 'b');
  assert.equal(ta.nextNeedsYou(order, null, needs), 'b');
  assert.equal(ta.nextNeedsYou(order, 'b', (id) => id === 'b'), null);
  assert.equal(ta.nextNeedsYou([], null, needs), null);
});

test('next / previous thread and the list keys', () => {
  const order = ['a', 'b', 'c'];
  assert.equal(ta.stepThread(order, 'b', 1), 'c');
  assert.equal(ta.stepThread(order, 'c', 1), 'c');
  assert.equal(ta.stepThread(order, 'a', -1), 'a');
  assert.equal(ta.stepThread(order, 'zz', 1), 'a');
  assert.equal(ta.stepThread(order, null, -1), 'c');
  assert.equal(ta.rovingIndex('ArrowDown', 0, 3), 1);
  assert.equal(ta.rovingIndex('ArrowDown', 2, 3), 2);
  assert.equal(ta.rovingIndex('ArrowUp', 0, 3), 0);
  assert.equal(ta.rovingIndex('Home', 2, 3), 0);
  assert.equal(ta.rovingIndex('End', 0, 3), 2);
  assert.equal(ta.rovingIndex('Enter', 0, 3), null);
  const k = (key: string, extra: Record<string, unknown> = {}) => ({ key, code: '', metaKey: true, ctrlKey: false, altKey: true, shiftKey: false, ...extra });
  assert.equal(ta.threadChord(k('ArrowDown')), 'next');
  assert.equal(ta.threadChord(k('ArrowUp')), 'previous');
  assert.equal(ta.threadChord(k('˜', { code: 'KeyN' })), 'needsYou');
  assert.equal(ta.threadChord(k('ArrowDown', { altKey: false })), null);
  assert.equal(ta.threadChord(k('ArrowDown', { shiftKey: true })), null);
});

test('a phone opens on the most recent thread that needs you, else the newest', () => {
  assert.equal(ta.initialThread(['new', 'mid', 'old'], ['old', 'mid']), 'old');
  assert.equal(ta.initialThread(['new', 'mid'], ['gone']), 'new');
  assert.equal(ta.initialThread([], []), null);
});

test('sidebar order: pinned, projects, then side groups; no duplicates or closed rows', () => {
  const s = (id: string, archived = false) => ({ id, archived }) as Session;
  assert.deepEqual(
    sidebarOrder([s('p')], [{ items: [{ session: s('a') }, { session: s('p') }] }], [{ items: [s('r'), s('x', true)] }]),
    ['p', 'a', 'r'],
  );
});

const row = (over: Partial<RuntimeSession> = {}): RuntimeSession =>
  ({ id: 'kid', workspace_id: 'w', provider: 'codex', title: 'Kid', cwd: '/r', status: 'idle', access_mode: 'approval-required', created_by: 'u', created_at: '2026-11-16T10:00:00Z', last_active_at: '2026-11-16T10:00:00Z', last_sequence: 0, ...over }) as RuntimeSession;

test('Agents & tasks: a waiting child needs you (listed with the active ones); failed / stopped are not done', () => {
  assert.equal(childThreadStatusOf('awaiting_permission'), 'needs-you');
  assert.equal(childThreadStatusOf('working'), 'running');
  assert.equal(childThreadStatusOf('idle', 'failed'), 'failed');
  assert.equal(childThreadStatusOf('idle', 'interrupted'), 'stopped');
  assert.equal(childThreadStatusOf('closed'), 'stopped');
  assert.equal(childThreadStatusOf('idle', 'completed'), 'done');
  assert.equal(childThreadStatusOf('idle'), 'done');
  const child = (id: string, status: RuntimeSession['status'], lastTurnStatus?: 'failed') =>
    ({ id, title: id, provider: 'codex', model: null, status, createdAt: '2026-11-16T10:00:00Z', updatedAt: '2026-11-16T10:01:00Z', lastTurnStatus });
  const t = agentTasks([], [], { childThreads: [child('ask', 'awaiting_permission'), child('bad', 'idle', 'failed')] });
  assert.deepEqual(t.running.map((r) => [r.label, r.status]), [['ask', 'needs-you']]);
  assert.deepEqual(t.finished.map((r) => [r.label, r.status]), [['bad', 'failed']]);
});

test('childState: with no projection loaded the row says how the last turn ended', () => {
  assert.equal(ct.childState(row({ last_turn_status: 'failed' }), null).tone, 'failed');
  assert.equal(ct.childState(row({ last_turn_status: 'interrupted' }), null).tone, 'stopped');
  assert.equal(ct.childState(row({ last_turn_status: 'completed' }), null).tone, 'done');
  assert.equal(ct.childState(row({ status: 'awaiting_permission', last_turn_status: 'failed' }), null).tone, 'needs-you');
  assert.equal(ct.childState(row(), null).tone, 'starting');
  assert.equal(ct.turnOutcome(row({ last_turn_status: 'failed' }), null), 'failed');
  assert.equal(ct.turnOutcome(row(), null), null);
});

function loadThreads() {
  const noted: [string, boolean, string | undefined][] = [];
  const synced: unknown[] = [];
  const { runtimeThreads, asThreadSession, turnOutcomeOf, keepOutcome } = loadSource(new URL('../src/modules/threads/runtime/runtimeThreads.svelte.ts', import.meta.url), {
    '../../../lib/api/client': { api: { get: async (p: string) => (p === '/runtime-sessions' ? [row({ id: 'W', status: 'awaiting_permission', title: 'Waits' }), row({ id: 'I' })] : []) } },
    '../../../lib/stores/runtimeDraftRecovery.svelte': { installRuntimeDraftRecovery: () => {} },
    '../../../lib/live': { onLive: () => () => {}, appLive: { onResync: () => () => {} } },
    '../../../lib/router.svelte': { router: { module: 'agents' } },
    '../../../lib/stores/runtimeAttention.svelte': { noteRuntimeWaiting: (id: string, w: boolean, title?: string) => noted.push([id, w, title]), syncRuntimeWaiting: (rows: unknown) => synced.push(rows) },
    '../../../lib/stores/uiControl.svelte': { uiControl: { addSource: () => {} } },
    '../threadsStore.svelte': { threads: { focusedSessionId: null } },
    '../../../lib/stores/runtimeDrafts.svelte': { runtimeDrafts: { forget: () => {} } },
    '../../../lib/providers': { isRuntimeProvider: () => true, RUNTIME_PROVIDERS: ['codex'], sessionProviderSlug: (r: { provider: string }) => r.provider },
  });
  return { runtimeThreads, asThreadSession, turnOutcomeOf, keepOutcome, noted, synced };
}

test('runtime rows carry their attention into the Threads row; the list syncs the waiting set', async () => {
  const { runtimeThreads, asThreadSession, synced } = loadThreads();
  assert.equal(asThreadSession(row({ status: 'awaiting_permission' })).meta.runtime_waiting, true);
  assert.equal(asThreadSession(row({ last_turn_status: 'failed', last_error_class: 'usage_limit' })).meta.last_error_class, 'usage_limit');
  assert.equal(asThreadSession(row()).meta.last_turn_status, undefined);
  await runtimeThreads.loadAll();
  assert.deepEqual(plain(synced), [[{ id: 'W', title: 'Waits' }]]);
});

test('a turn that ends live sets the row outcome (older daemon); a new turn clears it', () => {
  const { runtimeThreads, turnOutcomeOf } = loadThreads();
  runtimeThreads.byId = { A: row({ id: 'A' }) };
  runtimeThreads.onEvent({ session_id: 'A', sequence: 2, ts: 'now', type: 'turn.terminal', turn_id: 't', status: 'failed', failure: { class: 'provider_error', message: 'You have hit your usage limit' } });
  assert.deepEqual([runtimeThreads.byId.A.last_turn_status, runtimeThreads.byId.A.last_error_class], ['failed', 'usage_limit']);
  runtimeThreads.onEvent({ session_id: 'A', sequence: 3, ts: 'now', type: 'provider_session.updated', provider_session: { status: 'running', background_tasks: [] } });
  assert.equal(runtimeThreads.byId.A.last_turn_status, null);
  assert.equal(runtimeThreads.byId.A.status, 'working');
  // A list re-read from a daemon without the field keeps what this tab saw; a newer daemon's value wins.
  const { keepOutcome } = loadThreads();
  const seen = row({ last_turn_status: 'failed', last_error_class: 'auth' });
  assert.equal(keepOutcome(row(), seen).last_turn_status, 'failed');
  assert.equal(keepOutcome(row({ last_turn_status: 'completed' }), seen).last_turn_status, 'completed');
  assert.equal(keepOutcome(row(), undefined).last_turn_status, undefined);
  assert.deepEqual(plain(turnOutcomeOf('cancelled')), { last_turn_status: 'interrupted', last_error_class: null });
  assert.deepEqual(plain(turnOutcomeOf('failed', { class: 'x', message: 'Please run /login' })), { last_turn_status: 'failed', last_error_class: 'auth' });
});

test('the waiting store: reads the old id-array shape, keeps titles, syncs to the list', () => {
  const store = new Map<string, string>([['otto_runtime_waiting', JSON.stringify(['old-id'])]]);
  const ra = loadSource(new URL('../src/lib/stores/runtimeAttention.svelte.ts', import.meta.url), {
    '../storage': { lsGet: (k: string) => store.get(k) ?? null, lsSet: (k: string, v: string) => store.set(k, v) },
  });
  assert.deepEqual(plain(ra.runtimeWaiting().map((w: { id: string }) => w.id)), ['old-id']);
  assert.equal(ra.runtimeFlagged({}), 1, 'a pending request counts without the sticky flag');
  ra.noteRuntimeWaiting('new', true, 'New one');
  assert.equal(ra.isRuntimeWaiting('new'), true);
  ra.syncRuntimeWaiting([{ id: 'new', title: 'Renamed' }]);
  assert.deepEqual(plain(ra.runtimeWaiting().map((w: { id: string; title: string }) => [w.id, w.title])), [['new', 'Renamed']]);
  ra.noteRuntimeWaiting('new', false);
  assert.equal(ra.runtimeFlagged({}), 0);
  assert.equal(JSON.parse(store.get('otto_runtime_waiting')!).length, 0);
});

test('a Limited thread (failed on the usage limit) rolls up and ranks exactly like Failed', async () => {
  const { sessionState } = await import('../src/lib/status.ts');
  const { rollup, rollupNote, mostUrgent } = await import('../src/modules/threads/threadAttention.ts');
  const limited = sessionState({ kind: 'agent', status: 'idle', meta: { last_turn_status: 'failed', last_error_class: 'usage_limit' } } as never, 'idle');
  assert.equal(limited.label, 'Limited');
  assert.equal(limited.key, 'failed');
  const idle = { key: 'idle', label: 'Idle' };
  assert.deepEqual(rollup([limited, idle]), { needs: 0, working: 0, failed: 1, total: 2 });
  assert.equal(rollupNote(rollup([limited]))?.text, '1 failed');
  assert.equal(mostUrgent([idle, limited]), limited);
});
