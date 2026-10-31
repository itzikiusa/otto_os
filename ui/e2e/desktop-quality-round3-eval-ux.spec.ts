import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDirtyRepo } from './seed';
import { expectNoHorizontalOverflow, expectFullyInViewport } from './helpers';

test.use({ serviceWorkers: 'block' });
let ctx: APIRequestContext, base: string, workspace: string, run: any;
async function post(path: string, data: unknown) {
  const response = await ctx.post(`${base}/api/v1${path}`, { data });
  expect(response.ok(), await response.text()).toBe(true);
  return response.json();
}
test.beforeEach(async ({ page }) => {
  ({ ctx, base } = await apiCtx());
  workspace = await seedWorkspace(ctx, base);
  const repo = await seedDirtyRepo(ctx, base, workspace);
  const golden = await post(`/workspaces/${workspace}/golden-tasks`, {
    name: 'Round 3 rating evidence', prompt: 'Keep command evidence when a rating changes.',
    skill: 'rating-evidence', test_cmd: 'true # cargo test', lint_cmd: 'true # cargo clippy',
  });
  const created = await post(`/workspaces/${workspace}/skill-evaluations`, {
    source: { kind: 'library', reference: '' }, task: '', impl_cli: '', validations: [],
    iterations: 1, mode: 'score_only', golden_task_id: golden.id, target: { kind: 'path', path: repo.dir },
  });
  await expect.poll(async () => {
    run = await (await ctx.get(`${base}/api/v1/skill-evaluations/${created.id}`)).json();
    return run.status;
  }, { timeout: 30_000 }).not.toBe('running');
  run = await post(`/skill-evaluations/${run.id}/iterations/${run.iterations[0].id}/rate`, { rating: 5, note: 'Initial rating' });
  await page.addInitScript(workspace => {
    localStorage.setItem('otto_workspace', workspace);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspace);
});
test.afterEach(async () => { await ctx?.dispose(); });
async function openRun(page: Page) {
  await page.goto('/#/skills-eval');
  await page.getByTestId('tab-evaluator').click();
  await expect(page.getByTestId('scorecard')).toBeVisible();
}

test('rating publication failure explains the pending state and offers in-place recovery', async ({ page }, info) => {
  const iteration = run.iterations[0];
  let pending: any = null;
  const ratePath = `**/skill-evaluations/${run.id}/iterations/${iteration.id}/rate`;
  await page.route(ratePath, async route => {
    pending = structuredClone(run);
    pending.best_iteration = null;
    pending.best_score = null;
    Object.assign(pending.iterations[0], { human_rating: 1, score: 0 });
    Object.assign(pending.iterations[0].scoring, { proof_status: 'pending', done_score: 0, composite: 82 });
    pending.iterations[0].scoring.human.rating = 1;
    pending.iterations[0].scoring.human.score = 20;
    await route.fulfill({ status: 503, json: { code: 'unavailable', message: 'Fixture proof publication unavailable' } });
  });
  await page.route(`**/skill-evaluations/${run.id}`, route => pending
    ? route.fulfill({ json: pending }) : route.continue());
  await openRun(page);
  await page.getByRole('button', { name: 'Rate 1 of 5', exact: true }).click();
  await expect(page.getByTestId('scorecard-proof')).toContainText('Pending');
  await expect(page.getByRole('button', { name: 'Promote winning skill' })).toHaveCount(0);
  await expect(page.getByText('Score update pending', { exact: true }).first()).toBeVisible();
  await expect(page.locator('.rd-summary')).toHaveCount(0);
  await expect(page.getByText('Provisional composite', { exact: true })).toBeVisible();
  const retry = page.locator('.score-pending').getByRole('button', { name: 'Retry', exact: true });
  await expect(retry).toBeVisible();
  await page.screenshot({ path: info.outputPath('rating-pending-desktop.png') });
  await page.setViewportSize({ width: 390, height: 844 });
  await retry.scrollIntoViewIfNeeded();
  await expectNoHorizontalOverflow(page);
  await expectFullyInViewport(page, retry);
  await page.screenshot({ path: info.outputPath('rating-pending-phone.png') });
  await page.unroute(ratePath);
  pending = null;
  await retry.click();
  await expect(page.getByTestId('scorecard-proof')).toContainText('Passed');
  await expect(page.locator('.rated')).toHaveText('1/5');
  await expect(retry).toHaveCount(0);
});
