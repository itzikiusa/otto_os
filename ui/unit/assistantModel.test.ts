// Assistant module pure helpers (node:test, Node's built-in type stripping):
// routing-hint parsing, labels, Spaces/Recent grouping, card mapping (turns +
// tasks → chat cards) and the chat timeline, the transcript merge, the
// needs-you reducer and its stale-frame guards, task grouping, routing rows.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  browserOf,
  buildTimeline,
  groupTasks,
  groupThreads,
  latestThreadId,
  limitNotice,
  loadShare,
  memoryChipOf,
  modelShort,
  needsYouByThread,
  parseKeywords,
  parseRouteHint,
  providerLabel,
  reduceNeedsYou,
  taskStateLabel,
  taskTone,
  threadCards,
  upsertNewer,
  type NeedsYouState,
} from '../src/modules/assistant/model.ts';
import { mergeWithTranscript, messageAuthor, inferProvider } from '../src/modules/assistant/chat.ts';
import { deliverLabel, whenLabel } from '../src/modules/assistant/format.ts';
import type { AssistantTask, AssistantTurn } from '../src/lib/api/types.ts';
import type { RenderItem } from '../src/modules/assistant/chat.ts';

const T0 = '2026-09-24T10:30:00Z';
const at = (min: number, sec = 0): string => new Date(Date.parse(T0) + min * 60_000 + sec * 1000).toISOString();

function turn(id: string, role: AssistantTurn['role'], kind: AssistantTurn['kind'], created_at: string, extra: Partial<AssistantTurn> = {}): AssistantTurn {
  return { id, thread_id: 'th', role, kind, text: id, provider: null, model: null, route_reason: null, session_id: null, attachments: [], data: null, created_at, ...extra };
}
function task(id: string, extra: Partial<AssistantTask> = {}): AssistantTask {
  return {
    id,
    thread_id: 'th',
    kind: 'task',
    state: 'running',
    title: id,
    detail: '',
    origin: 'app',
    run_at: null,
    timezone: 'UTC',
    schedule_id: null,
    agent_id: null,
    agent_run_id: null,
    needs_you: null,
    result: null,
    created_at: T0,
    updated_at: T0,
    finished_at: null,
    ...extra,
  };
}

// ── routing hints ──
test('a leading @codex routes the turn; the daemon-bound text drops it', () => {
  assert.deepEqual(parseRouteHint('@codex fix this script'), { provider: 'codex', text: 'fix this script' });
  assert.deepEqual(parseRouteHint('  @Claude, plan my week'), { provider: 'claude', text: 'plan my week' });
  assert.deepEqual(parseRouteHint('@claude'), { provider: 'claude', text: '' });
});

test('mid-sentence mentions, emails, handles and other providers never route', () => {
  for (const s of ['ask @codex about it', 'mail me@codex.dev', '@claudette hi', '@gemini hi', 'no hint', 'a@claude']) {
    assert.deepEqual(parseRouteHint(s), { provider: null, text: s }, s);
  }
});

// ── labels ──
test('provider labels read like the mockup chip', () => {
  assert.equal(modelShort('claude-sonnet-4-5'), 'Sonnet');
  assert.equal(modelShort('OPUS'), 'Opus');
  assert.equal(modelShort('gpt-5-codex'), 'gpt-5-codex');
  assert.equal(providerLabel('claude', 'claude-sonnet-4-5'), 'Claude · Sonnet');
  assert.equal(providerLabel('codex', null), 'Codex');
  assert.equal(providerLabel(null, null), 'Auto');
  assert.equal(providerLabel('grok', null), 'Grok');
});

// ── threads ──
const th = (id: string, slot: 1 | 2 | 3 | 4 | null, updated: string) => ({ id, space_slot: slot, updated_at: updated });

test('threads split into Spaces 01–04 and Recent (newest first)', () => {
  const g = groupThreads([th('a', 2, at(0)), th('b', null, at(1)), th('c', null, at(3)), th('d', 2, at(4))]);
  assert.deepEqual(g.spaces.map((s) => s?.id ?? null), [null, 'a', null, null]);
  // A second thread claiming a taken slot falls back to Recent.
  assert.deepEqual(g.recent.map((t) => t.id), ['d', 'c', 'b']);
});

