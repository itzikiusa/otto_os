import { test, expect } from '@playwright/test';
import { agentStep, chain, createWorkflow, needsYou, settled, startRun, waitRun, wfApi, type WfApi } from './workflowsFixture';
import { openRun, runsTab } from './workflowsUi';

// Needs you (ux.md §2.1, §4.3, §12; test-plan.md §7): a question answered
// from the run page; the usage-limit card (no buttons, the run continues at
// the reset time) and the unknown-reset failure with Retry step; the
// approval card; the sidebar badge counts waiting runs.

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

test('a question waits in Needs you and is answered in the step\'s conversation', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Question', chain(agentStep('a', 'Ask the person', ['ask Which environment?', 'write $HANDOFF done'])));
  const run = await startRun(api, wf);
  await waitRun(api, run, needsYou);
  await runsTab(page);
  await expect(page.getByTestId('wf-runs-needs').locator(`[data-run-id="${run}"]`)).toContainText('Question', { timeout: 20_000 });
  await openRun(page, wf, run);
  const card = page.locator('[data-testid="wf-run-card"][data-kind="question"]');
  await expect(card).toContainText('Which environment?');
  await card.getByRole('button', { name: 'Answer in the conversation' }).click();
  await expect(page.getByTestId('wf-step-tab-conversation')).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByTestId('wf-step-conversation')).toBeVisible();
});

test('a usage limit with a known reset has no buttons; the run says when it goes on', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Limited', chain(agentStep('a', 'Hit the limit', ['fail usage_limit'])));
  const run = await startRun(api, wf);
  const r = await waitRun(api, run, (x) => needsYou(x) || settled(x));
  await openRun(page, wf, run);
  if (r.overview?.attention?.[0]?.kind === 'usage_limit') {
    const card = page.locator('[data-testid="wf-run-card"][data-kind="limit"]');
    await expect(card).toBeVisible();
    await expect(card.getByRole('button')).toHaveCount(0);
    await expect(page.getByTestId('wf-run-sub')).toContainText('Otto continues it at');
  } else {
    // An unknown reset time fails the step: Retry step is the primary.
    await expect(page.getByTestId('wf-run-primary')).toHaveAttribute('data-action', 'retry');
  }
});

test('the sidebar badge counts runs that wait on a person', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Badge', chain(agentStep('a', 'Approve me', ['approve Bash'])));
  const run = await startRun(api, wf);
  await waitRun(api, run, needsYou);
  await runsTab(page);
  await expect(page.getByTestId('wf-tab-runs')).toContainText(/\d/);
  await expect(page.getByTestId('sidebar-modules').getByRole('button', { name: /Workflows.*waiting on you/ })).toBeVisible();
  await api.post(`/workflow-runs/${run}/cancel`);
});
