import { test } from 'node:test';
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { loadSource } from './sourceHarness.ts';

function teardownFixture({ staleInitially = false, replaceOnConfigRead = false } = {}) {
  const meta = {
    telemetry: true, pid: 4242, dataDir: '/fixture/otto-e2e-owned', port: '7843',
    command: '/fixture/ottod', startedAt: 'Mon Oct 5 10:00:00 2026',
  };
  let stale = staleInitially;
  const calls = { contexts: 0, gets: [] as string[], puts: [] as string[], signals: [] as unknown[], deletes: [] as unknown[], scans: [] as unknown[], warnings: [] as unknown[] };
  const response = (value: unknown) => ({ ok: () => true, status: () => 200, json: async () => value, dispose: async () => {} });
  const reachableApi = {
    async get(url: string) {
      calls.gets.push(url);
      if (url.endsWith('/config')) {
        if (replaceOnConfigRead) stale = true;
        return response({ enabled: true, native_profiling: false });
      }
      return response({ enabled: false, collector_ready: false });
    },
    async put(url: string) { calls.puts.push(url); return response({ enabled: false }); },
    async dispose() {},
  };
  const module = loadSource(new URL('../e2e/global-teardown.ts', import.meta.url), {
    'node:path': { join },
    'node:fs': {
      readFileSync(path: string) {
        if (path.endsWith('daemon.json')) return JSON.stringify(meta);
        if (path.endsWith('state.json')) return JSON.stringify({ origins: [{ localStorage: [{ name: 'otto_token', value: 'still-valid-fixture-token' }] }] });
        throw new Error(`Unexpected fixture read: ${path}`);
      },
      readdirSync: () => [], realpathSync: (path: string) => path,
      rmSync: (...args: unknown[]) => { calls.deletes.push(args); },
    },
    'node:child_process': {
      execFileSync(command: string, args: string[]) {
        if (command === 'ps') return args.at(-1) === 'command=' ? meta.command : stale ? 'Mon Oct 5 10:01:00 2026' : meta.startedAt;
        if (command === 'pgrep') return '';
        throw new Error(`Unexpected fixture command: ${command}`);
      },
      execSync: (...args: unknown[]) => { calls.scans.push(args); return ''; },
    },
    '@playwright/test': { request: { async newContext() { calls.contexts++; return reachableApi; } } },
  }, {
    process: { env: { OTTO_E2E_SLOT: 'ownership-regression' }, cwd: () => '/fixture/ui', kill: (...args: unknown[]) => { calls.signals.push(args); } },
    console: { warn: (...args: unknown[]) => { calls.warnings.push(args); }, log() {} },
  });
  return { calls, teardown: module.default as () => Promise<void> };
}

test('stale daemon identity with a reachable API cannot mutate, signal or delete a fixture', async () => {
  const fixture = teardownFixture({ staleInitially: true });
  await fixture.teardown();
  assert.equal(fixture.calls.contexts, 0, 'ownership must be checked before constructing an API client');
  assert.deepEqual(fixture.calls.gets, []);
  assert.deepEqual(fixture.calls.puts, []);
  assert.deepEqual(fixture.calls.signals, []);
  assert.deepEqual(fixture.calls.deletes, []);
  assert.deepEqual(fixture.calls.scans, []);
  assert.equal(fixture.calls.warnings.length, 1, 'preserving the fixture must be reported');
});

test('replacement during config retrieval cannot receive the disable mutation or lose its data', async () => {
  const fixture = teardownFixture({ replaceOnConfigRead: true });
  await fixture.teardown();
  assert.equal(fixture.calls.contexts, 1);
  assert.equal(fixture.calls.gets.length, 1, 'only the original read may finish');
  assert.deepEqual(fixture.calls.puts, []);
  assert.deepEqual(fixture.calls.signals, []);
  assert.deepEqual(fixture.calls.deletes, []);
  assert.deepEqual(fixture.calls.scans, []);
  assert.equal(fixture.calls.warnings.length, 1);
});
