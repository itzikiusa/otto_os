import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import type { SecurityScanSummary } from '../src/lib/api/types/ide-security.ts';
import { loadSource } from './sourceHarness.ts';

const problems = readFileSync(new URL('../src/modules/ide/IdeProblemsPanel.svelte', import.meta.url), 'utf8');
const security = readFileSync(new URL('../src/modules/ide/security/IdeSecurityPage.svelte', import.meta.url), 'utf8');

test('the Clear diagnostics button is disabled when only security findings remain', () => {
  assert.match(problems, /aria-label="Clear diagnostics"[\s\S]*?disabled=\{ide\.problems\.length === 0\}\s*onclick=\{\(\) => ide\.clearProblems\(\)\}/);
});

test('opening security settings rebuilds the form from the saved project overview', () => {
  assert.match(security, /if \(showSettings && overview\) \{\s*resetSettings\(overview\);/);
  assert.match(security, /settings = \{ \.\.\.saved\.settings, checks: \[\.\.\.saved\.settings\.checks\]/);
});

test('the security poller takes the slower cadence while idle', () => {
  assert.match(security, /get ms\(\) \{ return active \? 3500 : 60_000; \}/);
});

test('security polling refreshes detail only on scan identity or state change', () => {
  const { scanDetailChanged } = loadSource(new URL('../src/modules/ide/security/projectSecurity.svelte.ts', import.meta.url), {
    '../../../lib/api/client': { api: {} },
  });
  const running: SecurityScanSummary = {
    id: 'test', status: 'running', created_at: '2026-11-27T12:00:00Z', started_at: null,
    finished_at: null, error: null, count: 0, engines: [],
  };
  assert.equal(scanDetailChanged(undefined, undefined), false);
  assert.equal(scanDetailChanged(running, { ...running }), false);
  assert.equal(scanDetailChanged(undefined, running), true);
  assert.equal(scanDetailChanged(running, { ...running, id: 'another' }), true);
  assert.equal(scanDetailChanged(running, { ...running, status: 'completed' }), true);
  assert.equal(scanDetailChanged(running, { ...running, finished_at: '2026-11-27T12:00:02Z' }), true);
});

test('SARIF and baseline commands live in the Security overflow menu', () => {
  const toolbar = security.match(/<PageHeader[\s\S]*?<\/PageHeader>/)?.[0] ?? '';
  const actions = toolbar.match(/\{#snippet actions\(\)\}([\s\S]*?)\{\/snippet\}/)?.[1] ?? '';
  for (const label of ['Import SARIF', 'Export SARIF', 'Set baseline']) {
    assert.ok(!actions.includes(label), label + ' is in the toolbar actions');
    assert.ok(security.includes(label), label + ' is missing');
  }
  assert.match(toolbar, /more=\{securityMore\}/);
});
