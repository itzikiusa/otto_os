import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// The second way a workflow is scheduled: a Scheduled task of kind `workflow`.
// Its Triggers panel says so ("Also started by scheduled task …") and links to
// the task, so "why did it run at 9?" has an answer on the workflow itself.

const V1 = '/api/v1';
let base = '';
let ctx: APIRequestContext;
let ws = '';

const node = (id: string, kind: string) => ({ id, kind, name: id, x: 0, y: 0, params: null });

test.beforeAll(async ({}, testInfo) => {
  if (testInfo.project.name !== 'desktop-browser') return;
  const a = await apiCtx();
  ctx = a.ctx;
  base = a.base;
  ws = await seedWorkspace(ctx, base, 'E2E Scheduled By');
});

test.afterAll(async () => {
  await ctx?.dispose();
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
});

test('a workflow launched by a scheduled task names it in Triggers, and the link opens the task', async ({ page }) => {
  const wf = await ctx.post(`${base}${V1}/workspaces/${ws}/workflows`, {
    data: {
      name: 'E2E Scheduled By Flow',
      description: 'e2e',
      graph: { nodes: [node('trigger', 'manual_trigger'), node('tail', 'log')], edges: [{ id: 'trigger-tail', source: 'trigger', target: 'tail' }] },
    },
  });
  expect(wf.ok(), await wf.text()).toBeTruthy();
  const wfId = (await wf.json()).id as string;

  const t = await ctx.post(`${base}${V1}/workspaces/${ws}/scheduled-tasks`, {
    data: {
      name: 'E2E Morning Launcher',
      kind: 'workflow',
      workflow_id: wfId,
      schedule: { cadence: 'daily', at: '09:00' },
      timezone: 'UTC',
      destination: { type: 'none' },
      // Paused: the test must not depend on (or be raced by) a real fire.
      enabled: false,
    },
  });
  expect(t.ok(), await t.text()).toBeTruthy();
  const taskId = (await t.json()).id as string;

  await page.goto(`/#/workflows/${wfId}`);
  await page.locator('[data-testid="page-header"]').getByRole('button', { name: 'More actions' }).click();
  await page.locator('.ctx-menu').getByRole('menuitemcheckbox', { name: 'Triggers', exact: true }).click();

  const line = page.getByTestId('scheduled-by');
  await expect(line).toBeVisible({ timeout: 15_000 });
  await expect(line).toContainText('Also started by scheduled task');
  await expect(line).toContainText('E2E Morning Launcher');
  await expect(line).toContainText('Daily at 09:00 · UTC · paused');

  await line.getByRole('button', { name: 'E2E Morning Launcher' }).click();
  await expect(page).toHaveURL(new RegExp(`#/scheduled-tasks/${taskId}$`));
  await expect(page.locator(`[data-task-id="${taskId}"]`)).toBeVisible({ timeout: 15_000 });
});
