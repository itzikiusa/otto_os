// The Threads work log's quiet presentation (modules/threads/runtime/
// workLog.ts, banners.ts, agentRows.ts): a finished stretch of tool calls
// folds into "Worked for …" unless it holds something a person must see; a
// running group names its live call; labels lose `cd <dir> &&` and the
// thread's folder; a failed command reads "exit N"; notices above the
// composer stack by priority with one quiet one shown.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { loadSource } from './sourceHarness.ts';
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
  assert.deepEqual(names(splitBanners([b('plan'), b('queue'), b('fresh')])), [['plan'], ['queue', 'fresh']]);
  assert.deepEqual(names(splitBanners([b('error', true), b('questions', true), b('ui', true), b('plan')])), [['error', 'questions', 'ui'], ['plan']]);
  assert.deepEqual(names(splitBanners([b('error', true), b('plan'), b('queue')])), [['error'], ['plan', 'queue']]);
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

test('a folded stretch says how long and what was done', () => {
  const edit = (id: string, s: number, e: number): TurnItem => ({ ...cmd(id, s, e), type: 'file_change', changes: [] }) as unknown as TurnItem;
  const think = (id: string): TurnItem => ({ ...cmd(id, 0, 1), type: 'reasoning', text: 'hm' }) as unknown as TurnItem;
  const e = (item: TurnItem) => ({ item });
  assert.equal(wl.foldLine([e(cmd('a', 0, 60)), e(cmd('b', 60, 134)), e(edit('c', 70, 80)), e(think('r'))]), 'Worked for 2m 14s · ran 2 commands and edited 1 file');
  assert.equal(wl.foldLine([e(think('r')), e(think('s'))]), wl.workedFor([e(think('r')), e(think('s'))]), 'thinking alone: just the time');
});

test('tool-call visibility: hide successful keeps failures, gated and running calls; hide all keeps failures and gates; never messages', () => {
  const ok = cmd('ok', 0, 1);
  const bad = cmd('bad', 1, 2, { status: 'failed', error: 'Exit code 1' });
  const live = cmd('live', 2, null);
  const gated = cmd('gated', 3, 4);
  const approval = { id: 'ap', type: 'approval_request', request_id: 'r1', status: 'completed' } as unknown as never;
  const msg = { kind: 'message', item: { id: 'm', type: 'assistant_message', text: 'done' } } as never;
  const rows = [
    msg,
    { kind: 'work', id: 'w1', parts: [{ kind: 'item', entry: { item: ok } }, { kind: 'item', entry: { item: bad } }, { kind: 'item', entry: { item: live } }, { kind: 'item', entry: { item: gated, approval } }] },
    { kind: 'work', id: 'w2', parts: [{ kind: 'group', id: 'g', entries: [{ item: cmd('x', 0, 1) }, { item: cmd('y', 0, 1) }] }] },
  ] as never[];
  const ids = (r: ReturnType<typeof wl.visibleTools>) => r.map((row) => (row.kind === 'work' ? `${row.id}:${row.parts.map((p) => (p.kind === 'item' ? p.entry.item.id : p.entries.map((x) => x.item.id).join('+'))).join(',')}` : row.kind));
  assert.equal(wl.visibleTools(rows, 'all'), rows, 'show all is a no-op');
  assert.deepEqual(ids(wl.visibleTools(rows, 'hide-ok')), ['message', 'w1:bad,live,gated']);
  assert.deepEqual(ids(wl.visibleTools(rows, 'none')), ['message', 'w1:bad,gated']);
});

test('a running stretch folds its finished steps before the live one; a failure stops the fold', () => {
  const part = (item: TurnItem) => ({ kind: 'item' as const, entry: { item } });
  const done = [cmd('a', 0, 1), cmd('b', 1, 2), cmd('c', 2, 3)].map(part);
  assert.equal(wl.earlierSteps([...done, part(cmd('live', 3, null))], none), 3);
  assert.equal(wl.earlierSteps(done, none), 0, 'nothing runs: the finished fold handles it');
  assert.equal(wl.earlierSteps([done[0], part(cmd('live', 3, null))], none), 0, 'one earlier step stays as is');
  const bad = part(cmd('bad', 1, 2, { status: 'failed', error: 'Exit code 1' }));
  assert.equal(wl.earlierSteps([done[0], done[1], bad, done[2], part(cmd('live', 3, null))], none), 2);
  assert.equal(wl.earlierSteps([bad, done[0], done[1], part(cmd('live', 3, null))], none), 0);
});

test('a short screen (or the keyboard up) folds every quiet notice; urgent ones still show', () => {
  const b = (key: string, urgent = false) => ({ key, label: key, ...(urgent ? { urgent } : {}) });
  const names = (s: ReturnType<typeof splitBanners>) => [s.shown.map((x) => x.key), s.folded.map((x) => x.key)];
  assert.deepEqual(names(splitBanners([b('plan'), b('queue')], 0)), [[], ['plan', 'queue']]);
  assert.deepEqual(names(splitBanners([b('error', true), b('queue')], 0)), [['error'], ['queue']]);
});

test('the thread’s next action (Revive / Reopen) is never folded behind "N more"; quiet notices are', () => {
  const base = { sessionError: null, uiAsk: false, hasQuestions: false, hasPlan: true, queueShown: true, needsRevive: false, freshContext: true };
  const keys = (s: ReturnType<typeof splitBanners>) => [s.shown.map((x) => x.key), s.folded.map((x) => x.key)];
  // A stopped thread whose revive just failed: the error AND the Revive line show.
  const { buildRuntimeBanners } = loadSource(new URL('../src/modules/threads/runtime/runtimeComposer.ts', import.meta.url), {
    '../../../lib/toastError': { toastError: () => {} },
    '../../../lib/telemetry': { startMeasurement: () => null },
    './hostCommands': { runHostSlash: () => false },
    './timeline': { errorText: (s: string) => s },
    '../../../lib/stores/runtimeDrafts.svelte': { runtimeDrafts: {} },
    '../../../lib/providerRegistry': { sessionProviderSlug: () => '' },
  }) as typeof import('../src/modules/threads/runtime/runtimeComposer.ts');
  const revive = buildRuntimeBanners({ ...base, sessionError: 'Agent executable unavailable.', needsRevive: true });
  assert.deepEqual(keys(splitBanners(revive)), [['error', 'queue'], ['plan', 'fresh']]);
  assert.deepEqual(keys(splitBanners(revive, 0)), [['error', 'queue'], ['plan', 'fresh']], 'not even with the keyboard up');
  // An armed provider switch ("Next message continues on …") never folds either; a failed one is the error line.
  const sw = buildRuntimeBanners({ ...base, switchTo: 'codex' });
  assert.deepEqual(keys(splitBanners(sw, 0)), [['switch'], ['plan', 'queue']]);
  assert.deepEqual(keys(splitBanners(buildRuntimeBanners({ ...base, switchTo: 'codex', sessionError: 'Codex is not signed in.' }), 0)), [['error', 'switch'], ['plan', 'queue']]);
  // A live queue is a quiet notice.
  assert.deepEqual(keys(splitBanners(buildRuntimeBanners(base))), [['plan'], ['queue', 'fresh']]);
});
