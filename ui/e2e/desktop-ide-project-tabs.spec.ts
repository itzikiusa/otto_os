import { mkdirSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// IDE Settings › Appearance › "Show projects as tabs": every project is a tab
// in the header row — Scratches first (no delete), then the named projects —
// and the trailing ▾ keeps the switcher popover. Stacked layouts (phones) keep
// the dropdown.

let wsId = '';
const names: string[] = [];
// PROJECT_TABS_SHOTS=<dir> saves light/dark screenshots of the header.
const SHOTS = process.env.PROJECT_TABS_SHOTS;

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base, 'Project Tabs');
  const parent = mkdtempSync(join(tmpdir(), 'otto-e2e-ptabs-'));
  // Unique per worker: each worker runs this beforeAll against the same daemon.
  const stamp = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  for (const label of ['Alpha', 'Beta', 'Gamma service', 'Delta platform', 'Epsilon gateway', 'Zeta long project name']) {
    const repo = join(parent, label.toLowerCase().replace(/\s+/g, '-'));
    mkdirSync(repo);
    const name = `${label} ${stamp}`;
    const resp = await ctx.post(`${base}/api/v1/ide/projects`, { data: { name, primary_root: repo, folders: [repo], pinned: true } });
    expect(resp.ok()).toBe(true);
    names.push(name);
  }
  await ctx.dispose();
});

async function boot(page: Page): Promise<void> {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, wsId);
  await page.goto('/#/ide?scratches=1');
}

const tabStrip = (page: Page) => page.getByRole('tablist', { name: 'IDE projects' });
const switcher = (page: Page) => page.getByTestId('ide-target-switcher');

async function turnOn(page: Page): Promise<void> {
  await page.getByTestId('ide-more-btn').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: /^Open settings/ }).click();
  const settings = page.getByTestId('ide-settings-modal');
  await settings.getByTestId('ide-settings-nav-appearance.ui').click();
  const toggle = settings.getByRole('switch', { name: 'Show projects as tabs' });
  await expect(toggle).toHaveAttribute('aria-checked', 'false');
  await toggle.click();
  await page.getByTestId('ide-settings-ok').click();
  await expect(settings).toHaveCount(0);
}

test('projects as tabs: Scratches first, click switches, Scratches has no delete', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await boot(page);
  await expect(switcher(page)).toContainText('Scratches');
  await expect(page.getByTestId('ide-project-tabs')).toHaveCount(0); // off by default

  await turnOn(page);
  const tabs = tabStrip(page).getByRole('tab');
  const scratches = tabStrip(page).getByTestId('ide-project-tab-scratches');
  await expect(tabs.first()).toHaveAttribute('data-testid', 'ide-project-tab-scratches');
  await expect(scratches).toHaveText('Scratches');
  await expect(scratches).toHaveAttribute('aria-selected', 'true');
  // The named projects follow, in the switcher's order.
  await expect(tabStrip(page).getByTestId('ide-project-tab').filter({ hasText: names[names.length - 1] })).toHaveCount(1);
  const ours = (texts: string[]) => texts.map((t) => names.find((n) => t.includes(n))).filter(Boolean);
  const tabOrder = ours(await tabStrip(page).getByTestId('ide-project-tab').allInnerTexts());
  expect(tabOrder).toHaveLength(names.length);
  await switcher(page).click();
  await expect(page.getByTestId('ide-named-project-option').filter({ hasText: names[0] })).toBeVisible();
  const popOrder = ours(await page.getByTestId('ide-named-project-option').allInnerTexts());
  await page.keyboard.press('Escape');
  expect(tabOrder).toEqual(popOrder);
  // Scratches carries no close/delete affordance: nothing inside the tab, and its menu is locked.
  await expect(scratches.locator('button, [role="button"]')).toHaveCount(0);
  await scratches.click({ button: 'right' });
  await expect(page.getByRole('menuitem', { name: 'Delete project…' })).toHaveAttribute('aria-disabled', 'true');
  await page.keyboard.press('Escape');

  // A click switches project.
  const alpha = tabStrip(page).getByRole('tab', { name: names[0] });
  await alpha.click();
  await expect(alpha).toHaveAttribute('aria-selected', 'true');
  await expect(scratches).toHaveAttribute('aria-selected', 'false');
  await expect(page.getByTestId('ide-project-tree').getByTestId('ide-tree-root-node')).toContainText(/alpha/i);
  // …and a named project's menu can delete it.
  await alpha.click({ button: 'right' });
  await expect(page.getByRole('menuitem', { name: 'Delete project…' })).not.toHaveAttribute('aria-disabled', 'true');
  await page.keyboard.press('Escape');

  // The ▾ keeps the switcher (workspaces, search, New project).
  await switcher(page).click();
  await expect(page.getByTestId('ide-workspace-popover')).toBeVisible();
  await expect(page.getByTestId('ide-popover-top-new-project')).toBeVisible();
  await page.keyboard.press('Escape');

  // Back to Scratches by keyboard: Home on the strip.
  await alpha.focus();
  await page.keyboard.press('Home');
  await expect(scratches).toHaveAttribute('aria-selected', 'true');
  await expect(scratches).toBeFocused();

  if (SHOTS) {
    for (const scheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme: scheme });
      await page.evaluate((v) => localStorage.setItem('otto_scheme', v), scheme);
      await page.reload();
      await alpha.click();
      await expect(alpha).toHaveAttribute('aria-selected', 'true');
      await page.screenshot({ path: `${SHOTS}/tabs-${scheme}.png`, clip: { x: 0, y: 0, width: 1280, height: 140 } });
      await page.screenshot({ path: `${SHOTS}/tabs-${scheme}-full.png` });
      await scratches.click();
      await expect(scratches).toHaveAttribute('aria-selected', 'true');
      await page.screenshot({ path: `${SHOTS}/tabs-scratches-${scheme}.png`, clip: { x: 0, y: 0, width: 1280, height: 140 } });
    }
  }
});

