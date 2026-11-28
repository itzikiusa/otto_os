import { test, expect, type Page } from '@playwright/test';
import { bootToolsIde, openTool, putRun, shot } from './ide-side-tools';

test.use({ serviceWorkers: 'block' });

// Desktop (1440×900): the IDE's tool-window lists and dialog lists on the shared
// side panes (rc6-side-b2) — header (name · count · ⋯), rows, the keyboard walk,
// widths under otto.pane.ide.* (old keys read once) — and the editor rule: the
// agent Detail pane becomes a sheet before the editor drops under 640 px.

const pane = (page: Page, name: string) => page.locator(`aside[aria-label="${name}"]`);

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop');
});

test('tool lists: Tests, Changes, Frames + Variables, Devices', async ({ page }) => {
  test.setTimeout(150_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(() => {
    if (!localStorage.getItem('otto.ide.runTreeW')) localStorage.setItem('otto.ide.runTreeW', '333');
  });
  const { root } = await bootToolsIde(page, 'files');
  await putRun(page, root);
  await openTool(page, 'run');
  const tests = pane(page, 'Tests');
  await expect(tests).toBeVisible();
  // The old width key was read once into the shared one.
  expect(await page.evaluate(() => localStorage.getItem('otto.pane.ide.runTree.w'))).toBe('333');
  // One keyboard walk: ↓ from the suite moves to its first test.
  await tests.getByRole('treeitem', { name: /otto-web\/app/ }).focus();
  await page.keyboard.press('ArrowDown');
  await expect(tests.getByRole('treeitem', { name: /TestInitApp/ })).toBeFocused();
  await page.keyboard.press('ArrowLeft');
  await expect(tests.getByRole('treeitem', { name: /otto-web\/app/ })).toBeFocused();
  await tests.getByRole('treeitem', { name: /TestComputeBonus/ }).click();
  await expect(tests.getByRole('treeitem', { name: /TestComputeBonus/ })).toHaveAttribute('aria-selected', 'true');
  await shot(page, 'ide-desktop-tests');

  await openTool(page, 'git');
  const changes = pane(page, 'Changes');
  await expect(changes).toContainText('go.mod', { timeout: 20_000 });
  await changes.locator('[data-side-row]', { hasText: 'notes.txt' }).click({ button: 'right' });
  await expect(page.getByRole('menuitem', { name: 'Delete File…' })).toBeVisible();
  await page.keyboard.press('Escape');
  await shot(page, 'ide-desktop-changes');

  await openTool(page, 'debug');
  await expect(pane(page, 'Frames')).toBeVisible({ timeout: 20_000 });
  await expect(pane(page, 'Variables')).toContainText('otto-ledger');
  await pane(page, 'Frames').locator('.dbg-frame', { hasText: 'main.main' }).click();
  await shot(page, 'ide-desktop-debug');

  await openTool(page, 'devices');
  const devices = pane(page, 'Devices');
  await expect(devices).toBeVisible({ timeout: 20_000 });
  await devices.locator('[data-side-row]', { hasText: 'iPhone 15' }).click();
  await shot(page, 'ide-desktop-devices');
});

test('agent: a sheet at 1280, inline beside a 640 px editor when there is room', async ({ page }) => {
  test.setTimeout(90_000);
  await page.setViewportSize({ width: 1280, height: 800 });
  await bootToolsIde(page, 'files');
  await page.getByTestId('ide-project-tree').locator('button.tree-item[title="go.mod"]').first().click();
  await page.getByTestId('ide-toggle-agent').click();
  const agent = page.getByTestId('ide-agent');
  await expect(agent).toBeVisible();
  await expect(agent).toHaveClass(/sheet/);
  await shot(page, 'ide-desktop-1280-agent-sheet');
  await page.keyboard.press('Escape');
  await expect(agent).toHaveCount(0);
  // Wider: inline, and the editor keeps 640.
  await page.setViewportSize({ width: 1600, height: 900 });
  await page.getByTestId('ide-toggle-agent').click();
  await expect(agent).toBeVisible();
  await expect(agent).not.toHaveClass(/sheet/);
  const editor = await page.getByTestId('ide-editor-area').boundingBox();
  expect(editor!.width).toBeGreaterThanOrEqual(640);
  await shot(page, 'ide-desktop-agent-inline');
});

test('dialog lists: settings tree, local history, run configurations', async ({ page }) => {
  test.setTimeout(90_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await bootToolsIde(page, 'files');
  await page.keyboard.press('Meta+,');
  const settings = page.getByTestId('ide-settings-modal');
  await expect(settings).toBeVisible();
  const tree = page.getByTestId('ide-settings-tree');
  await tree.getByRole('treeitem').first().focus();
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  await expect(tree.locator('.side-row.selected')).toHaveCount(1);
  await shot(page, 'ide-desktop-settings');
  await page.keyboard.press('Escape');
  await page.keyboard.press('Escape');
  await expect(settings).toHaveCount(0);

  await page.evaluate(async () => {
    // @ts-expect-error Vite serves the module in the browser
    const { ide } = await import('/src/modules/ide/ideStore.svelte.ts');
    ide.openEnvironmentsModal('configs');
  });
  await expect(page.locator('.env-cfg-rail')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.env-cfg-rail').locator('aside[aria-label="Configurations"]')).toBeVisible();
  await shot(page, 'ide-desktop-run-configs');
});