test('the page opens on the remembered thread, else the latest', () => {
  const ts = [th('a', 1, at(0)), th('b', null, at(9))];
  assert.equal(latestThreadId(ts, 'a'), 'a');
  assert.equal(latestThreadId(ts, 'gone'), 'b');
  assert.equal(latestThreadId([], null), null);
});

// ── card mapping ──
test('turn kinds map to cards; one card per task; unreferenced thread tasks still show', () => {
  const turns = [
    turn('u1', 'user', 'message', at(0)),
    turn('m1', 'system', 'memory', at(1), { data: { action: 'remembered', memory_ids: ['x'], undo: { kind: 'delete', memory_id: 'x' } } }),
    turn('a1', 'assistant', 'message', at(2)),
    turn('r1', 'system', 'reminder', at(2, 10), { data: { task_id: 'rem' } }),
    turn('r2', 'system', 'reminder', at(40), { data: { task_id: 'rem' } }), // the reminder firing: same card
    turn('d1', 'system', 'delegation', at(3), { data: { task_id: 'del' } }),
    turn('x1', 'system', 'route', at(4), { data: { from: 'claude', to: 'codex' } }),
    turn('s1', 'system', 'message', at(5)),
  ];
  const cards = threadCards(turns, [task('rem'), task('del'), task('appr', { kind: 'approval', created_at: at(6) }), task('other', { thread_id: 'elsewhere' }), task('mr', { kind: 'memory_review' })], 'th');
  assert.deepEqual(
    cards.map((c) => `${c.kind}:${c.kind === 'task' ? c.task_id : c.id}`),
    ['memory:m1', 'task:rem', 'task:del', 'line:x1', 'line:s1', 'task:appr'],
  );
});

test('memory chip payloads are validated', () => {
  assert.deepEqual(memoryChipOf(turn('m', 'system', 'memory', T0, { data: { action: 'forgot', memory_ids: ['a'], undo: { kind: 'restore', undo_tokens: ['t'] } } })), {
    action: 'forgot',
    memory_ids: ['a'],
    undo: { kind: 'restore', undo_tokens: ['t'] },
  });
  assert.equal(memoryChipOf(turn('m', 'system', 'memory', T0, { data: { action: 'weird' } })), null);
  assert.equal(memoryChipOf(turn('m', 'system', 'task', T0, { data: { action: 'remembered' } })), null);
});

// ── timeline ──
test('memory chips sit on the reply that follows them; cards land by time', () => {
  const msgs = [
    { id: 'u1', role: 'user' as const, ts: at(0) },
    { id: 'a1', role: 'assistant' as const, ts: at(2) },
    { id: 'a2', role: 'assistant' as const, ts: at(8) },
  ];
  const cards = threadCards(
    [
      turn('m1', 'system', 'memory', at(1), { data: { action: 'remembered', memory_ids: [], undo: null } }),
      turn('r1', 'system', 'reminder', at(2, 30), { data: { task_id: 'rem' } }),
      turn('m2', 'system', 'memory', at(9), { data: { action: 'remembered', memory_ids: [], undo: null } }),
    ],
    [],
    'th',
  );
  const out = buildTimeline(msgs, cards);
  assert.deepEqual(
    out.map((e) => (e.kind === 'turn' ? `${e.turn.id}[${e.memory.map((m) => m.id).join(',')}]` : e.card.id)),
    ['u1[]', 'a1[m1]', 'r1', 'a2[]', 'm2'],
  );
});

test('timestamps with different precision still order as instants', () => {
  const out = buildTimeline([{ id: 'a', role: 'assistant' as const, ts: '2026-09-24T10:00:00.500+00:00' }], threadCards([turn('c', 'system', 'route', '2026-09-24T10:00:00Z')], [], 'th'));
  assert.deepEqual(out.map((e) => (e.kind === 'turn' ? e.turn.id : e.card.id)), ['c', 'a']);
});

