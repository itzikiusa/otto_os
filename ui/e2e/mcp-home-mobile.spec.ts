// MCP approvals on a phone: a waiting request is one tap target; it opens as
// a sheet from the bottom with 44 px Approve / Deny, and Back closes it.
import { test, expect } from '@playwright/test';
import { apiCtx, seedGitRepo, seedWorkspace } from './seed';
import { enableMcpFixtureCatalog } from './mcp-catalog-fixture';
import { expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block' });

test('an approval opens as a bottom sheet with finger-sized actions; Back closes it', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await enableMcpFixtureCatalog(ctx, base);
  const { repoId } = await seedGitRepo(ctx, base, ws);
  const title = `Phone approval ${Date.now()}`;
  const r = await ctx.post(`${base}/api/v1/mcp/otto-tools/invoke`, {
    data: { tool: 'otto.create_pr', arguments: { repo_id: repoId, title, description: 'Body', source_branch: `e2e/phone-${Date.now()}`, target_branch: 'main' } },
  });
  expect(((await r.json()) as { decision: string }).decision).toBe('pending_approval');
  await ctx.dispose();
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), ws);
  await page.goto('/#/mcp');
  const row = page.getByTestId('mcp-home-approval').filter({ hasText: title });
  await expect(row).toBeVisible({ timeout: 30_000 });
  test.skip(!(await page.evaluate(() => window.innerWidth <= 640)), 'phone layouts only');
  // On a phone the row is the one target: its inline buttons are not shown.
  await expect(row.getByRole('button', { name: 'Approve', exact: true })).toBeHidden();
  await expectNoHorizontalOverflow(page);
  await row.locator('.tr-main').tap();
  const sheet = page.getByRole('dialog', { name: 'otto.create_pr' });
  await expect(sheet).toBeVisible();
  // Docked to the bottom edge once its entrance animation settles.
  await expect
    .poll(async () => {
      const box = (await sheet.boundingBox())!;
      return Math.round(Math.abs(box.y + box.height - (await page.evaluate(() => window.innerHeight))));
    }, { message: 'docked to the bottom edge' })
    .toBeLessThanOrEqual(2);
  for (const name of ['Approve', 'Deny…']) {
    const b = (await sheet.getByRole('button', { name, exact: true }).boundingBox())!;
    expect(b.height, `${name} is finger-sized`).toBeGreaterThanOrEqual(44);
  }
  await page.screenshot({ path: test.info().outputPath('mcp-approval-sheet.png') });
  await page.goBack();
  await expect(sheet).toHaveCount(0);
  await expect(page).toHaveURL(/#\/mcp$/);
  await row.locator('.tr-main').tap();
  await page.getByRole('dialog', { name: 'otto.create_pr' }).getByRole('button', { name: 'Approve', exact: true }).tap();
  await expect(row).toHaveCount(0);
});
