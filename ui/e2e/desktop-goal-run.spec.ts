// One screen answers "is my goal done?" (goal.md §3.5, test-plan.md GL8.1): the
// goal card with Met / Not met / Verify, the method chips, the evidence line and
// the command output tail in mono, the budget line; the hold card's actions.
// Needs a first real run: the run page must be routed (WorkflowsPage switch).
import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';
import { GOAL_RUN, GOAL_WF, MULTI_LOOP_RUN, MULTI_LOOP_WF, serveGoalRun, serveMultiLoopGoalRun } from './fixtures/goalRun';

async function open(page: import('@playwright/test').Page, scheme: 'light' | 'dark', state: Parameters<typeof serveGoalRun>[2]) {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript(({ ws, scheme }) => { localStorage.setItem('otto_workspace', ws); localStorage.setItem('otto_scheme', scheme); }, { ws, scheme });
  const served = await serveGoalRun(page, ws, state);
  await page.goto(`/#/workflows/${GOAL_WF}/runs/${GOAL_RUN}`);
  await expect(page.getByTestId('wf-run-page')).toHaveAttribute('data-state', state);
  return served;
}

for (const scheme of ['light', 'dark'] as const) {
  test(`a running goal shows each criterion with its evidence and the budget (${scheme})`, async ({ page }) => {
    await open(page, scheme, 'running');
    await expect(page.getByTestId('wf-run-card')).toHaveAttribute('data-kind', 'goal');
    const rows = page.getByTestId('wf-run-card').getByTestId('wf-goal-criterion');
    await expect(rows).toHaveCount(3);
    await expect(rows.nth(0)).toHaveAttribute('data-met', 'false');
    await expect(rows.nth(0)).toHaveAttribute('data-method', 'command');
    await expect(rows.nth(1)).toHaveAttribute('data-met', 'true');
    await expect(rows.nth(2)).not.toHaveAttribute('data-met', /.+/);
    await expect(rows.nth(0)).toContainText('Not met');
    await expect(rows.nth(1)).toContainText('Met');
    await expect(rows.nth(1)).toContainText('Agent');
    // The command's output tail opens under its headline, in mono.
    await rows.nth(0).getByText('verify command failed (exit 1)').click();
    const out = rows.nth(0).locator('pre');
    await expect(out).toContainText('FAIL: TestRejectsBlankRow');
    expect(await out.evaluate((el) => getComputedStyle(el).fontFamily)).toMatch(/mono|Menlo|SF Mono|Courier/i);
    await expect(page.getByTestId('wf-goal-budget').first()).toHaveText(/Iteration 4 of 6 · 1h 12m of 2h active/);
    await expect(page.getByTestId('wf-run-notes')).toHaveText(/1 note waiting for the next step/);
    await expectNoHorizontalOverflow(page);
  });
}

test('out of budget: the hold card offers only Extend and resume, proposed above what is used', async ({ page }) => {
  const { writes } = await open(page, 'light', 'exhausted');
  await expect(page.getByTestId('wf-run-card')).toHaveAttribute('data-kind', 'hold');
  await expect(page.getByTestId('wf-hold-resume')).toHaveCount(0);
  await expect(page.getByTestId('wf-run-primary')).toHaveCount(0);
  await page.getByTestId('wf-hold-extend').click();
  const dialog = page.getByRole('dialog', { name: 'Extend and resume' });
  await expect(dialog.getByLabel('Iterations')).toHaveValue('9');
  await expect(dialog.getByLabel('Active time (minutes)')).toHaveValue('180');
  await dialog.getByRole('button', { name: 'Extend and resume', exact: true }).click();
  await expect.poll(() => writes).toEqual([{ path: `/workflow-runs/${GOAL_RUN}/resume`, body: { extend: { iterations: 9, active_s: 10_800 } } }]);
});

test('blocked: Resume in the card, a note first, no header primary', async ({ page }) => {
  await open(page, 'light', 'blocked');
  await expect(page.getByTestId('wf-hold-resume')).toBeVisible();
  await expect(page.getByTestId('wf-hold-note')).toBeVisible();
  await expect(page.getByTestId('wf-run-primary')).toHaveCount(0);
});

test('two review-and-fix loops with the same criterion id show each meter, trend, blocker, and headline', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript((ws) => { localStorage.setItem('otto_workspace', ws); localStorage.setItem('otto_scheme', 'light'); }, ws);
  await serveMultiLoopGoalRun(page, ws);
  await page.goto(`/#/workflows/${MULTI_LOOP_WF}/runs/${MULTI_LOOP_RUN}`);
  await expect(page.getByTestId('wf-run-page')).toHaveAttribute('data-state', 'running');
  await expect(page.getByTestId('wf-run-page')).toContainText('Round 3 of 5 — UI 72 / 95, server 81 / 95');
  const rows = page.getByTestId('wf-run-card').getByTestId('wf-goal-criterion');
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toHaveAttribute('data-step-id', 'ui.1');
  await expect(rows.nth(0).getByTestId('wf-criterion-ratio')).toHaveText('72 / 95');
  await expect(rows.nth(0).getByTestId('wf-criterion-trend')).toHaveText('35 → 58 → 72');
  await expect(rows.nth(0).getByTestId('wf-criterion-blocker')).toHaveText('3 must-fix findings left');
  await expect(rows.nth(0).getByRole('progressbar')).toHaveAttribute('aria-valuenow', '72');
  await expect(rows.nth(1)).toHaveAttribute('data-step-id', 'server.1');
  await expect(rows.nth(1).getByTestId('wf-criterion-ratio')).toHaveText('81 / 95');
  await expect(rows.nth(1).getByTestId('wf-criterion-trend')).toHaveText('60 → 80 → 81');
  await expect(rows.nth(1).getByTestId('wf-criterion-blocker')).toHaveText('1 must-fix finding left');
  await expectNoHorizontalOverflow(page);
});
