import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// A database that can't be reached used to say so only in tooltips (the tab's
// red dot, the header "Disconnected" chip) and in the schema tree — collapsed
// on phones. The main area now carries the reason, Retry and Edit connection.
test('an unreachable database shows its error inline with Retry and Edit connection', async ({ page }) => {
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
  const banner = page.getByTestId('db-conn-error-banner');
  await expect(banner).toBeVisible({ timeout: 30_000 });
  await expect(banner).toContainText(`Couldn’t connect to ${name}`);

  await banner.getByRole('button', { name: 'Retry', exact: true }).click();
  // Still unreachable: the banner comes back with the reason, not a toast.
  await expect(banner).toBeVisible({ timeout: 30_000 });

  await banner.getByRole('button', { name: 'Edit connection', exact: true }).click();
  await expect(page.getByRole('dialog').getByLabel('Name', { exact: true })).toHaveValue(name);
  await ctx.dispose();
});
