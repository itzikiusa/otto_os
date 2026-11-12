// Threads inside a thread (docs/features/threads-structured-runtime.md
// "Threads inside a thread"): the daemon's notice markers, a child's state
// in one word, its activity / answer / elapsed time, the `@` agent menu, and
// the sidebar's families (children nested, foldable, detached when the
// parent is gone).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import type { ProviderTurn, RuntimeSession, Session, TurnItem } from '../src/lib/api/types';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));

const ct = await import('../src/modules/threads/runtime/childThreads.ts');
const aa = await import('../src/modules/threads/runtime/askAgents.ts');
const { inboxOrder } = await import('../src/modules/threads/threadFamilies.ts');
const { emptyProjection } = await import('../src/modules/threads/runtime/projection.ts');

const T0 = Date.parse('2026-11-12T10:00:00Z');
const at = (s: number): string => new Date(T0 + s * 1000).toISOString();
const row = (over: Partial<RuntimeSession> = {}): RuntimeSession =>
  ({ id: 'kid', workspace_id: 'w', provider: 'codex', title: 'Kid', cwd: '/r', status: 'idle', access_mode: 'approval-required', created_by: 'u', created_at: at(0), last_active_at: at(0), last_sequence: 0, ...over }) as RuntimeSession;
const turn = (id: string, ordinal: number, status: ProviderTurn['status'], start: number, end: number | null): ProviderTurn =>
  ({ id, ordinal, status, started_at: at(start), completed_at: end === null ? null : at(end) }) as ProviderTurn;
const say = (id: string, turnId: string, text: string): TurnItem =>
  ({ id, turn_id: turnId, ordinal: 0, status: 'completed', title: null, started_at: at(0), completed_at: at(0), updated_at: at(0), type: 'assistant_message', message_id: id, text, streaming: false }) as TurnItem;
const cmd = (id: string, turnId: string, input: string): TurnItem =>
  ({ id, turn_id: turnId, ordinal: 1, status: 'running', title: null, started_at: at(1), completed_at: null, updated_at: at(1), type: 'command_execution', input }) as TurnItem;

test('the daemon’s notice marker names the children and what happened', () => {
  assert.deepEqual(JSON.parse(JSON.stringify(ct.threadNoticeOf('You asked Codex, Gemini: Why? [otto-thread:a1,b2:started]'))), {
    text: 'You asked Codex, Gemini: Why?',
    ids: ['a1', 'b2'],
    event: 'started',
  });
  assert.equal(ct.threadNoticeOf('Codex finished: ok [otto-thread:a1:finished]')?.event, 'finished');
  assert.equal(ct.threadNoticeOf('Codex finished: ok'), null);
  assert.equal(ct.threadNoticeOf('Run done [otto-run:r1]'), null);
});

test('child_of reads only a well-formed opener', () => {
  assert.deepEqual(ct.childOf(row({ meta: { child_of: { id: 'p', by: 'agent' } } })), { id: 'p', by: 'agent' });
  assert.equal(ct.childOf(row({ meta: { child_of: { id: 'p', by: 'robot' } } })), null);
  assert.equal(ct.childOf(row()), null);
});

test('a child’s state: needs-you outranks work, work outranks how it ended', () => {
  const p = emptyProjection();
  assert.equal(ct.childState(null, null).tone, 'starting');
  assert.equal(ct.childState(row({ status: 'closed' }), p).tone, 'stopped');
  assert.equal(ct.childState(row({ status: 'awaiting_permission' }), p).label, 'Needs you');
  assert.equal(ct.childState(row({ status: 'working' }), p).tone, 'working');
  assert.equal(ct.childState(row(), { ...p, turns: [turn('t1', 1, 'failed', 0, 5)] }).tone, 'failed');
  assert.equal(ct.childState(row(), { ...p, turns: [turn('t1', 1, 'interrupted', 0, 5)] }).tone, 'stopped');
  assert.equal(ct.childState(row(), { ...p, turns: [turn('t1', 1, 'completed', 0, 5)] }).label, 'Done');
  assert.equal(ct.rowTone('needs-you'), 'ask');
  assert.equal(ct.rowTone('starting'), 'live');
});

