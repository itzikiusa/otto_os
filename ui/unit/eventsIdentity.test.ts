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
  assert.deepEqual(h.log, [], 'not a resync of the old identity\'s stores');
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
