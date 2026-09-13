import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function client(response: Response) {
  return loadSource(new URL('../src/lib/api/client.ts', import.meta.url), {
    '../stores/serviceHealth.svelte': { serviceHealth: { report() {} } },
  }, { location: { port: '7700', origin: 'http://localhost:7700' }, fetch: async () => response }).api;
}

test('empty error responses reject instead of reporting a successful mutation', async () => {
  for (const status of [403, 500]) {
    await assert.rejects(client(new Response(null, { status, headers: { 'content-length': '0' } })).post('/test'),
      (e: any) => e.status === status);
  }
});

test('accepted JSON responses retain the updated object', async () => {
  const result = await client(new Response('{"id":"accepted"}', { status: 202 })).post('/test');
  assert.equal(result.id, 'accepted');
});

test('genuinely empty accepted and no-content responses resolve without JSON parsing', async () => {
  for (const status of [202, 204]) {
    assert.equal(await client(new Response(null, { status })).post('/test'), undefined);
  }
});
