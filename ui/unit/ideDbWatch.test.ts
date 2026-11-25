// DB Watch (IDE): the model's projections and choices, and the store's
// auto-arm / prod-confirm / agent-read rules against a scripted daemon.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  canCompareStart,
  cellChanges,
  changesAsText,
  countsShort,
  DB_WATCH_OFF,
  memoryKey,
  memoryOf,
  parseFilter,
  parseMemory,
  pickAutoItems,
  pickAutoTarget,
  recordFields,
  rowPreview,
  unverifiedNames,
} from '../src/modules/ide/dbwatch/ideDbWatchModel.ts';
import type {
  IdeDbWatch,
  IdeDbWatchItemDiff,
  IdeDetectedDbAccess,
  IdeEphemeralDbTarget,
  IdeSavedDbTarget,
} from '../src/lib/api/types/ide-db-watch.ts';
import { deferred, loadSource } from './sourceHarness.ts';

const det = (object_name: string, engine = 'mysql', verified: boolean | null = true): IdeDetectedDbAccess =>
  ({ engine, object_name, kind: 'table', operations: ['write'], source_file: '/r/dao.go', line: 3, verified }) as IdeDetectedDbAccess;
const tc: IdeEphemeralDbTarget = { target_kind: 'ephemeral', ephemeral_id: 'tc1', name: 'mariadb Testcontainer', engine: 'mysql', host: '127.0.0.1', port: 55123, database: 'pr_bo_1', source: 'testcontainers', container_image: 'mariadb:11.4' };
const otherContainer: IdeEphemeralDbTarget = { ...tc, ephemeral_id: 'c9', name: 'someone-elses-mysql', source: 'container' };
const saved = (over: Partial<IdeSavedDbTarget> = {}): IdeSavedDbTarget => ({ target_kind: 'saved', connection_id: 'stg', name: 'pr_bo staging', engine: 'mysql', host: '10.0.4.12', port: 3306, database: 'pr_bo_1', environment: 'staging', local: false, ...over });
const watchOf = (over: Partial<IdeDbWatch> = {}): IdeDbWatch => ({
  id: 'w1', root: '/r', target: { kind: 'ephemeral', ephemeral_id: 'tc1', engine: 'mysql', label: 'mariadb Testcontainer', local: true },
  database: 'pr_bo_1', items: [{ object: 'session_info', kind: 'table', primary_key: ['id'] }], checkpoint_count: 1, busy: false, created_at_ms: 1, ...over,
});
const itemDiff = (over: Partial<IdeDbWatchItemDiff> = {}): IdeDbWatchItemDiff => ({
  object: 'session_info', engine: 'mysql', executed_query: 'SELECT …', window_note: 'all 37 rows', window_full: false, columns: ['id', 'status'],
  added_count: 0, modified_count: 0, removed_count: 0, unchanged_count: 37, diffs: [], more_count: 0, ...over,
});

// ── Model ──────────────────────────────────────────────────────────────────

test('auto target: the run’s own container of the best-ranked engine; never a saved connection or another project’s container', () => {
  const redisTc = { ...tc, ephemeral_id: 'mr', engine: 'redis' as const, source: 'process' as const };
  assert.equal(pickAutoTarget([det('session:*', 'redis'), det('session_info')], [tc, redisTc])?.ephemeral_id, 'mr');
  assert.equal(pickAutoTarget([det('session_info')], [otherContainer]), null, 'a plain container may be another project’s');
  assert.equal(pickAutoTarget([det('session_info')], []), null, 'saved connections are not candidates at all');
});

test('auto tables: verified, one engine, deduplicated, 8 on a local target and 3 on a remote one', () => {
  const d = [det('GetAllConfigs', 'mysql', false), det('session_info'), det('session_info'), det('session:*', 'redis'), ...'abcdefghij'.split('').map((c) => det(`t_${c}`))];
  const local = pickAutoItems(d, 'mysql', true);
  assert.deepEqual(local.slice(0, 2), [{ object: 'session_info' }, { object: 't_a' }]);
  assert.equal(local.length, 8);
  assert.ok(!local.some((i) => i.object === 'GetAllConfigs'), 'a name the target does not have is never watched');
  assert.equal(pickAutoItems(d, 'mysql', false).length, 3);
});

