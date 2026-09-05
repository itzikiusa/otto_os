import { test, expect } from '@playwright/test';
import type { APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';
import { openPage } from './helpers';

// New Session sheet: starting SEVERAL sessions in one go, and pointing them at
// any folder on the machine without creating a workspace for it.
//
// Before this, the sheet started exactly one session of one provider: "2 codex
// and 3 claude" meant opening it five times (or typing it at the command
// palette), and a folder outside the workspace meant typing an absolute path
// from memory.

let wsA = '';
let api: { ctx: APIRequestContext; base: string } | null = null;

test.beforeAll(async () => {
  const a = await apiCtx();
  api = { ctx: a.ctx, base: a.base };
  wsA = await seedWorkspace(a.ctx, a.base);
  // An existing session so the Agents page renders panes (not the first-run coach).
  await seedShellSession(a.ctx, a.base, wsA);
});

test.afterAll(async () => {
  await api?.ctx.dispose();
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, wsA);
});

/** Open the New Session sheet (⌘T, falling back to the TabBar + button). */
async function openSheet(page: import('@playwright/test').Page) {
  const dialog = page.locator('.sheet[role="dialog"][aria-label="New Session"]');
  await page.keyboard.press('Meta+t');
  if (!(await dialog.isVisible().catch(() => false))) {
    await page.getByTitle('New session (⌘T)').click();
  }
  await expect(dialog).toBeVisible();
  return dialog;
}

async function sessionCount(): Promise<number> {
  const r = await api!.ctx.get(`${api!.base}/api/v1/workspaces/${wsA}/sessions`);
  expect(r.ok(), 'list sessions').toBeTruthy();
  return ((await r.json()) as unknown[]).length;
}

test.describe('new-session batch', () => {
  test('the ± stepper starts several sessions at once', async ({ page }) => {
    await openPage(page, 'agents');
    const before = await sessionCount();
    const dialog = await openSheet(page);

    // Only the plain shell needs no external CLI in the throwaway daemon.
    const shell = dialog.locator('.provider-card', { hasText: 'shell' });
    await shell.locator('.card-main').click();
    await expect(shell.locator('.count')).toHaveText('1');
    // A single session reads as it always did — no batch language.
    await expect(page.getByRole('button', { name: 'Start Session' })).toBeVisible();

    // Ask for three of them.
    await dialog.getByLabel('One more shell session').click();
    await dialog.getByLabel('One more shell session').click();
    await expect(shell.locator('.count')).toHaveText('3');
    await expect(dialog.locator('.batch')).toContainText('Starting 3 sessions');
    await expect(dialog.locator('.batch')).toContainText('3 shell');

    // One click starts all three, and the count is what the button promised.
    const start = page.getByRole('button', { name: 'Start 3 Sessions' });
    await expect(start).toBeVisible();
    await start.click();
    await expect(dialog).toHaveCount(0, { timeout: 15_000 });
    await expect.poll(sessionCount, { timeout: 15_000 }).toBe(before + 3);

    // More than one → they all land open and tiled, like a palette multi-spawn
    // (whatever else the Agents page already had open stays open too).
    await expect
      .poll(() => page.locator('.pane').count(), { timeout: 10_000 })
      .toBeGreaterThanOrEqual(3);
  });

  test('a session can be pointed at any folder via the folder picker', async ({ page }) => {
    await openPage(page, 'agents');
    const dialog = await openSheet(page);

    // `~` is outside the workspace root — the point of the field.
    await dialog.locator('#ns-cwd').fill('~');
    await dialog.getByRole('button', { name: 'Browse…' }).first().click();

    const picker = page.locator('.sheet[role="dialog"][aria-label="Choose working directory"]');
    await expect(picker).toBeVisible();
    // The daemon resolved `~` to an absolute path and listed it.
    await expect(picker.locator('.crumb')).toContainText('/');
    const chosen = (await picker.locator('.crumb').innerText()).trim();

    await picker.getByRole('button', { name: 'Use this folder' }).click();
    await expect(picker).toHaveCount(0);
    await expect(dialog.locator('#ns-cwd')).toHaveValue(chosen);

    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
  });
});
