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
