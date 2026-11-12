import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// A run's own address and a trigger's last outcome, against the ISOLATED
// OTTO_E2E daemon (agent turns stubbed; these graphs run no agent at all):
//   - `#/workflows/<workflow id>/runs/<run id>` (the run header's "Copy link")
//     opens that workflow AND that run — not just the workflow.
//   - the Triggers panel says whether a trigger works: "Never fired" for a
//     fresh one, "Last fired … · started" once its webhook started a run.

const V1 = '/api/v1';
let base = '';
let ctx: APIRequestContext;
let ws = '';

const node = (id: string, kind: string, params?: unknown) => ({ id, kind, name: id, x: 0, y: 0, params: params ?? null });

async function createWorkflow(name: string): Promise<string> {
  const r = await ctx.post(`${base}${V1}/workspaces/${ws}/workflows`, {
    data: {
      name,
      description: 'e2e',
      graph: { nodes: [node('trigger', 'manual_trigger'), node('tail', 'log')], edges: [{ id: 'trigger-tail', source: 'trigger', target: 'tail' }] },
    },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

async function waitRun(runId: string): Promise<string> {
  const deadline = Date.now() + 60_000;
  for (;;) {
    const run = await (await ctx.get(`${base}${V1}/workflow-runs/${runId}`)).json();
    if (run.status !== 'running' && run.status !== 'pending') return run.status as string;
    if (Date.now() > deadline) throw new Error(`run ${runId} did not finish`);
    await new Promise((res) => setTimeout(res, 300));
  }
}

async function openTriggers(page: import('@playwright/test').Page): Promise<void> {
  await page.locator('[data-testid="page-header"]').getByRole('button', { name: 'More actions' }).click();
  await page.locator('.ctx-menu').getByRole('menuitemcheckbox', { name: 'Triggers', exact: true }).click();
}

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
});

test.beforeAll(async ({}, testInfo) => {
  if (testInfo.project.name !== 'desktop-browser') return;
  const a = await apiCtx();
  ctx = a.ctx;
  base = a.base;
  ws = await seedWorkspace(ctx, base);
});

test.afterAll(async () => {
  await ctx?.dispose();
});

test('a run deep link opens that workflow and that run', async ({ page }) => {
  const wfId = await createWorkflow('E2E Run Link');
  const runs: string[] = [];
  for (let i = 0; i < 2; i++) {
    const r = await ctx.post(`${base}${V1}/workflows/${wfId}/run`, { data: {} });
    expect(r.ok(), await r.text()).toBeTruthy();
    runs.push((await r.json()).id as string);
    expect(await waitRun(runs[i])).toBe('success');
  }
  // The OLDER run, so "the latest run" can't pass for it.
  await page.goto(`/#/workflows/${wfId}/runs/${runs[0]}`);
  await expect(page.locator('.timeline')).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId('run-status')).toBeVisible();
  const copy = page.getByTestId('run-copy-link');
  await expect(copy).toBeVisible();
  await expect(copy).toHaveAttribute('aria-label', 'Copy link to this run');
  // The page kept the run in its address (a reload lands on it again).
  await expect(page).toHaveURL(new RegExp(`#/workflows/${wfId}/runs/${runs[0]}$`));
  await page.reload();
  await expect(page.locator('.timeline')).toBeVisible({ timeout: 15_000 });
});

test('the Triggers panel says whether a trigger has fired', async ({ page }) => {
  const wfId = await createWorkflow('E2E Trigger Status');
  const created = await ctx.post(`${base}${V1}/workflows/${wfId}/triggers`, {
    data: { kind: 'webhook', spec: {}, enabled: true },
  });
  expect(created.ok(), await created.text()).toBeTruthy();
  const trigger = await created.json();

  await page.goto(`/#/workflows/${wfId}`);
  await openTriggers(page);
  await expect(page.getByTestId('trigger-status')).toHaveText('Never fired');

  const fired = await ctx.post(`${base}${V1}/workflows/${wfId}/webhook/${trigger.spec.token}`, { data: { note: 'e2e' } });
  expect(fired.ok(), await fired.text()).toBeTruthy();
  const row = (await (await ctx.get(`${base}${V1}/workflows/${wfId}/triggers`)).json())[0];
  expect(row.last_outcome).toBe('started');
  expect(row.last_fired_at).toBeTruthy();

  await page.reload();
  await openTriggers(page);
  await expect(page.getByTestId('trigger-status')).toContainText(/^Last fired .* · started$/);
});
