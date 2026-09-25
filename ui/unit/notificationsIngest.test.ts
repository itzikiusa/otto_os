import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// The real store, with the transport and workspace stubbed out.
function store() {
  const { notifications } = loadSource(new URL('../src/lib/stores/notifications.svelte.ts', import.meta.url), {
    svelte: { untrack: (fn: () => unknown) => fn() },
    '../api/client': { api: { get: async () => [], post: async () => ({}), del: async () => ({}) } },
    '../toast.svelte': { toasts: { warn() {}, info() {} } },
    '../external': { openExternal: async () => {} },
    './workspace.svelte': { ws: { sessions: [] } },
    '../desktop': { isEmbedded: false },
  });
  return notifications;
}

const notice = (over: Record<string, unknown> = {}) => ({
  id: 'n1',
  created_at: '2026-09-24T10:00:00Z',
  read: false,
  kind: 'session',
  severity: 'info',
  title: 'Session awaiting input',
  body: 'build · waiting',
  source_key: 'session:s1:waiting',
  action: null,
  ...over,
});

test('a re-fired notice (same id, refreshed by the daemon) replaces the read copy', () => {
  const n = store();
  n.notices = [notice({ read: true }), notice({ id: 'n0', created_at: '2026-09-24T11:00:00Z', source_key: null, kind: 'system' })];
  n.ingest(notice({ created_at: '2026-09-24T12:00:00Z', body: 'build · waiting again' }));
  assert.equal(n.notices.length, 2, 'no duplicate row');
  assert.equal(n.notices[0].id, 'n1', 'moved to the top');
  assert.equal(n.notices[0].read, false, 'unread again, like the server');
  assert.equal(n.notices[0].body, 'build · waiting again');
});

test('an identical re-delivery is ignored', () => {
  const n = store();
  const first = notice({ read: true });
  n.notices = [first];
  n.ingest(notice());
  assert.equal(n.notices.length, 1);
  assert.equal(n.notices[0], first, 'kept the local (read) copy');
});

test('a new notice is prepended', () => {
  const n = store();
  n.notices = [notice()];
  n.ingest(notice({ id: 'n2', created_at: '2026-09-24T12:00:00Z' }));
  assert.equal(n.notices.map((x: { id: string }) => x.id).join(','), 'n2,n1');
});
