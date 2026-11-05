import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deepLinkErrorText, resolveDeepLink, type DeepLinkFetchers } from '../src/lib/deepLink.ts';

class HttpErr extends Error {
  status: number;
  constructor(status: number) {
    super(`HTTP ${status}`);
    this.status = status;
  }
}

const sessions: Record<string, string> = { S1: 'ws-a', S2: 'ws-b', SCR: 'scratch' };
const threads: Record<string, string> = { T1: 'ws-b' };

const fetchers = (over: Partial<DeepLinkFetchers> = {}): DeepLinkFetchers => ({
  session: async (id) => {
    if (id === 'DENIED') throw new HttpErr(403);
    if (id in sessions) return { workspace_id: sessions[id] };
    throw new HttpErr(404);
  },
  runtime: async (id) => (id in threads ? { workspace_id: threads[id] } : null),
  ...over,
});

test('a session in the current workspace needs no switch', async () => {
  assert.deepEqual(await resolveDeepLink('S1', 'ws-a', 'scratch', fetchers()), {
    kind: 'ok',
    workspaceId: 'ws-a',
    switchTo: null,
  });
});

test('a session in another workspace switches there (the phone-QR bug)', async () => {
  assert.deepEqual(await resolveDeepLink('S2', 'ws-a', 'scratch', fetchers()), {
    kind: 'ok',
    workspaceId: 'ws-b',
    switchTo: 'ws-b',
  });
  // No workspace selected yet (a fresh phone boot) still resolves a switch.
  assert.equal((await resolveDeepLink('S2', null, 'scratch', fetchers())).kind, 'ok');
});

test('scratch sessions are always loaded — no switch', async () => {
  const r = await resolveDeepLink('SCR', 'ws-a', 'scratch', fetchers());
  assert.deepEqual(r, { kind: 'ok', workspaceId: 'scratch', switchTo: null });
});

test('a runtime thread falls back to the runtime lookup', async () => {
  assert.deepEqual(await resolveDeepLink('T1', 'ws-a', 'scratch', fetchers()), {
    kind: 'ok',
    workspaceId: 'ws-b',
    switchTo: 'ws-b',
  });
  // …but not for a plain session link (no runtime fetcher).
  const noRuntime = fetchers({ runtime: undefined });
  assert.deepEqual(await resolveDeepLink('T1', 'ws-a', 'scratch', noRuntime), { kind: 'missing' });
});

test('unreadable and missing items explain themselves', async () => {
  const denied = await resolveDeepLink('DENIED', 'ws-a', 'scratch', fetchers());
  assert.deepEqual(denied, { kind: 'forbidden' });
  assert.match(deepLinkErrorText(denied, 'session') ?? '', /don’t have access/);
  const gone = await resolveDeepLink('NOPE', 'ws-a', 'scratch', fetchers());
  assert.deepEqual(gone, { kind: 'missing' });
  assert.match(deepLinkErrorText(gone, 'thread') ?? '', /thread doesn’t exist/);
  const broken = await resolveDeepLink('X', 'ws-a', 'scratch', {
    session: async () => {
      throw new Error('network down');
    },
  });
  assert.deepEqual(broken, { kind: 'error', message: 'network down' });
  assert.equal(deepLinkErrorText({ kind: 'ok', workspaceId: 'w', switchTo: null }, 'session'), null);
});