test('memory: a saved pick or "the run’s container"; never prod; garbage reads as nothing', () => {
  assert.deepEqual(memoryOf(watchOf())?.target, { kind: 'auto', engine: 'mysql' });
  const s = watchOf({ target: { kind: 'saved', connection_id: 'stg', engine: 'mysql', label: 'stg', environment: 'staging', local: false } });
  assert.deepEqual(memoryOf(s)?.target, { kind: 'saved', connection_id: 'stg' });
  assert.equal(memoryOf(watchOf({ target: { ...s.target, environment: 'production' } })), null);
  assert.deepEqual(parseMemory(JSON.stringify(memoryOf(s))), memoryOf(s));
  assert.equal(parseMemory('{"target":{"kind":"saved"},"items":[]}'), null);
  assert.equal(parseMemory(DB_WATCH_OFF), null);
  assert.equal(parseMemory('not json'), null);
});

test('filters are column = value only, typed like the literal', () => {
  assert.deepEqual(parseFilter('account_id = 77017'), { column: 'account_id', value: 77017 });
  assert.deepEqual(parseFilter("status = 'open'"), { column: 'status', value: 'open' });
  assert.deepEqual(parseFilter('status=open'), { column: 'status', value: 'open' });
  assert.deepEqual(parseFilter('closed_at = NULL'), { column: 'closed_at', value: null });
  assert.deepEqual(parseFilter('code = "007"'), { column: 'code', value: '007' });
  assert.equal(parseFilter('1=1 OR 1'), null);
  assert.equal(parseFilter("name = 'a b'"), null, 'the daemon only takes letters, digits and _ . : @ -');
  assert.equal(parseFilter("status = 'x'' OR 1=1 --'"), null);
  assert.deepEqual(parseFilter('email = ava@x.io'), { column: 'email', value: 'ava@x.io' });
  assert.equal(parseFilter('status'), null);
});

test('a changed row lists only the changed cells; nested documents diff leaf by leaf; masked reads as hidden', () => {
  const c = cellChanges([
    { column: 'status', before: 'open', after: 'closed' },
    { column: 'context', before: { geo: { city: 'Valletta', country: 'MT' }, tags: ['a'] }, after: { geo: { city: 'Sliema', country: 'MT' }, tags: ['a', 'b'] } },
    { column: 'api_secret', before: '[redacted]', after: '[redacted]', masked: true },
  ]);
  assert.deepEqual(c, [
    { path: 'status', before: 'open', after: 'closed', hidden: false },
    { path: 'context.geo.city', before: 'Valletta', after: 'Sliema', hidden: false },
    { path: 'context.tags.1', before: '—', after: 'b', hidden: false },
    { path: 'api_secret', before: '', after: '', hidden: true },
  ]);
});

test('the expanded record puts the changed fields first, with their previous value', () => {
  const f = recordFields(['id', 'status', 'doc'], {
    key: '17', kind: 'modified', row: [17, 'closed', { a: { b: 2 } }], previous_row: [17, 'open', { a: { b: 1 } }],
    changes: [{ column: 'status', before: 'open', after: 'closed' }, { column: 'doc', before: { a: { b: 1 } }, after: { a: { b: 2 } } }],
  });
  assert.deepEqual(f.map((x) => [x.path, x.before ?? null, x.value]), [['status', 'open', 'closed'], ['doc.a.b', '1', '2'], ['id', null, '17']]);
});

test('row preview skips empty cells and clips long ones; BSON scalars read as ObjectId / ISODate', () => {
  assert.equal(rowPreview(['id', 'a', 'b', 'c'], [42, null, '', 'x'.repeat(60)]), `id=42  c=${'x'.repeat(39)}…`);
  assert.equal(rowPreview(['_id'], [{ $oid: 'abc' }]), '_id=ObjectId("abc")');
});

