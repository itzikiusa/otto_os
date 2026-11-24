import { test, expect, type Page } from '@playwright/test';

// "Show what he uses" (lib/sidebarMore.ts): the sidebar lists pinned modules,
// the ones used lately (cached from GET /telemetry/navigation) — or, before any
// usage is known, Agents · Git · Connections · Threads · Home · Usage — and ONE
// "More" row that expands the rest in place, remembered. Customize still lists
// everything. The global storage state marks every module used; these specs
// start from a clean profile instead.

async function boot(page: Page, route: string, used: string[] | null = null): Promise<void> {
  await page.addInitScript((u) => {
    if (sessionStorage.getItem('kit-more-seeded')) return; // a reload keeps what the test did
    sessionStorage.setItem('kit-more-seeded', '1');
    for (const k of ['otto_sidebar_used', 'otto_sidebar_more_open', 'otto_sidebar_favorites', 'otto_sidebar_hidden', 'otto_sidebar_order']) localStorage.removeItem(k);
    if (u) localStorage.setItem('otto_sidebar_used', JSON.stringify(u));
    localStorage.setItem('otto_rail_expanded', '1');
  }, used);
  await page.goto(`/#/${route}`);
  await expect(page.locator('.navigator')).toBeVisible({ timeout: 30_000 });
}

const nav = (page: Page) => page.locator('.navigator [data-testid="sidebar-modules"]');
const navRow = (page: Page, id: string) => nav(page).locator(`.nav-item[data-nav-id="${id}"]`);
const more = (page: Page) => page.getByTestId('sidebar-more-toggle').first();

test('no usage yet: the defaults show, the rest folds under one More row that is remembered', async ({ page }) => {
  await boot(page, 'home');
  for (const id of ['agents', 'git', 'connections', 'threads', 'home', 'usage']) await expect(navRow(page, id), id).toHaveCount(1);
  for (const id of ['vault', 'kubernetes', 'workflows', 'aws']) await expect(navRow(page, id), id).toHaveCount(0);
  await expect(more(page)).toHaveAttribute('aria-expanded', 'false');
  await expect(more(page)).toContainText('More');

  await more(page).click();
  await expect(more(page)).toHaveAttribute('aria-expanded', 'true');
  await expect(navRow(page, 'vault')).toBeVisible();
  await page.reload();
  await expect(page.locator('.navigator')).toBeVisible();
  await expect(more(page)).toHaveAttribute('aria-expanded', 'true');

  // A module opened from More stays in the list while it is the page.
  await navRow(page, 'vault').click();
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/vault');
  await more(page).click();
  await expect(navRow(page, 'vault')).toBeVisible();
  await expect(navRow(page, 'kubernetes')).toHaveCount(0);
});

test('used lately replaces the defaults; a favorite from More moves up; Customize lists everything', async ({ page }) => {
  await boot(page, 'agents', ['agents', 'kubernetes']);
  await expect(navRow(page, 'kubernetes')).toBeVisible();
  await expect(navRow(page, 'git')).toHaveCount(0);

  await more(page).click();
  await navRow(page, 'vault').click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Add to Favorites' }).click();
  await more(page).click(); // fold More again: Vault stays, as a favorite
  await expect(page.getByTestId('sidebar-group-favorites').locator('.nav-item[data-nav-id="vault"]')).toBeVisible();

  await page.getByTestId('sidebar-edit-toggle').click();
  await expect(page.getByTestId('sidebar-edit-row-workflows')).toBeVisible();
  await expect(page.getByTestId('sidebar-more')).toHaveCount(0);
});

test('the collapsed rail follows the same rule', async ({ page }) => {
  await boot(page, 'agents', ['agents', 'git']);
  await page.locator('.navigator').getByRole('button', { name: 'Collapse sidebar' }).click();
  const rail = page.locator('.rail');
  await expect(rail.getByTestId('rail-git')).toBeVisible();
  await expect(rail.getByTestId('rail-vault')).toHaveCount(0);
  await rail.getByTestId('sidebar-more-toggle').click();
  await expect(rail.getByTestId('rail-vault')).toBeVisible();
});
