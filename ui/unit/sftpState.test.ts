import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
test('failed first listing retains attempted state until explicit retry', async () => {
  let calls = 0;
  const { sftp } = loadSource(new URL('../src/lib/stores/sftp.svelte.ts', import.meta.url), {
    '../api/client': { api: { get: async () => { calls++; throw new Error('authentication failed'); } } },
  });
  await sftp.list('ssh');
  assert.equal(sftp.state('ssh').attempted, true);
  assert.equal(sftp.state('ssh').loading, false);
  assert.match(sftp.state('ssh').error, /authentication failed/);
  assert.equal(calls, 1);
});
test('transfer polling waits through finalizing and returns completed bytes', async () => {
  let polls = 0;
  const base = { id: 'transfer', direction: 'download', local_path: '/fixture/out', remote_path: '/fixture/source', bytes: 12, total_bytes: 12, elapsed_secs: 1, error: null };
  const { sftp } = loadSource(new URL('../src/lib/stores/sftp.svelte.ts', import.meta.url), {
    '../api/client': { api: {
      post: async () => ({ ...base, status: 'running' }),
      get: async () => [{ ...base, status: ++polls === 1 ? 'finalizing' : 'completed' }],
    } },
  }, { setTimeout: (fn: () => void) => { queueMicrotask(fn); return 0; } });
  const result = await sftp.download('ssh', '/fixture/source', '/fixture/out');
  assert.equal(polls, 2);
  assert.equal(result.bytes, 12);
  assert.equal(result.local_path, '/fixture/out');
});