test('counts, compare toggle and copy-as-text', () => {
  const it = itemDiff({ added_count: 1, modified_count: 1, diffs: [
    { key: '42', kind: 'added', row: [42, 'open'], changes: [] },
    { key: '17', kind: 'modified', row: [17, 'closed'], previous_row: [17, 'open'], changes: [{ column: 'status', before: 'open', after: 'closed' }] },
  ] });
  assert.equal(countsShort(it), '+1 ~1');
  assert.equal(countsShort(itemDiff()), '');
  assert.equal(canCompareStart({ checkpoint_count: 2 }), false, 'with one stop both views are the same');
  assert.equal(canCompareStart({ checkpoint_count: 3 }), true);
  assert.equal(changesAsText([it, itemDiff({ object: 'session_route' })]), 'session_info: +1 ~1\n  + 42  id=42  status=open\n  ~ 17  status: open → closed\nsession_route: no changes');
});

// ── Store ──────────────────────────────────────────────────────────────────

class ApiError extends Error {
  status: number;
  code: string;
  constructor(status: number, problem: { code: string; message: string }) {
    super(problem.message);
    this.status = status;
    this.code = problem.code;
  }
}

interface Call { method: string; url: string; body?: unknown }

function harness(opts: { routes?: (c: Call) => unknown; prompt?: (msg: string) => string | null; ls?: Map<string, string> } = {}) {
  const calls: Call[] = [];
  const handlers: Record<string, (ev: Record<string, unknown>) => void> = {};
  const resets: Array<() => void> = [];
  const prompts: string[] = [];
  const ls = opts.ls ?? new Map<string, string>();
  const reply = async (c: Call) => {
    calls.push(c);
    const r = opts.routes?.(c);
    if (r instanceof Error) throw r;
    return r ?? {};
  };
  const mod = loadSource(
    new URL('../src/modules/ide/dbwatch/ideDbWatchState.svelte.ts', import.meta.url),
    {
      '../../../lib/api/client': {
        api: {
          get: (url: string) => reply({ method: 'GET', url }),
          post: (url: string, body?: unknown) => reply({ method: 'POST', url, body }),
          patch: (url: string, body?: unknown) => reply({ method: 'PATCH', url, body }),
          del: (url: string) => reply({ method: 'DELETE', url }),
        },
      },
      '../../../lib/confirm.svelte': {
        confirmer: { promptText: async (msg: string) => { prompts.push(msg); return opts.prompt?.(msg) ?? null; } },
      },
      '../../../lib/live': {
        appLive: { on: (types: string[], fn: (ev: Record<string, unknown>) => void) => { for (const t of types) handlers[t] = fn; return () => {}; }, onResync: () => () => {} },
      },
      '../../../lib/storage': { lsGet: (k: string) => ls.get(k) ?? null, lsSet: (k: string, v: string) => void ls.set(k, v) },
      '../store/ideStateSlices.svelte': { onIdeTargetReset: (fn: () => void) => resets.push(fn) },
    },
    {},
    { relative: true },
  );
  return { store: mod.ideDbWatch, calls, handlers, resets, prompts, ls };
}

const plain = (v: unknown) => JSON.parse(JSON.stringify(v));
const flush = () => new Promise((r) => setTimeout(r, 0));
const readOf = (w: IdeDbWatch, items: IdeDbWatchItemDiff[] = [itemDiff()], from_seq: number | null = 0) => ({ watch: w, compare: 'previous_stop', from_seq, to_seq: 1, items });

test('reattaches the project’s watch after a reload and counts its changed rows for the tab badge', async () => {
  const w = watchOf({ checkpoint_count: 2 });
  const h = harness({ routes: (c) => (c.url.startsWith('/ide/db-watch/watches?root=') ? { watches: [w] } : c.url === '/ide/db-watch/watches/w1?compare=previous_stop' ? readOf(w, [itemDiff({ added_count: 2, modified_count: 1 })]) : undefined) });
  h.store.follow({ root: '/r' });
  await flush(); await flush();
  assert.equal(h.store.watch?.id, 'w1');
  assert.equal(h.store.changeBadgeCount, 3);
  assert.equal(h.calls.filter((c) => c.method === 'POST').length, 0, 'restoring never starts a watch');
});

