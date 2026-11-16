import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

test('ottoTelemetry analyze completes across unmount, decouples usageUi scope, and splits busy flags', async () => {
  const analyzeGate = deferred<any[]>();
  const toggleGate = deferred<{ enabled: boolean; retention_days: number }>();
  const dismissGate = deferred<void>();
  const getCalls: string[] = [];
  let serverSuggestions = [
    { id: 'sug-1', component: 'git', name: 'git.execute', dismissed: false },
    { id: 'sug-2', component: 'ide', name: 'ide.lsp', dismissed: false },
  ];
  const { usageUi } = loadSource(
    new URL('../src/modules/usage/usageUiState.svelte.ts', import.meta.url),
    {},
  );
  const storeMod = loadSource(
    new URL('../src/modules/usage/ottoTelemetryStore.svelte.ts', import.meta.url),
    {
      '../../lib/announce.svelte': { announce() {} },
      '../../lib/api/client': {
        api: {
          get: async (url: string) => {
            getCalls.push(url);
            if (url === '/telemetry/config') return { enabled: true, retention_days: 7 };
            if (url === '/telemetry/status') return { enabled: true, collector_ready: true };
            if (url.startsWith('/telemetry/overview')) return { spans: 42, operations: [] };
            if (url === '/telemetry/suggestions') {
              return serverSuggestions;
            }
            throw new Error(`unexpected GET ${url}`);
          },
          post: async (url: string) => {
            if (url === '/telemetry/analyze') {
              const res = await analyzeGate.promise;
              serverSuggestions = res;
              return res;
            }
            if (url === '/telemetry/suggestions/sug-2/dismiss') {
              await dismissGate.promise;
              serverSuggestions = serverSuggestions.map((s) => (s.id === 'sug-2' ? { ...s, dismissed: true } : s));
              return;
            }
            throw new Error(`unexpected POST ${url}`);
          },
          put: async () => toggleGate.promise,
        },
      },
      '../../lib/poll': {
        pollWhileVisible: () => ({ stop() {} }),
      },
      '../../lib/telemetryBoot': { telemetrySettingsChanged() {} },
    },
    {},
    { relative: true },
  );

  const { ottoTelemetry } = storeMod;
  usageUi.resetForTests();
  ottoTelemetry.resetForTests();

  usageUi.view = 'otto';
  usageUi.telemetryHours = 168;
  ottoTelemetry.operationPage = 2;

  const unmountFirst = ottoTelemetry.mount({
    getScope: () => ({ hours: usageUi.telemetryHours }),
  });
  await new Promise((r) => setTimeout(r, 0));

  // Independent action flags: Analyze, Dismiss, and Toggle can overlap without blocking unrelated controls!
  const analyzePromise = ottoTelemetry.analyze();
  const dismissPromise = ottoTelemetry.dismiss('sug-2');
  const togglePromise = ottoTelemetry.toggleEnabled(false);
  assert.equal(ottoTelemetry.analyzing, true);
  assert.equal(ottoTelemetry.clearing, true);
  assert.equal(ottoTelemetry.dismissingId, 'sug-2');
  assert.equal(ottoTelemetry.togglingEnabled, true);
  assert.equal(ottoTelemetry.savingRetention, false);

  dismissGate.resolve();
  await dismissPromise;
  assert.equal(ottoTelemetry.clearing, false);
  assert.equal(ottoTelemetry.dismissingId, null);
  assert.equal(ottoTelemetry.analyzing, true);

  toggleGate.resolve({ enabled: false, retention_days: 7 });
  await togglePromise;
  assert.equal(ottoTelemetry.togglingEnabled, false);

  // User navigates away while Analyze is still running.
  unmountFirst();
  assert.equal(ottoTelemetry.analyzing, true);
  assert.equal(usageUi.view, 'otto');

  // Analyze finishes in the background while on another page.
  analyzeGate.resolve([{ id: 'sug-1', component: 'git', name: 'git.execute', dismissed: false }]);
  await analyzePromise;

  assert.equal(ottoTelemetry.analyzing, false);
  assert.equal(ottoTelemetry.suggestions.length, 1);
  assert.equal(ottoTelemetry.suggestions[0].id, 'sug-1');
  assert.equal(usageUi.telemetryHours, 168);
  assert.equal(ottoTelemetry.operationPage, 2);
  assert.ok(getCalls.includes('/telemetry/overview?hours=168'));

  // Returning to Otto Telemetry keeps the sub-view, filter window, page, and suggestions.
  const unmountSecond = ottoTelemetry.mount({
    getScope: () => ({ hours: usageUi.telemetryHours }),
  });
  assert.equal(usageUi.view, 'otto');
  assert.equal(ottoTelemetry.suggestions[0].id, 'sug-1');
  unmountSecond();
});

