// The Threads work log's quiet presentation (modules/threads/runtime/
// workLog.ts, banners.ts, agentRows.ts): a finished stretch of tool calls
// folds into "Worked for …" unless it holds something a person must see; a
// running group names its live call; labels lose `cd <dir> &&` and the
// thread's folder; a failed command reads "exit N"; notices above the
// composer stack by priority with at most two shown.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import type { ProviderTurn, TurnItem } from '../src/lib/api/types';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));

const wl = await import('../src/modules/threads/runtime/workLog.ts');
const { splitBanners } = await import('../src/modules/threads/runtime/banners.ts');
const { threadCall, elapsedBetween } = await import('../src/modules/threads/runtime/agentRows.ts');

const T0 = '2026-11-12T10:00:00Z';
const at = (s: number): string => new Date(Date.parse(T0) + s * 1000).toISOString();
const cmd = (id: string, start: number, end: number | null, extra: Partial<Record<string, unknown>> = {}): TurnItem =>
  ({
    id, turn_id: 't1', ordinal: start, status: end === null ? 'running' : 'completed', title: null,
    started_at: at(start), completed_at: end === null ? null : at(end), updated_at: at(end ?? start),
    type: 'command_execution', input: `cd /repo && npm test ${id}`, ...extra,
  }) as TurnItem;
const turn = (status: ProviderTurn['status']): ProviderTurn => ({ id: 't1', ordinal: 1, status, started_at: T0, completed_at: null });
const none = new Set<string>();

test('a leading cd prefix is dropped, never the whole command', () => {
  assert.equal(wl.stripCdPrefix('cd /repo && npm test'), 'npm test');
  assert.equal(wl.stripCdPrefix('cd "/my repo"; ls -la'), 'ls -la');
  assert.equal(wl.stripCdPrefix("pushd 'x' && make"), 'make');
  assert.equal(wl.stripCdPrefix('cd /repo'), 'cd /repo');
  assert.equal(wl.stripCdPrefix('echo cd /x && y'), 'echo cd /x && y');
});

test('paths read relative to the thread folder', () => {
  assert.equal(wl.relativeToCwd('Edited /Users/me/repo/src/a.ts', '/Users/me/repo/'), 'Edited src/a.ts');
  assert.equal(wl.relativeToCwd('cat /Users/me/repo-other/x', '/Users/me/repo'), 'cat /Users/me/repo-other/x');
  assert.equal(wl.relativeToCwd('/etc/hosts', '/'), '/etc/hosts');
  assert.equal(wl.rowLabel(cmd('a', 0, 1, { input: 'cd /Users/me/repo && cat /Users/me/repo/README.md' }), '/Users/me/repo'), 'cat README.md');
});

test('durations and exit codes read short', () => {
  assert.equal(wl.formatDuration(820), '820ms');
  assert.equal(wl.formatDuration(14_000), '14s');
  assert.equal(wl.formatDuration(134_000), '2m 14s');
  assert.equal(wl.formatDuration(3_780_000), '1h 3m');
  assert.equal(wl.itemDuration(cmd('a', 0, 3)), '3s');
  assert.equal(wl.itemDuration(cmd('a', 0, null)), null);
  assert.equal(wl.exitText('Error: Exit code 2'), 'exit 2');
  assert.equal(wl.exitText('Exit code 127'), 'exit 127');
  assert.equal(wl.exitText('permission denied'), 'permission denied');
});

test('a finished stretch of tool calls folds into "Worked for"', () => {
  const entries = [{ item: cmd('a', 0, 60) }, { item: cmd('b', 60, 134) }];
  assert.equal(wl.shouldFold(entries, [turn('completed')], none), true);
  assert.equal(wl.workedFor(entries), 'Worked for 2m 14s');
  // One step has nothing to fold; a running turn or call stays open.
  assert.equal(wl.shouldFold(entries.slice(0, 1), [turn('completed')], none), false);
  assert.equal(wl.shouldFold(entries, [turn('running')], none), false);
  assert.equal(wl.shouldFold([{ item: cmd('a', 0, 5) }, { item: cmd('b', 5, null) }], [turn('completed')], none), false);
});

test('never folds an error, a gated call or news', () => {
  const ok = { item: cmd('a', 0, 5) };
  const done = [turn('completed')];
  assert.equal(wl.shouldFold([ok, { item: cmd('b', 5, 6, { status: 'failed', error: 'Exit code 1' }) }], done, none), false);
  const approval = { id: 'ap', type: 'approval_request', request_id: 'r1', request_kind: 'command', options: [] } as unknown as Extract<TurnItem, { type: 'approval_request' }>;
  assert.equal(wl.shouldFold([ok, { item: cmd('b', 5, 6), approval }], done, none), false);
  const notice = { ...cmd('n', 6, 6), type: 'system_notice', message: 'Background command finished' } as unknown as TurnItem;
  assert.equal(wl.shouldFold([ok, { item: notice }], done, none), false);
});

test('a running group names its live call, then +N more', () => {
  const entries = [{ item: cmd('a', 0, 5) }, { item: cmd('b', 1, null) }, { item: cmd('c', 2, 4) }];
  assert.equal(wl.liveSummary(entries, '/repo'), 'npm test b +2 more');
  assert.equal(wl.liveSummary([{ item: cmd('a', 0, 5) }], '/repo'), null);
});

test('banners: urgent ones always show; the rest fill two, then fold', () => {
  const b = (key: string, urgent = false) => ({ key, label: key, urgent });
  const names = (r: { shown: { key: string }[]; folded: { key: string }[] }) => [r.shown.map((x) => x.key), r.folded.map((x) => x.key)];
  assert.deepEqual(names(splitBanners([b('plan'), b('queue'), b('fresh')])), [['plan', 'queue'], ['fresh']]);
  assert.deepEqual(names(splitBanners([b('error', true), b('questions', true), b('ui', true), b('plan')])), [['error', 'questions', 'ui'], ['plan']]);
  assert.deepEqual(names(splitBanners([b('error', true), b('plan'), b('queue')])), [['error', 'plan'], ['queue']]);
  assert.deepEqual(names(splitBanners([])), [[], []]);
});

test('thread_open / thread_message calls read as an agent row', () => {
  const tool = (name: string, input: unknown, output?: string): TurnItem =>
    ({ id: 'x', turn_id: 't1', ordinal: 1, status: 'completed', title: null, started_at: T0, completed_at: at(2), updated_at: at(2), type: 'dynamic_tool', tool_name: name, input, output }) as TurnItem;
  const opened = threadCall(tool('mcp__otto__otto_thread_open', { provider: 'codex', model: 'gpt-5', effort: 'high', prompt: '\nReview the PR\nthen report' }, '{"id":"01JZ0000000000000000000000","access_mode":"x"}'));
  assert.deepEqual(JSON.parse(JSON.stringify(opened)), { verb: 'Opened', targetId: '01JZ0000000000000000000000', provider: 'codex', title: null, model: 'gpt-5', effort: 'high', text: 'Review the PR' });
  const sent = threadCall(tool('otto.thread_message', { to_session_id: 'S1', text: 'Done yet?' }));
  assert.equal(sent?.verb, 'Messaged');
  assert.equal(sent?.targetId, 'S1');
  assert.equal(sent?.text, 'Done yet?');
  assert.equal(threadCall(tool('mcp__otto__otto_thread_read', {})), null);
  assert.equal(elapsedBetween(T0, Date.parse(at(125))), '2m 5s');
  assert.equal(elapsedBetween(null, 0), null);
});
