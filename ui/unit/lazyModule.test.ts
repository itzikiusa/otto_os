import { test } from 'node:test';
import assert from 'node:assert/strict';
import { lazyModule } from '../src/lib/lazyModule.ts';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

test('lazyModule: loads once, queues calls in order, then runs synchronously', async () => {
  let loads = 0;
  const seen: string[] = [];
  const m = lazyModule(async () => {
    loads += 1;
    return { push: (s: string) => seen.push(s) };
  });
  assert.equal(m.peek(), null);
  m.use((x) => x.push('a'));
  m.use((x) => x.push('b'));
  assert.equal(seen.length, 0);
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(seen, ['a', 'b']);
  m.use((x) => x.push('c')); // loaded: synchronous
  assert.deepEqual(seen, ['a', 'b', 'c']);
  assert.equal(loads, 1);
  assert.ok(m.peek());
});

test('lazyModule: a failed import drops its calls and retries on the next use', async () => {
  let attempt = 0;
  const seen: number[] = [];
  const m = lazyModule(async () => {
    attempt += 1;
    if (attempt === 1) throw new Error('offline');
    return { n: attempt };
  });
  m.use((x) => seen.push(x.n));
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(seen.length, 0);
  m.use((x) => seen.push(x.n));
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(seen, [2]);
});

test('lazyModule: peek() sees a module that announced itself under the key, and never imports', async () => {
  const { announceModule } = await import('../src/lib/lazyModule.ts');
  let loads = 0;
  const m = lazyModule(async () => {
    loads += 1;
    return { id: 'via-handle' };
  }, 'g2-test-store');
  const other = lazyModule(async () => ({ id: 'x' }), 'g2-other');
  assert.equal(m.peek(), null);
  // A page imported the store statically: it announces itself at evaluation.
  const store = { id: 'static' };
  announceModule('g2-test-store', store);
  assert.equal(m.peek(), store);
  assert.equal(other.peek(), null, 'another key stays unloaded');
  assert.equal(loads, 0, 'peek never imports');
});

/** The dispatcher and its per-feature route tables, as one text. */
function eventsSources(): string {
  const lib = join(import.meta.dirname, '..', 'src', 'lib');
  const routes = join(lib, 'events', 'routes');
  return [join(lib, 'events.svelte.ts'), ...readdirSync(routes).filter((f) => f.endsWith('.ts')).sort().map((f) => join(routes, f))]
    .map((f) => readFileSync(f, 'utf8'))
    .join('\n');
}

// Events that only matter to a store someone already uses route through
// peek() (perf G2): `use()` there imported the page store into every document.
test('events: reconnect / restart / periodic events peek page stores instead of loading them', async () => {
  const src = join(import.meta.dirname, '..', 'src');
  const ev = eventsSources();
  for (const call of [
    'databaseStore.peek()?.onDaemonRestart()',
    'usageStore.peek()?.applyMetricsTick()',
    'k8sStore.peek()?.applyEvent(parsed)',
    'personalAgentsStore.peek()?.resyncRooms()',
    // perf H1: per-event page-store handlers.
    'apiClientStore.peek()?.noteHistoryAppended(parsed.workspace_id, parsed.entry_id)',
    'apiClientStore.peek()?.noteRunProgress(parsed.run_id, parsed.status)',
    'awsStore.peek()?.applyEvent(parsed)',
    'personalAgentsStore.peek()?.applyRunEvent(parsed)',
    'personalAgentsStore.peek()?.applyRoomEvent(parsed)',
  ]) {
    assert.ok(ev.includes(call), `expected ${call}`);
  }
  for (const banned of ['database.onDaemonRestart()', 'usage.applyMetricsTick()']) {
    assert.ok(!ev.includes(`.use((${banned.split('.')[0]}) => ${banned})`) && !ev.includes(`=> void ${banned}`), banned);
  }
  // Every peeked handle has a key, and its store announces itself under it.
  const files: Record<string, string> = {
    database: 'lib/stores/database.svelte.ts',
    usage: 'lib/api/usage.svelte.ts',
    personalAgents: 'lib/stores/personalAgents.svelte.ts',
    k8s: 'lib/stores/k8s.svelte.ts',
    apiClient: 'lib/stores/apiClient.svelte.ts',
    aws: 'lib/stores/aws.svelte.ts',
    scheduledTasks: 'lib/stores/scheduledTasks.svelte.ts',
    product: 'lib/stores/product.svelte.ts',
    loops: 'lib/stores/loops.svelte.ts',
    browser: 'lib/stores/browser.svelte.ts',
    browserLive: 'lib/stores/browserLive.svelte.ts',
  };
  for (const [key, file] of Object.entries(files)) {
    assert.match(ev, new RegExp(`const ${key}Store = lazyModule\\([^\\n]*, '${key}'\\);`), `${key}Store needs its key`);
    assert.match(readFileSync(join(src, file), 'utf8'), new RegExp(`^announceModule\\('${key}', ${key}\\);$`, 'm'), file);
  }
});

// perf H1: `use()` imports a page store into every document — main window,
// pop-out, side pane — that receives the event. Only events whose payload must
// survive until the page mounts (an agent session / proposal to attach) may do
// that; every other handler peeks. A new `.use(` fails here until it is
// justified and added to the list.
test('events: only allow-listed events load a page store (`.use`)', async () => {
  const ev = eventsSources();
  const sites: string[] = [];
  for (const m of ev.matchAll(/(\w+)Store\.use\(/g)) {
    // The route the call sits in: the nearest `<event_type>: (parsed) =>` key
    // of a route table (lib/events/routes/*.ts, architecture-2-04).
    const head = ev.slice(0, m.index);
    const keys = [...head.matchAll(/^\s+(\w+): \(parsed\) =>/gm)];
    const key = keys.at(-1);
    assert.ok(key, `.use( at offset ${m.index} is not inside an event route`);
    assert.ok(!head.slice(key.index).includes('\n};'), `.use( at offset ${m.index} is outside a route table`);
    sites.push(`${key[1]} → ${m[1]}`);
  }
  assert.deepEqual(sites.sort(), [
    'canvas_session_started → canvas',
    'db_assist_session_started → database',
    'db_assist_updated → database',
    'db_mcp_activity → database',
    'mockup_session_started → mockupAssist',
    'mockup_updated → mockupAssist',
  ]);
});
