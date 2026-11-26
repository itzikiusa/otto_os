import { test, expect } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { agentStep, cancelStarted, chain, createWorkflow, fork, reviewStep, settled, startRun, waitRun, wfApi, type WfApi } from './workflowsFixture';
import { openRun, runRow, runsTab, stepPane, stepRow } from './workflowsUi';

// The run page (ux.md §4, §5, §16; test-plan.md §7, gate candidate) on the
// isolated daemon with fake agents: a 12-reviewer fan-out as named rows and a
// ring with one tick each, a parallel group as one block, the header's one
// primary per state, Retry step, Stop run's confirm, the old link format.

let api: WfApi;
test.beforeAll(async ({}, info) => {
  if (!(info.project.name === 'desktop-browser')) return;
  api = await wfApi();
});
test.afterEach(async () => {
  await cancelStarted(api);
});
test.afterAll(async () => {
  await api?.ctx.dispose();
});

/** The open step's ⋯ menu items. */
async function stepMenu(page: import('@playwright/test').Page): Promise<string[]> {
  await stepPane(page).getByRole('button', { name: 'Step actions' }).click();
  const items = await page.locator('.ctx-menu').getByRole('menuitem').allTextContents();
  await page.keyboard.press('Escape');
  return items.map((t) => t.trim());
}
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
  // Only a failed step re-runs alone (CR1-044).
  await expect(stepPane(page)).toHaveAttribute('data-node-id', 'flaky');
  expect(await stepMenu(page)).toContain('Retry only this step');
  await primary.click();
  await expect(page.getByTestId('wf-run-page')).toHaveAttribute('data-state', 'succeeded', { timeout: 30_000 });
  await expect(page.getByTestId('wf-run-primary')).toHaveAttribute('data-action', 'run-again');
  await stepRow(page, 'flaky').click();
  await expect(stepPane(page).getByTestId('wf-step-error')).toHaveCount(0);
  const items = await stepMenu(page);
  expect(items).toContain('Retry from here');
  expect(items).not.toContain('Retry only this step');
});

test('a second failed step says its own failure in the pane; the card keeps the first (CR1-047)', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Two failures', fork([agentStep('p', 'Provider down', ['fail provider']), agentStep('a', 'Signed out', ['fail auth'])], agentStep('join', 'Join')));
  const run = await startRun(api, wf);
  const done = await waitRun(api, run, settled);
  const first = done.overview?.failed_step?.node_id;
  const other = done.nodes.find((n) => n.status === 'error' && n.node_id !== first);
  expect(first && other, JSON.stringify(done.nodes.map((n) => [n.node_id, n.status]))).toBeTruthy();
  await openRun(page, wf, run);
  await expect(page.getByTestId('wf-run-card')).toHaveAttribute('data-kind', 'error');
  // The card's own step: its failure is the card's, not repeated in the pane.
  await stepRow(page, first!).click();
  await expect(stepPane(page).getByTestId('wf-step-error')).toHaveCount(0);
  await stepRow(page, other!.node_id).click();
  await expect(stepPane(page).getByTestId('wf-step-error')).toContainText(other!.node_id === 'a' ? 'Not logged in' : 'API Error: 500');
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

test('a settled step\'s conversation is read-only: no Revive, Stop or limit card (CR1-038)', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Read-only conversation', chain(agentStep('one', 'One step')));
  const run = await startRun(api, wf);
  await waitRun(api, run, settled);
  await openRun(page, wf, run, 'one');
  await expect(page.getByTestId('wf-step-conversation')).toBeVisible({ timeout: 20_000 });
  // The step persona's answer (fake_script.rs): the real thread, read-only.
  await expect(page.getByTestId('wf-step-conversation').getByText('step done').first()).toBeVisible();
  for (const id of ['runtime-closed', 'runtime-reopen', 'runtime-background', 'runtime-background-stop-all']) await expect(page.getByTestId(id)).toHaveCount(0);
});

test('switching from a step whose thread is gone to a live one shows the live conversation (CR1-039)', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Gone then live', chain(agentStep('gone', 'Gone step'), agentStep('kept', 'Kept step')));
  const run = await startRun(api, wf);
  const done = await waitRun(api, run, settled);
  const goneThread = done.nodes.find((n) => n.node_id === 'gone')?.thread?.thread_id;
  expect(goneThread, 'the step ran on a thread').toBeTruthy();
  await page.route(`**/api/v1/runtime-sessions/${goneThread}**`, (r) => r.fulfill({ status: 404, json: { error: 'not found' } }));
  await openRun(page, wf, run, 'gone');
  await expect(page.getByTestId('wf-step-digest')).toBeVisible({ timeout: 20_000 });
  await stepRow(page, 'kept').click();
  await expect(page.getByTestId('wf-step-conversation')).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId('wf-step-digest')).toHaveCount(0);
});

test('files sent with the message show under Result → Input: a text one previews, any downloads (CR1-043)', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Attachments', chain(agentStep('one', 'One step')));
  const run = await startRun(api, wf, { msg: 'see the files' });
  await waitRun(api, run, settled);
  await page.route(`**/api/v1/workflow-runs/${run}`, async (route) => {
    const res = await route.fetch();
    const body = { ...(await res.json()), attachments: [
      { name: 'trace.log', mime: 'text/plain', size_bytes: 2048, path: 'attachments/trace.log', source: 'start' },
      { name: 'screen.png', mime: 'image/png', source: 'steer', error: 'too large' },
    ] };
    await route.fulfill({ response: res, json: body });
  });
  // The run's context read (api.md: `…/context/read?path=…[&download=1]`).
  const reads: string[] = [];
  await page.route(`**/api/v1/workflow-runs/${run}/context/read?*`, async (route) => {
    reads.push(route.request().url());
    if (route.request().url().includes('download=1')) await route.fulfill({ status: 200, contentType: 'text/plain', body: 'line one\nline two' });
    else await route.fulfill({ json: { path: 'attachments/trace.log', content: 'line one\nline two', language: 'text', truncated: false } });
  });
  await openRun(page, wf, run);
  await page.getByTestId('wf-step-tab-input').click();
  await expect(page.getByTestId('wf-run-attachment')).toHaveCount(2);
  const failed = page.locator('[data-testid="wf-run-attachment"][data-error="true"]');
  await expect(failed).toContainText('not downloaded');
  await expect(failed.getByRole('button')).toHaveCount(0);
  const trace = page.locator('[data-testid="wf-run-attachment"]').filter({ hasText: 'trace.log' });
  await trace.getByRole('button', { name: 'Preview' }).click();
  await expect(page.getByTestId('wf-run-attachment-preview')).toContainText('line two');
  await trace.getByRole('button', { name: 'Download' }).click();
  await expect.poll(() => reads.some((u) => u.includes('path=attachments%2Ftrace.log') && u.includes('download=1'))).toBe(true);
});
