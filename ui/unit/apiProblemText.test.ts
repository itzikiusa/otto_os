import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// problemOf: how a non-OK daemon response becomes the ApiError a toast shows.
function client(response: Response) {
  return loadSource(new URL('../src/lib/api/client.ts', import.meta.url), {
    '../stores/serviceHealth.svelte': { serviceHealth: { report() {} } },
    './lane': loadSource(new URL('../src/lib/api/lane.ts', import.meta.url), {}),
  }, { location: { port: '7700', origin: 'http://localhost:7700' }, fetch: async () => response });
}

test('a JSON Problem body is the error', async () => {
  const resp = new Response('{"code":"conflict","message":"Branch is checked out"}', { status: 409, statusText: 'Conflict' });
  await assert.rejects(client(resp).api.post('/x', {}), (e: any) => e.status === 409 && e.code === 'conflict' && e.message === 'Branch is checked out');
});

test('a plain-text extractor rejection keeps its reason instead of the status phrase', async () => {
  const body = 'Failed to deserialize the JSON body into the target type: unknown variant `x`';
  const resp = new Response(`${body}\n`, { status: 422, statusText: 'Unprocessable Entity', headers: { 'content-type': 'text/plain' } });
  await assert.rejects(client(resp).api.post('/x', {}), (e: any) => e.status === 422 && e.code === 'invalid' && e.message === body);
});

test('a long text body is capped; an HTML page or an empty body falls back to statusText', async () => {
  const long = 'x'.repeat(1000);
  await assert.rejects(client(new Response(long, { status: 400 })).api.post('/x', {}), (e: any) => e.message === `${'x'.repeat(300)}…`);
  await assert.rejects(client(new Response('<!DOCTYPE html><html>…</html>', { status: 502, statusText: 'Bad Gateway' })).api.get('/x'),
    (e: any) => e.message === 'Bad Gateway' && e.code === 'internal');
  await assert.rejects(client(new Response('', { status: 503, statusText: 'Service Unavailable' })).api.get('/x'), (e: any) => e.message === 'Service Unavailable');
});

test('authedResponse sends the bearer token and throws the typed error', async () => {
  const seen: { headers: Headers | null } = { headers: null };
  const mod = loadSource(new URL('../src/lib/api/client.ts', import.meta.url), {
    '../stores/serviceHealth.svelte': { serviceHealth: { report() {} } },
    './lane': loadSource(new URL('../src/lib/api/lane.ts', import.meta.url), {}),
  }, {
    location: { port: '7700', origin: 'http://localhost:7700' },
    fetch: async (_url: string, init: RequestInit) => {
      seen.headers = new Headers(init.headers);
      return new Response('nope', { status: 403, statusText: 'Forbidden' });
    },
  });
  mod.setToken('tok');
  await assert.rejects(mod.authedResponse('/files/a', { method: 'PUT', body: 'x', contentType: 'text/plain' }), (e: any) => e.status === 403 && e.message === 'nope');
  assert.equal(seen.headers?.get('content-type'), 'text/plain');
  assert.equal(seen.headers?.get('authorization'), 'Bearer tok');
});
