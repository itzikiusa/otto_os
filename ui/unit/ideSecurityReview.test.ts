import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { ProjectScans, SecurityScanSummary } from '../src/lib/api/types/ide-security.ts';
import { componentFunctions } from './componentFunctions.ts';
import { loadSource } from './sourceHarness.ts';

const problems = new URL('../src/modules/ide/IdeProblemsPanel.svelte', import.meta.url);
const security = new URL('../src/modules/ide/security/IdeSecurityPage.svelte', import.meta.url);
const running: SecurityScanSummary = {
  id: 'test', status: 'running', created_at: '2026-11-27T12:00:00Z', started_at: null,
  finished_at: null, engine: null, metrics: null, error: null, count: 0, engines: [],
};
const completed: SecurityScanSummary = { ...running, status: 'completed', finished_at: '2026-11-27T12:00:02Z' };

function project(scans: SecurityScanSummary[] = []): ProjectScans {
  return {
    settings: { checks: ['sast'], excludes: ['generated/**'], parallelism: 1, memory_mib: 1024, timeout_secs: 180 },
    schedule: { cron: '0 9 * * *', timezone: 'UTC', next_runs: ['2026-11-28T09:00:00Z'] },
    scans, baseline: null, engine: null, opengrep: null,
  };
}

test('only clearable diagnostics enable the Problems action', () => {
  const finding = { path: '/project/src/main.rs', line: 1 };
  const ide = { problems: [] as typeof finding[] };
  const state = componentFunctions(problems, ['hasClearableDiagnostics'], {
    ide, allProblems: [finding],
  });
  assert.equal(state.hasClearableDiagnostics(), false);
  ide.problems.push(finding);
  assert.equal(state.hasClearableDiagnostics(), true);
});

test('reopening settings discards an edited draft and restores the saved schedule', () => {
  const overview = project();
  const state = componentFunctions(security, ['toggleSettings', 'resetSettings'], {
    overview, showSettings: false, refreshPassive() {}, settings: null,
    excludes: '', cron: '', timezone: 'UTC', schedulePreview: null,
  });
  state.toggleSettings();
  state.settings.memory_mib = 64;
  state.settings.checks.pop();
  state.excludes = 'another/**';
  state.cron = 'invalid';
  state.timezone = 'GMT';
  state.schedulePreview = null;
  assert.equal(overview.settings.memory_mib, 1024);
  assert.equal(overview.settings.checks.length, 1);

  state.toggleSettings();
  state.toggleSettings();
  assert.deepEqual(Array.from(state.settings.checks), ['sast']);
  assert.equal(state.settings.memory_mib, 1024);
  assert.equal(state.excludes, 'generated/**');
  assert.equal(state.cron, '0 9 * * *');
  assert.equal(state.timezone, 'UTC');
  assert.equal(state.schedulePreview, overview.schedule);
});

test('idle polling is slower and skips unchanged findings and diagnostics', async () => {
  const { diagnostics, latestFileScan, scanDetailChanged } = loadSource(
    new URL('../src/modules/ide/security/projectSecurityData.ts', import.meta.url),
    { '../../../lib/api/client': { api: {} } },
    {}, { relative: true },
  );
  let scan = running;
  let diagnosticsUpdates = 0;
  const calls: string[] = [];
  const get = async (path: string) => {
    calls.push(path);
    return path === '/security' ? project([scan]) : { scan, findings: [], annotations: { suppressions: {} } };
  };
  const state = componentFunctions(security, ['scanPollMs', 'refresh', 'resetSettings'], {
    active: undefined, alive: true, base: '/security', detail: null, overview: null,
    settings: null, selected: '', lastDiagnostics: undefined, poller: undefined, request: 0,
    excludes: '', cron: '', timezone: 'UTC', schedulePreview: null,
    error: '', loading: false, api: { get, bg: { get } }, loadErrorText: String,
    diagnostics, latestFileScan, scanDetailChanged, onDiagnostics() { diagnosticsUpdates++; },
  });
  assert.equal(state.scanPollMs(), 60_000);
  state.active = running;
  assert.equal(state.scanPollMs(), 3500);
  const signal = new AbortController().signal;
  await state.refresh(undefined, signal);
  await state.refresh(undefined, signal);
  assert.deepEqual(calls, ['/security', '/security/scans/test', '/security']);
  assert.equal(diagnosticsUpdates, 0);
  scan = completed;
  await state.refresh(undefined, signal);
  assert.deepEqual(calls.slice(3), ['/security', '/security/scans/test']);
  assert.equal(diagnosticsUpdates, 1);
});

test('the Security overflow menu uses the selected scan and baseline state', () => {
  let opened = false;
  const overview = project();
  const state = componentFunctions(security, ['securityMore'], {
    overview, detail: { scan: running }, acting: false,
    importInput: { click() { opened = true; } }, exportSarif() {}, selectBaseline() {},
  });
  const choices = state.securityMore();
  assert.deepEqual(Array.from(choices, (item: {label: string}) => item.label), ['Import SARIF', 'Export SARIF', 'Set baseline']);
  assert.equal(choices[1].disabled, true);
  assert.equal(choices[2].disabled, true);
  choices[0].action();
  assert.equal(opened, true);
  state.detail = { scan: completed };
  overview.baseline = completed.id;
  assert.equal(state.securityMore()[1].disabled, false);
  assert.equal(state.securityMore()[2].label, 'Clear baseline');
});
