import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

// architecture-1-10: the DatabaseStore façade split its connections, schema
// tree, object operations, query runs and workbench persistence into
// lib/stores/database/*.svelte.ts. The parts share one open-generation token
// (`ConnEpochs`) instead of a private map on the façade. These drive the REAL
// parts against a fake host (the façade's reactive working set) and a scripted
// transport.

type Node = { id: string; label: string; kind: string; has_children?: boolean };

function setup() {
  const gets: Array<{ url: string; d: ReturnType<typeof deferred<unknown>> }> = [];
  const posts: Array<{ url: string; body: unknown; d: ReturnType<typeof deferred<unknown>> }> = [];
  const warned: string[] = [];
  const client = {
    api: {
      get: (url: string) => { const d = deferred<unknown>(); gets.push({ url, d }); return d.promise; },
      post: (url: string, body: unknown) => { const d = deferred<unknown>(); posts.push({ url, body, d }); return d.promise; },
    },
    isAbortError: () => false,
    ApiError: class extends Error {},
    dbCloseConnection: async () => {},
  };
  const imports = {
    'lib/api/client': client,
    'lib/toast.svelte': { toasts: { error() {}, success() {}, info() {}, warn: (t: string) => warned.push(t) } },
    'lib/toastError': { toastError() {} },
    'lib/stores/workspace.svelte': { ws: { currentId: 'w1' } },
    'lib/stores/dbPrefs.svelte': { dbPrefs: {}, saveFlag() {} },
    'lib/poll': { mapLimit: async () => {}, pollWhileVisible: () => ({ stop() {} }) },
    'lib/clipboard': { copyTextOrThrow: async () => {} },
    'lib/stores/database/model': { KEEP_ALIVE_MS: 1, WARM_CONCURRENCY: 1, blankTab: () => ({}), isDbKind: () => true },
  };
  const dir = new URL('../src/lib/stores/database/', import.meta.url);
  const { ConnEpochs } = loadSource(new URL('connections.svelte.ts', dir), imports, {}, { relative: true });
  const { DbSchemaTreeStore } = loadSource(new URL('schemaTree.svelte.ts', dir), imports, {}, { relative: true });
  const { DbObjectOpsStore } = loadSource(new URL('objectOps.svelte.ts', dir), imports, {}, { relative: true });

  const tabs: Array<{ statement: string; name?: string }> = [];
  const statuses: Array<[string, unknown]> = [];
  // The façade's working set the parts read and write (their `…Host` contract).
  const host: Record<string, any> = {
    selectedConnId: 'c1',
    openConnIds: ['c1'],
    selectedConn: { id: 'c1', kind: 'mysql' },
    schemaRoot: [] as Node[],
    childrenCache: new Map<string, Node[]>(),
    builderTablesCache: new Map(),
    expanded: new Set<string>(),
    loadingNodes: new Set<string>(),
    nodeFilters: new Map<string, string>(),
    schemaLoading: false,
    showCounts: false,
    activeDb: null,
    connBase: (id: string) => `/connections/${id}/db`,
    refreshCompletions: async () => {},
    conn: {
      mergeConnStatus: (id: string, s: unknown) => statuses.push([id, s]),
      setConnStatus: (id: string, s: unknown) => statuses.push([id, s]),
    },
    newTab: (statement: string) => { tabs.push({ statement }); },
    get tab() { return tabs[tabs.length - 1]; },
  };
  host.epochs = new ConnEpochs(() => host.openConnIds);
  host.tree = new DbSchemaTreeStore(host);
  host.objects = new DbObjectOpsStore(host);
  /** Close then re-open `id` — what `closeConnection` + `openConnection` do to the epoch. */
  const reconnect = (id: string) => {
    host.epochs.bump(id);
    host.openConnIds = host.openConnIds.filter((x: string) => x !== id);
    host.openConnIds = [...host.openConnIds, id];
  };
  return { host, gets, posts, tabs, statuses, warned, reconnect };
}

const db: Node = { id: 'db:shop', label: 'shop', kind: 'database', has_children: true };

test('a children response from before a reconnect is dropped; the new generation lands', async () => {
  const h = setup();
  const first = h.host.tree.expand(db);
  assert.equal(h.posts.length, 1);
  assert.ok(h.host.loadingNodes.has(db.id), 'the node shows as loading');

  h.reconnect('c1');
  h.posts[0].d.resolve([{ id: 'db:shop/table:stale', label: 'stale', kind: 'table' }]);
  await first;
  assert.equal(h.host.childrenCache.has(db.id), false, 'the stale generation wrote nothing');

  // The re-opened connection loads its own children, and those land.
  h.host.expanded = new Set();
  const second = h.host.tree.expand(db);
  h.posts[1].d.resolve([{ id: 'db:shop/table:orders', label: 'orders', kind: 'table' }]);
  await second;
  assert.deepEqual(h.host.childrenCache.get(db.id).map((n: Node) => n.label), ['orders']);
});

test('a schema root from before a reconnect neither replaces the tree nor touches status', async () => {
  const h = setup();
  h.host.schemaRoot = [db];
  const load = h.host.tree.refreshSchema();
  h.reconnect('c1');
  h.gets[0].d.resolve([{ id: 'db:other', label: 'other', kind: 'database' }]);
  await load;
  assert.deepEqual(h.host.schemaRoot.map((n: Node) => n.id), ['db:shop']);
  assert.deepEqual(h.statuses.map(([, s]) => (s as { phase: string }).phase), ['connecting'], 'only the start-of-load status, no "ready"');
  assert.equal(h.host.epochs.live('c1', 0), false);
  assert.equal(h.host.epochs.live('c1', 1), true);
});

test('a refresh after a drop removes the dropped table from the tree', async () => {
  const h = setup();
  const orders: Node = { id: 'db:shop/table:orders', label: 'orders', kind: 'table' };
  const users: Node = { id: 'db:shop/table:users', label: 'users', kind: 'table' };
  const opened = h.host.tree.expand(db);
  h.posts[0].d.resolve([orders, users]);
  await opened;
  assert.deepEqual(h.host.childrenCache.get(db.id).map((n: Node) => n.label), ['orders', 'users']);

  // Drop queues a reviewed statement in a new tab (it never runs on its own).
  await h.host.objects.dropObject(users);
  assert.match(h.tabs[0].statement, /^DROP TABLE .*users.*;$/);
  assert.equal(h.tabs[0].name, 'DROP users');
  assert.equal(h.warned[0], 'Review before running');

  // After the DROP ran, Refresh re-reads the tree from the server.
  const refreshed = h.host.tree.refreshSchema();
  assert.equal(h.host.childrenCache.size, 0, 'refresh drops the cached children');
  h.gets[0].d.resolve([db]);
  await refreshed;
  const reopened = h.host.tree.expand(db);
  h.posts[1].d.resolve([orders]);
  await reopened;
  assert.deepEqual(h.host.childrenCache.get(db.id).map((n: Node) => n.label), ['orders'], 'the dropped table is gone');
  assert.equal(h.statuses.at(-1)?.[1] && (h.statuses.at(-1)![1] as { phase: string }).phase, 'ready');
});
