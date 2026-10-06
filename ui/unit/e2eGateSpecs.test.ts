import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import {
  DESKTOP_GATE_PROJECT,
  DESKTOP_GATE_SPECS,
  GATE_MIN_EXPECTED,
  MOBILE_GATE_PROJECT,
  MOBILE_GATE_SPECS,
  gateMatcher,
} from '../e2e/gate-specs.ts';
// @ts-expect-error — plain .mjs CI script, no declaration file
import { ratchet, summaryMarkdown, tally, verdict } from '../scripts/e2e-flaky-check.mjs';

const e2e = new URL('../e2e/', import.meta.url);

test('every blocking e2e gate spec exists (a rename must not empty the gate)', () => {
  for (const f of [...DESKTOP_GATE_SPECS, ...MOBILE_GATE_SPECS]) {
    assert.ok(existsSync(new URL(f, e2e)), `e2e/${f} is listed in gate-specs.ts but missing`);
  }
});

test('gate lists respect the project split: desktop-* only at desktop width, none at phone width', () => {
  for (const f of DESKTOP_GATE_SPECS) assert.match(f, /^desktop-.*\.spec\.ts$/);
  for (const f of MOBILE_GATE_SPECS) assert.doesNotMatch(f, /^desktop-/);
  // Perf specs belong to perf-gates (scaled budgets), never to the smoke gate.
  for (const f of DESKTOP_GATE_SPECS) assert.doesNotMatch(f, /perf/);
  assert.equal(new Set(DESKTOP_GATE_SPECS).size, DESKTOP_GATE_SPECS.length);
});

test('gateMatcher matches listed files exactly, not by prefix', () => {
  const re = gateMatcher(['desktop-shell.spec.ts']);
  assert.ok(re.test('/repo/ui/e2e/desktop-shell.spec.ts'));
  assert.ok(!re.test('/repo/ui/e2e/desktop-shell-reopen.spec.ts'));
  assert.ok(!re.test('/repo/ui/e2e/xdesktop-shell.spec.ts'));
});

// S12-301: a gate spec whose project guard excludes the gate project skips
// every test there, and the gate passes vacuously. Parse each gate spec's
// `project.name !== '<x>'` and `![...].includes(…project.name)` guards.
function guardViolations(src: string, gateProject: string): string[] {
  const bad: string[] = [];
  for (const m of src.matchAll(/project\.name\s*!==?\s*['"]([^'"]+)['"]/g)) {
    if (m[1] !== gateProject) bad.push(`project.name !== '${m[1]}'`);
  }
  for (const m of src.matchAll(/!\s*\[([^\]]*)\]\s*\.includes\([^)]*project\.name\)/g)) {
    const names = [...m[1].matchAll(/['"]([^'"]+)['"]/g)].map((n) => n[1]);
    if (!names.includes(gateProject)) bad.push(`![${names.join(', ')}].includes(project.name)`);
  }
  return bad;
}

test('no gate spec self-skips on its gate project (project.name guards)', () => {
  const lists: [readonly string[], string][] = [
    [DESKTOP_GATE_SPECS, DESKTOP_GATE_PROJECT],
    [MOBILE_GATE_SPECS, MOBILE_GATE_PROJECT],
  ];
  for (const [files, project] of lists) {
    for (const f of files) {
      const src = readFileSync(new URL(f, e2e), 'utf8');
      assert.deepEqual(guardViolations(src, project), [], `e2e/${f} skips on the gate project ${project}`);
    }
  }
});

test('the guard parser catches a foreign project literal and an excluding include-list', () => {
  assert.deepEqual(guardViolations(`test.skip(info.project.name !== 'desktop-gate', 'x')`, 'desktop-browser'), [
    "project.name !== 'desktop-gate'",
  ]);
  assert.equal(guardViolations(`test.skip(!['ipad-landscape'].includes(t.project.name), 'x')`, 'iphone-portrait').length, 1);
  assert.deepEqual(guardViolations(`test.skip(!['iphone-portrait', 'x'].includes(t.project.name))`, 'iphone-portrait'), []);
  assert.deepEqual(guardViolations(`test.skip(info.project.name !== 'desktop-browser')`, 'desktop-browser'), []);
});

test('the gate runs on the real projects (playwright.config has no separate gate project)', () => {
  const cfg = readFileSync(new URL('../playwright.config.ts', import.meta.url), 'utf8');
  assert.doesNotMatch(cfg, /name:\s*'(desktop|iphone)-gate'/);
  assert.match(cfg, /GATE \? gateMatcher\(DESKTOP_GATE_SPECS\)/);
  assert.match(cfg, /GATE \? gateMatcher\(MOBILE_GATE_SPECS\)/);
  assert.ok(GATE_MIN_EXPECTED > 0);
});

test('flaky-check --gate fails on skipped tests and on a pass count below the floor', () => {
  const report = (stats: Record<string, number>) => ({ stats, suites: [] });
  const ok = tally([report({ expected: 166, unexpected: 0, flaky: 0, skipped: 0 })]);
  assert.deepEqual(verdict(ok, { gate: true, minExpected: 166 }), []);
  const skipped = tally([report({ expected: 76, unexpected: 0, flaky: 0, skipped: 90 })]);
  const errs = verdict(skipped, { gate: true, minExpected: 166 });
  assert.equal(errs.length, 2);
  assert.match(errs[0], /90 gate test\(s\) SKIPPED/);
  assert.match(errs[1], /below the floor/);
  // Outside gate mode a skip is fine (advisory shards skip by design); flaky never is.
  assert.deepEqual(verdict(skipped), []);
  assert.equal(verdict(tally([report({ expected: 1, flaky: 1 })])).length, 1);
  // Shards add up.
  const two = tally([report({ expected: 100, skipped: 0 }), report({ expected: 66, unexpected: 2 })]);
  assert.equal(two.expected, 166);
  assert.equal(two.unexpected, 2);
});

test('flaky-check names failed tests in the step summary', () => {
  const t = tally([
    {
      stats: { expected: 1, unexpected: 1 },
      suites: [{ title: 'a.spec.ts', specs: [{ title: 'boom', tests: [{ status: 'unexpected', projectName: 'desktop-browser' }] }] }],
    },
  ]);
  const md = summaryMarkdown(t, 'E2E shard 1');
  assert.match(md, /\| 1 \| 1 \| 0 \| 0 \|/);
  assert.match(md, /a\.spec\.ts › boom \[desktop-browser\]/);
});

test('the advisory red-count ratchet fails only when the count rises', () => {
  const t = (unexpected: number) => tally([{ stats: { expected: 10, unexpected }, suites: [] }]);
  assert.match(ratchet(t(57), 56).error, /rose: 57 failed > baseline 56/);
  assert.deepEqual(ratchet(t(56), 56), {});
  assert.match(ratchet(t(40), 56).notice, /lower the baseline/);
  const baseline = JSON.parse(readFileSync(new URL('../e2e/advisory-red-baseline.json', import.meta.url), 'utf8'));
  assert.ok(Number.isInteger(baseline.max_failed) && baseline.max_failed >= 0);
});