test('first time: nothing is armed at session start; at the FIRST STOP the run’s Testcontainer is watched with the verified tables, bound to the session, and remembered', async () => {
  const w = watchOf({ bind: { debug_session_id: 'd1' } });
  const h = harness({ routes: (c) => {
    if (c.url.startsWith('/ide/db-watch/watches?root=')) return { watches: [] };
    if (c.url === '/ide/db-watch/detect') {
      const verified = Boolean((c.body as { target?: unknown }).target);
      return { detected_objects: [det('GetAllConfigs', 'mysql', verified ? false : null), det('session_info', 'mysql', verified ? true : null), det('session_route', 'mysql', verified ? true : null)], saved_connections: [saved({ local: true, host: '127.0.0.1' })], ephemeral_targets: [otherContainer, tc] };
    }
    if (c.method === 'POST' && c.url === '/ide/db-watch/watches') return w;
    if (c.method === 'GET' && c.url.startsWith('/ide/db-watch/watches/w1')) return readOf(w, [itemDiff()], null);
    return undefined;
  } });
  h.store.follow({ root: '/r', debug: { id: 'd1', state: 'running', stop_seq: 0 } });
  await flush(); await flush();
  assert.equal(h.calls.filter((c) => c.url === '/ide/db-watch/watches' && c.method === 'POST').length, 0);
  h.store.follow({ root: '/r', debug: { id: 'd1', state: 'paused', stop_seq: 1 } });
  for (let i = 0; i < 6; i++) await flush();
  const start = h.calls.find((c) => c.method === 'POST' && c.url === '/ide/db-watch/watches');
  assert.deepEqual(plain(start?.body), {
    target: { kind: 'ephemeral', ephemeral_id: 'tc1' }, database: 'pr_bo_1',
    items: [{ object: 'session_info' }, { object: 'session_route' }], bind: { debug_session_id: 'd1' }, root: '/r',
  });
  assert.equal(JSON.parse(h.ls.get(memoryKey('/r')) ?? '{}').target.kind, 'auto');
  // A second stop of the same session does not start again (the daemon checkpoints).
  h.store.follow({ root: '/r', debug: { id: 'd1', state: 'paused', stop_seq: 2 } });
  await flush();
  assert.equal(h.calls.filter((c) => c.method === 'POST' && c.url === '/ide/db-watch/watches').length, 1);
});

test('never auto-arms a saved connection nobody picked, even a local one', async () => {
  const h = harness({ routes: (c) => (c.url.startsWith('/ide/db-watch/watches?root=') ? { watches: [] } : c.url === '/ide/db-watch/detect' ? { detected_objects: [det('session_info')], saved_connections: [saved({ local: true })], ephemeral_targets: [] } : undefined) });
  h.store.follow({ root: '/r', debug: { id: 'd1', state: 'paused', stop_seq: 1 } });
  for (let i = 0; i < 6; i++) await flush();
  assert.equal(h.calls.filter((c) => c.method === 'POST' && c.url === '/ide/db-watch/watches').length, 0);
});

test('a remembered saved pick arms at session start — unless that connection is now tagged prod', async () => {
  for (const [env, expected] of [['staging', 1], ['prod', 0]] as const) {
    const ls = new Map([[memoryKey('/r'), JSON.stringify({ target: { kind: 'saved', connection_id: 'stg' }, items: [{ object: 'session_info' }] })]]);
    const h = harness({ ls, prompt: () => 'pr_bo staging', routes: (c) => (c.url.startsWith('/ide/db-watch/watches?root=') ? { watches: [] } : c.url === '/ide/db-watch/detect' ? { detected_objects: [], saved_connections: [saved({ environment: env })], ephemeral_targets: [] } : c.url === '/ide/db-watch/watches' ? watchOf() : undefined) });
    h.store.follow({ root: '/r', debug: { id: 'd1', state: 'running', stop_seq: 0 } });
    for (let i = 0; i < 6; i++) await flush();
    const starts = h.calls.filter((c) => c.method === 'POST' && c.url === '/ide/db-watch/watches');
    assert.equal(starts.length, expected, env);
    assert.equal(h.prompts.length, 0, 'Otto never confirms prod on its own');
    if (expected) assert.deepEqual(plain((starts[0].body as { bind: unknown }).bind), { debug_session_id: 'd1' });
  }
});

