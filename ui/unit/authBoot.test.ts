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
      getImpersonationToken: () => null, setImpersonationToken() {},
    },
    '../storage': { lsGet: () => null, lsSet() {}, lsRemove() {}, ssGet: () => null, ssSet() {}, ssRemove() {} },
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
      getImpersonationToken: () => null, setImpersonationToken() {},
    },
    '../storage': { lsGet: () => null, lsSet() {}, lsRemove() {}, ssGet: () => null, ssSet() {}, ssRemove() {} },
  }, {
    // Compress the 5 s deadline so the test is instant.
    setTimeout: (fn: () => void, ms: number) => setTimeout(fn, ms === 5_000 ? 5 : ms),
  });
  await auth.boot();
  assert.equal(auth.phase, 'offline');
  assert.equal(aborted, true, 'the stalled request is aborted, not leaked');
});

/** A two-layer token model like `api/client`: a SHARED sign-in token
 *  (localStorage) under a per-tab impersonation overlay (sessionStorage). */
function impHarness(api: Record<string, unknown>) {
  const ss = new Map<string, string>();
  const lsRemoved: string[] = [];
  const shared = { token: 'admin' as string | null, writes: [] as (string | null)[] };
  const tab = { imp: null as string | null };
  const mod = loadSource(new URL('../src/lib/stores/auth.svelte.ts', import.meta.url), {
    '../api/client': {
      api, ApiError, UNAUTHORIZED_EVENT: 'u', setAltLoopbackBase() {},
      getToken: () => tab.imp ?? shared.token,
      setToken: (t: string | null) => { shared.token = t; shared.writes.push(t); },
      getImpersonationToken: () => tab.imp,
      setImpersonationToken: (t: string | null) => { tab.imp = t; },
    },
    '../storage': {
      lsGet: () => null, lsSet() {}, lsRemove: (k: string) => void lsRemoved.push(k),
      ssGet: (k: string) => ss.get(k) ?? null, ssSet: (k: string, v: string) => void ss.set(k, v), ssRemove: (k: string) => void ss.delete(k),
    },
  });
  return { ...mod, ss, lsRemoved, shared, tab };
}

const me = (id: string, real = 'root') => ({ user: { id }, real_user: { id: real } });

test('impersonation start lives in this tab\'s sessionStorage, not shared localStorage (S13-07)', async () => {
  const api = {
    post: async () => ({ token: 'imp' }),
    get: async (path: string) => (path === '/auth/me' ? me('x') : { capabilities: {} }),
  };
  const h = impHarness(api);
  assert.ok(h.lsRemoved.includes('otto_imp_start_ms'), 'purges the old shared copy');
  assert.ok(h.lsRemoved.includes('otto_admin_token'), 'purges a legacy admin-token copy');
  assert.equal(h.impersonationStartedMs(), null);
  const before = Date.now();
  await h.auth.impersonate('x');
  const started = h.impersonationStartedMs();
  assert.ok(started !== null && started >= before);
});

test('the impersonation bearer stays in this tab — the shared token is never overwritten (S13-303)', async () => {
  const api = {
    post: async (path: string) => (path.startsWith('/admin/impersonate/') && path !== '/admin/impersonate/stop' ? { token: 'imp' } : {}),
    get: async (path: string) => (path === '/auth/me' ? me('x') : { capabilities: {} }),
  };
  const h = impHarness(api);
  await h.auth.impersonate('x');
  assert.equal(h.tab.imp, 'imp', 'this tab now sends the impersonation bearer');
  assert.equal(h.shared.token, 'admin', 'other windows keep the admin token');
  assert.deepEqual(h.shared.writes, [], 'shared otto_token is never written');
  assert.equal(h.ss.get('otto_admin_token'), undefined, 'the admin bearer is not copied anywhere');
  await h.auth.stopImpersonating();
  assert.equal(h.tab.imp, null);
  assert.equal(h.shared.token, 'admin');
  assert.equal(h.impersonationStartedMs(), null);
});

test('an expired impersonation (4401 → 401 path) falls back to the admin session (S13-302)', async () => {
  let impExpired = false;
  const api = {
    post: async () => ({ token: 'imp' }),
    get: async (path: string) => {
      const bearer = h.tab.imp ?? h.shared.token;
      if (path === '/meta') return { needs_onboarding: false };
      if (bearer === 'imp' && impExpired) throw new ApiError(401);
      if (path === '/auth/me') return bearer === 'imp' ? me('x') : me('root');
      return { capabilities: {} };
    },
  };
  const h = impHarness(api);
  h.auth.phase = 'ready';
  await h.auth.impersonate('x');
  assert.equal(h.auth.me.id, 'x');
  impExpired = true;
  await h.auth.handleUnauthorized('imp');
  assert.equal(h.tab.imp, null, 'the dead overlay is dropped');
  assert.equal(h.shared.token, 'admin', 'the admin token is kept');
  assert.equal(h.auth.phase, 'ready');
  assert.equal(h.auth.me.id, 'root', 'back as the admin');
});

test('a re-boot whose impersonation token expired lands on the admin, not sign-in (S13-302)', async () => {
  const api = {
    post: async () => ({ token: 'imp' }),
    get: async (path: string) => {
      const bearer = h.tab.imp ?? h.shared.token;
      if (path === '/meta') return { needs_onboarding: false };
      if (bearer === 'imp') throw new ApiError(401);
      return path === '/auth/me' ? me('root') : { capabilities: {} };
    },
  };
  const h = impHarness(api);
  h.tab.imp = 'imp';
  await h.auth.boot(true);
  assert.equal(h.tab.imp, null);
  assert.deepEqual(h.shared.writes, [], 'the admin token is not cleared');
  assert.equal(h.auth.phase, 'ready');
  assert.equal(h.auth.me.id, 'root');
});
