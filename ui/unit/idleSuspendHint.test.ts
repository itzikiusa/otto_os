import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  DEFAULT_ENGINE_GRACE_SECS,
  DEFAULT_IDLE_SUSPEND_POLICY,
  DEFAULT_MANUAL_GRACE_SECS,
  DEFAULT_MAX_LIVE_AGENT_SESSIONS,
  formatGrace,
  formatLeft,
  policyFromSettings,
  suspendHint,
} from '../src/lib/idleSuspend.ts';

const MIN = 60_000;

test('user-started sessions count down on the 24-h manual grace, engine ones on 5 min', () => {
  const p = DEFAULT_IDLE_SUSPEND_POLICY;
  assert.equal(suspendHint(4 * MIN, true, p)?.label, '4m idle · suspends in 24h');
  assert.equal(suspendHint(4 * MIN, false, p)?.label, '4m idle · suspends in 1m');
  assert.equal(suspendHint(6 * MIN, false, p)?.label, '6m idle · suspending…');
  assert.equal(suspendHint(30 * 1000, true, p)?.label, '30s idle · suspends in 24h');
  assert.equal(suspendHint(5 * 60 * MIN, true, p)?.label, '5h idle · suspends in 19h');
  assert.equal(suspendHint(23 * 60 * MIN + 30 * MIN, true, p)?.label, '23h idle · suspends in 30m');
  assert.match(suspendHint(4 * MIN, true, p)!.title, /after 24 h of quiet/);
  assert.equal(suspendHint(-1, true, p), null);
});

test('durations read naturally at every scale', () => {
  assert.equal(formatLeft(26 * MIN), '26m');
  assert.equal(formatLeft(89 * MIN), '89m');
  assert.equal(formatLeft(90 * MIN), '2h');
  assert.equal(formatLeft(3 * 24 * 60 * MIN), '3d');
  assert.equal(formatGrace(1800), '30 min');
  assert.equal(formatGrace(86_400), '24 h');
  assert.equal(formatGrace(7 * 86_400), '7 days');
});

test('settings override the grace; manual 0 = never suspended, so no hint', () => {
  const p = policyFromSettings({ manual_idle_suspend_secs: 600, idle_suspend_grace_secs: 120, max_live_agent_sessions: 0 });
  assert.equal(suspendHint(4 * MIN, true, p)?.label, '4m idle · suspends in 6m');
  assert.equal(suspendHint(1 * MIN, false, p)?.label, '1m idle · suspends in 1m');
  assert.doesNotMatch(suspendHint(1 * MIN, false, p)!.title, /live agent sessions/, 'cap off → no cap note');
  const never = policyFromSettings({ manual_idle_suspend_secs: 0 });
  assert.equal(suspendHint(90 * MIN, true, never), null);
  assert.ok(suspendHint(90 * MIN, false, never), 'engine sessions still suspend');
  assert.match(suspendHint(1 * MIN, true, DEFAULT_IDLE_SUSPEND_POLICY)!.title, /more than 12 live agent sessions/);
});

test('unset or malformed settings fall back to the daemon defaults', () => {
  assert.deepEqual(policyFromSettings(null), DEFAULT_IDLE_SUSPEND_POLICY);
  assert.deepEqual(
    policyFromSettings({ manual_idle_suspend_secs: '60', idle_suspend_grace_secs: -5, max_live_agent_sessions: 1.5 }),
    DEFAULT_IDLE_SUSPEND_POLICY,
  );
});

test('defaults mirror the daemon constants in manager.rs', () => {
  const rs = readFileSync(new URL('../../crates/otto-sessions/src/manager.rs', import.meta.url), 'utf8');
  const secs = (name: string) => {
    const m = rs.match(new RegExp(`const ${name}: Duration = Duration::from_secs\\(([^)]+)\\)`));
    assert.ok(m, `${name} not found`);
    return Function(`return ${m[1].replace(/_/g, '')}`)() as number;
  };
  assert.equal(secs('SUSPEND_GRACE'), DEFAULT_ENGINE_GRACE_SECS);
  assert.equal(secs('MANUAL_IDLE_SUSPEND'), DEFAULT_MANUAL_GRACE_SECS);
  const cap = rs.match(/const MAX_LIVE_AGENT_SESSIONS: usize = (\d+)/);
  assert.equal(Number(cap?.[1]), DEFAULT_MAX_LIVE_AGENT_SESSIONS);
});