// ── transcript merge ──
const ri = (id: string, role: 'user' | 'assistant', ts: string, model: string | null = null): RenderItem => ({
  id,
  role,
  turns: [],
  blocks: [{ kind: 'text', md: id }],
  system: [],
  duration_ms: null,
  ts,
  model,
  reasoning_steps: 0,
});

test('replies of the current session come from the transcript; older providers stay from the index', () => {
  const index = [
    turn('u1', 'user', 'message', at(0), { session_id: 's-old' }),
    turn('a1', 'assistant', 'message', at(1), { session_id: 's-old', provider: 'claude' }),
    turn('u2', 'user', 'message', at(5), { session_id: 's-new' }),
    turn('a2', 'assistant', 'message', at(6, 5), { session_id: 's-new', provider: 'codex' }),
    turn('mem', 'system', 'memory', at(6)),
  ];
  const live = [ri('pkt', 'user', at(5)), ri('L2', 'assistant', at(6)), ri('L3', 'assistant', at(9))];
  const out = mergeWithTranscript(index, live, 's-new');
  // The pasted hand-off packet (a transcript USER item) never replaces the user's own words.
  assert.deepEqual(out.map((m) => m.item.id), ['u1', 'a1', 'u2', 'L2', 'L3']);
  assert.equal(out[3].turn?.id, 'a2'); // badge still comes from the index turn
  assert.equal(out[4].turn, null); // streamed, not indexed yet
  assert.deepEqual(messageAuthor(out[3], { provider: 'claude', model: null }), { provider: 'codex', model: null });
});

test('without a session the index is the whole conversation', () => {
  const out = mergeWithTranscript([turn('u', 'user', 'message', at(0)), turn('s', 'system', 'message', at(1))], [ri('x', 'assistant', at(2))], null);
  assert.deepEqual(out.map((m) => m.item.id), ['u']);
});

test('provider inference from a model id', () => {
  assert.equal(inferProvider('claude-opus-4-1'), 'claude');
  assert.equal(inferProvider('gpt-5-codex'), 'codex');
  assert.equal(inferProvider('o3'), 'codex');
  assert.equal(inferProvider('grok-4'), null);
});

// ── needs you ──
const empty: NeedsYouState = { items: [], seen: {} };
const ny = (id: string, updated: string, state: AssistantTask['state'] = 'needs_you', thread = 't1') =>
  task(id, { kind: 'approval', state, thread_id: thread, updated_at: updated });

test('the queue holds exactly the tasks whose newest row is needs_you', () => {
  let s = reduceNeedsYou(empty, { type: 'load', items: [ny('a', at(0)), ny('b', at(0), 'done')], startedAt: at(0) });
  assert.deepEqual(s.items.map((i) => i.id), ['a']);
  s = reduceNeedsYou(s, { type: 'upsert', task: ny('a', at(1), 'running') });
  assert.equal(s.items.length, 0);
  // It can come back: a later question on the same task re-enters the queue.
  s = reduceNeedsYou(s, { type: 'upsert', task: ny('a', at(2)) });
  assert.deepEqual(s.items.map((i) => i.id), ['a']);
});

test('stale frames and slow snapshots never roll the queue back', () => {
  let s = reduceNeedsYou(empty, { type: 'upsert', task: ny('a', at(5), 'running') });
  // An older "needs_you" frame for a task we know moved on is ignored…
  s = reduceNeedsYou(s, { type: 'upsert', task: ny('a', at(1)) });
  assert.equal(s.items.length, 0);
  // …and so is an older row inside a snapshot.
  s = reduceNeedsYou(s, { type: 'load', items: [ny('a', at(2))], startedAt: at(3) });
  assert.equal(s.items.length, 0);
});

test('a snapshot drops what left the queue but keeps what arrived while it was in flight', () => {
  let s = reduceNeedsYou(empty, { type: 'upsert', task: ny('old', at(0)) });
  s = reduceNeedsYou(s, { type: 'upsert', task: ny('new', at(10)) });
  s = reduceNeedsYou(s, { type: 'load', items: [], startedAt: at(5) });
  assert.deepEqual(s.items.map((i) => i.id), ['new']);
});

