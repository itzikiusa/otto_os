import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

test('ottoTelemetry analyze completes across unmount and preserves view, hours and page', async () => {
  const analyzeGate = deferred<any[]>();
  const getCalls: string[] = [];
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
              return [{ id: 'sug-1', component: 'git', name: 'git.execute', dismissed: false }];
            }
            throw new Error(`unexpected GET ${url}`);
          },
          post: async (url: string) => {
            if (url === '/telemetry/analyze') return analyzeGate.promise;
            throw new Error(`unexpected POST ${url}`);
          },
          put: async () => ({ enabled: true, retention_days: 7 }),
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

  const { ottoTelemetry, usageUi } = storeMod;
  usageUi.resetForTests();
  ottoTelemetry.resetForTests();

  usageUi.view = 'otto';
  ottoTelemetry.hours = 168;
  ottoTelemetry.operationPage = 2;

  const unmountFirst = ottoTelemetry.mount();
  const analyzePromise = ottoTelemetry.analyze();
  assert.equal(ottoTelemetry.analyzing, true);

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
  assert.equal(ottoTelemetry.hours, 168);
  assert.equal(ottoTelemetry.operationPage, 2);
  assert.ok(getCalls.includes('/telemetry/overview?hours=168'));

  // Returning to Otto Telemetry keeps the sub-view, filter window, page, and suggestions.
  const unmountSecond = ottoTelemetry.mount();
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
