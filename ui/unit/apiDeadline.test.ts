// S13-05 / S13-10: reads carry a default deadline (a stalled daemon no longer
// pends forever), writes and the long lane do not, and the raw helpers share
// request()'s 401 → login handling.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

type Init = RequestInit & { signal?: AbortSignal };

function client(fetchImpl: (url: string, init: Init) => Promise<Response>, extra: Record<string, unknown> = {}) {
  return loadSource(new URL('../src/lib/api/client.ts', import.meta.url), {
    '../stores/serviceHealth.svelte': { serviceHealth: { report() {} } },
    './lane': loadSource(new URL('../src/lib/api/lane.ts', import.meta.url), {}),
  }, {
    location: { port: '7700', origin: 'http://localhost:7700' },
    fetch: fetchImpl,
    // Compress the 20 s read deadline.
    setTimeout: (fn: () => void, ms: number) => setTimeout(fn, ms === 20_000 ? 10 : ms),
    ...extra,
  });
}

/** A fetch that never answers until its signal aborts. */
const stall = (seen: Init[]) => (_url: string, init: Init) => {
  seen.push(init);
  return new Promise<Response>((_, reject) => {
    init.signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError')));
  });
};

test('a stalled interactive GET rejects with a timeout ApiError', async () => {
  const seen: Init[] = [];
  const { api } = client(stall(seen));
  await assert.rejects(api.get('/workspaces'), (e: any) => e.status === 0 && e.code === 'timeout');
  assert.equal(seen[0].signal?.aborted, true);
});

test('a caller abort stays an AbortError (not reported as a timeout)', async () => {
  const { api, isAbortError } = client(stall([]));
  const ctl = new AbortController();
  const p = api.get('/workspaces', ctl.signal);
  ctl.abort();
  await assert.rejects(p, (e: unknown) => isAbortError(e));
});

test('writes and the long lane carry no default deadline', async () => {
  const seen: Init[] = [];
  const { api } = client(stall(seen));
  void api.post('/sessions', {});
  void api.long.get('/repos/r1/fetch');
  await new Promise((r) => setTimeout(r, 40));
  assert.equal(seen.length, 2);
  assert.ok(seen.every((i) => !i.signal || !i.signal.aborted), 'still pending after the read deadline');
});

test('authedText: a 401 dispatches the unauthorized event like api.*', async () => {
  const events: string[] = [];
  const window = { dispatchEvent: (e: Event) => { events.push(e.type); return true; } };
  const { authedText } = client(
    async () => new Response(JSON.stringify({ code: 'unauthorized', message: 'expired' }), { status: 401 }),
    {
      window,
      CustomEvent: class extends Event { detail: unknown; constructor(t: string, o?: { detail?: unknown }) { super(t); this.detail = o?.detail; } },
      localStorage: { getItem: (k: string) => (k === 'otto_token' ? 'tok' : null), setItem() {}, removeItem() {} },
    },
  );
  await assert.rejects(authedText('/reports/x.md'), (e: any) => e.status === 401 && e.message === 'expired');
  assert.deepEqual(events, ['otto:unauthorized']);
});