test('per-thread counts', () => {
  const s = reduceNeedsYou(empty, { type: 'load', items: [ny('a', at(0)), ny('b', at(0)), ny('c', at(0), 'needs_you', 't2'), ny('d', at(0), 'needs_you', '')], startedAt: at(0) });
  assert.deepEqual(needsYouByThread(s.items), { t1: 2, t2: 1 });
});

// ── tasks ──
test('tasks group into board columns, newest change first', () => {
  const g = groupTasks([
    task('1', { state: 'running', updated_at: at(0) }),
    task('2', { state: 'running', updated_at: at(1) }),
    task('3', { state: 'done', updated_at: at(0) }),
    task('4', { state: 'cancelled', updated_at: at(2) }),
  ]);
  assert.deepEqual(g.running.map((t) => t.id), ['2', '1']);
  assert.deepEqual(g.finished.map((t) => t.id), ['4', '3']);
  assert.equal(g.needs_you.length, 0);
});

test('reminders read Scheduled / Delivered', () => {
  assert.equal(taskStateLabel({ kind: 'reminder', state: 'queued' }), 'Scheduled');
  assert.equal(taskTone({ kind: 'reminder', state: 'queued' }), 'ok');
  assert.equal(taskStateLabel({ kind: 'reminder', state: 'done' }), 'Delivered');
  assert.equal(taskStateLabel({ kind: 'task', state: 'needs_you' }), 'Needs you');
});

test('upsertNewer ignores stale rows', () => {
  const a = [{ id: 'k', updated_at: at(5), v: 2 }];
  assert.equal(upsertNewer(a, { id: 'k', updated_at: at(1), v: 1 })[0].v, 2);
  assert.equal(upsertNewer(a, { id: 'k', updated_at: at(6), v: 3 })[0].v, 3);
  assert.deepEqual(upsertNewer(a, { id: 'n', updated_at: at(0), v: 0 }).map((x) => x.id), ['n', 'k']);
});

test('browser progress is read defensively from result.browser', () => {
  assert.equal(browserOf(task('t')), null);
  const b = browserOf(task('t', { result: { browser: { url: 'https://x.example', steps: [{ label: 'a', state: 'done' }, { label: 'b', state: 'weird' }, { nope: 1 }] } } }));
  assert.deepEqual(b?.steps, [
    { label: 'a', state: 'done' },
    { label: 'b', state: 'todo' },
  ]);
  assert.equal(b?.url, 'https://x.example');
  assert.equal(b?.tab_id, null);
});

// ── routing / misc ──
test('keyword lists and load share', () => {
  assert.deepEqual(parseKeywords(' Terraform, jq ,, jq\nregex '), ['terraform', 'jq', 'regex']);
  const s = loadShare([
    { provider: 'claude', total_tokens: 2 },
    { provider: 'codex', total_tokens: 1 },
  ]);
  assert.deepEqual(s.map((x) => [x.provider, x.pct]), [['claude', 67], ['codex', 33]]);
  assert.equal(s.reduce((n, x) => n + x.pct, 0), 100);
  assert.deepEqual(loadShare([]), []);
});

test('limit notice text', () => {
  assert.equal(limitNotice('claude', at(0), 'codex', () => '14:00'), 'Claude limit reached until 14:00 — continue on Codex?');
  assert.equal(limitNotice('codex', null, null, () => ''), 'Codex limit reached');
});

test('reminder labels', () => {
  const now = new Date('2026-09-24T08:00:00');
  assert.match(whenLabel('2026-09-24T17:00:00', now), /^Today, /);
  assert.match(whenLabel('2026-09-25T09:00:00', now), /^Tomorrow, /);
  assert.equal(deliverLabel('phone'), 'Delivered to your phone + a notification');
  assert.equal(deliverLabel('app'), 'Delivered here + a notification');
});
