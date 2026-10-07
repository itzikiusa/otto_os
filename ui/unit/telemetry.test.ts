import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import { webcrypto } from 'node:crypto';
import { performance } from 'node:perf_hooks';
import { loadSource, deferred } from './sourceHarness.ts';

function load(): Record<string, any> & { clocks: () => number } {
  let clocks = 0;
  const telemetry = loadSource(new URL('../src/lib/telemetryRuntime.ts', import.meta.url), {
    './telemetry': { installTelemetryRuntime() {} },
    './poll': { pollWhileVisible: () => { clocks++; return { stop: () => clocks-- }; } },
  }, { crypto: webcrypto, performance });
  return { ...telemetry, clocks: () => clocks };
}

test('initial application hooks need no runtime; an opt-in import connects the same hooks', () => {
  const hooks = loadSource(new URL('../src/lib/telemetry.ts', import.meta.url), {});
  assert.equal(hooks.startMeasurement('ui.render', 'agents'), null);
  assert.equal(hooks.apiComponent('/repos/private'), 'other');
  const runtime = loadSource(new URL('../src/lib/telemetryRuntime.ts', import.meta.url), {
    './telemetry': hooks,
    './poll': { pollWhileVisible: () => ({ stop() {} }) },
  });
  hooks.configureTelemetry(true, async () => {});
  assert.equal(hooks.isTelemetryEnabled(), true);
  assert.equal(hooks.apiComponent('/repos/private'), 'git');
  hooks.startMeasurement('ui.render', 'agents').finish();
  assert.equal(runtime.telemetryState().queued, 1);
  hooks.configureTelemetry(false);
  assert.equal(runtime.telemetryState().queued, 0);
});

test('opt-out creates no clock, span, queue or export; disabling drops in-flight work', async () => {
  const t = load();
  let sends = 0;
  assert.equal(t.startMeasurement('ui.render', 'agents'), null);
  assert.equal(t.clocks(), 0);
  t.configureTelemetry(true, async () => { sends++; });
  assert.equal(t.clocks(), 1);
  const stale = t.startMeasurement('ui.render', 'agents');
  t.configureTelemetry(false);
  stale.finish();
  await t.flushTelemetry();
  assert.equal(t.clocks(), 0);
  assert.equal(t.telemetryState().queued, 0);
  assert.equal(sends, 0);
});

test('queue is bounded, exports are batched and failures do not grow an offline backlog', async () => {
  const t = load();
  const batches: unknown[][] = [];
  t.configureTelemetry(true, async (batch: unknown[]) => { batches.push(batch); throw new Error('offline'); });
  for (let i = 0; i < 600; i++) t.startMeasurement('ui.render', 'agents').finish();
  assert.equal(t.telemetryState().queued, 500);
  assert.equal(t.telemetryState().dropped, 100);
  await t.flushTelemetry();
  assert.equal(batches[0].length, 100);
  assert.equal(t.telemetryState().queued, 400);
  assert.equal(t.telemetryState().dropped, 200);
  t.configureTelemetry(false);
});

test('navigation and request spans share W3C context, and arbitrary fields never leave', async () => {
  const t = load();
  let spans: any[] = [];
  t.configureTelemetry(true, async (batch: any[]) => { spans = batch; });
  const done = t.beginNavigation('git');
  const request = t.startMeasurement('http.client', t.apiComponent('/repos/private-project/prs?token=secret'), 'client');
  request.finish('ok', { 'http.request.method': 'GET', password: 'secret', url: '/private', 'http.response.status_code': 200 });
  done();
  await t.flushTelemetry();
  assert.equal(spans.length, 2);
  assert.equal(spans[0].trace_id, spans[1].trace_id);
  assert.equal(spans[0].parent_span_id, spans[1].span_id);
  assert.equal(spans[0].component, 'git');
  assert.match(request.traceparent, /^00-[a-f0-9]{32}-[a-f0-9]{16}-01$/);
  assert.ok(!JSON.stringify(spans).includes('secret'));
  assert.ok(!JSON.stringify(spans).includes('private'));
  assert.equal(t.apiComponent('/workspaces/private-project/vault/vaults/secret'), 'vault');
  assert.equal(t.apiComponent('/private-name'), 'other');
  t.configureTelemetry(false);
});

