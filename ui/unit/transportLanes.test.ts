import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// Request lanes (src/lib/api/client.ts + lane.ts + poll.ts): the alias host is
// suspended — never dropped for good — on a network failure; poll ticks ride
// the background lane without opting in; the bg cap holds across documents.

const flush = () => new Promise((r) => setImmediate(r));

function manualTimers() {
  let seq = 0;
  const pending = new Map<number, { fn: () => void; ms: number }>();
  return {
    setTimeout: (fn: () => void, ms: number) => {
      pending.set(++seq, { fn, ms });
      return seq;
    },
    clearTimeout: (id: number) => void pending.delete(id),
    pending,
    fireAll() {
      const due = [...pending.values()];
      pending.clear();
      for (const t of due) t.fn();
      return due.map((t) => t.ms);
    },
  };
}

/** A minimal Web Locks manager shared by several "documents". */
function fakeLocks() {
  const held = new Set<string>();
  const queues = new Map<string, (() => void)[]>();
  const grant = async (name: string, cb: (lock: unknown) => unknown) => {
    held.add(name);
    try {
      return await cb({ name });
    } finally {
      held.delete(name);
      const next = queues.get(name)?.shift();
      next?.();
    }
  };
  return {
    held,
    request(name: string, opts: { ifAvailable?: boolean; signal?: AbortSignal }, cb: (lock: unknown) => unknown) {
      if (!held.has(name)) return grant(name, cb);
      if (opts.ifAvailable) return Promise.resolve(cb(null));
      return new Promise((resolve, reject) => {
        const go = () => {
          opts.signal?.removeEventListener('abort', onAbort);
          grant(name, cb).then(resolve, reject);
        };
        const onAbort = () => {
          const q = queues.get(name) ?? [];
          const i = q.indexOf(go);
          if (i >= 0) q.splice(i, 1);
          reject(new DOMException('aborted', 'AbortError'));
        };
        opts.signal?.addEventListener('abort', onAbort, { once: true });
        if (!queues.has(name)) queues.set(name, []);
        queues.get(name)!.push(go);
      });
    },
  };
}

function loadLane() {
  return loadSource(new URL('../src/lib/api/lane.ts', import.meta.url), {});
}

function loadClient(opts: { lane: Record<string, any>; fetch: (url: string, init?: RequestInit) => Promise<Response>; timers?: ReturnType<typeof manualTimers>; locks?: unknown }) {
  return loadSource(
    new URL('../src/lib/api/client.ts', import.meta.url),
    { '../stores/serviceHealth.svelte': { serviceHealth: { report() {} } }, './lane': opts.lane },
    {
      location: { port: '', protocol: 'tauri:', origin: 'tauri://localhost' },
      fetch: opts.fetch,
      ...(opts.timers ? { setTimeout: opts.timers.setTimeout, clearTimeout: opts.timers.clearTimeout } : {}),
      navigator: opts.locks ? { locks: opts.locks } : {},
    },
  );
}

const ALT = 'http://localhost:7700';
const INT = 'http://127.0.0.1:7700';
const ok = (body = '{}') => new Response(body, { status: 200 });

test('alias: a network failure suspends it, retries the GET on 127.0.0.1, and a health probe re-enables it', async () => {
  const timers = manualTimers();
  const urls: string[] = [];
  let aliasDown = true;
  const client = loadClient({
    lane: loadLane(),
    timers,
    fetch: async (url) => {
      urls.push(url);
      if (url.startsWith(ALT) && aliasDown) throw new TypeError('Load failed');
      if (url.endsWith('/health')) return ok('{"ok":true}');
      return ok('{"v":1}');
    },
  });
  client.setAltLoopbackBase(ALT);
  assert.equal(client.altLoopbackState(), 'active');

  assert.equal(JSON.stringify(await client.api.bg.get('/things')), '{"v":1}');
  assert.deepEqual(urls, [`${ALT}/api/v1/things`, `${INT}/api/v1/things`], 'GET retried on the interactive base');
  assert.equal(client.altLoopbackState(), 'suspended', 'suspended, not dropped');
  assert.equal(client.laneBase('long'), INT, 'every lane falls back while suspended');

  // First probe fails (daemon still down) → backoff doubles.
  assert.deepEqual(timers.fireAll(), [5000]);
  await flush();
  assert.equal(client.altLoopbackState(), 'suspended');
  aliasDown = false;
  assert.deepEqual(timers.fireAll(), [10_000], 'backoff doubled');
  await flush();
  await flush();
  assert.equal(client.altLoopbackState(), 'active', 'probe passed → alias back');
  assert.equal(client.laneBase('bg'), ALT);
});

test('alias: suspend on socket loss, resume probes at once, re-arm clears the suspension', async () => {
  const timers = manualTimers();
  const client = loadClient({ lane: loadLane(), timers, fetch: async () => ok('{"ok":true}') });
  client.setAltLoopbackBase(ALT);
  client.suspendAltLoopback();
  assert.equal(client.altLoopbackState(), 'suspended');
  assert.equal(timers.pending.size, 0, 'no probe until the socket is back');
  client.resumeAltLoopback();
  assert.deepEqual(timers.fireAll(), [0]);
  await flush();
  await flush();
  assert.equal(client.altLoopbackState(), 'active');
  client.suspendAltLoopback(30_000);
  client.setAltLoopbackBase(ALT);
  assert.equal(client.altLoopbackState(), 'active', 're-arm from /meta wins');
  assert.equal(timers.pending.size, 0, 're-arm cancels the pending probe');
  client.setAltLoopbackBase(null);
  assert.equal(client.altLoopbackState(), 'none', 'a daemon that no longer holds the alias drops it');
});

