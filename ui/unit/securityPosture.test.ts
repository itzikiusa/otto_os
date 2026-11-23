import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { SecurityPostureCheck } from '../src/lib/api/types/auth.ts';
import {
  CATEGORY_ORDER,
  CHECK_STATUS_RANK,
  categoryLabel,
  checkStatusInfo,
  deriveOverallPosture,
  groupChecksByCategory,
  hasRemediation,
  overallPostureInfo,
  postureSummaryLine,
  sortChecks,
  summarizeChecks,
} from '../src/modules/settings/securityPosture.ts';

function makeCheck(overrides: Partial<SecurityPostureCheck> = {}): SecurityPostureCheck {
  return {
    id: 'secrets_backend',
    category: 'secrets',
    title: 'Secrets backend',
    status: 'pass',
    detail: 'Secrets are stored in macOS Keychain.',
    remediation: null,
    ...overrides,
  };
}

test('sortChecks orders fail -> warn -> pass and breaks ties by title then id', () => {
  const input: SecurityPostureCheck[] = [
    makeCheck({ id: 'z_pass', title: 'SQLite integrity', status: 'pass' }),
    makeCheck({ id: 'b_warn', title: 'Network exposure', status: 'warn' }),
    makeCheck({ id: 'a_fail', title: 'Secrets backend', status: 'fail' }),
    makeCheck({ id: 'a_warn', title: 'Device shares', status: 'warn' }),
    makeCheck({ id: 'b_fail', title: 'Database permissions', status: 'fail' }),
    makeCheck({ id: 'a_pass', title: 'Plugin capabilities', status: 'pass' }),
  ];

  const sorted = sortChecks(input);
  // Original array must not be mutated.
  assert.equal(input[0].id, 'z_pass');
  assert.deepEqual(
    sorted.map((c) => `${c.status}:${c.id}`),
    [
      'fail:b_fail',
      'fail:a_fail',
      'warn:a_warn',
      'warn:b_warn',
      'pass:a_pass',
      'pass:z_pass',
    ],
  );
  assert.equal(CHECK_STATUS_RANK.fail < CHECK_STATUS_RANK.warn, true);
  assert.equal(CHECK_STATUS_RANK.warn < CHECK_STATUS_RANK.pass, true);
});

test('sortChecks breaks identical titles deterministically by id', () => {
  const sorted = sortChecks([
    makeCheck({ id: 'check_b', title: 'Duplicate title', status: 'warn' }),
    makeCheck({ id: 'check_a', title: 'Duplicate title', status: 'warn' }),
  ]);
  assert.deepEqual(sorted.map((c) => c.id), ['check_a', 'check_b']);
});

test('summarizeChecks counts fail, warn, pass and handles empty/undefined', () => {
  assert.deepEqual(summarizeChecks(undefined), { fail: 0, warn: 0, pass: 0, total: 0 });
  assert.deepEqual(summarizeChecks([]), { fail: 0, warn: 0, pass: 0, total: 0 });

  const counts = summarizeChecks([
    makeCheck({ id: '1', status: 'fail' }),
    makeCheck({ id: '2', status: 'fail' }),
    makeCheck({ id: '3', status: 'warn' }),
    makeCheck({ id: '4', status: 'pass' }),
    makeCheck({ id: '5', status: 'pass' }),
    makeCheck({ id: '6', status: 'pass' }),
  ]);
  assert.deepEqual(counts, { fail: 2, warn: 1, pass: 3, total: 6 });
});

test('groupChecksByCategory groups checks, sorts inside groups, and ranks worst categories first', () => {
  const checks: SecurityPostureCheck[] = [
    makeCheck({ id: 'sqlite_health', category: 'database', title: 'SQLite health', status: 'pass' }),
    makeCheck({ id: 'network_exposure', category: 'network', title: 'Network exposure', status: 'warn' }),
    makeCheck({ id: 'plugins_cap_ok', category: 'plugins', title: 'Plugin sandbox', status: 'pass' }),
    makeCheck({ id: 'plugins_cap_fail', category: 'plugins', title: 'Plugin capabilities', status: 'fail' }),
    makeCheck({ id: 'db_and_tls_perms', category: 'filesystem', title: 'File permissions', status: 'fail' }),
    makeCheck({ id: 'secrets_backend', category: 'secrets', title: 'Secrets backend', status: 'pass' }),
    makeCheck({ id: 'custom_check', category: 'custom_audit', title: 'Custom audit', status: 'pass' }),
  ];

  const groups = groupChecksByCategory(checks);
  // Failing groups first (in canonical category order: filesystem before plugins),
  // then warning group (network), then passing groups (secrets, database, custom_audit).
  assert.deepEqual(
    groups.map((g) => `${g.category}:${g.worstStatus}`),
    [
      'filesystem:fail',
      'plugins:fail',
      'network:warn',
      'secrets:pass',
      'database:pass',
      'custom_audit:pass',
    ],
  );

  // Within plugins, fail precedes pass and counts match.
  const pluginsGroup = groups.find((g) => g.category === 'plugins')!;
  assert.deepEqual(pluginsGroup.checks.map((c) => c.id), ['plugins_cap_fail', 'plugins_cap_ok']);
  assert.deepEqual(pluginsGroup.counts, { fail: 1, warn: 0, pass: 1, total: 2 });
  assert.equal(pluginsGroup.label, 'Plugins');

  // Unknown category gets sentence-cased.
  const customGroup = groups.find((g) => g.category === 'custom_audit')!;
  assert.equal(customGroup.label, 'Custom audit');
  assert.ok(CATEGORY_ORDER.includes('secrets'));
});

