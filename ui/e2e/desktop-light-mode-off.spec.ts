import { test, expect } from '@playwright/test';
import { openPage } from './helpers';

// The phone's light version (lib/lightMode.svelte.ts) needs a phone-sized
// screen. A desktop window whose storage says "light" (the same browser used
// at phone width once) is the full app: every module in the sidebar and ⌘K,
// the bare route on Agents, hidden modules open, and Settings ▸ Appearance
// does not offer the switch.

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
});

test('a stored light choice changes nothing on desktop', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('otto_phone_light', '1'));

  // The bare route stays the full app's default (no redirect to Threads).
  await openPage(page, '');
  await expect.poll(() => page.evaluate(() => location.hash)).not.toBe('#/threads');
  await expect(page.locator('nav.bottomnav')).toHaveCount(0);

  // Modules outside the five are in the sidebar and open normally.
  for (const id of ['agents', 'home', 'kubernetes', 'git', 'usage']) {
    await expect(page.locator(`.sidebar [data-nav-id="${id}"]`).first()).toBeVisible();
  }
  await openPage(page, 'kubernetes');
  await expect(page.getByText('isn’t in the light version')).toHaveCount(0);

  // No phone-only switch on desktop.
  await openPage(page, 'settings/appearance');
  // The row sits right under Direction: wait for that, then for its absence.
  await expect(page.getByRole('group', { name: 'Direction' })).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId('appearance-light-version')).toHaveCount(0);
  await expect(page.getByText('Light version', { exact: true })).toHaveCount(0);
});
