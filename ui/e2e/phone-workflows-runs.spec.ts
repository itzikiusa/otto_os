import { test, expect } from '@playwright/test';
import { expectNoHorizontalOverflow } from './helpers';
import { agentStep, chain, createWorkflow, settled, startRun, waitRun, wfApi, type WfApi } from './workflowsFixture';
import { openRun, runRow, runsTab, stepPane, stepRow, workflowRow, workflowsTab } from './workflowsUi';

// Runs on a phone (ux.md §10, §16; test-plan.md §7, iphone-portrait is the
// gate candidate): the inbox's order and words, a running row opens the run
// page with the sentence, the ring and the current step on the first screen,
// tapping a step pushes the step screen with its conversation, back returns.
// Touch only (tap), light and dark, RTL, no horizontal scroll.

let api: WfApi;
test.beforeAll(async ({}, info) => {
  if (!(/phone|iphone/.test(info.project.name))) return;
  api = await wfApi();
});
test.afterAll(async () => {
  await api?.ctx.dispose();
});
test.beforeEach(async ({ page }, info) => {
  test.skip(!(/phone|iphone/.test(info.project.name)), 'not this project');
  await page.addInitScript((ws) => {
    localStorage.setItem('otto_workspace', ws as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, api.ws);
});

async function seed(): Promise<{ wf: string; running: string; done: string }> {
  const wf = await createWorkflow(api, 'E2E Phone runs', chain(agentStep('a', 'Gather'), agentStep('b', 'Work for a while', ['sleep 120000'])));
  const done = await startRun(api, wf);
  await waitRun(api, done, (r) => r.nodes.some((n) => n.node_id === 'b' && n.status === 'running'));
  await api.post(`/workflow-runs/${done}/cancel`);
  await waitRun(api, done, settled);
  const running = await startRun(api, wf);
  await waitRun(api, running, (r) => r.nodes.some((n) => n.node_id === 'b' && n.status === 'running'));
  return { wf, running, done };
}

for (const scheme of ['light', 'dark'] as const) test(`inbox → run → step → back with touch (${scheme})`, async ({ page }) => {
  await page.emulateMedia({ colorScheme: scheme });
  const { wf, running, done } = await seed();
  await runsTab(page);
  const live = runRow(page, running);
  await expect(page.getByTestId('wf-runs-running').locator(`[data-run-id="${running}"]`)).toBeVisible({ timeout: 20_000 });
  await expect(live).toHaveAttribute('data-state', 'running');
  await expect(live).toContainText(/Working/);
  await expect(page.getByTestId('wf-runs-recent').locator(`[data-run-id="${done}"]`)).toBeVisible();
  await expectNoHorizontalOverflow(page);

  await live.getByRole('button').first().tap();
  await expect(page).toHaveURL(new RegExp(`#/workflows/${wf}/runs/${running}$`));
  const vh = page.viewportSize()!.height;
  for (const id of ['wf-run-headline', 'wf-run-orbit']) {
    const b = (await page.getByTestId(id).boundingBox())!;
    expect(b.y + b.height, `${id} on the first screen`).toBeLessThanOrEqual(vh);
  }
  const current = stepRow(page, 'b');
  const cb = (await current.boundingBox())!;
  expect(cb.y, 'the current step on the first screen').toBeLessThan(vh);
  await expectNoHorizontalOverflow(page);

  await current.tap();
  await expect(page).toHaveURL(new RegExp(`#/workflows/${wf}/runs/${running}/b$`));
  await expect(stepPane(page)).toBeVisible();
  await expect(page.getByTestId('wf-step-tab-conversation')).toBeVisible();
  await page.getByRole('button', { name: 'Back to the run' }).tap();
  await expect(page).toHaveURL(new RegExp(`#/workflows/${wf}/runs/${running}$`));
  await api.post(`/workflow-runs/${running}/cancel`);
});

test('right-to-left: the inbox and the run page keep inside the screen', async ({ page }) => {
  const { wf, running } = await seed();
  await page.addInitScript(() => document.documentElement.setAttribute('dir', 'rtl'));
  await runsTab(page);
  await expect(runRow(page, running)).toBeVisible({ timeout: 20_000 });
  await expectNoHorizontalOverflow(page);
  await openRun(page, wf, running);
  await expectNoHorizontalOverflow(page);
  await api.post(`/workflow-runs/${running}/cancel`);
});

test('the Workflows tab lists the definitions; New workflow is not offered on a phone', async ({ page }) => {
  const { wf } = await seed();
  await workflowsTab(page);
  await expect(workflowRow(page, wf)).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole('button', { name: 'New workflow' })).toHaveCount(0);
});