test('deriveOverallPosture and overallPostureInfo respect explicit verdict and fall back to checks', () => {
  assert.equal(deriveOverallPosture('critical', []), 'critical');
  assert.equal(deriveOverallPosture('warn', [makeCheck({ status: 'pass' })]), 'warn');
  assert.equal(deriveOverallPosture('healthy', [makeCheck({ status: 'fail' })]), 'healthy');

  // Derived fallback when overall is omitted.
  assert.equal(deriveOverallPosture(undefined, [makeCheck({ status: 'fail' }), makeCheck({ status: 'warn' })]), 'critical');
  assert.equal(deriveOverallPosture(undefined, [makeCheck({ status: 'warn' }), makeCheck({ status: 'pass' })]), 'warn');
  assert.equal(deriveOverallPosture(undefined, [makeCheck({ status: 'pass' })]), 'healthy');
  assert.equal(deriveOverallPosture(undefined, undefined), 'healthy');

  assert.equal(overallPostureInfo('critical', undefined).tone, 'danger');
  assert.equal(overallPostureInfo('critical', undefined).label, 'Critical');
  assert.equal(overallPostureInfo('warn', undefined).tone, 'warning');
  assert.equal(overallPostureInfo('warn', undefined).label, 'Needs attention');
  assert.equal(overallPostureInfo('healthy', undefined).tone, 'success');
  assert.equal(overallPostureInfo('healthy', undefined).label, 'Healthy');
});

test('checkStatusInfo maps statuses to semantic tones and badge tones', () => {
  assert.deepEqual(checkStatusInfo('fail'), { key: 'fail', label: 'Fail', tone: 'danger', badgeTone: 'bad' });
  assert.deepEqual(checkStatusInfo('warn'), { key: 'warn', label: 'Warning', tone: 'warning', badgeTone: 'warn' });
  assert.deepEqual(checkStatusInfo('pass'), { key: 'pass', label: 'Pass', tone: 'success', badgeTone: 'ok' });
});

test('categoryLabel formats known, custom, and blank categories', () => {
  assert.equal(categoryLabel('secrets'), 'Secrets');
  assert.equal(categoryLabel('DATABASE'), 'Database');
  assert.equal(categoryLabel('tls-certs'), 'Tls certs');
  assert.equal(categoryLabel('remote_access'), 'Remote access');
  assert.equal(categoryLabel('   '), 'General');
});

test('hasRemediation detects non-blank remediation guidance', () => {
  assert.equal(hasRemediation(makeCheck({ remediation: undefined })), false);
  assert.equal(hasRemediation(makeCheck({ remediation: null })), false);
  assert.equal(hasRemediation(makeCheck({ remediation: '   ' })), false);
  assert.equal(hasRemediation(makeCheck({ remediation: 'Run chmod 600 on state.db.' })), true);
});

test('postureSummaryLine produces clean singular/plural summary copy', () => {
  assert.equal(postureSummaryLine({ fail: 0, warn: 0, pass: 0, total: 0 }), 'No security checks reported');
  assert.equal(postureSummaryLine({ fail: 0, warn: 0, pass: 1, total: 1 }), '1 check passed');
  assert.equal(postureSummaryLine({ fail: 0, warn: 0, pass: 6, total: 6 }), 'All 6 checks passed');
  assert.equal(postureSummaryLine({ fail: 1, warn: 1, pass: 4, total: 6 }), '1 failed · 1 warning · 4 passed');
  assert.equal(postureSummaryLine({ fail: 2, warn: 2, pass: 0, total: 4 }), '2 failed · 2 warnings');
});
