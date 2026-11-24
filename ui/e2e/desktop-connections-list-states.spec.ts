import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';
import { pickMenuItem } from './helpers';

// Connections list states the review found wrong: a pending Delete survives a
// page change, parent sections don't claim to be empty, a failed Kafka read
// says so with Retry, the hover ⋯ never covers line 3's text, and a slow deep
// link can't leave the workbench's URL behind. Desktop browser only.
test.use({ serviceWorkers: 'block' });

let wsId = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  await ctx.dispose();
});
test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((w) => {
    localStorage.setItem('otto_workspace', w as string);
    localStorage.setItem('otto_connhub_filter', 'all');
  }, wsId);
});

async function seedSsh(name: string, host = 'undo.invalid', section_id: string | null = null): Promise<string> {
  const { ctx, base } = await apiCtx();
  const res = await ctx.post(`${base}/api/v1/workspaces/${wsId}/connections`, { data: { name, kind: 'ssh', params: { host }, secret: null, section_id } });
  expect(res.ok(), await res.text()).toBeTruthy();
  const { id } = await res.json();
  await ctx.dispose();
  return id as string;
}

test('a row deleted with Undo stays hidden after leaving the page and coming back (R1-006)', async ({ page }) => {
  const name = `away-${Date.now()}`;
  await seedSsh(name);
  await page.goto('/#/connections');
  const row = page.locator('.conn-row', { hasText: name });
  await pickMenuItem(page, () => row.click({ button: 'right' }), 'Delete');
  await expect(row).toHaveCount(0);
  await page.locator('.toast', { hasText: name }).hover(); // holds the Undo window open
  await page.evaluate(() => (location.hash = '#/git'));
  await expect(page.locator('.conn-root')).toHaveCount(0); // the list page is unmounted
  await page.evaluate(() => (location.hash = '#/connections'));
  await expect(page.locator('.conn-row').first()).toBeVisible();
  await expect(row).toHaveCount(0);
  await page.getByRole('button', { name: 'Undo', exact: true }).click();
  await expect(row).toBeVisible();
});
