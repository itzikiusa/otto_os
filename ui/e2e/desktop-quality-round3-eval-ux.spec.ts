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
for (const scheme of ['light', 'dark'] as const) {
  test(`rating refreshes open proof evidence and remains usable on phone ${scheme}`, async ({ page }, info) => {
    await page.addInitScript(scheme => localStorage.setItem('otto_scheme', scheme), scheme);
    await page.setViewportSize({ width: 1440, height: 900 });
    await openRun(page);
    await page.getByTestId('scorecard-proofpack-btn').click();
    const approval = page.locator('.scorecard .artifact').filter({ hasText: 'Human rating' });
    await expect(approval).toContainText('rating: 5/5');
    await page.getByRole('button', { name: 'Rate 1 of 5', exact: true }).click();
    await expect(page.locator('.rated')).toHaveText('1/5');
    await expect(approval).toContainText('rating: 1/5');
    await expect(approval).not.toContainText('rating: 5/5');
    await page.screenshot({ path: info.outputPath(`rating-desktop-${scheme}.png`) });
    await page.setViewportSize({ width: 390, height: 844 });
    await page.getByRole('button', { name: 'Rate 1 of 5', exact: true }).scrollIntoViewIfNeeded();
    await expectNoHorizontalOverflow(page);
    await expectFullyInViewport(page, page.getByRole('button', { name: 'Rate 1 of 5', exact: true }));
    await page.screenshot({ path: info.outputPath(`rating-phone-${scheme}.png`) });
  });
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

test('proof load failure stays inline and keyboard Retry restores its evidence', async ({ page }) => {
  let fail = true;
  await page.route(`**/skill-evaluations/${run.id}/iterations/${run.iterations[0].id}/proof-pack`, route => fail
    ? route.fulfill({ status: 503, json: { code: 'unavailable', message: 'Fixture evidence unavailable' } })
    : route.continue());
  await openRun(page);
  await page.getByTestId('scorecard-proofpack-btn').click();
  await expect(page.locator('.scorecard').getByTestId('load-error')).toBeVisible();
  fail = false;
  await page.locator('.scorecard').getByRole('button', { name: 'Retry', exact: true }).press('Enter');
  await expect(page.locator('.scorecard .artifact').filter({ hasText: 'Human rating' })).toContainText('rating: 5/5');
});

test('a late proof response cannot restore evidence from before the rating change', async ({ page }) => {
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  let captured = false;
  let first = true;
  await page.route(`**/skill-evaluations/${run.id}/iterations/${run.iterations[0].id}/proof-pack`, async route => {
    if (!first) return route.continue();
    first = false;
    const response = await route.fetch();
    const old = await response.json();
    captured = true;
    await held;
    await route.fulfill({ json: old });
  });
  try {
    await openRun(page);
    await page.getByTestId('scorecard-proofpack-btn').click();
    await expect.poll(() => captured).toBe(true);
    await page.getByRole('button', { name: 'Rate 1 of 5', exact: true }).click();
    const approval = page.locator('.scorecard .artifact').filter({ hasText: 'Human rating' });
    await expect(approval).toContainText('rating: 1/5');
    const response = page.waitForResponse(`**/skill-evaluations/${run.id}/iterations/${run.iterations[0].id}/proof-pack`);
    release();
    await response;
    await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
    await expect(approval).toContainText('rating: 1/5');
    await expect(approval).not.toContainText('rating: 5/5');
  } finally { release(); }
});

test('workspace switch replaces the open Proof pack with the new workspace contents', async ({ page }) => {
  const second = await seedWorkspace(ctx, base, 'Round 3 second workspace');
  const firstPack = await post(`/workspaces/${workspace}/proof-packs`, {
    work_item_kind: 'manual', work_item_id: `round3-first-${workspace}`, title: 'First workspace proof',
  });
  await post(`/workspaces/${second}/proof-packs`, {
    work_item_kind: 'manual', work_item_id: `round3-second-${second}`, title: 'Second workspace proof',
  });
  await page.addInitScript(() => localStorage.setItem('otto_rail_expanded', '1'));
  await page.goto(`/#/proof/${firstPack.id}`);
  const title = page.getByTestId('page-header').locator('h1');
  await expect(title).toHaveText('First workspace proof');
  await page.getByTestId('agents-current-ws').click();
  const filter = page.getByPlaceholder('Switch workspace…');
  if (await filter.count()) await filter.fill('Round 3 second workspace');
  await page.getByRole('menuitemcheckbox', { name: 'Round 3 second workspace', exact: true }).click();
  await expect(page.getByTestId('agents-current-ws')).toContainText('Round 3 second workspace');
  await expect(title).toHaveText('Second workspace proof');
  await expect(page.getByText('First workspace proof', { exact: true })).toHaveCount(0);
  await expect(page).not.toHaveURL(new RegExp(firstPack.id));
});