test('one flush at a time; consent revocation aborts delivery and invalidates old spans', async () => {
  const t = load();
  const pending = deferred<void>();
  let calls = 0;
  let signal: AbortSignal | undefined;
  t.configureTelemetry(true, async (_: unknown, s: AbortSignal) => { calls++; signal = s; return pending.promise; });
  t.startMeasurement('ui.render', 'agents').finish();
  const flush = t.flushTelemetry();
  t.startMeasurement('ui.render', 'agents').finish();
  await t.flushTelemetry();
  assert.equal(calls, 1);
  t.configureTelemetry(false);
  assert.equal(signal?.aborted, true);
  pending.resolve();
  await flush;
  assert.equal(t.telemetryState().queued, 0);
});


test('failed transport spans coexist with successful calls without invented HTTP status', async () => {
  const t = load();
  let spans: any[] = [];
  t.configureTelemetry(true, async (batch: any[]) => { spans = batch; });
  t.startMeasurement('http.client', 'git', 'client').finish('error', { 'http.request.method': 'GET' });
  t.startMeasurement('http.client', 'git', 'client').finish('ok', { 'http.response.status_code': 200 });
  await t.flushTelemetry();
  assert.equal(spans.length, 2);
  assert.equal(spans[0].status, 'error');
  assert.ok(!('http.response.status_code' in spans[0].attributes));
  assert.equal(spans[1].attributes['http.response.status_code'], 200);
  t.configureTelemetry(false);
});

test('production HTTP wrapper omits a nonexistent response status on rejection', async () => {
  const t = load();
  let spans: any[] = [];
  t.configureTelemetry(true, async (batch: any[]) => { spans = batch; });
  const { api } = loadSource(new URL('../src/lib/api/client.ts', import.meta.url), {
    '../telemetry': { ...t, clientSpanName: loadSource(new URL('../src/lib/telemetry.ts', import.meta.url), {}).clientSpanName },
    '../stores/serviceHealth.svelte': { serviceHealth: { report() {} } },
    './lane': loadSource(new URL('../src/lib/api/lane.ts', import.meta.url), {}),
  }, { location: { port: '7700', origin: 'http://localhost:7700' }, fetch: async () => { throw new Error('network failed'); } });
  await assert.rejects(api.get('/repos/secret/status'));
  await t.flushTelemetry();
  assert.equal(spans.length, 1);
  assert.equal(spans[0].status, 'error');
  assert.ok(!('http.response.status_code' in spans[0].attributes));
  t.configureTelemetry(false);
});

test('configuration fetch retries a temporary failure and never reactivates after disposal', async () => {
  let calls = 0;
  const timers = new Map<number, () => void>();
  let id = 0;
  const states: boolean[] = [];
  const boot = loadSource(new URL('../src/lib/telemetryBoot.ts', import.meta.url), {
    './telemetryRuntime': {},
    './api/client': { api: { get: async () => { if (++calls === 1) throw new Error('temporary'); return { enabled: true }; } }, baseUrl: () => '', getToken: () => 'fixture' },
    './telemetry': { configureTelemetry: (active: boolean) => states.push(active), beginNavigation: () => () => {} },
  }, {
    window: { addEventListener() {}, removeEventListener() {} },
    requestAnimationFrame: (fn: () => void) => fn(),
    setTimeout: (fn: () => void) => { timers.set(++id, fn); return id; },
    clearTimeout: (key: number) => timers.delete(key),
  });
  const stop = boot.bootTelemetry('agents');
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(calls, 1);
  assert.equal(timers.size, 1, 'a failed initial request schedules recovery');
  const retry = [...timers.values()][0]; timers.clear(); retry();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(calls, 2);
  assert.equal(states.at(-1), true);
  stop();
  assert.equal(states.at(-1), false);
  assert.equal(timers.size, 0);
});

test('hiding a view during paint discards render and navigation instead of recording timeout latency', async () => {
  const listeners = new Map<string, () => void>();
  const doc = { hidden: false, addEventListener: (name: string, fn: () => void) => listeners.set(name, fn), removeEventListener: (name: string) => listeners.delete(name) };
  let frame: (() => void) | undefined;
  const t = loadSource(new URL('../src/lib/telemetryRuntime.ts', import.meta.url), {
    './telemetry': { installTelemetryRuntime() {} },
    './poll': { pollWhileVisible: () => ({ stop() {} }) },
  }, { crypto: webcrypto, performance, document: doc, requestAnimationFrame: (fn: () => void) => { frame = fn; return 1; }, cancelAnimationFrame: () => { frame = undefined; } });
  t.configureTelemetry(true, async () => {});
  const finish = t.beginNavigation('git');
  t.finishNavigationPaint('git', finish);
  assert.ok(frame);
  doc.hidden = true; listeners.get('visibilitychange')?.();
  assert.equal(t.telemetryState().queued, 0);
  assert.equal(frame, undefined);
  assert.equal(listeners.size, 0);
  t.configureTelemetry(false);
});

