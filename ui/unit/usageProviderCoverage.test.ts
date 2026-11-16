// Usage ▸ By provider (modules/usage/providerCoverage.ts): provider rows use
// the shared registry's names, and the providers whose TERMINAL sessions Usage
// cannot measure (gemini / opencode / pi / muse — only their Threads turns are
// recorded) are named in a note instead of silently reading as zero.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  TRANSCRIPT_TAILED,
  terminalUnmeasured,
  unmeasuredTerminalNote,
  usageProviderLabel,
} from '../src/modules/usage/providerCoverage.ts';
import { KNOWN_AGENT_FAMILIES } from '../src/lib/providerRegistry.ts';

test('every known agent family is either tailed or flagged as terminal-unmeasured', () => {
  const unmeasured = KNOWN_AGENT_FAMILIES.filter((p) => terminalUnmeasured(p));
  assert.deepEqual([...unmeasured].sort(), ['gemini', 'muse', 'opencode', 'pi']);
  for (const p of TRANSCRIPT_TAILED) {
    assert.ok((KNOWN_AGENT_FAMILIES as readonly string[]).includes(p), p);
    assert.equal(terminalUnmeasured(p), false, p);
  }
  // Not an agent CLI: nothing to say about it.
  assert.equal(terminalUnmeasured('shell'), false);
  assert.equal(terminalUnmeasured('claudework'), false);
});

test('provider rows use the registry labels; custom slugs stay as recorded', () => {
  assert.equal(usageProviderLabel('muse'), 'Muse');
  assert.equal(usageProviderLabel('opencode'), 'OpenCode');
  assert.equal(usageProviderLabel('pi'), 'Pi');
  assert.equal(usageProviderLabel('gemini'), 'Gemini');
  assert.equal(usageProviderLabel('grok'), 'Grok');
  assert.equal(usageProviderLabel('agy'), 'Antigravity');
  assert.equal(usageProviderLabel('claude'), 'Claude Code');
  assert.equal(usageProviderLabel('claudework'), 'claudework');
});

test('the note names only the unmeasured providers in use or installed, in registry order', () => {
  assert.equal(unmeasuredTerminalNote(['claude', 'codex', 'grok', 'agy']), null);
  assert.equal(unmeasuredTerminalNote([]), null);
  assert.equal(
    unmeasuredTerminalNote(['muse', 'claude']),
    'Muse tokens count in Threads only: their terminal sessions are not measured (the CLI keeps no usage record Otto can read).',
  );
  const all = unmeasuredTerminalNote(['muse', 'pi', 'claude', 'opencode', 'gemini']);
  assert.ok(all?.startsWith('Gemini, OpenCode, Pi and Muse tokens count in Threads only'), all ?? '');
});
