// I-14: the settings pages read the shared provider registry instead of
// their own lists, and that registry's update commands stay in step with
// the daemon's built-in provider table.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import {
  BUILTIN_PROVIDER_UPDATES,
  isAgentCliTool,
  KNOWN_AGENT_FAMILIES,
} from '../src/lib/providerRegistry.ts';

const read = (rel: string) => readFileSync(new URL(`../${rel}`, import.meta.url), 'utf8');

test('every known agent family counts as an agent CLI on the onboarding tool check', () => {
  for (const fam of ['claude', 'codex', 'agy', 'gemini', 'grok', 'opencode', 'pi', 'muse']) {
    assert.ok(isAgentCliTool(fam), fam);
  }
  assert.ok(!isAgentCliTool('git'));
  assert.ok(!isAgentCliTool('clickhouse'));
});

test('built-in update commands match the daemon provider table', () => {
  const rust = read('../crates/otto-sessions/src/providers.rs');
  const names = BUILTIN_PROVIDER_UPDATES.map((b) => b.name);
  for (const fam of KNOWN_AGENT_FAMILIES) assert.ok(names.includes(fam), fam);
  for (const { name, updateCmd } of BUILTIN_PROVIDER_UPDATES) {
    if (updateCmd === null) continue;
    if (name === 'muse') {
      assert.ok(
        rust.includes(`"${updateCmd.replace(' muse --version', ' {program} --version')}"`),
        'muse update command drifted from muse_update_command',
      );
      assert.ok(rust.includes('update_command: Some(muse_update_command("muse"))'));
      continue;
    }
    assert.ok(rust.includes(`update_command: Some("${updateCmd}".into())`), `${name}: ${updateCmd}`);
  }
});

test('terminal effort follows the daemon map, falling back on an older daemon', async () => {
  const { terminalTakesEffort } = await import('../src/lib/providerRegistry.ts');
  const meta = { provider_terminal_effort: { claude: true, muse: false, work: true } };
  assert.equal(terminalTakesEffort('muse', meta, true), false, 'Muse has no terminal effort flag');
  assert.equal(terminalTakesEffort('work', meta, false), true, 'a custom claude-family slug');
  assert.equal(terminalTakesEffort('claude', {}, true), true, 'older daemon: fallback');
  assert.equal(terminalTakesEffort('muse', null, false), false);
});