test('projects as tabs: a long strip scrolls the active tab into view; the ▾ stays put', async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 800 });
  await page.addInitScript(() => localStorage.setItem('otto.ide.preferences.v1', JSON.stringify({ scratchesTab: true }))); // the old setting migrates on
  await boot(page);
  await expect(tabStrip(page)).toBeVisible();
  // Switch to the strip's last project from the popover: its tab scrolls into view.
  await expect.poll(() => tabStrip(page).getByRole('tab').count()).toBeGreaterThan(names.length); // projects loaded
  const lastId = await tabStrip(page).getByRole('tab').last().getAttribute('data-tab-id');
  const last = tabStrip(page).locator(`[data-tab-id="${lastId}"]`);
  const lastName = (await last.innerText()).trim();
  await expect.poll(async () => last.evaluate((el) => el.getBoundingClientRect().right > el.parentElement!.getBoundingClientRect().right + 1)).toBe(true); // the strip overflows
  await switcher(page).click();
  await page.getByTestId('ide-workspace-search').fill(lastName);
  await page.getByTestId('ide-named-project-option').filter({ hasText: lastName }).first().click();
  await expect(last).toHaveAttribute('aria-selected', 'true');
  const inStrip = async () => last.evaluate((el) => {
    const s = el.parentElement!.getBoundingClientRect();
    const r = el.getBoundingClientRect();
    return r.left >= s.left - 1 && r.right <= s.right + 1;
  });
  await expect.poll(inStrip).toBe(true);
  await expect(switcher(page)).toBeInViewport({ ratio: 1 });
});

test('projects as tabs fall back to the dropdown on a phone width', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('otto.ide.preferences.v1', JSON.stringify({ projectTabs: true })));
  await page.setViewportSize({ width: 390, height: 844 });
  await boot(page);
  await expect(switcher(page)).toBeVisible();
  await expect(switcher(page)).toContainText('Scratches');
  await expect(page.getByTestId('ide-project-tabs')).toHaveCount(0);
  if (SHOTS) {
    await page.screenshot({ path: `${SHOTS}/phone-dropdown.png` });
  }
  // Wide again: the tabs come back.
  await page.setViewportSize({ width: 1280, height: 800 });
  await expect(tabStrip(page)).toBeVisible();
});