test('route resume points preserve Usage sub-views (#/usage/otto and #/usage/report)', () => {
  const routeKeys = loadSource(new URL('../src/lib/routeKeys.ts', import.meta.url), {});
  assert.equal(routeKeys.knownHash('#/usage'), true);
  assert.equal(routeKeys.knownHash('#/usage/otto'), true);
  assert.equal(routeKeys.knownHash('#/usage/report'), true);
  assert.equal(routeKeys.ephemeralHash('#/usage/otto'), false);

  const raw = JSON.stringify({
    ws1: {
      usage: '#/usage/otto',
      'skills-eval': '#/skills-eval/review',
      settings: '#/settings/providers',
    },
  });
  const resumed = routeKeys.parseResumePoints(raw, 'ws1');
  assert.equal(resumed.get('usage'), '#/usage/otto');
  assert.equal(resumed.get('skills-eval'), '#/skills-eval/review');
  assert.equal(resumed.get('settings'), '#/settings/providers');
});

test('skillsLabState and workflowsUi preserve filters, selections, and in-flight actions across navigation', async () => {
  const evalGate = deferred<any>();
  const reviewGate = deferred<any>();
  const wfGate = deferred<any>();

  const { skillsLabState } = loadSource(
    new URL('../src/lib/stores/skillsLabState.svelte.ts', import.meta.url),
    {
      '../api/skillsEval': {
        skillsEvalApi: { start: () => evalGate.promise },
      },
      '../api/skillReview': {
        skillReviewApi: { start: () => reviewGate.promise },
      },
      '../toast.svelte': { toasts: { success() {}, info() {}, error() {} } },
      '../toastError': { toastError() {} },
    },
  );
  skillsLabState.resetForTests();
  skillsLabState.browser.query = 'golang';
  skillsLabState.browser.selName = 'golang-testing';
  skillsLabState.browser.detailTab = 'edit';

  const evalPromise = skillsLabState.startEval('ws-1', {
    skill_name: 'golang-testing',
    skill_source: 'library',
  } as any);
  const reviewPromise = skillsLabState.startReview('ws-1', {
    skill_name: 'golang-testing',
    skill_source: 'library',
    providers: ['claude'],
    agent_mode: 'static',
    instructions: 'check paths',
  });
  assert.equal(skillsLabState.startingEval, true);
  assert.equal(skillsLabState.startingReview, true);

  evalGate.resolve({
    id: 'eval-1',
    workspace_id: 'ws-1',
    skill_name: 'golang-testing',
    skill_source: 'library',
    status: 'running',
    iterations: [],
    config: {},
  });
  reviewGate.resolve({
    id: 'rev-1',
    workspace_id: 'ws-1',
    skill_name: 'golang-testing',
    status: 'running',
  });
  await Promise.all([evalPromise, reviewPromise]);

  assert.equal(skillsLabState.evalFor('ws-1').selectedId, 'eval-1');
  assert.equal(skillsLabState.evalFor('ws-1').mode, 'detail');
  assert.equal(skillsLabState.reviewFor('ws-1').selected?.id, 'rev-1');
  assert.equal(skillsLabState.browser.query, 'golang');
  assert.equal(skillsLabState.browser.detailTab, 'edit');

  const remembered: Array<[string, string]> = [];
  const { workflowsUi } = loadSource(
    new URL('../src/modules/workflows/workflowsUiState.svelte.ts', import.meta.url),
    {
      '../../lib/api/client': {
        api: { post: () => wfGate.promise },
      },
      '../../lib/lastSelection': {
        rememberSelection: (k: string, id: string) => remembered.push([k, id]),
      },
      '../../lib/toast.svelte': { toasts: { success() {} } },
      '../../lib/toastError': { toastError() {} },
      '../../lib/api/workflows': { patchWorkflow() {} },
      '../../lib/confirm.svelte': { confirmer: {} },
    },
  );
  workflowsUi.resetForTests();
  workflowsUi.prompt = 'Summarize repo daily';
  workflowsUi.instructionsOpen = true;
  workflowsUi.selectedNodeByWorkflow.set('wf-existing', 'node-2');

  const genPromise = workflowsUi.generate('ws-1');
  assert.equal(workflowsUi.generating, true);
  wfGate.resolve({ id: 'wf-generated', workspace_id: 'ws-1', name: 'Daily summary', graph: { nodes: [], edges: [] } });
  const created = await genPromise;
  assert.equal(created?.id, 'wf-generated');
  assert.equal(workflowsUi.prompt, '');
  assert.equal(workflowsUi.instructionsOpen, true);
  assert.equal(workflowsUi.selectedNodeByWorkflow.get('wf-existing'), 'node-2');
  assert.deepEqual(remembered, [['workflows', 'wf-generated']]);
});

