import test from 'node:test';
import assert from 'node:assert/strict';
import { blockScriptNetwork, SCRIPT_BLOCKED_GLOBALS } from '../src/lib/api/scripts.ts';

// A stand-in for a Worker's global scope: `fetch` inherited from a prototype
// (as on WorkerGlobalScope), constructors as own properties, and a navigator.
class FakeScopeProto {
  fetch(): string { return 'network'; }
}

test('every network entry point throws after the lock, inherited ones included', () => {
  const scope = Object.create(FakeScopeProto.prototype) as Record<string, unknown>;
  scope.WebSocket = function WebSocket() { return 'socket'; };
  scope.XMLHttpRequest = function XMLHttpRequest() { return 'xhr'; };
  scope.navigator = { sendBeacon: () => true };
  blockScriptNetwork(scope);
  for (const name of SCRIPT_BLOCKED_GLOBALS) {
    assert.throws(() => (scope[name] as () => unknown)(), /not available in API-client scripts/, name);
  }
  const proto = Object.getPrototypeOf(scope) as { fetch: () => unknown };
  assert.throws(() => proto.fetch(), /fetch is not available/);
  assert.throws(() => (scope.navigator as { sendBeacon: () => unknown }).sendBeacon(), /sendBeacon/);
  // A script can't put the original back.
  assert.throws(() => { (scope as { fetch: unknown }).fetch = () => 'again'; }, TypeError);
});

test('locking twice is harmless', () => {
  const scope: Record<string, unknown> = {};
  blockScriptNetwork(scope);
  blockScriptNetwork(scope);
  assert.throws(() => (scope.fetch as () => unknown)(), /fetch is not available/);
});
