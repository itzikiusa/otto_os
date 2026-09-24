import { test, expect, type Page } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// Settings navigation (desktop-browser only). The sections come from ONE
// registry (modules/settings/sections.ts), so:
//   • the nav filter narrows sections by label + keywords; Enter opens the
//     first match and the page title is exactly the nav label;
//   • ↓ walks from the filter into the results, Esc clears the filter;
//   • ⌘K "Settings: <section>" opens that section (generated per section);
//   • an unknown section id shows an empty state with a way back instead of
//     silently rendering Appearance.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop Settings nav + floating bar');
});

const nav = (page: Page) => page.locator('.settings-nav');
const filter = (page: Page) => page.getByRole('searchbox', { name: 'Filter settings' });
const title = (page: Page) => page.locator('.settings-body h1.ph-title');
const activeRow = (page: Page) => nav(page).locator('.settings-nav-item[aria-current="page"] .settings-nav-label');

async function boot(page: Page, route: string): Promise<void> {
  await page.goto(`/#/${route}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 60_000 });
}

test('filter narrows the nav; Enter opens the first match titled like its nav row', async ({ page }) => {
  await boot(page, 'settings/appearance');
  await expect(title(page)).toHaveText('Appearance');

  // A keyword ("pat") finds a section whose label doesn't contain it.
  await filter(page).fill('pat');
  await expect(nav(page).getByRole('button', { name: /Personal access tokens/ })).toBeVisible();
  await filter(page).fill('jira');
  const first = nav(page).locator('.settings-nav-item').first();
  await expect(first.locator('.settings-nav-label')).toHaveText('Jira accounts');
  // Group headings give way to a flat result list with the group as detail.
  await expect(nav(page).locator('.settings-nav-heading')).toHaveCount(0);
  await expect(first.locator('.settings-nav-detail')).toHaveText('Integrations');

  await filter(page).press('Enter');
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/settings/jira');
  await expect(title(page)).toHaveText('Jira accounts');
  await expect(activeRow(page)).toHaveText('Jira accounts');

  // No match → a quiet line, not an empty column.
  await filter(page).fill('zzz no such section');
  await expect(nav(page).getByText('No sections match')).toBeVisible();

  // Esc clears; the grouped list comes back.
  await filter(page).press('Escape');
  await expect(filter(page)).toHaveValue('');
  await expect(nav(page).locator('.settings-nav-heading').first()).toBeVisible();

  // ↓ from the filter focuses the first result; its click opens it with a
  // title that matches the nav label.
  await filter(page).fill('gmail');
  await filter(page).press('ArrowDown');
  const focused = nav(page).locator('.settings-nav-item:focus .settings-nav-label');
  await expect(focused).toHaveText('Sharing');
  await page.keyboard.press('Enter');
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/settings/sharing');
  await expect(title(page)).toHaveText('Sharing');
});

test('every nav row opens a page titled with the same label', async ({ page }) => {
  await boot(page, 'settings/appearance');
  const labels = await nav(page).locator('.settings-nav-item .settings-nav-label').allTextContents();
  expect(labels.length).toBeGreaterThan(10);
  for (const label of labels) {
    await nav(page).getByRole('button', { name: label, exact: true }).click();
    await expect(title(page)).toHaveText(label);
  }
});

test('⌘K "Settings: <section>" opens that section', async ({ page }) => {
  await boot(page, 'home');
  await page.keyboard.press('Meta+k');
  await page.keyboard.type('settings: mcp servers');
  const item = page
    .getByTestId('floating-bar')
    .getByRole('option')
    .filter({ hasText: 'Settings: MCP servers' })
    .first();
  await expect(item).toBeVisible();
  await expect(item.locator('.opt-detail')).toHaveText('Integrations');
  await item.click();
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/settings/mcp-servers');
  await expect(title(page)).toHaveText('MCP servers');
  await expect(activeRow(page)).toHaveText('MCP servers');
});

test('an unknown section shows an empty state with a way back', async ({ page }) => {
  await boot(page, 'settings/no-such-section');
  await expect(page.getByTestId('page-empty')).toContainText('No settings section called');
  await expect(activeRow(page)).toHaveCount(0);
  await page.getByTestId('page-empty').getByRole('button', { name: 'Open Appearance' }).click();
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/settings/appearance');
  await expect(title(page)).toHaveText('Appearance');
});
