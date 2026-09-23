import { test, expect, type Page } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// Phone shell navigation (≤640px live width):
//   • the Navigator drawer has its OWN open-state — closed on every load, even
//     when the desktop "sidebar expanded" preference (otto_rail_expanded=1) is
//     set on the device (it used to reuse that flag and open over every page);
//   • the top-bar title is the registry label, not the raw route id
//     ("Mission Control", not "Mission-Control"; "Skills Lab", not "Skills-Eval");
//   • tapping a module in the drawer navigates AND closes the drawer.
// ─────────────────────────────────────────────────────────────────────────────

async function isPhone(page: Page): Promise<boolean> {
  return page.evaluate(() => window.innerWidth <= 640);
}

async function boot(page: Page, route: string): Promise<void> {
  await page.addInitScript(() => localStorage.setItem('otto_rail_expanded', '1'));
  await page.goto(`/#/${route}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
}

const drawer = (page: Page) => page.locator('.drawer[aria-label="Navigator"]');
const title = (page: Page) => page.locator('.mtop-title');

test('phone: drawer closed on load; top-bar title uses registry labels', async ({ page }) => {
  await boot(page, 'mission-control');
  test.skip(!(await isPhone(page)), 'phone-mode only (≤640px live viewport)');
  await expect(drawer(page)).toHaveCount(0);

  // A reload doesn't bring it back either.
  await page.reload();
  await expect(page.locator('.shell')).toBeVisible();
  await expect(drawer(page)).toHaveCount(0);

  // Pages that draw their own PageHeader aren't titled twice: the bar's title
  // steps aside (its text is still the registry label, never the raw id).
  await expect(page.getByTestId('page-header').first()).toBeVisible();
  await expect(title(page)).toHaveText('Mission Control');
  await expect(title(page)).toBeHidden();
  await page.goto('/#/skills-eval');
  await expect(title(page)).toHaveText('Skills Lab');
  // Agents has no PageHeader (its TabBar is the chrome): the bar titles it.
  await page.goto('/#/agents');
  await expect(page.getByTestId('page-header')).toHaveCount(0);
  await expect(title(page)).toBeVisible();
});

test('phone: tapping a module in the drawer navigates and closes it', async ({ page }) => {
  await boot(page, 'home');
  test.skip(!(await isPhone(page)), 'phone-mode only (≤640px live viewport)');
  await page.getByRole('button', { name: 'Open navigator' }).click();
  await expect(drawer(page)).toBeVisible();
  // Sections render in the drawer too.
  await expect(drawer(page).getByTestId('sidebar-group-build')).toBeVisible();
  await drawer(page).getByRole('button', { name: 'Vault', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.location.hash)).toBe('#/vault');
  await expect(drawer(page)).toHaveCount(0);
  await expect(title(page)).toHaveText('Vault');
});
