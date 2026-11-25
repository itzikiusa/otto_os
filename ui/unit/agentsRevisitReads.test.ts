// A revisit of Agents reads nothing the first visit already read (perf p2):
// the task roll-up once per workspace (events keep it current), the usage
// limits not again inside their own 60 s cadence.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

test('the task roll-up is read once per workspace; a failure or an identity change reads again', async () => {
  const calls: { path: string; response: ReturnType<typeof deferred<any>> }[] = [];
  const get = (path: string) => { const response = deferred<any>(); calls.push({ path, response }); return response.promise; };
  const { activity } = loadSource(new URL('../src/lib/stores/activity.svelte.ts', import.meta.url), {
    '../api/client': { getToken: () => 't', api: { get, post: get } },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
  });
  activity.ensureSummary('w1');
  calls[0].response.resolve([{ session_id: 's', total: 1, done: 0, in_progress: null, last_ts: null }]);
  await calls[0].response.promise;
  activity.ensureSummary('w1'); // the revisit
  assert.equal(calls.length, 1);
  activity.ensureSummary('w2'); // another workspace
  assert.equal(calls.length, 2);
  calls[1].response.reject(new Error('down'));
  await calls[1].response.promise.catch(() => {});
  await new Promise((r) => setImmediate(r));
  activity.ensureSummary('w2'); // the failed read is retried
  assert.equal(calls.length, 3);
  activity.reset();
  activity.ensureSummary('w2');
  assert.equal(calls.length, 4);
  assert.match(calls[3].path, /\/workspaces\/w2\/activity\/summary$/);
});

test('usage limits: a remount inside the cadence waits for it instead of reading', async () => {
  const polls: { immediate?: boolean }[] = [];
  let run: ((s: AbortSignal) => Promise<unknown>) | null = null;
  const { limits } = loadSource(new URL('../src/lib/stores/usageLimits.svelte.ts', import.meta.url), {
    '../api/client': { api: { get: async () => ({ accounts: [], stale_after_secs: 600 }) } },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../poll': { pollWhileVisible: (fn: typeof run, o: { immediate?: boolean }) => { run = fn; polls.push(o); return { stop() {}, now() {} }; } },
  });
  let off = limits.mount();
  assert.notEqual(polls[0].immediate, false, 'the first mount reads');
  await run!(new AbortController().signal);
  off();
  off = limits.mount(); // the revisit, a moment later
  assert.equal(polls[1].immediate, false, 'a fresh answer is not re-read');
  off();
});

test('an unfocused window re-reads usage limits and machine status at a slow pace, not every tick', async () => {
  let focused = false;
  const document = { hasFocus: () => focused };
  const make = (url: string, imports: Record<string, unknown>) => {
    let run: ((s: AbortSignal) => Promise<unknown>) | null = null;
    let calls = 0;
    const m = loadSource(new URL(url, import.meta.url), {
      ...imports,
      [Object.keys(imports).find((k) => k.endsWith('/poll'))!]: { pollWhileVisible: (fn: typeof run) => { run = fn; return { stop() {}, now() {} }; } },
      [Object.keys(imports).find((k) => k.endsWith('api/client'))!]: { api: { get: async () => (calls++, { accounts: [], stale_after_secs: 600 }) } },
    }, { document });
    return { m, tick: () => run!(new AbortController().signal), calls: () => calls };
  };
  const lim = make('../src/lib/stores/usageLimits.svelte.ts', { '../api/client': 0, '../loadError': { loadErrorText: String }, '../poll': 0 });
  const mac = make('../src/modules/settings/machineStatus.svelte.ts', { '../../lib/api/client': 0, '../../lib/loadError': { loadErrorText: String }, '../../lib/poll': 0, svelte: { untrack: (f: () => unknown) => f() } });
  lim.m.limits.mount(); mac.m.machineStatus.watch();
  for (const p of [lim, mac]) {
    await p.tick();
    assert.equal(p.calls(), 1, 'the first read always goes out');
    await p.tick(); await p.tick();
    assert.equal(p.calls(), 1, 'unfocused ticks inside the slow window read nothing');
  }
  focused = true;
  for (const p of [lim, mac]) { await p.tick(); assert.equal(p.calls(), 2, 'focused: the normal cadence'); }
});
