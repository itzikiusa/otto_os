import { test, expect, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { IDE_SHORTCUTS } from '../src/modules/ide/shell/ideShortcuts';
import { expectFullyInViewport } from './helpers';
import { bootPreviewIde, treeFile } from './ide-preview-fixtures';

// IDE quick-open and shortcut truth (review I1-03/04/08/09/10/22/27, I2-06):
// ⌘P opens one top-anchored picker on recent files whose top edge never moves as
// results arrive; Files shows no Find-in-path options; ⌘G is its `:` line mode; ⌘E
// opens it on recents; ⋯ is short; Settings › Keymap is generated from the one
// shortcut table and the Standard preset's ⌘J / ⌘B really work.

test.describe.configure({ mode: 'serial' });

async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  const out = join(dir, 'fix-IDE-QUICKOPEN');
  mkdirSync(out, { recursive: true });
  await page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));
  await page.screenshot({ path: join(out, `${name}.png`) });
}

async function openFile(page: Page, rel: string): Promise<void> {
  const parts = rel.split('/');
  for (let i = 0; i < parts.length - 1; i++) {
    const dir = treeFile(page, parts.slice(0, i + 1).join('/'));
    await expect(dir).toBeVisible({ timeout: 15_000 });
    if ((await dir.getAttribute('aria-expanded')) !== 'true') await dir.click();
  }
  const file = treeFile(page, rel);
  await expect(file).toBeVisible({ timeout: 15_000 });
  await file.dblclick();
  await expect(page.getByTestId('ide-tabs-strip').locator('.ide-tab.active')).toContainText(parts[parts.length - 1], { timeout: 15_000 });
}

const quickOpen = (page: Page) => page.getByTestId('ide-quick-open');
const rowsOf = (page: Page, kind: string) => quickOpen(page).locator(`[data-testid="ide-qo-row"][data-kind="${kind}"]`);

async function focusEditor(page: Page): Promise<void> {
  await page.locator('.ide-editor-area .cm-content').first().click();
}

/** The cursor line, from the gutter's active line (or the `Ln N, Col M` readout). */
async function cursorLine(page: Page): Promise<number | null> {
  return page.evaluate(() => {
    const g = document.querySelector('.ide-editor-area .cm-activeLineGutter');
    const n = Number.parseInt(g?.textContent?.trim() ?? '', 10);
    if (Number.isFinite(n)) return n;
    const m = /Ln (\d+), Col \d+/.exec(document.querySelector('[data-testid="ide-page"]')?.textContent ?? '');
    return m ? Number(m[1]) : null;
  });
}

for (const scheme of ['dark', 'light'] as const) {
  test(`⌘P quick-open on recents, fixed top, no find options on Files, ⌘G :3, ⌘E, short ⋯ (${scheme})`, async ({ page }) => {
    test.setTimeout(180_000);
    await page.setViewportSize({ width: 1280, height: 800 });
    await bootPreviewIde(page, scheme);
    await openFile(page, 'README.md');
    await openFile(page, 'docs/guide.md');
    await openFile(page, 'app/main.go');
    await focusEditor(page);

    // ⌘P: Files tab, empty query = recent files (newest first, the active file left out).
    await page.keyboard.press('Meta+p');
    await expect(quickOpen(page)).toBeVisible({ timeout: 5_000 });
    await expect(quickOpen(page).getByTestId('ide-se-tab-files')).toHaveAttribute('aria-selected', 'true');
    await expect(quickOpen(page).getByTestId('ide-search-input')).toBeFocused();
    await expect(rowsOf(page, 'recent').first()).toContainText('guide.md');
    await expect(rowsOf(page, 'recent').filter({ hasText: 'README.md' })).toHaveCount(1);
    await expect(rowsOf(page, 'recent').first()).toContainText('docs/');
    await expect(quickOpen(page).getByText('Recent files')).toBeVisible();
    // No dialog title bar / footer; Find-in-path options only on that tab.
    await expect(quickOpen(page).locator('header, footer')).toHaveCount(0);
    await expect(quickOpen(page).getByTestId('ide-se-find-options')).toHaveCount(0);
    await expect(quickOpen(page).getByTestId('ide-se-mask-input')).toHaveCount(0);
    await expectFullyInViewport(page, quickOpen(page), 'quick-open');
    const emptyTop = (await quickOpen(page).boundingBox())!.y;
    await shot(page, `qo-empty-1280-${scheme}`);

    // Results arrive: the top edge stays where it was.
    await quickOpen(page).getByTestId('ide-search-input').fill('main');
    await expect(rowsOf(page, 'hit').first()).toBeVisible({ timeout: 15_000 });
    const resultsTop = (await quickOpen(page).boundingBox())!.y;
    expect(Math.abs(resultsTop - emptyTop), 'top edge moved between empty and results').toBeLessThan(1);
    await shot(page, `qo-results-1280-${scheme}`);

    // Find in path is the one tab with match options.
    await quickOpen(page).getByTestId('ide-se-tab-text').click();
    await expect(quickOpen(page).getByTestId('ide-se-find-options')).toBeVisible();
    expect(Math.abs((await quickOpen(page).boundingBox())!.y - emptyTop)).toBeLessThan(1);
    await page.keyboard.press('Escape');
    await expect(quickOpen(page)).toHaveCount(0);

    // ⌘G: the same picker in `:` mode; `3` + ⏎ moves the cursor to line 3.
    await focusEditor(page);
    await page.keyboard.press('Meta+g');
    await expect(quickOpen(page)).toBeVisible();
    await expect(quickOpen(page).getByTestId('ide-search-input')).toHaveValue(':');
    await page.keyboard.type('3');
    await expect(rowsOf(page, 'line')).toContainText('Go to line 3');
    await page.keyboard.press('Enter');
    await expect(quickOpen(page)).toHaveCount(0);
    await expect.poll(() => cursorLine(page), { timeout: 10_000 }).toBe(3);

    // ⌘E: recent files, same picker (no menu hung off ⋯).
    await focusEditor(page);
    await page.keyboard.press('Meta+e');
    await expect(quickOpen(page)).toBeVisible();
    await expect(quickOpen(page).getByTestId('ide-search-input')).toHaveValue('');
    await expect(rowsOf(page, 'recent').first()).toBeVisible();
    await expect(page.locator('.ctx-menu')).toHaveCount(0);
    await page.keyboard.press('Escape');

    // ⋯ holds only what has no other home: at most 9 top-level rows.
    await page.getByTestId('ide-more-btn').click();
    const menu = page.locator('.ctx-menu:not(.ctx-submenu)').first();
    await expectFullyInViewport(page, menu, '⋯ menu');
    const top = menu.locator('[role="menuitem"], [role="menuitemcheckbox"]');
    expect(await top.count(), '⋯ top-level rows').toBeLessThanOrEqual(9);
    await expect(menu.getByRole('menuitem', { name: /Format on save|Multi-line/ })).toHaveCount(0);
    await expect(menu.getByRole('menuitem', { name: /Search everywhere/ })).toHaveCount(0);
    await menu.getByRole('menuitem', { name: 'Tool windows' }).click();
    const sub = page.locator('.ctx-submenu');
    await expect(sub.getByRole('menuitem', { name: /Live DB watch/ }).locator('svg')).toHaveCount(1);
    await shot(page, `more-menu-1280-${scheme}`);
    await page.keyboard.press('Escape');
  });
}

