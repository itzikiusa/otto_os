import { test } from 'node:test';
import assert from 'node:assert/strict';
import { byRecent, groupAgentSessions, matchesQuery, sessionListStatus, type SessionListStatus } from '../src/modules/agents/agentsInbox.ts';
import type { Session } from '../src/lib/api/types';
import { SCRATCH_ID } from '../src/lib/stores/sessionBuckets.ts';
import { componentFunctions } from './componentFunctions.ts';

const s = (id: string, at = '2026-11-24T00:00:00Z', extra: Partial<Session> = {}): Session =>
  ({ id, title: id, provider: 'claude', cwd: '/w/' + id, last_active_at: at, ...extra }) as unknown as Session;

const inWorkspace = (workspace_id: string) => ({ workspace_id });
const old = '2026-11-01T00:00:00Z';
const recent = '2026-11-02T00:00:00Z';
const latest = '2026-11-03T00:00:00Z';

test('single workspace: needs you, working, idle, then done; newest first within each status', () => {
  const status: Record<string, SessionListStatus> = { a: 'idle', b: 'working', c: 'done', d: 'needs', e: 'idle', f: 'done', g: 'working' };
  const list = [s('a', old), s('b', old), s('c', old), s('d', old), s('e', latest), s('f', latest), s('g', recent)];
  const [group] = groupAgentSessions(list, (x) => status[x.id], 'single', 'w1');
  assert.equal(group.kind, 'flat');
  assert.deepEqual(group.sessions.map((x) => x.id), ['d', 'g', 'b', 'e', 'a', 'f', 'c']);
});

test('six working sessions are the flat list itself', () => {
  const list = ['a', 'b', 'c', 'd', 'e', 'f'].map((id) => s(id));
  const groups = groupAgentSessions(list, () => 'working', 'single', 'w1');
  assert.equal(groups.length, 1);
  assert.equal(groups[0].kind, 'flat');
  assert.deepEqual(groups[0].sessions, list);
});

test('an explicit manual or token sort keeps its input order within a status in single-workspace scope', () => {
  const [group] = groupAgentSessions([s('z', old), s('y', latest), s('x', recent)], () => 'idle', 'single', 'w1', true);
  assert.deepEqual(group.sessions.map((x) => x.id), ['z', 'y', 'x']);
});

test('workspaces sort by newest session with the current workspace first', () => {
  const list = [
    s('a-idle', old, inWorkspace('a')),
    s('b-needs', old, inWorkspace('b')),
    s('a-done', latest, inWorkspace('a')),
    s('c-idle', recent, inWorkspace('c')),
    s('a-needs', recent, inWorkspace('a')),
    s('scratch', old, inWorkspace(SCRATCH_ID)),
  ];
  const status: Record<string, SessionListStatus> = { 'a-done': 'done', 'a-needs': 'needs', 'b-needs': 'needs' };
  const groups = groupAgentSessions(list, (x) => status[x.id] ?? 'idle', 'workspace', 'b');
  assert.deepEqual(groups.map(({ kind, key, sessions }) => [kind, key, sessions.map((x) => x.id)]), [
    ['workspace', 'b', ['b-needs']],
    ['workspace', 'a', ['a-needs', 'a-idle', 'a-done']],
    ['workspace', 'c', ['c-idle']],
    ['workspace', SCRATCH_ID, ['scratch']],
  ]);
});

test('status groups span workspaces in priority order and omit empty groups', () => {
  const status: Record<string, SessionListStatus> = { a: 'done', b: 'working', c: 'working', d: 'needs' };
  const list = [s('a', latest), s('b', old, inWorkspace('a')), s('c', recent, inWorkspace('b')), s('d', old)];
  const groups = groupAgentSessions(list, (x) => status[x.id], 'status', 'b');
  assert.deepEqual(groups.map(({ kind, key, sessions }) => [kind, key, sessions.map((x) => x.id)]), [
    ['status', 'needs', ['d']],
    ['status', 'working', ['c', 'b']],
    ['status', 'done', ['a']],
  ]);
});

test('no grouping puts the newest session first across workspaces regardless of status', () => {
  const list = [s('a', old, inWorkspace('a')), s('b', latest, inWorkspace('b')), s('c', recent, inWorkspace('a'))];
  const [group] = groupAgentSessions(list, () => { throw new Error('No status lookup needed'); }, 'none', 'a');
  assert.equal(group.kind, 'flat');
  assert.deepEqual(group.sessions.map((x) => x.id), ['b', 'c', 'a']);
});

test('status groups follow the row status, with ended, stopped and failed sessions under Done', () => {
  assert.equal(sessionListStatus('running', 'needs'), 'needs', 'a pending input outranks the provider status');
  assert.equal(sessionListStatus('working', 'working'), 'working');
  assert.equal(sessionListStatus('running', 'rest'), 'idle', 'a live but calm provider is idle');
  assert.equal(sessionListStatus('suspended', 'rest'), 'idle');
  assert.equal(sessionListStatus('idle', 'done'), 'done', 'unread completion');
  assert.equal(sessionListStatus('ended', 'rest'), 'done');
  assert.equal(sessionListStatus('stopped', 'rest'), 'done');
  assert.equal(sessionListStatus('failed', 'failed'), 'done');
});

test('byRecent puts the newest activity first', () => {
  const out = byRecent([
    s('same-b', '2026-11-24T00:00:00Z'),
    s('new', '2026-11-24T00:00:00.123Z'),
    s('same-a', '2026-11-24T02:00:00+02:00'),
    s('old', '2026-11-01T00:00:00Z'),
  ]);
  assert.deepEqual(out.map((x) => x.id), ['new', 'same-a', 'same-b', 'old']);
});

test('byRecent keeps sub-millisecond daemon timestamps in order', () => {
  const out = byRecent([
    s('a-first', '2026-11-24T00:00:00.123000012Z'),
    s('z-last', '2026-11-24T00:00:00.123987Z'),
  ]);
  assert.deepEqual(out.map((x) => x.id), ['z-last', 'a-first']);
});

test('search matches title, provider, folder and extra fields', () => {
  const row = s('fix-bug', undefined, { provider: 'codex', cwd: '/repos/otto' } as Partial<Session>);
  assert.ok(matchesQuery(row, 'FIX'));
  assert.ok(matchesQuery(row, 'codex'));
  assert.ok(matchesQuery(row, 'otto'));
  assert.ok(matchesQuery(row, 'main', ['main']));
  assert.ok(!matchesQuery(row, 'nope', ['main']));
  assert.ok(matchesQuery(row, '  '));
});

for (const [name, query, needsYouFilter] of [
  ['search', '  Beta  ', false],
  ['the attention filter', '', true],
] as const) {
  test(`${name} reveals matching sessions in a saved collapsed workspace`, () => {
    const ws = { needsYouFilter };
    const collapsed = new Set(['beta']);
    const state = componentFunctions(new URL('../src/modules/agents/AgentsInbox.svelte', import.meta.url),
      ['isFiltering', 'isCollapsed'], { query, ws, collapsed });
    const group = { kind: 'workspace', key: 'beta' };
    assert.equal(state.isCollapsed(group), false);

    state.query = '  ';
    ws.needsYouFilter = false;
    assert.equal(state.isCollapsed(group), true, 'clearing filters restores the saved fold');
  });
}
