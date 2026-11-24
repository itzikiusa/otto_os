import { test } from 'node:test';
import assert from 'node:assert/strict';
import { agentsInbox, byRecent, matchesQuery } from '../src/modules/agents/agentsInbox.ts';
import type { Session } from '../src/lib/api/types';
import type { InboxClass } from '../src/lib/statusWords.ts';

const s = (id: string, at = '2026-11-24T00:00:00Z', extra: Partial<Session> = {}): Session =>
  ({ id, title: id, provider: 'claude', cwd: '/w/' + id, last_active_at: at, ...extra }) as unknown as Session;

test('one list: needs you, failed, done, then working, then the rest; ties keep the incoming order', () => {
  const cls: Record<string, InboxClass> = { a: 'rest', b: 'working', c: 'failed', d: 'needs', e: 'rest', f: 'done', g: 'working' };
  const out = agentsInbox(['a', 'b', 'c', 'd', 'e', 'f', 'g'].map((id) => s(id)), (x) => cls[x.id]);
  assert.deepEqual(out.map((x) => x.id), ['d', 'c', 'f', 'b', 'g', 'a', 'e']);
});

test('six working sessions are the list itself (no fold)', () => {
  const out = agentsInbox(['a', 'b', 'c', 'd', 'e', 'f'].map((id) => s(id)), () => 'working');
  assert.deepEqual(out.map((x) => x.id), ['a', 'b', 'c', 'd', 'e', 'f']);
});

test('a manual order is kept among calm rows', () => {
  const out = agentsInbox([s('z'), s('y'), s('x')], () => 'rest');
  assert.deepEqual(out.map((x) => x.id), ['z', 'y', 'x']);
});

test('byRecent puts the newest activity first', () => {
  const out = byRecent([s('old', '2026-11-01T00:00:00Z'), s('new', '2026-11-24T00:00:00Z')]);
  assert.deepEqual(out.map((x) => x.id), ['new', 'old']);
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