test('the card’s line: latest activity while it works, the answer once it ends', () => {
  const p = { ...emptyProjection(), turns: [turn('t1', 1, 'completed', 0, 30), turn('t2', 2, 'running', 40, null)], items: [say('a', 't1', '## Root cause\nThe flaky test.'), cmd('c', 't2', 'npm test')] };
  assert.equal(ct.activityOf(p), 'npm test');
  assert.equal(ct.answerOf(p), '## Root cause\nThe flaky test.');
  assert.equal(ct.firstLine(ct.answerOf(p)), 'Root cause');
  assert.equal(ct.elapsedMs(p, T0 + 60_000), 60_000);
  const ended = { ...p, turns: [turn('t1', 1, 'completed', 0, 30)] };
  assert.equal(ct.elapsedMs(ended, T0 + 99_000), 30_000);
  assert.equal(ct.elapsedMs(emptyProjection(), T0), null);
});

test('children of a thread: only the ones it opened, oldest first', () => {
  const kids = ct.childrenOf(
    [row({ id: 'b', created_at: at(5), meta: { child_of: { id: 'p', by: 'person' } } }), row({ id: 'a', created_at: at(1), meta: { child_of: { id: 'p', by: 'agent' } } }), row({ id: 'x', meta: { child_of: { id: 'q', by: 'person' } } })],
    'p',
  );
  assert.deepEqual(kids.map((k) => k.id), ['a', 'b']);
});

test('typing @ offers agents and models; a pick replaces the mention', () => {
  assert.deepEqual(aa.mentionAt('ask @cod'), { query: 'cod', start: 4 });
  assert.equal(aa.mentionAt('mail me@x'), null, 'an email is not a mention');
  assert.equal(aa.dropMention('ask @cod', 4), 'ask ');
  assert.equal(aa.dropMention('@cod', 0), '');
  const models = (p: string): { id: string; label: string }[] => (p === 'claude' ? [{ id: 'opus', label: 'Claude Opus' }] : p === 'codex' ? [{ id: 'gpt-5', label: 'GPT-5' }] : []);
  const name = (p: string): string => p.charAt(0).toUpperCase() + p.slice(1);
  const opus = aa.mentionOptions('opus', models, name);
  assert.equal(opus[0].model, 'opus');
  assert.equal(opus[0].provider, 'claude');
  const codex = aa.mentionOptions('codex', models, name);
  assert.deepEqual(codex.map((o) => o.model), [null, 'gpt-5'], 'the provider (its default model) first');
  assert.equal(aa.mentionOptions('opus:codex', models, name).length, 0, 'every word must match');
  assert.equal(aa.mentionOptions('', models, name).length, 8, 'capped');
});

test('picked agents: no duplicates, at most four', () => {
  let list = aa.addTarget([], { provider: 'codex' });
  list = aa.addTarget(list, { provider: 'codex' });
  assert.equal(list.length, 1);
  for (const p of ['claude', 'gemini', 'grok', 'pi'] as const) list = aa.addTarget(list, { provider: p });
  assert.equal(list.length, aa.ASK_MAX_TARGETS);
});

test('the sidebar nests children under their parent, folds them, and marks orphans detached', () => {
  const s = (id: string, parent: string | null, active: number): Session =>
    ({ id, title: id, provider: 'codex', status: 'idle', last_active_at: at(active), meta: parent ? { parent_session_id: parent } : {} }) as unknown as Session;
  const list = [s('kid1', 'lead', 5), s('lead', null, 1), s('kid2', 'lead', 3), s('orphan', 'gone', 9)];
  const parentOf = (x: Session): string | null => (x.meta as { parent_session_id?: string }).parent_session_id ?? null;
  const rows = inboxOrder(list, () => 2, parentOf);
  assert.deepEqual(rows.map((r) => [r.session.id, r.child, r.detached]), [
    ['orphan', false, true],
    ['lead', false, false],
    ['kid1', true, false],
    ['kid2', true, false],
  ]);
  assert.deepEqual(rows[1].kids, ['kid1', 'kid2']);
  const folded = inboxOrder(list, () => 2, parentOf, (id) => id === 'lead');
  assert.deepEqual(folded.map((r) => r.session.id), ['orphan', 'lead']);
  assert.deepEqual(folded[1].kids, ['kid1', 'kid2'], 'a folded parent still counts its children');
});