for (const pendingPhase of ['config', 'runtime'] as const) {
  test(`disposal during pending ${pendingPhase} cannot activate collection`, async () => {
    const config = deferred<{ enabled: boolean }>();
    const runtime = deferred<object>();
    const states: boolean[] = [];
    const timers = new Set<unknown>();
    const boot = loadSource(new URL('../src/lib/telemetryBoot.ts', import.meta.url), {
      './api/client': { api: { get: () => config.promise }, baseUrl: () => '', getToken: () => 'fixture' },
      // Keep the deferred module promise intact through TypeScript's __importStar.
      './telemetryRuntime': Object.assign(runtime.promise, { __esModule: true }),
      './telemetry': { configureTelemetry: (active: boolean) => states.push(active), beginNavigation: () => () => {} },
    }, {
      window: { addEventListener() {}, removeEventListener() {} },
      requestAnimationFrame: (fn: () => void) => fn(),
      setTimeout: (fn: unknown) => { timers.add(fn); return fn; },
      clearTimeout: (key: unknown) => timers.delete(key),
    });
    const stop = boot.bootTelemetry('agents');
    if (pendingPhase === 'runtime') {
      config.resolve({ enabled: true });
      await new Promise((resolve) => setImmediate(resolve));
    }
    stop();
    config.resolve({ enabled: true });
    runtime.resolve({});
    await new Promise((resolve) => setImmediate(resolve));
    assert.ok(!states.includes(true));
    assert.equal(states.at(-1), false);
    assert.equal(timers.size, 0);
  });
}

test('a newer disabled configuration wins over a pending runtime import', async () => {
  const runtime = deferred<object>();
  const states: boolean[] = [];
  let calls = 0;
  let refresh: (() => void) | undefined;
  const boot = loadSource(new URL('../src/lib/telemetryBoot.ts', import.meta.url), {
    './api/client': { api: { get: async () => ({ enabled: ++calls === 1 }) }, baseUrl: () => '', getToken: () => 'fixture' },
    // Keep the deferred module promise intact through TypeScript's __importStar.
      './telemetryRuntime': Object.assign(runtime.promise, { __esModule: true }),
    './telemetry': { configureTelemetry: (active: boolean) => states.push(active), beginNavigation: () => () => {} },
  }, {
    window: { addEventListener: (_: string, fn: () => void) => { refresh = fn; }, removeEventListener() {} },
    requestAnimationFrame: (fn: () => void) => fn(),
  });
  const stop = boot.bootTelemetry('agents');
  await new Promise((resolve) => setImmediate(resolve));
  refresh?.();
  await new Promise((resolve) => setImmediate(resolve));
  runtime.resolve({});
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(calls, 2);
  assert.deepEqual(states, [false]);
  stop();
});

test('actual HTTP producer conforms to the shared ingest contract without exporting route identities', async () => {
  const contract = JSON.parse(readFileSync(new URL('../../crates/otto-telemetry/fixtures/ui-ingest.json', import.meta.url), 'utf8'));
  const hooks = loadSource(new URL('../src/lib/telemetry.ts', import.meta.url), {});
  const t = load();
  let spans: any[] = [];
  t.configureTelemetry(true, async (batch: any[]) => { spans = batch; });
  const { api } = loadSource(new URL('../src/lib/api/client.ts', import.meta.url), {
    '../telemetry': { ...t, clientSpanName: hooks.clientSpanName },
    '../stores/serviceHealth.svelte': { serviceHealth: { report() {} } },
    './lane': loadSource(new URL('../src/lib/api/lane.ts', import.meta.url), {}),
  }, { location: { port: '7700', origin: 'http://localhost:7700' }, fetch: async () => new Response('{}', { headers: { 'x-otto-route': current.route } }) });
  let current = contract.requests[0];
  const done = t.beginNavigation('git');
  for (current of contract.requests) {
    if (current.method === 'POST') await api.post('/repos/private-project/fetch');
    else await api.get('/repos/private-project');
  }
  t.startMeasurement('ui.render', 'git').finish();
  done();
  await t.flushTelemetry();
  const clients = spans.filter((span) => span.kind === 'client');
  assert.deepEqual(Array.from(clients, (span) => span.name), contract.requests.map((request: any) => request.name));
  const navigation = spans.find((span) => span.name === 'ui.navigation');
  assert.ok(navigation);
  assert.ok(clients.every((span) => span.parent_span_id === navigation.span_id));
  assert.ok(!JSON.stringify(spans).includes('private'));
  t.configureTelemetry(false);
});
