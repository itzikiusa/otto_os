import { test, expect, type Page } from '@playwright/test';
import { openPage } from './helpers';

// Customizable sidebar: show/hide + reorder of the expanded Navigator's module
// list, persisted per-device in localStorage. Exercised on the desktop project
// where the full Navigator (not the phone bottom-nav) renders.

test.describe('sidebar customize', () => {
  // Start each test from a clean sidebar config with the Navigator expanded.
  // The expanded Navigator + drag/edit UI is the desktop surface, so skip the
  // phone/tablet device projects.
  test.beforeEach(async ({ page }, testInfo) => {
    test.skip(
      testInfo.project.name !== 'desktop-browser',
      'expanded Navigator + drag/edit UI is the desktop surface',
    );
    await openPage(page, 'agents');
    await page.evaluate(() => {
      localStorage.removeItem('otto_sidebar_order');
      localStorage.removeItem('otto_sidebar_hidden');
      localStorage.setItem('otto_rail_expanded', '1');
    });
    await page.reload();
    await expect(page.locator('.navigator')).toBeVisible({ timeout: 15_000 });
  });

  const usageItem = (page: Page) =>
    page.locator('.navigator').getByRole('button', { name: 'Usage', exact: true });

  test('hide a module → gone, persists across reload, reset restores it', async ({ page }) => {
    // Visible by default.
    await expect(usageItem(page)).toBeVisible();

    // Enter edit mode, hide "Usage", leave edit mode.
    await page.getByTestId('sidebar-edit-toggle').click();
    await expect(page.getByTestId('sidebar-edit-row-usage')).toBeVisible();
    await page.getByTestId('sidebar-hide-usage').click();
    await page.getByTestId('sidebar-edit-toggle').click();

    // Gone from the Navigator + recorded in localStorage.
    await expect(usageItem(page)).toHaveCount(0);
    const hidden = await page.evaluate(() =>
      JSON.parse(localStorage.getItem('otto_sidebar_hidden') || '[]'),
    );
    expect(hidden).toContain('usage');

    // Survives a reload.
    await page.reload();
    await expect(page.locator('.navigator')).toBeVisible();
    await expect(usageItem(page)).toHaveCount(0);

    // Reset brings it back and clears the hidden set.
    await page.getByTestId('sidebar-edit-toggle').click();
    await page.getByTestId('sidebar-reset').click();
    await page.getByTestId('sidebar-edit-toggle').click();
    await expect(usageItem(page)).toBeVisible();
    const hiddenAfter = await page.evaluate(() =>
      JSON.parse(localStorage.getItem('otto_sidebar_hidden') || '[]'),
    );
    expect(hiddenAfter).toEqual([]);
  });

  test('reorder a module up → order changes and persists', async ({ page }) => {
    await page.getByTestId('sidebar-edit-toggle').click();

    const rowIds = () =>
      page
        .locator('[data-testid^="sidebar-edit-row-"]')
        .evaluateAll((els) =>
          els.map((e) => e.getAttribute('data-testid')!.replace('sidebar-edit-row-', '')),
        );

    const before = await rowIds();
    const i = before.indexOf('usage');
    expect(i).toBeGreaterThan(0); // not already first

    // Move "Usage" up one slot.
    await page.getByRole('button', { name: 'Move Usage up' }).click();

    const after = await rowIds();
    expect(after.indexOf('usage')).toBe(i - 1);

    // Persisted as the full saved order.
    const saved = await page.evaluate(() =>
      JSON.parse(localStorage.getItem('otto_sidebar_order') || '[]'),
    );
    expect(saved.indexOf('usage')).toBe(i - 1);

    // Survives a reload (still reflected in the edit list).
    await page.reload();
    await expect(page.locator('.navigator')).toBeVisible();
    await page.getByTestId('sidebar-edit-toggle').click();
    expect((await rowIds()).indexOf('usage')).toBe(i - 1);
  });
});