test('insightsStore silently refreshes on remount while preserving paged older reports and surfaces inline olderError', async () => {
  let listCalls = 0;
  let failOlder = true;
  let toastErrors = 0;
  const mkReport = (start: string) => ({
    kind: 'daily',
    period_start: start,
    period_end: start,
    generated_at: `${start}T08:00:00Z`,
    html_path: null,
    md_path: null,
    summary: `Summary ${start}`,
  });

  const { insightsStore } = loadSource(
    new URL('../src/modules/insights/insightsStore.svelte.ts', import.meta.url),
    {
      '../../lib/api/insights': {
        insightsApi: {
          getConfig: async () => null,
          activeRuns: async () => [],
          reportStatus: async () => ({ report: null, html_revision: null }),
          readText: async () => '{}',
          listReports: async (offset?: number) => {
            listCalls += 1;
            if (offset && offset > 0) {
              if (failOlder) throw new Error('archive offline');
              return [mkReport('2026-11-01')];
            }
            return listCalls === 1
              ? [mkReport('2026-11-14'), mkReport('2026-11-13')]
              : [mkReport('2026-11-15'), mkReport('2026-11-14'), mkReport('2026-11-13')];
          },
        },
      },
      '../../lib/confirm.svelte': { confirmer: { ask: async () => false } },
      '../../lib/toast.svelte': {
        toasts: {
          info() {},
          success() {},
          error() {
            toastErrors += 1;
          },
        },
      },
      '../../lib/toastError': { toastError() {} },
      './capabilities': { capabilitiesApi: { list: async () => [] } },
    },
    {},
    { relative: true },
  );

  insightsStore.resetForTests();
  await insightsStore.load(null, true);
  assert.equal(insightsStore.loading, false);
  assert.equal(insightsStore.loadedOnce, true);
  assert.equal(insightsStore.reports.length, 2);

  // Failing older page sets inline `olderError` instead of firing a toast.
  await insightsStore.loadOlderReports(2, 2);
  assert.match(insightsStore.olderError ?? '', /archive offline/);
  assert.equal(toastErrors, 0);

  // Retrying older page clears `olderError` and appends older archive rows.
  failOlder = false;
  await insightsStore.loadOlderReports(2, 2);
  assert.equal(insightsStore.olderError, null);
  assert.equal(insightsStore.reports.length, 3);
  insightsStore.archiveOffset = 10;

  // Remounting triggers silent SWR refresh (`loading === false`) and retains paged older rows!
  insightsStore.mount(null);
  assert.equal(insightsStore.loading, false, 'remount must keep rendered list visible without loading flash');
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(insightsStore.reports.length, 4);
  assert.equal(insightsStore.reports[0].period_start, '2026-11-15');
  assert.equal(insightsStore.reports[3].period_start, '2026-11-01');
});

test('mcpUiState preserves selectedServerId and expandedServerId across same-workspace remounts and resets on workspace switch', () => {
  const { mcpUiState } = loadSource(
    new URL('../src/modules/mcp/mcpUiState.svelte.ts', import.meta.url),
    {},
  );
  mcpUiState.resetForTests();

  assert.equal(mcpUiState.syncWorkspace('ws-1'), true);
  mcpUiState.setServers(
    [
      { id: 'srv-1', name: 'github' } as any,
      { id: 'srv-2', name: 'postgres' } as any,
    ],
    (items: Array<{ id: string }>) => items[0]?.id ?? null,
  );
  assert.equal(mcpUiState.selectedServerId, 'srv-1');

  // User picks `srv-2` and expands its details, then switches tabs/routes within `ws-1`.
  mcpUiState.selectServer('srv-2');
  mcpUiState.expandedServerId = 'srv-2';
  assert.equal(mcpUiState.syncWorkspace('ws-1'), false);
  assert.equal(mcpUiState.selectedServerId, 'srv-2');
  assert.equal(mcpUiState.expandedServerId, 'srv-2');

  // Background server reload keeps `srv-2` selected; removing `srv-2` falls back cleanly.
  mcpUiState.setServers(
    [{ id: 'srv-1', name: 'github' } as any, { id: 'srv-2', name: 'postgres' } as any],
    (items: Array<{ id: string }>) => items[0]?.id ?? null,
  );
  assert.equal(mcpUiState.selectedServerId, 'srv-2');

  mcpUiState.setServers(
    [{ id: 'srv-1', name: 'github' } as any],
    (items: Array<{ id: string }>) => items[0]?.id ?? null,
  );
  assert.equal(mcpUiState.selectedServerId, 'srv-1');
  assert.equal(mcpUiState.expandedServerId, null);

  // Switching workspaces resets workspace-scoped selection.
  assert.equal(mcpUiState.syncWorkspace('ws-2'), true);
  assert.equal(mcpUiState.selectedServerId, null);
  assert.equal(mcpUiState.loaded, false);
});
