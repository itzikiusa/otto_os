import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedGitRepo } from './seed';

test.use({ serviceWorkers: 'block', viewport: { width: 390, height: 844 } });

test('finding reasons persist and a failed proof modal offers retry without export', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  const { repoId } = await seedGitRepo(ctx, base, ws);
  const reviewResponse = await ctx.post(`${base}/api/v1/workspaces/${ws}/__e2e/review`, {
    data: { repo_id: repoId, pr_number: 0, status: 'done', agents: [], comments: [] },
  });
  expect(reviewResponse.ok(), await reviewResponse.text()).toBeTruthy();
  const review = await reviewResponse.json();
  const findingResponse = await ctx.post(`${base}/api/v1/workspaces/${ws}/__e2e/findings`, {
    data: { review_id: review.id, repo_id: repoId, title: 'Explain this exception', body: 'Needs review', severity: 'high', category: 'correctness', status: 'open' },
  });
  expect(findingResponse.ok(), await findingResponse.text()).toBeTruthy();
  const finding = await findingResponse.json();
  await page.addInitScript(ws => localStorage.setItem('otto_workspace', ws), ws);
  await page.goto(`/#/git/${repoId}/review/${review.id}`);
  await page.getByRole('button', { name: /Explain this exception/ }).click();
  await page.getByRole('button', { name: 'More actions for this finding' }).click();
  await page.getByRole('menuitem', { name: 'Waive…', exact: true }).click();
  const reasonDialog = page.getByRole('dialog', { name: 'Waive finding' });
  await expect(reasonDialog).toBeVisible();
  await reasonDialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  const unchanged = await (await ctx.get(`${base}/api/v1/findings/${finding.id}`)).json();
  expect(unchanged.finding.status).toBe('open');
  await page.getByRole('button', { name: 'More actions for this finding' }).click();
  await page.getByRole('menuitem', { name: 'Waive…', exact: true }).click();
  await reasonDialog.getByRole('textbox').fill('Reviewed and accepted for this release');
  await reasonDialog.getByRole('button', { name: 'Waive', exact: true }).click();
  await expect(reasonDialog).not.toBeVisible();
  await expect.poll(async () => (await (await ctx.get(`${base}/api/v1/findings/${finding.id}`)).json()).finding.status).toBe('waived');
  const resolved = await (await ctx.get(`${base}/api/v1/findings/${finding.id}`)).json();
  expect(JSON.stringify(resolved.events)).toContain('Reviewed and accepted for this release');
  let fail = true;
  await page.route(`**/api/v1/reviews/${review.id}/proof-pack`, r => fail
    ? r.fulfill({ status: 503, json: { message: 'Proof temporarily unavailable' } })
    : r.continue());
  await page.getByRole('button', { name: 'Proof Pack', exact: true }).click();
  const modal = page.getByRole('dialog', { name: 'Proof Pack', exact: true });
  await expect(modal.getByRole('button', { name: 'Export', exact: true })).toBeDisabled();
  await expect(modal.getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
  fail = false;
  await modal.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(modal.getByRole('button', { name: 'Export', exact: true })).toBeEnabled();
  await ctx.dispose();
});
