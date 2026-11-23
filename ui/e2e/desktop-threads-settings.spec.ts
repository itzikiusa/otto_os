import { test, expect } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Settings › Threads: the per-device Threads prefs in one place, the same
// state the toolbar ⋯ › View switches (threadPrefs.svelte.ts, the thread
// store's sidebar grouping, threadsChrome's details card).
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });
const SHOTS = process.env.OTTO_E2E_SHOTS ?? '';

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.addInitScript(() => {
    if (sessionStorage.getItem('th-settings-init')) return;
    sessionStorage.setItem('th-settings-init', '1');
    localStorage.removeItem('otto_threads_prefs_v1');
  });
});

test('Settings › Threads holds the Threads prefs; ⋯ › View shows the same choice', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/settings/threads');
  const sec = page.getByTestId('settings-threads');
  await expect(sec).toBeVisible({ timeout: 30_000 });
  const mid = sec.getByRole('radiogroup', { name: 'While the agent works, Enter' });
  await expect(mid.getByRole('radio', { name: 'Queue' })).toHaveAttribute('aria-checked', 'true');
  await mid.getByRole('radio', { name: 'Steer' }).click();
  await expect(mid.getByRole('radio', { name: 'Steer' })).toHaveAttribute('aria-checked', 'true');
  // Arrow keys move within a group.
  const tools = sec.getByRole('radiogroup', { name: 'Tool calls' });
  await tools.getByRole('radio', { name: 'Show all' }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(tools.getByRole('radio', { name: 'Hide successful' })).toHaveAttribute('aria-checked', 'true');
  await sec.getByRole('radiogroup', { name: 'Open a subagent in' }).getByRole('radio', { name: 'Main view' }).click();
  const saved = JSON.parse((await page.evaluate(() => localStorage.getItem('otto_threads_prefs_v1'))) ?? '{}');
  expect(saved).toEqual({ midTurn: 'steer', tools: 'hide-ok', subagentIn: 'main' });
  await expectNoHorizontalOverflow(page);
  if (SHOTS) {
    mkdirSync(SHOTS, { recursive: true });
    for (const s of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme: s });
      await page.screenshot({ path: `${SHOTS}/settings-threads-desktop-${s}.png`, animations: 'disabled' });
    }
    await page.emulateMedia({ colorScheme: 'light' });
  }
  // The same prefs from the Threads toolbar's ⋯ › View.
  await page.goto('/#/threads');
  await page.getByTestId('threads-thread-menu').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'View' }).click();
  const view = page.locator('.ctx-submenu');
  await expect(view.getByRole('menuitemcheckbox', { name: 'While working, messages steer' })).toHaveAttribute('aria-checked', 'true');
  await expect(view.getByRole('menuitemcheckbox', { name: 'Tool calls: hide successful' })).toHaveAttribute('aria-checked', 'true');
});

test('the sidebar grouping toggle is the sidebar filter’s “Group by project”', async ({ page }) => {
  await page.goto('/#/settings/threads');
  const group = page.getByTestId('threads-pref-group');
  await expect(group).toBeVisible({ timeout: 30_000 });
  const before = await group.isChecked();
  await group.click();
  await expect(group).toBeChecked({ checked: !before });
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('otto_threads_v1') ?? '{}').groupByProject);
  expect(stored).toBe(!before);
  await group.click();
});
