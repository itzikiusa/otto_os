import { test, expect } from '@playwright/test';
import { agentStep, chain, createWorkflow, settled, startRun, waitRun, wfApi, type WfApi } from './workflowsFixture';
import { openRun, stepRow } from './workflowsUi';

// A failed run leads with the failing step's OWN error (never "one or more
// nodes failed"), inside the first viewport, with Retry step (ux.md §16).

let api: WfApi;
test.beforeAll(async ({}, info) => {
  if (!(info.project.name === 'desktop-browser')) return;
  api = await wfApi();
});
test.afterAll(async () => {
  await api?.ctx.dispose();
});
test.beforeEach(async ({ page }, info) => {
  test.skip(!(info.project.name === 'desktop-browser'), 'not this project');
  await page.addInitScript((ws) => {
    localStorage.setItem('otto_workspace', ws as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, api.ws);
});

for (const size of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) test(`the error card is the step's own words and is on the first screen at ${size.width} px`, async ({ page }) => {
  await page.setViewportSize(size);
  const wf = await createWorkflow(api, 'E2E Signed out', chain(agentStep('a', 'Gather'), agentStep('b', 'Write the summary', ['fail auth'])));
  const run = await startRun(api, wf);
  const done = await waitRun(api, run, settled);
  expect(done.status).toBe('error');
  await openRun(page, wf, run);
  const card = page.locator('[data-testid="wf-run-card"][data-kind="error"]');
  await expect(card).toBeVisible();
  await expect(card).not.toContainText('one or more nodes failed');
  await expect(card).toContainText(/signed out|log ?in/i);
  const box = (await card.boundingBox())!;
  const vh = page.viewportSize()!.height;
  expect(box.y + box.height, 'the error card fits the first screen').toBeLessThanOrEqual(vh);
  await expect(page.getByTestId('wf-run-primary')).toHaveAttribute('data-action', 'retry');
  await expect(stepRow(page, 'b')).toHaveAttribute('data-status', 'error');
});
