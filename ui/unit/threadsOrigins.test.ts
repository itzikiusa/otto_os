// The Threads sidebar keeps a personal agent's routine runs and other
// modules' helper threads out of the projects and History (threadOrigins.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const { threadOrigin, originGroups, EMBEDDED_THREAD_SOURCES } = loadSource(
  new URL('../src/modules/threads/threadOrigins.ts', import.meta.url),
  {},
  {},
  { relative: true },
);

let n = 0;
const row = (meta: Record<string, unknown>, last = `2026-11-12T10:${String(n % 60).padStart(2, '0')}:00Z`) => ({
  id: `t${++n}`,
  title: 'x',
  last_active_at: last,
  archived: false,
  meta,
});

test('an agent’s chat and plain threads are the person’s own; its runs are runs', () => {
  assert.equal(threadOrigin(row({})), 'main');
  assert.equal(threadOrigin(row({ source: 'personal_agent', personal_agent: 'a1', personal_agent_chat: true })), 'main');
  assert.equal(threadOrigin(row({ source: 'personal_agent', personal_agent: 'a1', run_id: 'r1', trigger: 'schedule' })), 'run');
  assert.equal(threadOrigin(row({ source: 'personal_agent', personal_agent: 'a1', delegated_by_agent: 'lead' })), 'run');
});

test('embedded and background sources are helpers (one notion with the session buckets)', () => {
  for (const src of EMBEDDED_THREAD_SOURCES) assert.equal(threadOrigin(row({ source: src })), 'helper');
  assert.equal(threadOrigin(row({ source: 'db_assist' })), 'helper');
  assert.equal(threadOrigin(row({ source: 'review' })), 'helper');
  assert.equal(threadOrigin(row({ source: 'channel' })), 'helper');
});

test('groups: Agent runs first, newest first; helpers by label; main threads left out', () => {
  const old = row({ source: 'personal_agent', run_id: 'r1' }, '2026-11-12T08:00:00Z');
  const fresh = row({ source: 'personal_agent', run_id: 'r2' }, '2026-11-12T09:00:00Z');
  const pr = row({ source: 'pr-agent' });
  const conn = row({ source: 'connection-diagnose' });
  const mine = row({});
  const groups = originGroups([old, mine, pr, fresh, conn, fresh]);
  // Array.from: the loaded module's arrays come from another realm.
  assert.deepEqual(Array.from(groups, (g: { label: string }) => g.label), ['Agent runs', 'Pull requests', 'Connections']);
  assert.deepEqual(Array.from(groups[0].items, (s: { id: string }) => s.id), [fresh.id, old.id]);
  assert.equal(groups[0].key, 'agent-runs');
  assert.equal(groups[1].key, 'helpers-pull-requests');
  assert.ok(groups.every((g: { items: { id: string }[] }) => !g.items.some((s) => s.id === mine.id)));
  assert.equal(originGroups([mine]).length, 0);
});