test('a non-GET is never replayed on the other host after a network failure', async () => {
  const urls: string[] = [];
  const client = loadClient({
    lane: loadLane(),
    timers: manualTimers(),
    fetch: async (url) => {
      urls.push(url);
      if (url.startsWith(ALT)) throw new TypeError('Load failed');
      return ok();
    },
  });
  client.setAltLoopbackBase(ALT);
  await assert.rejects(client.api.long.post('/x', {}));
  assert.deepEqual(urls, [`${ALT}/api/v1/x`]);
});

test('lanes: long paths, api.long and an ambient bg scope reach the alias; plain calls stay interactive', async () => {
  const lane = loadLane();
  const urls: string[] = [];
  const client = loadClient({ lane, timers: manualTimers(), fetch: async (url) => (urls.push(url), ok()) });
  client.setAltLoopbackBase(ALT);
  await client.api.get('/sessions');
  await client.api.get('/workspaces/w/api-client/execute');
  await client.api.post('/db/widgets/7/run', {});
  await client.api.get('/usage/summary');
  await client.api.long.post('/connections/c/open', {});
  await lane.inLane('bg', () => client.api.get('/sessions?x=1'));
  const ctl = new AbortController();
  lane.tagSignal(ctl.signal, 'bg');
  await client.api.get('/later', ctl.signal);
  assert.deepEqual(urls, [
    `${INT}/api/v1/sessions`,
    `${ALT}/api/v1/workspaces/w/api-client/execute`,
    `${ALT}/api/v1/db/widgets/7/run`,
    `${ALT}/api/v1/usage/summary`,
    `${ALT}/api/v1/connections/c/open`,
    `${ALT}/api/v1/sessions?x=1`,
    `${ALT}/api/v1/later`,
  ]);
});

test('bg cap holds ACROSS documents (Web Locks): 2 per document, 3 app-wide', async () => {
  const locks = fakeLocks();
  const pending: (() => void)[] = [];
  let inFlight = 0;
  let peak = 0;
  const fetch = () =>
    new Promise<Response>((resolve) => {
      inFlight++;
      peak = Math.max(peak, inFlight);
      pending.push(() => {
        inFlight--;
        resolve(ok());
      });
    });
  const docs = [0, 1, 2].map(() => loadClient({ lane: loadLane(), fetch, locks }));
  const all = docs.flatMap((d) => [d.api.bg.get('/a'), d.api.bg.get('/b'), d.api.bg.get('/c')]);
  for (let i = 0; i < 10; i++) await flush();
  assert.equal(inFlight, 3, 'three documents × BG_MAX = 6, but only 3 bg sockets app-wide');
  while (pending.length) {
    pending.shift()!();
    for (let i = 0; i < 10; i++) await flush();
    assert.ok(inFlight <= 3);
  }
  await Promise.all(all);
  assert.equal(peak, 3);
  assert.equal(locks.held.size, 0, 'every slot released');
});

test('poll: the first run and a manual now() are interactive; cadence and event ticks are background', async () => {
  const lane = loadLane();
  const timers = manualTimers();
  const poll = loadSource(new URL('../src/lib/poll.ts', import.meta.url), { './api/lane': lane }, {
    setTimeout: timers.setTimeout,
    clearTimeout: timers.clearTimeout,
  });
  const seen: string[] = [];
  const later: string[] = [];
  const p = poll.pollWhileVisible(
    async (signal: AbortSignal) => {
      seen.push(lane.inheritedLane() ?? 'none');
      await flush();
      later.push(lane.inheritedLane(signal) ?? 'none');
    },
    { ms: 1000, jitter: 0 },
  );
  await flush();
  await flush();
  timers.fireAll();
  await flush();
  await flush();
  p.now();
  await flush();
  await flush();
  p.now({ background: true });
  await flush();
  await flush();
  assert.deepEqual(seen, ['int', 'bg', 'int', 'bg']);
  assert.deepEqual(later, ['int', 'bg', 'int', 'bg'], 'after an await the tagged signal still carries the lane');
  p.stop();
});

test('createLimiter: a shared gate across independent callers, abortable while queued', async () => {
  const poll = loadSource(new URL('../src/lib/poll.ts', import.meta.url), { './api/lane': loadLane() });
  const gate = poll.createLimiter(2);
  let active = 0;
  let peak = 0;
  const releases: (() => void)[] = [];
  const task = () =>
    new Promise<void>((resolve) => {
      active++;
      peak = Math.max(peak, active);
      releases.push(() => {
        active--;
        resolve();
      });
    });
  const done = [gate(task), gate(task), gate(task)];
  const ctl = new AbortController();
  const aborted = gate(task, ctl.signal);
  await flush();
  assert.equal(active, 2);
  ctl.abort();
  await assert.rejects(aborted, (e: any) => e.name === 'AbortError');
  while (releases.length) {
    releases.shift()!();
    await flush();
  }
  await Promise.all(done);
  assert.equal(peak, 2, 'never more than the limit');
  assert.equal(active, 0, 'the aborted waiter never ran');
});
