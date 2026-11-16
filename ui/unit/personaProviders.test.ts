import { test } from 'node:test';
import assert from 'node:assert/strict';
import { inferProviderFamily } from '../src/lib/providerRegistry.ts';
import { loadSource } from './sourceHarness.ts';

const label = (p: string) => ({ claude: 'Claude Code', codex: 'Codex', agy: 'Antigravity' } as Record<string, string>)[p]
  ?? p.charAt(0).toUpperCase() + p.slice(1);
const m = loadSource(new URL('../src/modules/personal-agents/personaProviders.ts', import.meta.url), {
  '../../lib/providers': { harnessLabel: label, providerFamily: (p: string) => inferProviderFamily(p) },
});

test('a personal agent is offered only the CLIs that can load its persona', () => {
  const registered = ['agy', 'claude', 'codex', 'gemini', 'grok', 'opencode', 'pi', 'my-cli'];
  assert.deepEqual(JSON.parse(JSON.stringify(m.personaProviders(registered))), ['agy', 'claude', 'codex', 'grok', 'opencode']);
  assert.equal(m.canCarryPersona('claudework', { provider_defs: { claudework: { cmd: 'claude-work' } } }), true);
  // Empty is the daemon's default, Claude.
  assert.equal(m.canCarryPersona(''), true);
  assert.equal(m.canCarryPersona(undefined), true);
  for (const p of ['gemini', 'pi', 'my-cli']) assert.equal(m.canCarryPersona(p), false, p);
});

test('a new agent never starts on a provider the daemon would refuse', () => {
  assert.equal(m.personaDefault('codex'), 'codex');
  assert.equal(m.personaDefault('gemini'), 'claude');
  // Only a custom Claude-family slug is enabled: start on it, not on the disabled built-in.
  assert.equal(m.personaDefault('gemini', ['claudework', 'gemini']), 'claudework');
});

test('the refusal names the provider and what to pick instead', () => {
  const why = m.personaRefusal('gemini');
  assert.match(why, /^Gemini can’t load a persona/);
  assert.match(why, /Claude Code, Codex, Antigravity, Grok, Opencode/);
});
