import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// A database that can't be reached says so ONCE: the header line ("Can't
// connect — reason · Retry · Details"); the raw error, Diagnose and Edit
// connection are behind Details. No second Retry, no repeat in the tree.
test('an unreachable database says it once in the header, with Retry and Details', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  const name = `unreachable-${Date.now()}`;
  const created = await ctx.post(`${base}/api/v1/workspaces/${ws}/connections`, {
    // `.invalid` never resolves: the connect fails fast and contacts no host.
    data: { name, kind: 'redis', params: { host: 'fixture.invalid', port: 6379, database: '0' } },
  });
  expect(created.ok()).toBeTruthy();

  await openPage(page, 'connections');
  await page.locator('.conn-row', { hasText: name }).click();
  const state = page.getByTestId('db-conn-state');
  await expect(state).toContainText('Can’t connect', { timeout: 30_000 });
  await expect(page.getByRole('button', { name: 'Retry', exact: true })).toHaveCount(1);
  await expect(page.getByText('fixture.invalid')).toHaveCount(0); // the raw resolver text is not repeated
  await expect(page.locator('.db-side')).toContainText('No tables while it can’t connect.');
  const banner = page.getByTestId('db-conn-error-banner');
  await expect(banner).toHaveCount(0);

  await state.getByRole('button', { name: 'Retry', exact: true }).click();
  // Still unreachable: the header says so again, not a toast.
  await expect(state).toContainText('Can’t connect', { timeout: 30_000 });

  await state.getByRole('button', { name: 'Details', exact: true }).click();
  await expect(banner).toBeVisible();
  await banner.getByRole('button', { name: 'Edit connection', exact: true }).click();
  await expect(page.getByRole('dialog').getByLabel('Name', { exact: true })).toHaveValue(name);
  await ctx.dispose();
});
