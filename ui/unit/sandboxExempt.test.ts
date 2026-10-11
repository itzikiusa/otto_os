import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  exemptCandidates,
  readExempt,
  sameExempt,
  sandboxBody,
} from '../src/modules/settings/sandboxExempt.ts';

// Sweep 2026-10-11 prefs S-1: `exempt_providers` had no UI — the only way to
// let a provider run unconfined was hand-editing PUT /settings.

test('every provider can be exempted: registered, custom and already-exempt ones', () => {
  assert.deepEqual(
    exemptCandidates(['codex', 'claude'], { opencode: { cmd: 'opencode' } }, ['gone-cli']),
    ['claude', 'codex', 'gone-cli', 'opencode'],
  );
  assert.deepEqual(exemptCandidates([], null, []), []);
  assert.deepEqual(exemptCandidates(['claude'], ['not', 'an', 'object'], []), ['claude']);
});

test('the exempt list round-trips and keeps the keys the form does not edit', () => {
  const stored = { enabled: true, network: 'full' as const, providers: ['claude'], exempt_providers: ['opencode'] };
  const exempt = readExempt(stored);
  assert.deepEqual(exempt, ['opencode']);
  const body = sandboxBody(stored, true, 'loopback', [...exempt, 'agy']);
  assert.deepEqual(body, {
    enabled: true,
    network: 'loopback',
    providers: ['claude'],
    exempt_providers: ['agy', 'opencode'],
  });
  assert.deepEqual(readExempt(body), ['agy', 'opencode']);
  // Un-ticking every provider saves an explicit empty list (not the old one).
  assert.deepEqual(sandboxBody(stored, true, 'full', []).exempt_providers, []);
});

test('junk in the stored list is ignored and order does not make the form dirty', () => {
  assert.deepEqual(readExempt({ exempt_providers: ['b', 3 as unknown as string, 'a', 'b'] }), ['a', 'b']);
  assert.deepEqual(readExempt(undefined), []);
  assert.equal(sameExempt(['a', 'b'], ['b', 'a']), true);
  assert.equal(sameExempt(['a'], ['a', 'b']), false);
});
