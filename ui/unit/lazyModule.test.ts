import { test } from 'node:test';
import assert from 'node:assert/strict';
import { lazyModule } from '../src/lib/lazyModule.ts';

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

// Events that only matter to a store someone already uses route through
// peek() (perf G2): `use()` there imported the page store into every document.
test('events: reconnect / restart / periodic events peek page stores instead of loading them', async () => {
  const { readFileSync } = await import('node:fs');
  const { join } = await import('node:path');
  const src = join(import.meta.dirname, '..', 'src');
  const ev = readFileSync(join(src, 'lib/events.svelte.ts'), 'utf8');
  for (const call of [
    'swarmStore.peek()?.resync()',
    'databaseStore.peek()?.onDaemonRestart()',
    'usageStore.peek()?.applyMetricsTick()',
    'swarmStore.peek()?.applyEvent(parsed)',
    "if (parsed.type === 'k8s_monitor_cycle') k8sStore.peek()?.applyEvent(parsed)",
    'personalAgentsStore.peek()?.resyncRooms()',
  ]) {
    assert.ok(ev.includes(call), `expected ${call}`);
  }
  for (const banned of ['swarm.resync()', 'database.onDaemonRestart()', 'usage.applyMetricsTick()']) {
    assert.ok(!ev.includes(`.use((${banned.split('.')[0]}) => ${banned})`) && !ev.includes(`=> void ${banned}`), banned);
  }
  // Every peeked handle has a key, and its store announces itself under it.
  const files: Record<string, string> = {
    swarm: 'lib/stores/swarm.svelte.ts',
    database: 'lib/stores/database.svelte.ts',
    usage: 'lib/api/usage.svelte.ts',
    personalAgents: 'lib/stores/personalAgents.svelte.ts',
    k8s: 'lib/stores/k8s.svelte.ts',
  };
  for (const [key, file] of Object.entries(files)) {
    assert.match(ev, new RegExp(`const ${key}Store = lazyModule\\([^\\n]*, '${key}'\\);`), `${key}Store needs its key`);
    assert.match(readFileSync(join(src, file), 'utf8'), new RegExp(`^announceModule\\('${key}', ${key}\\);$`, 'm'), file);
  }
});
