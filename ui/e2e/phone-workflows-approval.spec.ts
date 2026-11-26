import { test, expect } from '@playwright/test';
import { expectFullyInViewport } from './helpers';
import { agentStep, chain, createWorkflow, needsYou, startRun, waitRun, wfApi, type WfApi } from './workflowsFixture';
import { openRun, runsTab } from './workflowsUi';

// Approving on a phone (ux.md §10, §16): approve by tap continues the run;
// Deny… opens its sheet fully in the viewport and stops the run; the row
// leaves Needs you. The step asks through the fake agent's `approve` verb.

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

async function waitingRun(): Promise<{ wf: string; run: string }> {
  const wf = await createWorkflow(api, 'E2E Phone approval', chain(agentStep('gate', 'Post the comments', ['approve Bash', 'write $HANDOFF done'])));
  const run = await startRun(api, wf);
  await waitRun(api, run, needsYou);
  return { wf, run };
}

test('Approve by tap: the run goes on and leaves Needs you', async ({ page }) => {
  const { wf, run } = await waitingRun();
  await runsTab(page);
  const row = page.getByTestId('wf-runs-needs').locator(`[data-run-id="${run}"]`);
  await expect(row).toContainText('Approval', { timeout: 20_000 });
  await row.getByRole('button').first().tap();
  const card = page.locator('[data-testid="wf-run-card"][data-kind="approval"]');
  await expect(card).toBeVisible();
  await card.getByRole('button', { name: 'Approve' }).tap();
  await expect(page.getByTestId('wf-run-page')).not.toHaveAttribute('data-state', 'needs_you', { timeout: 15_000 });
  await runsTab(page);
  await expect(page.getByTestId('wf-runs-needs').locator(`[data-run-id="${run}"]`)).toHaveCount(0);
  void wf;
});

test('Deny… opens a sheet inside the screen and stops the run', async ({ page }) => {
  const { wf, run } = await waitingRun();
  await openRun(page, wf, run);
  await page.locator('[data-testid="wf-run-card"][data-kind="approval"]').getByRole('button', { name: 'Deny…' }).tap();
  const sheet = page.getByRole('dialog');
  await expect(sheet).toBeVisible();
  await expectFullyInViewport(page, sheet);
  await sheet.getByRole('button', { name: /^Deny/ }).last().tap();
  await expect(page.getByTestId('wf-run-page')).toHaveAttribute('data-state', /failed|canceled/, { timeout: 20_000 });
});