test('Standard preset: Settings › Keymap is the table; ⌘J toggles the dock, ⌘B the explorer', async ({ page }) => {
  test.setTimeout(150_000);
  await page.setViewportSize({ width: 1280, height: 800 });
  await bootPreviewIde(page, 'dark');
  await openFile(page, 'app/main.go');

  await page.getByTestId('ide-more-btn').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: /^Open settings/ }).click();
  const settings = page.getByTestId('ide-settings-modal');
  await expect(settings).toBeVisible();
  await settings.getByTestId('ide-settings-nav-keymap').click();
  await expect(settings.getByTestId('ide-keymap-row')).toHaveCount(IDE_SHORTCUTS.length);
  await settings.getByTestId('ide-keymap-preset-standard').click();
  await expect(settings.getByTestId('ide-keymap-preset-standard')).toHaveAttribute('aria-pressed', 'true');
  const dockRow = settings.locator('[data-shortcut="toggleDock"]');
  await expect(dockRow).toContainText('⌘J');
  await expect(settings.locator('[data-shortcut="searchEverywhere"]')).toContainText('Double ⇧');
  await shot(page, 'keymap-standard-1280-dark');
  await page.getByTestId('ide-settings-ok').click();
  await expect(settings).toHaveCount(0);

  await focusEditor(page);
  // The dock's tab strip always shows; `is-open` is the expanded tool window.
  const dock = page.getByTestId('ide-bottom-dock');
  const dockShown = async () => /\bis-open\b/.test((await dock.first().getAttribute('class')) ?? '');
  const before = await dockShown();
  await page.keyboard.press('Meta+j');
  await expect.poll(dockShown).toBe(!before);
  await page.keyboard.press('Meta+j');
  await expect.poll(dockShown).toBe(before);

  const tree = page.getByTestId('ide-project-tree');
  await expect(tree).toBeVisible();
  await focusEditor(page);
  await page.keyboard.press('Meta+b');
  await expect(tree).toBeHidden();
  await page.keyboard.press('Meta+b');
  await expect(tree).toBeVisible();

  // ⌘P works in Standard too (same picker).
  await focusEditor(page);
  await page.keyboard.press('Meta+p');
  await expect(quickOpen(page)).toBeVisible();
  await page.keyboard.press('Escape');
});

test('Settings › Keymap in light', async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1280, height: 800 });
  await bootPreviewIde(page, 'light');
  await page.getByTestId('ide-more-btn').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: /^Open settings/ }).click();
  const settings = page.getByTestId('ide-settings-modal');
  await settings.getByTestId('ide-settings-nav-keymap').click();
  await expect(settings.getByTestId('ide-keymap-preset-jetbrains')).toHaveAttribute('aria-pressed', 'true');
  await expect(settings.locator('[data-shortcut="goToFile"]')).toContainText('⌘P');
  await shot(page, 'keymap-jetbrains-1280-light');
});
