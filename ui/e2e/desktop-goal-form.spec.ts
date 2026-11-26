// The Goal form builds the spec and lands on the run page (goal.md §3.3,
// test-plan.md GL21.1); Insights hands off to the form and nothing starts until
// Start goal (GL13.1). Needs a first real run: the Workflows page must mount
// GoalHost (WorkflowsPage switch) and the goals route must exist.
import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { GOAL_RUN, GOAL_WF, serveGoalRun } from './fixtures/goalRun';

async function boot(page: import('@playwright/test').Page, staged?: { title: string; prompt: string }) {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript(({ ws, staged }) => {
    localStorage.setItem('otto_workspace', ws);
    if (staged) sessionStorage.setItem('otto:automation-draft', JSON.stringify({ target: 'goal', ...staged }));
  }, { ws, staged });
  await page.route(`**/api/v1/workspaces/${ws}/repos`, (r) => r.fulfill({ json: [{ id: 'r1', workspace_id: ws, name: 'app', path: '/src/app', remote_url: null, provider: null, git_account_id: null, created_at: '2026-11-20T09:00:00Z' }] }));
  await page.route('**/api/v1/repos/r1/branches', (r) => r.fulfill({ json: [{ name: 'main', is_current: true, upstream: null }] }));
  const started: unknown[] = [];
  await page.route(`**/api/v1/workspaces/${ws}/goals`, async (r) => {
    started.push(r.request().postDataJSON());
    await r.fulfill({ json: { workflow: { id: GOAL_WF }, run: { id: GOAL_RUN } } });
  });
  await serveGoalRun(page, ws, 'running');
  return { ws, started };
}

test('a goal from the form: criteria, where it works, Start goal lands on its run', async ({ page }) => {
  const { started } = await boot(page);
  await page.goto('/#/workflows');
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+k' : 'Control+k');
  await page.keyboard.type('New goal');
  await page.keyboard.press('Enter');
  const form = page.getByTestId('wf-goal-form');
  await expect(form).toBeVisible();
  await page.getByTestId('wf-goal-field-goal').fill('Reject malformed rows in the CSV import');
  await expect(page.getByTestId('wf-goal-field-title')).toHaveValue('Reject malformed rows in the CSV import');
  const row = page.getByTestId('wf-goal-criterion-row').first();
  await row.getByLabel(/what must be true/).fill('The import tests pass');
  await row.getByLabel(/How criterion 1 is checked/).selectOption('command');
  await row.getByLabel(/Command for criterion 1/).fill('go test ./import/...');
  await form.getByRole('button', { name: 'Add criterion' }).click();
  const second = page.getByTestId('wf-goal-criterion-row').nth(1);
  await second.getByLabel(/what must be true/).fill('The banner reads well');
  await second.getByLabel(/How criterion 2 is checked/).selectOption('human');
  await expect(page.getByTestId('wf-goal-will')).toContainText('isolated copy of app');
  await expect(page.getByTestId('wf-goal-will')).toContainText('Nothing is committed to your branches, pushed or posted.');
  await page.getByTestId('wf-goal-start').click();
  await expect.poll(() => started.length).toBe(1);
  expect(started[0]).toMatchObject({
    title: 'Reject malformed rows in the CSV import',
    criteria: [{ id: 'c1', verify: 'command', command: 'go test ./import/...' }, { id: 'c2', verify: 'human' }],
    work: { kind: 'build', repo_id: 'r1' },
    budget: { iterations: 5, active_s: 3600 },
  });
  await expect(page).toHaveURL(new RegExp(`#/workflows/${GOAL_WF}/runs/${GOAL_RUN}$`));
  await expect(page.getByTestId('wf-run-page')).toBeVisible();
});

test('Insights hands off a goal: the form opens filled and nothing starts', async ({ page }) => {
  const { started } = await boot(page, { title: 'Cut review wait', prompt: 'Cut review wait\nMetric: wait (40m → 10m).' });
  await page.goto('/#/workflows');
  await expect(page.getByTestId('wf-goal-form')).toBeVisible();
  await expect(page.getByTestId('wf-goal-field-goal')).toHaveValue(/Metric: wait/);
  await expect(page.getByTestId('wf-goal-field-title')).toHaveValue('Cut review wait');
  await page.waitForTimeout(300); // ui-guards: allow — proves no start request follows
  expect(started).toEqual([]);
});

test('on a phone the form is a sheet inside the viewport', async ({ page }) => {
  await boot(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#/workflows');
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+k' : 'Control+k');
  await page.keyboard.type('New goal');
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('wf-goal-form')).toBeVisible();
  await expectFullyInViewport(page, page.getByTestId('wf-goal-start'));
  await expectNoHorizontalOverflow(page);
});
