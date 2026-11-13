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

test('a CLI older than the version Otto drives is not ready, and says why', () => {
  const providers = loadSource(new URL('../src/lib/providers.ts', import.meta.url), {
    './stores/auth.svelte': { auth: { meta: { providers: ['pi', 'claude'], tools: [
      { name: 'pi', found: true, version: '0.65.0' }, { name: 'claude', found: true, version: '2.1.323 (Claude Code)' },
    ] } } },
  });
  const pi = providers.providerReadiness('pi');
  assert.equal(pi.available, false);
  assert.match(pi.message, /0\.65\.0 is older than 0\.75\.0/);
  assert.equal(providers.providerReadiness('claude').available, true);
  assert.equal(providers.versionBelow('1.5.0', '0.75.0'), false);
  assert.equal(providers.versionBelow('v0.74', '0.75.0'), true);
  assert.equal(providers.versionBelow(null, '0.75.0'), false);
});