test('prod: start asks the typed confirm with who asks and what runs; cancel sends nothing, a match sends confirm_production', async () => {
  const prodWatch = watchOf({ target: { kind: 'saved', connection_id: 'p', engine: 'mysql', label: 'pr_bo prod', environment: 'production', local: false } });
  for (const answer of [null, 'pr_bo prod']) {
    const h = harness({ prompt: () => answer, routes: (c) => (c.url === '/ide/db-watch/detect' ? { detected_objects: [], saved_connections: [saved({ connection_id: 'p', name: 'pr_bo prod', environment: 'production' })], ephemeral_targets: [] } : c.url === '/ide/db-watch/watches' ? prodWatch : c.url.startsWith('/ide/db-watch/watches?') ? { watches: [] } : undefined) });
    h.store.follow({ root: '/r' });
    await h.store.discover();
    await h.store.start({ target: { kind: 'saved', connection_id: 'p' }, items: [{ object: 'session_info' }] }, { asker: 'Agent Messi' });
    const starts = h.calls.filter((c) => c.method === 'POST' && c.url === '/ide/db-watch/watches');
    assert.match(h.prompts[0], /PRODUCTION/);
    assert.match(h.prompts[0], /Asked by: Agent Messi/);
    assert.match(h.prompts[0], /session_info/);
    assert.equal(starts.length, answer ? 1 : 0);
    if (answer) assert.equal((starts[0].body as { confirm_production?: boolean }).confirm_production, true);
    // C3-006: a saved target is only started as the person's (or a confirmed agent's) pick.
    if (answer) assert.equal((starts[0].body as { confirm_target?: boolean }).confirm_target, true);
    assert.equal(h.ls.has(memoryKey('/r')), false, 'a prod target is never remembered');
  }
});

test('a checkpoint event for this watch refetches; another watch’s does not', async () => {
  const w = watchOf();
  const h = harness({ routes: (c) => (c.url.startsWith('/ide/db-watch/watches?root=') ? { watches: [w] } : c.url.startsWith('/ide/db-watch/watches/w1') ? readOf(w) : undefined) });
  h.store.follow({ root: '/r' });
  await flush(); await flush();
  const reads = () => h.calls.filter((c) => c.method === 'GET' && c.url.startsWith('/ide/db-watch/watches/w1')).length;
  const before = reads();
  h.handlers.ide_db_watch_checkpoint({ type: 'ide_db_watch_checkpoint', watch_id: 'other', seq: 2 });
  await flush();
  assert.equal(reads(), before);
  h.handlers.ide_db_watch_checkpoint({ type: 'ide_db_watch_checkpoint', watch_id: 'w1', seq: 2 });
  await flush();
  assert.equal(reads(), before + 1);
});

test('the agent read asks the daemon to mask and leaves the panel’s view alone', async () => {
  const w = watchOf();
  const h = harness({ routes: (c) => (c.url.startsWith('/ide/db-watch/watches?root=') ? { watches: [w] } : c.url.startsWith('/ide/db-watch/watches/w1') ? readOf(w, [itemDiff({ object: c.url.includes('for_agent') ? 'masked' : 'session_info' })]) : undefined) });
  h.store.follow({ root: '/r' });
  await flush(); await flush();
  const resp = await h.store.diffSince('start', { forAgent: true });
  assert.ok(h.calls.some((c) => c.url === '/ide/db-watch/watches/w1?compare=start&for_agent=1'));
  assert.equal(resp.items[0].object, 'masked');
  assert.equal(h.store.read.items[0].object, 'session_info');
});

