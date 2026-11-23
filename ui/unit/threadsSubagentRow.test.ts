// One timeline row per turn's subagents, and the waiting line's words
// (runtime/childThreads.ts `turnSubagents` / `subagentTally`,
// runtime/liveStatus.ts `waitingGroups`): agent-opened and person-delegated
// threads and the provider's own subagents of a turn under ONE header,
// subagents counted apart from commands.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import type { TurnItem } from '../src/lib/api/types';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));

const ct = await import('../src/modules/threads/runtime/childThreads.ts');
const live = await import('../src/modules/threads/runtime/liveStatus.ts');

const K1 = '01M84RK2CQ9VGADV5M5B40RAV1';
const K2 = '01M84RK2CQ9VGADV5M5B40RAV2';
const K3 = '01M84RK2CQ9VGADV5M5B40RAV3';
const base = { ordinal: 0, title: null, started_at: '2026-11-23T10:00:00Z', completed_at: null, updated_at: '2026-11-23T10:00:00Z' };
const open = (id: string, turn: string, target: string, provider: string, status = 'completed'): TurnItem =>
  ({ ...base, id, turn_id: turn, status, type: 'dynamic_tool', tool_name: 'mcp__otto__otto_thread_open', input: { provider, prompt: 'do it' }, output: `{"id":"${target}","access_mode":"full-access"}`, error: null }) as unknown as TurnItem;
const notice = (id: string, turn: string, ids: string[]): TurnItem =>
  ({ ...base, id, turn_id: turn, status: 'completed', type: 'system_notice', message: `You asked Codex: x [otto-thread:${ids.join(',')}:started]` }) as unknown as TurnItem;
const native = (id: string, turn: string, status = 'running'): TurnItem =>
  ({ ...base, id, turn_id: turn, status, type: 'subagent', tool_name: 'Task', description: 'Explore', tool_uses: 0 }) as unknown as TurnItem;

test('a turn’s subagents from every source are one row at the first of them', () => {
  const items = [open('a', 't1', K1, 'codex'), notice('n', 't1', [K2]), native('s', 't1'), open('b', 't2', K3, 'gemini')];
  const { groups, skip } = ct.turnSubagents(items);
  assert.deepEqual([...groups.keys()], ['a', 'b']);
  assert.deepEqual(groups.get('a')!.map(ct.kidKey), [K1, K2, 's']);
  assert.deepEqual(groups.get('a')![0], { kind: 'thread', id: K1, hint: { provider: 'codex', title: null, model: null, effort: null } });
  assert.deepEqual([...skip], ['n', 's']);
  assert.deepEqual(groups.get('b')!.map(ct.kidKey), [K3]);
});

test('an end notice draws a row only when its start is not in the timeline', () => {
  const ended = { ...notice('e', 't2', [K1]), message: `Codex finished: ok [otto-thread:${K1}:finished]` } as TurnItem;
  assert.equal(ct.turnSubagents([notice('n', 't1', [K1]), ended]).groups.size, 1);
  assert.deepEqual([...ct.turnSubagents([ended]).groups.keys()], ['e']);
});

test('a call still opening, a failed call and a sub-item are not subagents yet', () => {
  const failed = { ...open('c', 't1', 'k9', 'codex'), error: 'no such provider' } as TurnItem;
  const inner = { ...native('i', 't1'), parent_item_id: 's' } as TurnItem;
  const { groups } = ct.turnSubagents([open('a', 't1', 'k1', 'codex', 'running'), failed, inner]);
  assert.equal(groups.size, 0);
});

test('the tally reads in the five status words, most urgent first', () => {
  assert.deepEqual(ct.subagentTally(['working', 'done', 'starting', 'background']), { text: '3 working · 1 done', tone: 'live' });
  assert.deepEqual(ct.subagentTally(['needs-you', 'failed']), { text: '1 needs you · 1 failed', tone: 'ask' });
  assert.deepEqual(ct.subagentTally(['done', 'failed', 'stopped']), { text: '1 done · 1 failed · 1 stopped', tone: 'failed' });
  assert.equal(ct.childState(null, null).label, 'Working', 'never "Starting"');
});

test('the waiting line counts subagents apart from commands', () => {
  const t = (kind: string, child: string | null = null, label = kind) => ({ kind, childSessionId: child, label });
  assert.equal(live.waitingGroups([t('agent'), t('thread', 'k1'), t('command')]), '2 subagents and 1 command');
  assert.equal(live.waitingGroups([t('command'), t('timer'), t('agent')]), '1 subagent, 1 command and 1 timer');
  const f = live.deriveLiveFooter(null, null, false, [t('agent', 'k1', 'Share link API'), t('agent', 'k2', 'Share link docs')]);
  assert.deepEqual(f, { mode: 'waiting_bg', text: 'Waiting on 2 subagents', detail: 'Share link API, Share link docs' });
  assert.equal(live.isSubagentTask({ kind: 'command', childSessionId: null }), false);
});

test('the parent’s roll-up reads its subagents’ rows alone', () => {
  const row = (id: string, status: string, of: string | null, last: string | null = null) =>
    ({ id, status, last_turn_status: last, meta: of ? { child_of: { id: of, by: 'agent' } } : null }) as never;
  const rows = [row('p', 'idle', null), row('a', 'working', 'p'), row('b', 'awaiting_permission', 'p'), row('c', 'idle', 'p', 'failed'), row('d', 'closed', 'p', 'failed'), row('e', 'working', 'x')];
  assert.deepEqual(ct.childRollup(rows, 'p'), { working: 1, needs_you: 1, failed: 1 });
});
