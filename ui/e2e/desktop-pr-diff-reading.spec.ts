import { test, expect } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// PR Code tab reading aids (desktop-browser, MOCK mode — ui/src/lib/api/mock-prs.ts):
//   • added code keeps its own colour and weight (the row tint is background
//     only — the stats' green/bold no longer leaks onto `.vrow.dline.add`);
//   • "Hide whitespace changes" re-reads the diff and, on a forge that can't
//     (the mock answers `whitespace_ignored: false`), says so quietly;
//   • a Viewed tick in one commit's view leaves the all-commits marks alone.
// ─────────────────────────────────────────────────────────────────────────────

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_rail_expanded', '0');
  });
  await page.setViewportSize({ width: 1600, height: 950 });
});

async function openCode(page: import('@playwright/test').Page): Promise<void> {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  await page.getByRole('tab', { name: 'Code', exact: true }).click();
  await expect(page.locator('.dfile-head').first()).toBeVisible();
}

test('added code is not painted with the stats colour', async ({ page }) => {
  await openCode(page);
  await page.locator('.dfile-head').first().click();
  const row = page.locator('.diff-body .vrow.dline.add').first();
  await expect(row).toBeVisible();
  const [rowColor, ctxColor, weight] = await row.evaluate((el) => {
    const ctx = document.querySelector('.diff-body .vrow.dline.context');
    return [getComputedStyle(el).color, ctx ? getComputedStyle(ctx).color : '', getComputedStyle(el).fontWeight];
  });
  expect(weight).toBe('400');
  if (ctxColor) expect(rowColor).toBe(ctxColor);
});

test('Hide whitespace changes: toggles, persists, and notes a forge that cannot', async ({ page }) => {
  await openCode(page);
  const toggle = page.getByRole('button', { name: 'Hide whitespace changes' });
  await expect(toggle).toHaveAttribute('aria-pressed', 'false');
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('.pr-toolbar').filter({ hasText: 'Whitespace changes can’t be hidden here' }).first()).toBeVisible();
  // Persisted like the other view preferences.
  await page.reload();
  await expect(page.getByRole('button', { name: 'Hide whitespace changes' })).toHaveAttribute('aria-pressed', 'true', { timeout: 15_000 });
  await page.getByRole('button', { name: 'Hide whitespace changes' }).click();
  await expect(page.locator('.pr-toolbar').filter({ hasText: 'Whitespace changes can’t be hidden here' })).toHaveCount(0);
});

test('a Viewed tick in one commit’s view keeps the all-commits marks', async ({ page }) => {
  await openCode(page);
  const code = page.locator('.prd-code');
  const marks = code.getByRole('checkbox', { name: /^Mark .* as viewed$/ });
  const first = (await marks.first().getAttribute('aria-label')) ?? '';
  await marks.first().check();
  // One commit's view (its files don't include that first file): tick one there.
  await page.locator('.prd-commit-pick').first().click();
  await page.locator('.ctx-menu').getByText('active pins page').click();
  await expect(page.locator('.prd-commit-pick').first()).toContainText('active pins page');
  await code.getByRole('checkbox', { name: /^Mark .* as viewed$/ }).first().check();
  // Back to all commits: the first mark is still there.
  await page.locator('.prd-commit-pick').first().click();
  await page.locator('.ctx-menu').getByText('All commits').click();
  await expect(code.getByRole('checkbox', { name: first, exact: true })).toBeChecked();
});
