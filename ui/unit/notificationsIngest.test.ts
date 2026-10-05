import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// The real store, with the transport and workspace stubbed out.
function store() {
  const { notifications } = loadSource(new URL('../src/lib/stores/notifications.svelte.ts', import.meta.url), {
    svelte: { untrack: (fn: () => unknown) => fn() },
    '../api/client': { api: { get: async () => [], post: async () => ({}), del: async () => ({}) } },
    '../toast.svelte': { toasts: { warn() {}, info() {} } }, '../toastError': { toastError() {} },
    '../external': { openExternal: async () => {} },
    './workspace.svelte': { ws: { sessions: [], getSession: () => null } },
    '../desktop': { isEmbedded: false },
    '../router.svelte': { router: { go() {} } },
    '../noticeRoute': { parseNoticeRoute: () => null }, '../confirm.svelte': { confirmer: { ask: async () => true } },
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

// A2: "awaiting input" (info) earns a native banner only when not watched.
function storeWith(active: string | null) {
  const { notifications } = loadSource(new URL('../src/lib/stores/notifications.svelte.ts', import.meta.url), {
    svelte: { untrack: (fn: () => unknown) => fn() },
    '../api/client': { api: { get: async () => [], post: async () => ({}), del: async () => ({}) } },
    '../toast.svelte': { toasts: { warn() {}, info() {} } }, '../toastError': { toastError() {} },
    '../external': { openExternal: async () => {} },
    './workspace.svelte': { ws: { sessions: [], getSession: () => null, activeSessionId: active } },
    '../desktop': { isEmbedded: false },
    '../router.svelte': { router: { go() {} } },
    '../noticeRoute': { parseNoticeRoute: () => null }, '../confirm.svelte': { confirmer: { ask: async () => true } },
  });
  return notifications;
}

test('awaiting-input banners only for a session you are not watching (A2)', () => {
  const waiting = notice({ action: { type: 'open_session', session_id: 's1' } });
  assert.equal(storeWith('s2').wantsNative(waiting), true, 'another session is active');
  assert.equal(storeWith('s1').wantsNative(waiting), false, 'watching it right now');
  const n = storeWith('s2');
  n.settings = { ...n.settings, native_on_waiting: false };
  assert.equal(n.wantsNative(waiting), false, 'setting off');
  n.settings = { ...n.settings, native_on_waiting: true, native_enabled: false };
  assert.equal(n.wantsNative(waiting), false, 'native notifications off entirely');
  const other = storeWith('s2');
  assert.equal(other.wantsNative(notice({ source_key: 'session:s1:finished', action: { type: 'open_session', session_id: 's1' } })), false, 'other info notices stay quiet');
  assert.equal(other.wantsNative(notice({ severity: 'warn', action: null })), true, 'warn/error always banner');
});

test('canceling a workspace switch prevents a notification from navigating in the old workspace', async () => {
  const routes: string[]=[];
  const {notifications}=loadSource(new URL('../src/lib/stores/notifications.svelte.ts',import.meta.url),{
    svelte:{untrack:(fn:()=>unknown)=>fn()},
    '../api/client':{api:{}},'../toast.svelte':{toasts:{warn(){},error(){}}},'../toastError':{toastError(){}},
    '../external':{openExternal:async()=>{}},'../desktop':{isEmbedded:false},
    './workspace.svelte':{ws:{sessions:[],currentId:'A',workspaces:[{id:'A'},{id:'B'}],select:async()=>false}},
    '../router.svelte':{router:{go:(route:string)=>routes.push(route)}},
    '../noticeRoute':{parseNoticeRoute:(route:string)=>({kind:'route',route})},'../confirm.svelte':{confirmer:{}},
  });
  await notifications.openRoute('personal-agents/agent-b','B');
  assert.deepEqual(routes,[]);
});

// Needs-you notice for a session of ANOTHER workspace / an archived one.
function opener(row: Record<string, unknown> | null, confirm = true) {
  const calls: string[] = [];
  const { notifications } = loadSource(new URL('../src/lib/stores/notifications.svelte.ts', import.meta.url), {
    svelte: { untrack: (fn: () => unknown) => fn() },
    '../api/client': { api: { post: async () => ({}) } },
    '../toast.svelte': { toasts: { warn: (t: string) => calls.push(`warn ${t}`) } }, '../toastError': { toastError() {} },
    '../external': { openExternal: async () => {} }, '../desktop': { isEmbedded: false },
    './workspace.svelte': { ws: {
      sessions: [], currentId: 'A', workspaces: [{ id: 'A' }, { id: 'B' }],
      getSession: () => null,
      ensureSession: async () => row,
      unarchiveSession: async (id: string) => void calls.push(`unarchive ${id}`),
      openInWorkspace: async (w: string, id: string) => void calls.push(`open ${w}/${id}`),
      navigateToSession: (id: string) => void calls.push(`nav ${id}`),
    } },
    '../router.svelte': { router: { go() {} } },
    '../noticeRoute': { parseNoticeRoute: () => null },
    '../confirm.svelte': { confirmer: { ask: async () => (calls.push('ask'), confirm) } },
  });
  return { notifications, calls };
}
const open = (n: any) => n.runAction(notice({ action: { type: 'open_session', session_id: 's1' } }));

test('a notice for another workspace\'s session switches there and opens it', async () => {
  const h = opener({ id: 's1', workspace_id: 'B', archived: false, title: 'build' });
  h.notifications.markRead = async () => {};
  await open(h.notifications);
  assert.deepEqual(h.calls, ['open B/s1']);
});

test('a notice for an archived session offers Unarchive and open', async () => {
  const yes = opener({ id: 's1', workspace_id: 'B', archived: true, title: 'build' });
  yes.notifications.markRead = async () => {};
  await open(yes.notifications);
  assert.deepEqual(yes.calls, ['ask', 'unarchive s1', 'open B/s1']);
  const no = opener({ id: 's1', workspace_id: 'A', archived: true, title: 'build' }, false);
  no.notifications.markRead = async () => {};
  await open(no.notifications);
  assert.deepEqual(no.calls, ['ask'], 'declined: nothing changes');
});

test('a deleted session says so instead of failing silently', async () => {
  const h = opener(null);
  h.notifications.markRead = async () => {};
  await open(h.notifications);
  assert.deepEqual(h.calls, ['warn Session unavailable']);
});

test('load() keeps notices ingested while the GET was in flight and queues one trailing reload', async () => {
  let release!: (v: unknown[]) => void;
  let gets = 0;
  const { notifications: n } = loadSource(new URL('../src/lib/stores/notifications.svelte.ts', import.meta.url), {
    svelte: { untrack: (fn: () => unknown) => fn() },
    '../api/client': { api: { get: (p: string) => {
      if (p.endsWith('/settings')) return Promise.resolve({});
      gets++;
      return gets === 1 ? new Promise((r) => (release = r)) : Promise.resolve([notice({ id: 'live', created_at: '2026-09-24T12:00:00Z' }), notice()]);
    } } },
    '../toast.svelte': { toasts: {} }, '../toastError': { toastError() {} },
    '../external': { openExternal: async () => {} }, '../desktop': { isEmbedded: false },
    './workspace.svelte': { ws: { sessions: [], getSession: () => null, statusMap: {}, markNeedsYou() {}, needsYou: {} } },
    '../router.svelte': { router: { go() {} } }, '../noticeRoute': { parseNoticeRoute: () => null },
    '../confirm.svelte': { confirmer: {} },
  });
  n.wantsNative = () => false;
  const first = n.load();
  n.ingest(notice({ id: 'live', created_at: '2026-09-24T12:00:00Z' }));
  void n.load(); // overlapping request: queued, not dropped
  release([notice()]); // stale snapshot without the live notice
  await first;
  assert.equal(gets, 2, 'exactly one trailing reload');
  assert.deepEqual(n.notices.map((x: { id: string }) => x.id), ['live', 'n1']);
});
