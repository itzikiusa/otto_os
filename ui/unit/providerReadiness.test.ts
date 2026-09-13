import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

test('readiness follows registry tool detection for built-in and custom providers', () => {
  const providers = loadSource(new URL('../src/lib/providers.ts', import.meta.url), {
    './stores/auth.svelte': { auth: { meta: { providers: ['claude', 'grok', 'shell'], tools: [
      { name: 'claude', found: false }, { name: 'grok', found: true, version: '1.0' },
    ] } } },
  });
  assert.equal(providers.providerReadiness('claude').available, false);
  assert.equal(providers.providerReadiness('grok').available, true);
  assert.equal(providers.providerReadiness('grok').version, '1.0');
  assert.equal(providers.providerReadiness('shell').available, true);
  assert.equal(providers.providerReadiness('absent').available, false);
});

test('context-dependent and legacy unchecked providers remain launchable without claiming readiness', () => {
  const providers = loadSource(new URL('../src/lib/providers.ts', import.meta.url), {
    './stores/auth.svelte': { auth: { meta: { providers: ['custom', 'claude'], tools: [] } } },
  });
  for (const name of ['custom', 'claude']) {
    assert.equal(providers.providerReadiness(name).available, true);
    assert.equal(providers.providerReadiness(name).checked, false);
    assert.match(providers.providerReadiness(name).message, /not been checked/);
  }
});