test('Stop watching drops the watch and this project stops re-arming at the next stop', async () => {
  const w = watchOf();
  const h = harness({ routes: (c) => (c.url.startsWith('/ide/db-watch/watches?root=') ? { watches: [w] } : c.url.startsWith('/ide/db-watch/watches/w1') ? readOf(w) : c.url === '/ide/db-watch/detect' ? { detected_objects: [det('session_info')], saved_connections: [], ephemeral_targets: [tc] } : undefined) });
  h.store.follow({ root: '/r' });
  await flush(); await flush();
  await h.store.stop();
  assert.ok(h.calls.some((c) => c.method === 'DELETE' && c.url === '/ide/db-watch/watches/w1'));
  assert.equal(h.store.watch, null);
  h.store.follow({ root: '/r', debug: { id: 'd2', state: 'paused', stop_seq: 1 } });
  for (let i = 0; i < 6; i++) await flush();
  assert.equal(h.calls.filter((c) => c.method === 'POST' && c.url === '/ide/db-watch/watches').length, 0);
});

test('agent + auto-arm at the same first stop: starts run one after the other and the auto-arm never replaces a watch already bound to that session', async () => {
  const w = watchOf({ bind: { debug_session_id: 'd1' } });
  const gate = deferred<unknown>();
  let posts = 0;
  const h = harness({ routes: (c) => {
    if (c.method === 'POST' && c.url === '/ide/db-watch/watches') { posts += 1; return gate.promise; }
    if (c.method === 'GET' && c.url.startsWith('/ide/db-watch/watches/w1')) return readOf(w);
    return undefined;
  } });
  h.store.follow({ root: '/r' });
  const agent = h.store.start({ target: { kind: 'auto', engine: 'mysql' }, items: [{ object: 'orders' }], bind: { debug_session_id: 'd1' } }, { asker: 'Claude · Fix orders' });
  const auto = h.store.start({ target: { kind: 'ephemeral', ephemeral_id: 'tc1' }, items: [{ object: 'session_info' }], bind: { debug_session_id: 'd1' } }, { auto: true });
  await flush(); await flush();
  assert.equal(posts, 1, 'the second start waits for the first');
  gate.resolve(w);
  assert.equal((await agent)?.id, 'w1');
  assert.equal((await auto)?.id, 'w1');
  assert.equal(posts, 1, 'the auto-arm saw the session already watched');
});

test('an agent start on a saved prod connection the panel never listed still asks the typed confirm, naming the agent', async () => {
  const prodWatch = watchOf({ target: { kind: 'saved', connection_id: 'p', engine: 'mysql', label: 'pr_bo prod', environment: 'production', local: false } });
  const h = harness({ prompt: () => null, routes: (c) => (c.url === '/ide/db-watch/detect' ? { detected_objects: [], saved_connections: [saved({ connection_id: 'p', name: 'pr_bo prod', environment: 'production' })], ephemeral_targets: [] } : c.url === '/ide/db-watch/watches' ? prodWatch : c.url.startsWith('/ide/db-watch/watches?') ? { watches: [] } : undefined) });
  h.store.follow({ root: '/r' });
  await flush();
  const r = await h.store.start({ target: { kind: 'saved', connection_id: 'p' }, items: [{ object: 'session_info' }] }, { asker: 'Claude · Fix orders' });
  assert.equal(r, null);
  assert.match(h.prompts[0] ?? '', /Asked by: Claude · Fix orders/);
  assert.equal(h.calls.filter((c) => c.method === 'POST' && c.url === '/ide/db-watch/watches').length, 0);
});

test('switching the IDE target drops in-flight reads; failures read as people text', async () => {
  const w = watchOf();
  const gate = deferred<unknown>();
  let fail = false;
  const h = harness({ routes: (c) => {
    if (c.url.startsWith('/ide/db-watch/watches?root=')) return { watches: [w] };
    if (c.url.startsWith('/ide/db-watch/watches/w1')) return fail ? new ApiError(401, { code: 'unauthorized', message: 'unauthorized' }) : gate.promise;
    return undefined;
  } });
  h.store.follow({ root: '/r' });
  await flush();
  h.resets[0]();
  gate.resolve(readOf(w));
  await flush(); await flush();
  assert.equal(h.store.read, null, 'a stale read must not land after the switch');
  assert.equal(h.store.watch, null);
  fail = true;
  h.store.follow({ root: '/r' });
  await flush(); await flush(); await flush();
  assert.equal(h.store.error, 'Your session expired. Sign in again, then retry.');
});

