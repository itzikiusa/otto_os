import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { DESKTOP_GATE_SPECS, MOBILE_GATE_SPECS, gateMatcher } from '../e2e/gate-specs.ts';

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
