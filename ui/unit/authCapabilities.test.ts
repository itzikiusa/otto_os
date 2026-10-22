// error-handling-1-05: a failed /auth/capabilities read used to leave a
// non-root user on a silently all-deny shell until a manual reload. Now it is
// flagged (the shell shows "Couldn't load your permissions — Retry"), retried
// with backoff, and retried at once on an events-socket reconnect.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

class ApiError extends Error {
  status: number;
  constructor(status: number) { super(`HTTP ${status}`); this.status = status; }
}

const MEMBER = { id: 'u1', username: 'm', display_name: 'M', is_root: false };

function boot(capsFailures: number) {
  let token: string | null = 'tok';
  let capsCalls = 0;
  const timers: { fn: () => void; ms: number }[] = [];
  const api = {
    get: async (path: string) => {
      if (path === '/meta') return { needs_onboarding: false };
      if (path === '/auth/me') return { user: MEMBER, real_user: MEMBER };
      if (path === '/auth/capabilities') {
        capsCalls += 1;
        if (capsCalls <= capsFailures) throw new ApiError(503);
        return { capabilities: { agents: 'edit' } };
      }
      return {};
    },
  };
  const { auth, CAPABILITIES_RETRY_MS } = loadSource(new URL('../src/lib/stores/auth.svelte.ts', import.meta.url), {
    '../api/client': {
      api, ApiError, UNAUTHORIZED_EVENT: 'otto:unauthorized', setAltLoopbackBase() {},
      getToken: () => token, setToken: (t: string | null) => { token = t; },
      getImpersonationToken: () => null, setImpersonationToken() {},
    },
    '../storage': { lsGet: () => null, lsSet() {}, lsRemove() {}, ssGet: () => null, ssSet() {}, ssRemove() {} },
  }, {
    // Capture the timers instead of waiting on them (/meta's deadline is
    // never run — /meta answers at once).
    setTimeout: (fn: () => void, ms: number) => { timers.push({ fn, ms }); return timers.length; },
    clearTimeout: () => {},
  });
  return { auth, timers, CAPABILITIES_RETRY_MS, calls: () => capsCalls, signOut: () => { token = null; } };
}

// timers[0] is boot's /meta deadline; every later one is a capabilities retry.
const backoff = (h: ReturnType<typeof boot>) => h.timers.slice(1);
const flush = () => new Promise((r) => setImmediate(r));

test('a failed capabilities read is flagged, not a silent all-deny', async () => {
  const h = boot(1);
  await h.auth.boot();
  assert.equal(h.auth.phase, 'ready');
  assert.equal(h.auth.capabilitiesError, true);
  assert.equal(h.auth.can('agents', 'view'), false);
  assert.equal(backoff(h)[0]?.ms, h.CAPABILITIES_RETRY_MS[0], 'a retry is scheduled');
});

test('the backoff retry loads the real map and clears the flag', async () => {
  const h = boot(1);
  await h.auth.boot();
  backoff(h)[0].fn();
  await flush();
  assert.equal(h.auth.capabilitiesError, false);
  assert.equal(h.auth.can('agents', 'edit'), true, 'the member gets their real access back');
});

test('repeated failures back off further each time', async () => {
  const h = boot(3);
  await h.auth.boot();
  backoff(h).at(-1)!.fn();
  await flush();
  backoff(h).at(-1)!.fn();
  await flush();
  assert.deepEqual(backoff(h).map((t) => t.ms), [...h.CAPABILITIES_RETRY_MS.slice(0, 3)]);
  assert.equal(h.auth.capabilitiesError, true);
});

test('a reconnect retries at once; a healthy store does not refetch', async () => {
  const h = boot(1);
  await h.auth.boot();
  h.auth.noteConnected();
  await flush();
  assert.equal(h.auth.capabilitiesError, false);
  assert.equal(h.auth.can('agents', 'edit'), true);
  const calls = h.calls();
  h.auth.noteConnected();
  await flush();
  assert.equal(h.calls(), calls, 'no refetch without a failure');
});

test('signing out drops the flag and a pending retry does nothing', async () => {
  const h = boot(5);
  await h.auth.boot();
  await h.auth.logout();
  h.signOut();
  assert.equal(h.auth.capabilitiesError, false);
  const calls = h.calls();
  for (const t of backoff(h)) t.fn();
  await flush();
  assert.equal(h.calls(), calls, 'no capabilities read without a signed-in user');
});

// ux-design-2-07: the strip can be dismissed, and a dismiss holds only until
// the NEXT failed read — a daemon that keeps failing re-raises it.
test('a dismissed permissions notice comes back on the next failed retry', async () => {
  const h = boot(3);
  await h.auth.boot();
  assert.equal(h.auth.capabilitiesNoticeDismissed, false);
  h.auth.dismissCapabilitiesNotice();
  assert.equal(h.auth.capabilitiesNoticeDismissed, true);
  assert.equal(h.auth.capabilitiesError, true, 'dismissing hides the strip, not the failure');
  backoff(h).at(-1)!.fn();
  await flush();
  assert.equal(h.auth.capabilitiesError, true);
  assert.equal(h.auth.capabilitiesNoticeDismissed, false, 'the next failure re-raises it');
});
