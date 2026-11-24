// Unit tests for Wave 2 (SHOULD) fixes (S-51..S-56) across workflows, personal-agents,
// scheduled-tasks, loops, mcp, plugins, usage, and insights.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { nextEdgeId, computeFitTransform } from '../src/modules/workflows/canvasGeometry.ts';
import { nextForOnce } from '../src/modules/personal-agents/cadence.ts';

test('S-51: nextEdgeId avoids collisions after deletions and computeFitTransform bounds nodes', () => {
  const existing = [{ id: 'e-a-b-1' }, { id: 'e-a-b-2' }];
  const id = nextEdgeId(existing, 'a', 'b');
  assert.notEqual(id, 'e-a-b-1');
  assert.notEqual(id, 'e-a-b-2');
  assert.equal(id, 'e-a-b-3');

  const fit = computeFitTransform(
    [
      { id: 'n1', kind: 'prompt', name: 'A', x: 1000, y: 500, params: {} },
      { id: 'n2', kind: 'prompt', name: 'B', x: 1600, y: 700, params: {} },
    ],
    800,
    500,
  );
  assert.ok(fit.scale > 0.35 && fit.scale <= 1);
  // First node's transformed left edge stays visible inside the viewport.
  const left = 1000 * fit.scale + fit.tx;
  const right = (1600 + 210) * fit.scale + fit.tx;
  assert.ok(left >= 0 && right <= 800, `expected [${left}, ${right}] inside [0, 800]`);
});

test('S-52: nextForOnce parses wall time in a timezone and RoutineForm validates interval/cron', async () => {
  const d = nextForOnce('2026-12-01T09:30', 'UTC');
  assert.ok(d instanceof Date);
  assert.equal(d?.toISOString(), '2026-12-01T09:30:00.000Z');
  assert.equal(nextForOnce('not-a-date', 'UTC'), null);

  let created = 0;
  const form = componentFunctions(
    new URL('../src/modules/personal-agents/RoutineForm.svelte', import.meta.url),
    ['save'],
    {
      busy: false,
      error: '',
      repeats: 'interval',
      days: [1],
      at: '09:00',
      tz: 'UTC',
      other: { cadence: 'interval', everyMin: 2, at: '09:00', weekday: 0, cronExpr: '0 9 *', runAt: '' },
      name: '',
      directive: 'Brief me',
      permission: 'directed',
      init: null,
      draft: undefined,
      agent: { id: 'a1', name: 'A', enabled: true },
      personalAgents: {
        schedulesByAgent: {},
        routineDrafts: {},
        createSchedule: async () => { created++; },
        updateSchedule: async () => { created++; },
      },
      scheduleForDays: () => ({}),
      buildCadence: (c: unknown) => c,
      joinDirective: (_n: string, d: string) => d,
      routineLabel: (x: string) => x,
      confirmer: { ask: async () => true },
      loadErrorText: (e: unknown) => String(e),
      onclose: () => {},
    },
  );

  await form.save();
  assert.match(form.error, /5 minutes/);
  assert.equal(created, 0);

  form.repeats = 'cron';
  await form.save();
  assert.match(form.error, /5 fields/);
  assert.equal(created, 0);
});

test('S-52: ScheduledTasksPage validates empty fPrompt inline for agent_prompt and shell', async () => {
  const failed: [string, string][] = [];
  const page = componentFunctions(
    new URL('../src/modules/scheduled-tasks/ScheduledTasksPage.svelte', import.meta.url),
    ['save'],
    {
      busy: false,
      error: '',
      fieldError: null,
      intervalError: '',
      ws: { currentId: 'ws-1' },
      fName: 'Disk check',
      fPrompt: '   ',
      fKind: 'agent_prompt',
      fProvider: 'shell',
      fWorkflowId: '',
      fDestType: 'none',
      fEmailTo: '',
      fUrl: '',
      fCadence: 'interval',
      fRunAt: '',
      tzOk: true,
      cronFieldCount: 5,
      intervalTooShort: () => false,
      failField: (id: string, msg: string) => { failed.push([id, msg]); },
      focusField: async () => {},
    },
  );
  await page.save();
  assert.equal(failed.length, 1);
  assert.equal(failed[0][0], 'sched-prompt');
  assert.match(failed[0][1], /shell command/i);

  page.fProvider = 'claude';
  await page.save();
  assert.equal(failed.length, 2);
  assert.match(failed[1][1], /prompt/i);
});

test('S-54: OttoServerHome loadGatewayTools drops superseded A->B->A responses via gatewaySeq', async () => {
  const resolvers: ((v: { tools: { name: string }[] }) => void)[] = [];
  const home = componentFunctions(
    new URL('../src/modules/mcp/OttoServerHome.svelte', import.meta.url),
    ['loadGatewayTools'],
    {
      wsId: 'ws-a',
      gatewaySeq: 0,
      gatewayTools: [],
      gatewayLoading: false,
      gatewayError: null,
      loadErrorText: (e: unknown) => String(e),
      mcpCpExtraApi: {
        gatewayTools: () => new Promise((resolve) => { resolvers.push(resolve); }),
      },
    },
  );

  const first = home.loadGatewayTools('ws-a');
  // Workspace switched away and back: sequence advances.
  const second = home.loadGatewayTools('ws-a');
  resolvers[1]({ tools: [{ name: 'fresh.tool' }] });
  await second;
  resolvers[0]({ tools: [{ name: 'stale.tool' }] });
  await first;

  assert.deepEqual(JSON.parse(JSON.stringify(home.gatewayTools)), [{ name: 'fresh.tool' }]);
  assert.equal(home.gatewayLoading, false);
});

test('S-56: BudgetsEditor saveBudgets confirms turning block_on_exceed on and names exceeded scopes', async () => {
  const asks: { msg: string; opts: { danger?: boolean } }[] = [];
  let savedCalls = 0;
  let allow = false;

  const page = componentFunctions(
    new URL('../src/modules/usage/BudgetsEditor.svelte', import.meta.url),
    ['saveBudgets'],
    {
      onclose: () => {},
      budgetValidation: null,
      budgetsDirty: true,
      budgetCfg: {
        enforce: true,
        block_on_exceed: true,
        window_days: 30,
        workspaces: [{ workspace_id: 'ws-1', monthly_usd: 50 }],
        providers: [],
      },
      usage: {
        savingBudgets: false,
        budgets: {
          config: { enforce: true, block_on_exceed: false, window_days: 30, workspaces: [], providers: [] },
          rows: [{ scope: 'workspace', key: 'ws-1', label: 'Core', exceeded: true }],
        },
        saveBudgets: async () => {
          savedCalls++;
          return true;
        },
      },
      wsName: (id: string) => (id === 'ws-1' ? 'Core Workspace' : id),
      confirmer: {
        ask: async (msg: string, opts: { danger?: boolean }) => {
          asks.push({ msg, opts });
          return allow;
        },
      },
    },
  );

  await page.saveBudgets();
  assert.equal(asks.length, 1);
  assert.match(asks[0].msg, /Core Workspace/);
  assert.equal(asks[0].opts.danger, true);
  assert.equal(savedCalls, 0);

  allow = true;
  await page.saveBudgets();
  assert.equal(savedCalls, 1);
  assert.equal(page.budgetsDirty, false);
});