// ── Code review round 1 ────────────────────────────────────────────────────

test('C1-018: "Show current rows" keys a composite primary key by the whole tuple, and repeated keys never collide', async () => {
  const { currentAsDiffs, rowKeys } = await import('../src/modules/ide/dbwatch/ideDbWatchModel.ts');
  const cur = { columns: ['game_id', 'account_id', 'gp_session'], rows: [[1, 77, 'a'], [1, 78, 'b'], [2, 77, 'c']] };
  const rows = currentAsDiffs(cur, ['game_id', 'account_id']);
  assert.deepEqual(rows.map((r) => r.key), ['1, 77', '1, 78', '2, 77']);
  const keys = rowKeys([...rows, { key: '1, 77', kind: 'added', row: [], changes: [] }, { key: 'x', kind: 'added', row: [], changes: [] }, { key: 'x', kind: 'added', row: [], changes: [] }]);
  assert.equal(new Set(keys).size, keys.length, 'each-block keys must be unique');
  assert.deepEqual(currentAsDiffs({ columns: ['v'], rows: [[null], [null]] }, []).map((r) => r.key), ['1', '2'], 'no key column: the row number');
});

test('C1-035: names detected as SQL (guessed mysql) count on any SQL target once the target verified them', () => {
  const d = [det('orders', 'mysql', true), det('GetAllConfigs', 'mysql', false), det('session:*', 'redis', null)];
  assert.deepEqual(pickAutoItems(d, 'postgres', true), [{ object: 'orders' }]);
  assert.deepEqual(unverifiedNames(d, 'postgres'), ['GetAllConfigs']);
  assert.deepEqual(pickAutoItems(d, 'clickhouse', true), [{ object: 'orders' }]);
  assert.deepEqual(pickAutoItems(d, 'redis', true), [{ object: 'session:*' }], 'SQL names never land on Redis');
  assert.deepEqual(pickAutoItems([det('orders', 'mysql', null)], 'postgres', true), [], 'unverified cross-engine guesses stay out');
});

const prodW = (over: Partial<IdeDbWatch> = {}) => watchOf({ bind: { debug_session_id: 'd1' }, target: { kind: 'saved', connection_id: 'p', engine: 'mysql', label: 'pr_bo prod', environment: 'production', local: false }, ...over });

test('C2-012: a new debug session or run never pops the prod confirm: the prod watch waits, and one click re-binds it after the typed confirm', async () => {
  for (const follow of [{ debug: { id: 'd2', state: 'running', stop_seq: 0 } }, { run: { id: 'r2', status: 'running' } }]) {
    const w = prodW();
    const h = harness({ prompt: () => 'pr_bo prod', routes: (c) => (c.url.startsWith('/ide/db-watch/watches?root=') ? { watches: [w] } : c.method === 'PATCH' ? w : c.url.startsWith('/ide/db-watch/watches/w1') ? readOf(w) : undefined) });
    h.store.follow({ root: '/r' });
    await flush(); await flush();
    h.store.follow({ root: '/r', ...follow });
    for (let i = 0; i < 6; i++) await flush();
    assert.equal(h.prompts.length, 0, 'no modal on a launch');
    assert.equal(h.calls.filter((c) => c.method === 'PATCH').length, 0);
    assert.ok(h.store.prodWaiting, 'the panel / strip offer "Production target — click to watch"');
    await h.store.watchProd();
    assert.match(h.prompts[0] ?? '', /Asked by: You/);
    const patches = h.calls.filter((c) => c.method === 'PATCH');
    assert.deepEqual(plain(patches[0].body), { bind: 'debug' in follow ? { debug_session_id: 'd2' } : { run_id: 'r2' }, confirm_production: true });
    assert.equal(h.store.prodWaiting, null);
  }
});

