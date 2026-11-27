// A goal run on a phone with real touch (test-plan.md GL8.1): record a
// verification from the Verify card, Extend and resume from the hold card;
// everything stays inside the viewport. Mobile projects (touch devices).
// Needs a first real run: the run page must be routed (WorkflowsPage switch).
import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { GOAL_RUN, GOAL_WF, MULTI_LOOP_RUN, MULTI_LOOP_WF, serveGoalRun, serveMultiLoopGoalRun } from './fixtures/goalRun';

// Once sw.js claims the page its fetches bypass `page.route`, and the run
// served below never reaches the run page.
test.use({ serviceWorkers: 'block' });

async function open(page: import('@playwright/test').Page, state: 'needs_you' | 'exhausted') {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript((ws) => localStorage.setItem('otto_workspace', ws), ws);
  const served = await serveGoalRun(page, ws, state);
  await page.goto(`/#/workflows/${GOAL_WF}/runs/${GOAL_RUN}`);
  await expect(page.getByTestId('wf-run-page')).toHaveAttribute('data-state', state);
  return served;
}

test('record a person check from the Verify card', async ({ page }) => {
  const { writes } = await open(page, 'needs_you');
  await expect(page.getByTestId('wf-run-card')).toHaveAttribute('data-kind', 'verify');
  await page.getByLabel('What you verified').fill('Opened the import page; the banner names the bad row.');
  const record = page.getByTestId('wf-verify-record');
  await expectFullyInViewport(page, record);
  await record.tap();
  await expect.poll(() => writes).toEqual([{ path: `/workflow-runs/${GOAL_RUN}/nodes/loop/criteria/ui/verify`, body: { met: true, evidence: 'Opened the import page; the banner names the bad row.' } }]);
  await expectNoHorizontalOverflow(page);
});

test('Extend and resume from the hold card, the sheet inside the viewport', async ({ page }) => {
  const { writes } = await open(page, 'exhausted');
  await page.getByTestId('wf-hold-extend').tap();
  const sheet = page.getByRole('dialog', { name: 'Extend and resume' });
  const go = sheet.getByRole('button', { name: 'Extend and resume', exact: true });
  await expectFullyInViewport(page, go);
  await go.tap();
  await expect.poll(() => writes.map((w) => w.path)).toEqual([`/workflow-runs/${GOAL_RUN}/resume`]);
  await expectNoHorizontalOverflow(page);
});

test('loop rounds and waiting-thread links fit inside a phone viewport without horizontal overflow', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript((ws) => localStorage.setItem('otto_workspace', ws), ws);
  await serveMultiLoopGoalRun(page, ws);
  await page.goto(`/#/workflows/${MULTI_LOOP_WF}/runs/${MULTI_LOOP_RUN}`);
  await expect(page.getByTestId('wf-run-page')).toHaveAttribute('data-state', 'running');
  await expect(page.locator('[data-testid="wf-loop-rounds"][data-loop-id="ui"] [data-testid="wf-loop-round"]')).toHaveCount(3);
  await expect(page.getByTestId('wf-step-waiting').first()).toContainText('Security · Claude');
  await expectNoHorizontalOverflow(page);
});
