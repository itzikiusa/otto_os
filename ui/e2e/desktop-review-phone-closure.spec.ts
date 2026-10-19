import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedGitRepo } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block', viewport: { width: 390, height: 844 } });

for (const theme of ['light', 'dark']) {
  test(`proof phone list and detail stay separate (${theme})`, async ({ page }) => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const response = await ctx.post(`${base}/api/v1/workspaces/${ws}/proof-packs`, {
      data: { work_item_kind: 'manual', work_item_id: `phone-${theme}`, title: 'Phone proof evidence' },
    });
    expect(response.ok()).toBeTruthy();
    const pack = await response.json();
    await ctx.post(`${base}/api/v1/proof-packs/${pack.id}/artifacts`, {
      data: { kind: 'log', title: 'A long artifact title that must wrap on a phone', content: 'Evidence retained', status: 'info' },
    });
    await ctx.dispose();
    await page.addInitScript(({ ws, theme }) => {
      localStorage.setItem('otto_workspace', ws);
      localStorage.setItem('otto_scheme', theme);
      localStorage.setItem('otto_rail_expanded', '0');
    }, { ws, theme });
    await page.goto('/#/proof');
    const row = page.getByRole('button', { name: /Phone proof evidence/ });
    await expect(row).toBeVisible();
    await expect(page.getByText('No proof pack open', { exact: true })).not.toBeVisible();
    const filter = page.getByRole('button', { name: /^All/ }).first();
    expect((await filter.boundingBox())!.height).toBeGreaterThanOrEqual(36);
    await row.click();
    await expect(page.getByRole('button', { name: 'Back to list' })).toBeVisible();
    await expect(row).not.toBeVisible();
    const remove = page.getByRole('button', { name: 'Delete artifact A long artifact title that must wrap on a phone' });
    await expect(remove).toBeVisible();
    expect((await remove.boundingBox())!.height).toBeGreaterThanOrEqual(36);
    await expectNoHorizontalOverflow(page);
    await page.getByRole('button', { name: 'Back to list' }).click();
    await expect(row).toBeVisible();
    await expect(page.getByText('No proof pack open', { exact: true })).not.toBeVisible();
  });
}

test('run phone detail has Back and preserves the launcher draft', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await ctx.dispose();
  const run = {
    id: 'phone-run', workspace_id: ws, title: 'Phone run', source_kind: 'channel',
    source_ref: 'example', mode: 'single_agent', status: 'completed', provider: 'codex',
    goal: 'Review the outcome', findings_total: 0, findings_blocking: 0,
    created_at: '2026-10-18T10:00:00Z', updated_at: '2026-10-18T10:00:00Z',
  };
  await page.route(`**/api/v1/workspaces/${ws}/runs`, r => r.fulfill({ json: [run] }));
  await page.route('**/api/v1/runs/phone-run', r => r.fulfill({ json: run }));
  await page.route('**/api/v1/runs/phone-run/events', r => r.fulfill({ json: [] }));
  await page.addInitScript(ws => localStorage.setItem('otto_workspace', ws), ws);
  await page.goto('/#/run-with-otto');
  const input = page.getByRole('textbox', { name: 'Source or description' });
  await expect(input).toBeVisible();
  await input.fill('Keep this draft');
  const source = page.getByRole('button', { name: 'Jira', exact: true });
  expect((await source.boundingBox())!.height).toBeGreaterThanOrEqual(36);
  await page.locator('.run', { hasText: 'Phone run' }).click();
  await expect(page.getByRole('button', { name: 'Back to runs' })).toBeVisible();
  await expect(input).not.toBeVisible();
  await expect(page.getByRole('region', { name: 'Runs', exact: true })).not.toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.getByRole('button', { name: 'Back to runs' }).click();
  await expect(input).toHaveValue('Keep this draft');
  await expect(input).toBeVisible();
});


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