test('C2-012: a remembered connection now tagged prod stays un-armed on launch; the click starts it through the typed confirm', async () => {
  const ls = new Map([[memoryKey('/r'), JSON.stringify({ target: { kind: 'saved', connection_id: 'p' }, items: [{ object: 'session_info' }] })]]);
  const h = harness({ ls, prompt: () => 'pr_bo prod', routes: (c) => (c.url.startsWith('/ide/db-watch/watches?root=') ? { watches: [] } : c.url === '/ide/db-watch/detect' ? { detected_objects: [], saved_connections: [saved({ connection_id: 'p', name: 'pr_bo prod', environment: 'production' })], ephemeral_targets: [] } : c.url === '/ide/db-watch/watches' ? prodW() : c.url.startsWith('/ide/db-watch/watches/w1') ? readOf(prodW()) : undefined) });
  h.store.follow({ root: '/r', debug: { id: 'd1', state: 'running', stop_seq: 0 } });
  for (let i = 0; i < 6; i++) await flush();
  assert.equal(h.prompts.length, 0);
  assert.equal(h.calls.filter((c) => c.method === 'POST' && c.url === '/ide/db-watch/watches').length, 0);
  assert.ok(h.store.prodWaiting);
  await h.store.watchProd();
  const starts = h.calls.filter((c) => c.method === 'POST' && c.url === '/ide/db-watch/watches');
  assert.equal(h.prompts.length, 1);
  assert.equal((starts[0]?.body as { confirm_production?: boolean })?.confirm_production, true);
  assert.deepEqual(plain((starts[0].body as { bind: unknown }).bind), { debug_session_id: 'd1' });
});

test('C2-012: removing a table from a prod watch needs no typed confirm (it reads less); adding one still does', async () => {
  const w = prodW({ items: [{ object: 'a', kind: 'table', primary_key: ['id'] }, { object: 'b', kind: 'table', primary_key: ['id'] }] });
  const h = harness({ prompt: () => null, routes: (c) => (c.url.startsWith('/ide/db-watch/watches?root=') ? { watches: [w] } : c.method === 'PATCH' ? w : c.url.startsWith('/ide/db-watch/watches/w1') ? readOf(w) : undefined) });
  h.store.follow({ root: '/r' });
  await flush(); await flush();
  await h.store.removeItem('b');
  assert.equal(h.prompts.length, 0);
  assert.deepEqual(plain(h.calls.find((c) => c.method === 'PATCH')?.body), { items: [{ object: 'a' }], confirm_production: true });
  await h.store.addItem({ object: 'c' });
  assert.equal(h.prompts.length, 1, 'adding a table to prod asks');
  assert.equal(h.calls.filter((c) => c.method === 'PATCH').length, 1, 'cancelled: nothing sent');
});

test('C3-003: starting another watch or stopping clears the "Production target — click to watch" line', async () => {
  for (const act of ['start', 'stop'] as const) {
    const w = prodW();
    const local = watchOf({ id: 'w2' });
    const h = harness({ routes: (c) => (c.url.startsWith('/ide/db-watch/watches?root=') ? { watches: [w] } : c.method === 'POST' && c.url === '/ide/db-watch/watches' ? local : c.url.startsWith('/ide/db-watch/watches/') ? readOf(c.url.includes('w2') ? local : w) : undefined) });
    h.store.follow({ root: '/r' });
    await flush(); await flush();
    h.store.follow({ root: '/r', debug: { id: 'd2', state: 'running', stop_seq: 0 } });
    for (let i = 0; i < 6; i++) await flush();
    assert.ok(h.store.prodWaiting, act);
    if (act === 'start') await h.store.start({ target: { kind: 'ephemeral', ephemeral_id: 'tc1' }, items: [{ object: 'session_info' }] });
    else await h.store.stop();
    assert.equal(h.store.prodWaiting, null, act);
  }
});
