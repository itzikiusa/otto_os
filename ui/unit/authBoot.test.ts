// Boot: only a 401 on /auth/me means "sign in again". A timeout / 5xx used to
// drop a valid token holder on the login screen; now the app stays offline
// (and polls) — or, re-booting a running app, stays up.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

class ApiError extends Error {
  status: number;
  constructor(status: number) { super(`HTTP ${status}`); this.status = status; }
}

function boot(meStatus: number) {
  const tokens: (string | null)[] = [];
  const api = {
    get: async (path: string) => {
      if (path === '/meta') return { needs_onboarding: false };
      if (path === '/auth/me' || path === '/auth/capabilities') throw new ApiError(meStatus);
      return {};
    },
  };
  const { auth } = loadSource(new URL('../src/lib/stores/auth.svelte.ts', import.meta.url), {
    '../api/client': {
      api, ApiError, UNAUTHORIZED_EVENT: 'otto:unauthorized', setAltLoopbackBase() {},
      getToken: () => 'tok', setToken: (t: string | null) => void tokens.push(t),
    },
    '../storage': { lsGet: () => null, lsSet() {}, lsRemove() {} },
  });
  return { auth, tokens };
}

test('a 401 on /auth/me clears the token and asks for login', async () => {
  const h = boot(401);
  await h.auth.boot();
  assert.equal(h.auth.phase, 'login');
  assert.deepEqual(h.tokens, [null]);
});

test('a 5xx / transport failure keeps the token and stays offline (polled)', async () => {
  const h = boot(503);
  await h.auth.boot();
  assert.equal(h.auth.phase, 'offline');
  assert.deepEqual(h.tokens, [], 'token kept');
});

test('a failed re-boot of a running app keeps the shell up', async () => {
  const h = boot(503);
  h.auth.phase = 'ready';
  await h.auth.boot(true);
  assert.equal(h.auth.phase, 'ready');
});

test('a stalled /meta moves boot to offline instead of spinning forever (S13-05)', async () => {
  let aborted = false;
  const api = {
    get: (path: string, signal?: AbortSignal) => {
      if (path !== '/meta') return Promise.resolve({});
      signal?.addEventListener('abort', () => { aborted = true; });
      return new Promise(() => {}); // accepted, never answers
    },
  };
  const { auth } = loadSource(new URL('../src/lib/stores/auth.svelte.ts', import.meta.url), {
    '../api/client': {
      api, ApiError, UNAUTHORIZED_EVENT: 'otto:unauthorized', setAltLoopbackBase() {},
      getToken: () => null, setToken() {},
    },
    '../storage': { lsGet: () => null, lsSet() {}, lsRemove() {} },
  }, {
    // Compress the 5 s deadline so the test is instant.
    setTimeout: (fn: () => void, ms: number) => setTimeout(fn, ms === 5_000 ? 5 : ms),
  });
  await auth.boot();
  assert.equal(auth.phase, 'offline');
  assert.equal(aborted, true, 'the stalled request is aborted, not leaked');
});

test('a stalled /meta on an in-place re-boot keeps the running shell up (S13-304)', async () => {
  const api = {
    get: (path: string) => (path === '/meta' ? new Promise(() => {}) : Promise.resolve({})),
  };
  const deadlines: number[] = [];
  const { auth } = loadSource(new URL('../src/lib/stores/auth.svelte.ts', import.meta.url), {
    '../api/client': {
      api, ApiError, UNAUTHORIZED_EVENT: 'otto:unauthorized', setAltLoopbackBase() {},
      getToken: () => null, setToken() {},
    },
    '../storage': { lsGet: () => null, lsSet() {}, lsRemove() {} },
  }, {
    setTimeout: (fn: () => void, ms: number) => { deadlines.push(ms); return setTimeout(fn, 5); },
  });
  auth.phase = 'ready';
  await auth.boot(true);
  assert.equal(auth.phase, 'ready', 'never unmounts the shell to the offline screen');
  assert.ok(deadlines.includes(10_000), 'a re-boot gets the longer /meta deadline');
});

test('impersonation start lives in this tab\'s sessionStorage, not shared localStorage (S13-07)', async () => {
  const ss = new Map<string, string>();
  const lsRemoved: string[] = [];
  let token: string | null = 'admin';
  const api = {
    post: async () => ({ token: 'imp' }),
    get: async (path: string) => (path === '/auth/me' ? { user: { id: 'x' }, real_user: { id: 'root' } } : { capabilities: {} }),
  };
  const mod = loadSource(new URL('../src/lib/stores/auth.svelte.ts', import.meta.url), {
    '../api/client': { api, ApiError, UNAUTHORIZED_EVENT: 'u', setAltLoopbackBase() {}, getToken: () => token, setToken: (t: string | null) => { token = t; } },
    '../storage': {
      lsGet: () => null, lsSet() {}, lsRemove: (k: string) => void lsRemoved.push(k),
      ssGet: (k: string) => ss.get(k) ?? null, ssSet: (k: string, v: string) => void ss.set(k, v), ssRemove: (k: string) => void ss.delete(k),
    },
  });
  assert.ok(lsRemoved.includes('otto_imp_start_ms'), 'purges the old shared copy');
  assert.equal(mod.impersonationStartedMs(), null);
  const before = Date.now();
  await mod.auth.impersonate('x');
  const started = mod.impersonationStartedMs();
  assert.ok(started !== null && started >= before);
  assert.equal(ss.get('otto_admin_token'), 'admin');
});
