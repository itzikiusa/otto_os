// /ws/events binds the identity at upgrade: after impersonate / stop / re-login
// (`otto:auth-changed` with a different stored token) the client must drop the
// old socket and connect again — as a FIRST connect for the new identity — and
// stop() must not leave debounced timers behind.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

const SRC = new URL('../src/lib/', import.meta.url);

const inert: any = new Proxy(function () {}, {
  get: (_t, k) => (k === 'then' ? undefined : inert),
  apply: () => undefined,
});

function setup() {
  let token: string | null = 'admin';
  const sockets: any[] = [];
  const log: string[] = [];
  const listeners = new Map<string, (() => void)[]>();
  const target = {
    addEventListener: (t: string, fn: () => void) => listeners.set(t, [...(listeners.get(t) ?? []), fn]),
  };
  const fire = (t: string) => (listeners.get(t) ?? []).forEach((fn) => fn());
  const require = (p: string): unknown => {
    if (p === './api/client') {
      return {
        getToken: () => token,
        wsConnect: () => {
          const s: any = { sent: [], readyState: 1, send: (f: string) => s.sent.push(f), close: () => (s.closed = true) };
          sockets.push(s);
          return s;
        },
        resumeAltLoopback() {},
        suspendAltLoopback() {},
      };
    }
    if (p === './api/lane') return { inLane: (_l: string, fn: () => void) => fn() };
    if (p === './stores/notifications.svelte') {
      return { notifications: { resetForIdentity: () => log.push('notifications-reset'), load: async () => { log.push('notifications-load'); } } };
    }
    if (p === './stores/activity.svelte') {
      return { activity: { reset: () => log.push('activity-reset'), loadSummary: async () => {}, load: async () => {} } };
    }
    if (p === './lazyModule') return { lazyModule: () => ({ peek: () => null }) };
    if (p === './stores/proof.svelte') return { proof: { identityChanged: () => log.push('proof-reset') } };
    if (p === './stores/assistant.svelte') {
      return { assistant: { needsState: 'ready', resync: () => log.push('assistant-resync'), loadNeedsYou: async () => {} } };
    }
    if (p === './stores/workspace.svelte') {
      return { ws: { currentId: 'w', activeSessionId: null, otherWsSessions: [], statusMap: {},
        refreshSessions: async () => {}, refreshActiveWorkflowRuns: async () => {}, refreshOtherSessions: async () => {} } };
    }
    if (p === './live') return { appLive: { setConnected() {}, dispatch() {}, resync: () => log.push('resync') } };
    if (p === './uiCommands') {
      return new Proxy({ handleUiFrame: () => false, helloFrame: () => ({}), presenceFrame: () => ({}) }, {
        get: (t: any, k) => (k in t ? t[k] : () => {}),
      });
    }
    return inert;
  };
  const out = ts.transpileModule(readFileSync(new URL('events.svelte.ts', SRC), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports: Record<string, any> = {};
  const $state = Object.assign((v: unknown) => v, { raw: (v: unknown) => v, snapshot: (v: unknown) => v });
  runInNewContext(out, {
    exports, require, $state, $derived: (v: unknown) => v,
    setTimeout, clearTimeout, Promise, JSON, Object, Math, Date, Set, Map,
    WebSocket: { OPEN: 1 }, window: target, document: target,
  });
  return { events: exports.events, sockets, log, fire, setToken: (t: string | null) => (token = t) };
}

test('a token change reconnects /ws/events as a first connect for the new identity', () => {
  const h = setup();
  h.events.start();
  assert.equal(h.sockets.length, 1);
  h.sockets[0].onopen();
  // Same token (e.g. another window wrote the same value): nothing happens.
  h.fire('otto:auth-changed');
  assert.equal(h.sockets.length, 1);
  h.setToken('impersonated');
  h.fire('otto:auth-changed');
  assert.equal(h.sockets.length, 2, 'a fresh socket with the new token');
  assert.equal(h.sockets[0].closed, true, 'the old identity\'s socket is closed');
  assert.equal(h.sockets[0].onclose, null, 'and cannot schedule a competing reconnect');
  h.sockets[1].onopen();
  // S13-02: the new identity's first open drops the old identity's per-user
  // caches and refetches (the bell, needs-you, activity, proof).
  assert.deepEqual(
    h.log.filter((x) => x !== 'resync'),
    ['notifications-reset', 'activity-reset', 'proof-reset', 'notifications-load', 'assistant-resync'],
  );
  assert.ok(h.log.includes('resync'), 'live views refetch too');
});

test('a plain first connect does not reset identity caches', () => {
  const h = setup();
  h.events.start();
  h.sockets[0].onopen();
  assert.ok(!h.log.includes('notifications-reset'));
});

test('stop() detaches the socket so a quick restart never opens a second one (S13-08)', () => {
  const h = setup();
  h.events.start();
  h.sockets[0].onopen();
  const old = h.sockets[0];
  h.events.stop();
  assert.equal(old.onclose, null);
  assert.equal(old.onmessage, null);
  h.events.start();
  assert.equal(h.sockets.length, 2);
  // The old socket's close arriving late cannot schedule another connect.
  old.onclose?.();
  assert.equal(h.sockets.length, 2);
});

test('a token change mid-handshake still reconnects', () => {
  const h = setup();
  h.events.start();
  assert.equal(h.events.state, 'connecting');
  h.setToken('other');
  h.fire('otto:auth-changed');
  assert.equal(h.sockets.length, 2);
});

test('a stopped client ignores auth changes', () => {
  const h = setup();
  h.events.start();
  h.events.stop();
  h.setToken('other');
  h.fire('otto:auth-changed');
  assert.equal(h.sockets.length, 1);
});
