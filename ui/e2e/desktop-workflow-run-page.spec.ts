import { test, expect } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { agentStep, chain, createWorkflow, fork, reviewStep, settled, startRun, waitRun, wfApi, type WfApi } from './workflowsFixture';
import { openRun, runRow, runsTab } from './workflowsUi';

// The run page (ux.md §4, §5, §16; test-plan.md §7, gate candidate) on the
// isolated daemon with fake agents: a 12-reviewer fan-out as named rows and a
// ring with one tick each, a parallel group as one block, the header's one
// primary per state, Retry step, Stop run's confirm, the old link format.

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

test('a 12-reviewer fan-out: named reviewer rows, one ring tick each, no "Open session" wall', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Fan-out 12', chain(agentStep('prep', 'Prepare'), reviewStep('review', 12)));
  const run = await startRun(api, wf);
  await waitRun(api, run, (r) => (r.nodes.find((n) => n.node_id === 'review')?.children?.length ?? 0) >= 12);
  await openRun(page, wf, run, 'review');
  await expect(page.getByTestId('wf-run-orbit')).toHaveAttribute('data-children', '12');
  await expect(page.getByTestId('wf-run-orbit')).toHaveAttribute('data-grouped', 'false');
  // Reviewers are named rows (done ones may sit under their "N reviewers done" fold).
  const kids = page.getByTestId('wf-step-child');
  expect(await kids.count()).toBeGreaterThan(0);
  for (const k of await kids.all()) await expect(k).toContainText(/lens-\d+/);
  await expect(page.getByRole('button', { name: 'Open session' })).toHaveCount(0);
});

test('a parallel group renders as one block and the steps follow execution order', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Fork 3', fork([agentStep('a', 'Alpha'), agentStep('b', 'Beta'), agentStep('c', 'Gamma')], agentStep('join', 'Join')));
  const run = await startRun(api, wf);
  await waitRun(api, run, settled);
  await openRun(page, wf, run);
  const group = page.getByTestId('wf-step-group');
  await expect(group).toHaveCount(1);
  await expect(group.getByTestId('wf-step')).toHaveCount(3);
  const order = await page.getByTestId('wf-step').evaluateAll((els) => els.map((e) => e.getAttribute('data-node-id')));
  expect(order.indexOf('join')).toBeGreaterThan(Math.max(order.indexOf('a'), order.indexOf('b'), order.indexOf('c')));
});

test('one primary per state: Retry step on a failure (and it retries), Run again… when done', async ({ page }) => {
  const failing = await createWorkflow(api, 'E2E Fails once', chain(agentStep('flaky', 'Flaky step', ['fail-attempts 1 provider'])));
  const run = await startRun(api, failing);
  await waitRun(api, run, settled);
  await openRun(page, failing, run);
  const primary = page.getByTestId('wf-run-primary');
  await expect(primary).toHaveAttribute('data-action', 'retry');
  await expect(primary).toHaveText(/Retry step/);
  await primary.click();
  await expect(page.getByTestId('wf-run-page')).toHaveAttribute('data-state', 'succeeded', { timeout: 30_000 });
  await expect(page.getByTestId('wf-run-primary')).toHaveAttribute('data-action', 'run-again');
});

test('Stop run… asks first; Stop is never beside a primary', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Long step', chain(agentStep('slow', 'Slow step', ['sleep 60000'])));
  const run = await startRun(api, wf);
  await waitRun(api, run, (r) => r.status === 'running');
  await openRun(page, wf, run);
  await expect(page.getByTestId('wf-run-primary')).toHaveCount(0);
  await page.getByTestId('wf-run-stop').click();
  const dialog = page.getByRole('dialog', { name: 'Stop run?' });
  await expect(dialog).toBeVisible();
  await expectFullyInViewport(page, dialog);
  await dialog.getByRole('button', { name: 'Stop run' }).click();
  await expect(page.getByTestId('wf-run-page')).toHaveAttribute('data-state', 'canceled', { timeout: 30_000 });
});

test('the old link format and the inbox row both land on the run page', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Link', chain(agentStep('one', 'One step')));
  const run = await startRun(api, wf);
  await waitRun(api, run, settled);
  await page.goto(`/#/workflows/${wf}/runs/${run}`);
  await expect(page.locator(`[data-testid="wf-run-page"][data-run-id="${run}"]`)).toBeVisible({ timeout: 20_000 });
  await runsTab(page);
  await runRow(page, run).getByRole('button').first().click();
  await expect(page).toHaveURL(new RegExp(`#/workflows/${wf}/runs/${run}$`));
  await expectNoHorizontalOverflow(page);
});

test('files sent with the message show under Result → Input (one not downloaded)', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Attachments', chain(agentStep('one', 'One step')));
  const run = await startRun(api, wf, { msg: 'see the files' });
  await waitRun(api, run, settled);
  await page.route(`**/api/v1/workflow-runs/${run}`, async (route) => {
    const res = await route.fetch();
    const body = { ...(await res.json()), attachments: [
      { name: 'trace.log', mime: 'text/plain', size_bytes: 2048, source: 'start' },
      { name: 'screen.png', mime: 'image/png', source: 'steer', error: 'too large' },
    ] };
    await route.fulfill({ response: res, json: body });
  });
  await openRun(page, wf, run);
  await page.getByTestId('wf-step-tab-input').click();
  await expect(page.getByTestId('wf-run-attachment')).toHaveCount(2);
  await expect(page.locator('[data-testid="wf-run-attachment"][data-error="true"]')).toContainText('not downloaded');
});
