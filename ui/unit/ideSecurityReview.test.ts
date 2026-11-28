import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { ProjectScans, RunDastRequest, SecurityScanSummary } from '../src/lib/api/types/ide-security.ts';
import { componentFunctions } from './componentFunctions.ts';
import { loadSource } from './sourceHarness.ts';

const problems = new URL('../src/modules/ide/IdeProblemsPanel.svelte', import.meta.url);
const security = new URL('../src/modules/ide/security/IdeSecurityPage.svelte', import.meta.url);
const running: SecurityScanSummary = {
  id: 'test', status: 'running', created_at: '2026-11-27T12:00:00Z', started_at: null,
  finished_at: null, engine: null, metrics: null, error: null, count: 0, engines: [],
  trigger: 'manual', skipped: [],
};
const completed: SecurityScanSummary = { ...running, status: 'completed', finished_at: '2026-11-27T12:00:02Z' };

function project(scans: SecurityScanSummary[] = []): ProjectScans {
  return {
    settings: { checks: ['sast'], excludes: ['generated/**'], parallelism: 1, memory_mib: 1024, timeout_secs: 180 },
    schedule: { cron: '0 9 * * *', timezone: 'UTC', next_runs: ['2026-11-28T09:00:00Z'] },
    scans, baseline: null, engine: null, opengrep: null, nuclei: null,
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
    overview, detail: { scan: running }, acting: false, passiveRunReady: false,
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

test('active DAST binds explicit confirmation to the displayed target identity and passive remains default', async () => {
  const posts: Array<{ path: string; body?: unknown }> = [];
  const puts: Array<{ path: string; body?: unknown }> = [];
  const prompts: Array<{ message: string; title?: string }> = [];
  let accept = false;
  let swapTargetDuringPrompt = false;
  const confirmedA = {
    run_config_id: 'cfg-1',
    url: 'http://127.0.0.1:3000/app',
    allow_public: false,
    scheduled: false,
    confirmed_at: '2026-11-27T12:00:00.000Z',
  };
  const confirmedB = {
    run_config_id: 'cfg-2',
    url: 'http://127.0.0.1:4000/other',
    allow_public: false,
    scheduled: false,
    confirmed_at: '2026-11-27T12:05:00.000Z',
  };
  const candidateA = { run_config_id: 'cfg-1', run_config_name: 'Dev', url: confirmedA.url, public: false };
  const state = componentFunctions(security, ['runActiveDast', 'runPassive', 'confirmPassive', 'securityMore', 'action'], {
    base: '/security',
    acting: false,
    error: '',
    passiveLoading: false,
    active: undefined,
    passiveRunReady: true,
    passiveScheduled: true,
    passiveCandidate: candidateA,
    passive: { confirmed: confirmedA, candidates: [candidateA] },
    overview: project(),
    detail: { scan: completed },
    importInput: { click() {} },
    exportSarif() {},
    selectBaseline() {},
    loadErrorText: String,
    refresh: async () => {},
    refreshPassive: async () => {},
    confirmer: {
      async ask(message: string, opts?: { title?: string }) {
        prompts.push({ message, title: opts?.title });
        if (swapTargetDuringPrompt) {
          // Simulate another window updating confirmed target to B while the prompt for A is open.
          state.passive = { confirmed: confirmedB, candidates: [candidateA] };
        }
        return accept;
      },
    },
    api: {
      async post(path: string, body?: unknown) {
        posts.push({ path, body });
        return { id: 'scan-active' };
      },
      async put(path: string, body?: unknown) {
        puts.push({ path, body });
        return confirmedA;
      },
    },
  });

  // Overflow menu includes active scan only when target is confirmed and ready.
  assert.ok(state.securityMore().some((item: { label: string }) => item.label === 'Run active scan (Nuclei)…'));

  // Declining confirmation sends nothing.
  await state.runActiveDast();
  assert.equal(prompts.length, 1);
  assert.match(prompts[0].message, /test HTTP requests/i);
  assert.equal(posts.length, 0);

  // SEC-2: accepting confirmation posts target A's exact displayed identity
  // even if state.passive.confirmed changed to B while the dialog was open.
  accept = true;
  swapTargetDuringPrompt = true;
  await state.runActiveDast();
  assert.equal(posts.length, 1);
  assert.equal(posts[0].path, '/security/dast/run');
  assert.deepEqual(JSON.parse(JSON.stringify(posts[0].body)), {
    active: true,
    run_config_id: confirmedA.run_config_id,
    url: confirmedA.url,
    confirmed_at: confirmedA.confirmed_at,
  });

  // SEC-1: confirming a scheduled target sends passive-only consent (no scheduled_active).
  swapTargetDuringPrompt = false;
  await state.confirmPassive();
  assert.equal(puts.length, 1);
  assert.equal(puts[0].path, '/security/dast');
  assert.equal('scheduled_active' in (puts[0].body as Record<string, unknown>), false);

  // Passive run sends no active opt-in.
  state.runPassive();
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(posts.length, 2);
  assert.equal(posts[1].path, '/security/dast/run');
  assert.equal(posts[1].body, undefined);
});

test('latestFileScan ignores both passive HTTP and Nuclei DAST-only scans', () => {
  const { latestFileScan } = loadSource(
    new URL('../src/modules/ide/security/projectSecurityData.ts', import.meta.url),
    { '../../../lib/api/client': { api: {} } },
    {}, { relative: true },
  );
  const engine = (name: string) => ({
    name,
    version: { version: '1.0.0', updated_at: '2026-11-27T12:00:00Z', checked_at: '2026-11-27T12:00:00Z' },
    metrics: { wall_ms: 10, peak_cpu_percent: 1, peak_rss_bytes: 1024 },
    mode: 'full' as const,
  });
  const nucleiOnly: SecurityScanSummary = { ...completed, id: 'n1', engines: [engine('nuclei')] };
  const passiveOnly: SecurityScanSummary = { ...completed, id: 'p1', engines: [engine('otto-passive-http')] };
  const sastScan: SecurityScanSummary = { ...completed, id: 's1', engines: [engine('opengrep')] };
  assert.equal(latestFileScan([nucleiOnly, passiveOnly, sastScan])?.id, 's1');
  assert.equal(latestFileScan([nucleiOnly, passiveOnly]), undefined);
});

test('RunDastRequest requires full confirmation identity when active is true', () => {
  const validActive: RunDastRequest = {
    active: true,
    run_config_id: 'cfg-1',
    url: 'http://127.0.0.1:3000/app',
    confirmed_at: '2026-11-27T12:00:00.000Z',
  };
  const validPassiveEmpty: RunDastRequest = {};
  const validPassiveFalse: RunDastRequest = { active: false };
  // @ts-expect-error bare active request is rejected by the server with HTTP 400
  const invalidBareActive: RunDastRequest = { active: true };
  // @ts-expect-error active request missing confirmed_at is rejected with HTTP 400
  const invalidPartialActive: RunDastRequest = {
    active: true,
    run_config_id: 'cfg-1',
    url: 'http://127.0.0.1:3000/app',
  };
  assert.equal(validActive.active, true);
  assert.equal(validPassiveEmpty.active, undefined);
  assert.equal(validPassiveFalse.active, false);
  assert.equal(invalidBareActive.active, true);
  assert.equal(invalidPartialActive.active, true);
});

test('scheduled active checks are allowed only with the exact offer the confirm showed', async () => {
  const offer = {
    run_config_id: 'cfg-1', url: 'http://localhost:3000/', confirmed_at: '2026-11-27T12:00:00.000Z',
    addresses: ['127.0.0.1:3000', '[::1]:3000'], cron: '0 9 * * *', timezone: 'UTC',
    armed_at: '2026-11-27T11:00:00.000Z', template_policy: 'exposures-test',
  };
  const calls: unknown[][] = [];
  const prompts: Array<{ message: string; title?: string }> = [];
  let accept = false;
  const state = componentFunctions(security, ['toggleScheduledActive', 'action'], {
    base: '/security', acting: false, error: '', loadErrorText: String,
    overview: project(), when: (date: string) => date, day: (date: string) => date.slice(0, 10),
    passive: {
      confirmed: { run_config_id: 'cfg-1', url: offer.url, allow_public: false, scheduled: true, confirmed_at: offer.confirmed_at },
      candidates: [{ run_config_id: 'cfg-1', run_config_name: 'Dev', url: offer.url, public: false }],
      active_schedule: { grant: null, offer, blocked: null },
    },
    refreshPassive: async () => {},
    confirmer: {
      async ask(message: string, opts?: { title?: string }) {
        prompts.push({ message, title: opts?.title });
        return accept;
      },
    },
    api: {
      async put(path: string, body?: unknown) { calls.push(['put', path, body]); },
      async del(path: string) { calls.push(['del', path]); },
    },
  });

  // Declining sends nothing and leaves the switch off.
  const input = { checked: true };
  await state.toggleScheduledActive(input);
  assert.equal(input.checked, false);
  assert.equal(calls.length, 0);
  assert.equal(prompts[0].title, 'Allow scheduled active checks?');
  for (const shown of [/Target: http:\/\/localhost:3000\/ · run configuration “Dev”/, /127\.0\.0\.1:3000, \[::1\]:3000/,
    /0 9 \* \* \* \(UTC\), next 2026-11-28T09:00:00Z/, /Runs as you until \d{4}-\d\d-\d\d/, /test HTTP requests/]) {
    assert.match(prompts[0].message, shown);
  }

  // Accepting sends exactly the identity the confirm showed; turning off revokes.
  accept = true;
  input.checked = true;
  await state.toggleScheduledActive(input);
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [['put', '/security/dast/schedule', offer]]);
  input.checked = false;
  await state.toggleScheduledActive(input);
  assert.deepEqual(calls[1], ['del', '/security/dast/schedule']);
});
