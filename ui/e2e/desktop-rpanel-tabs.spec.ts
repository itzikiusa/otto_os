import { test, expect, type Page } from '@playwright/test';
import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';
import { openPage, expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// The Side panel (⌘J) is on every page and is ONE component in both states:
// the vertical icon rail never moves or changes; a rail click opens that tab's
// content beside it under one header (title + Expand + Collapse), the shown
// tab's icon collapses it again. Off an agent session, Git/Files work in the
// workspace folder and the session-only tabs (Activity/Outputs/Canvas/Info)
// are not on the rail. Files has the folder picker's navigation.

let wsA = '';
let root = '';
let root2 = '';
// Unique per worker: every worker seeds its own second workspace.
const otherWs = `Side panel other ${process.pid}-${Date.now().toString(36)}`;

test.beforeAll(async () => {
  const a = await apiCtx();
  wsA = await seedWorkspace(a.ctx, a.base);
  const list = (await (await a.ctx.get(`${a.base}/api/v1/workspaces`)).json()) as { id: string; root_path: string }[];
  root = list.find((w) => w.id === wsA)!.root_path;
  mkdirSync(join(root, 'src', 'lib'), { recursive: true });
  writeFileSync(join(root, 'README.md'), '# Side panel e2e\n');
  writeFileSync(join(root, '.env.example'), 'KEY=value\n');
  writeFileSync(join(root, 'src', 'lib', 'util.ts'), 'export const x = 1;\n');
  await seedShellSession(a.ctx, a.base, wsA);
  // A second workspace: its folder is offered by the folder bar's menu.
  const ws2 = await seedWorkspace(a.ctx, a.base, otherWs);
  const all = (await (await a.ctx.get(`${a.base}/api/v1/workspaces`)).json()) as { id: string; root_path: string }[];
  root2 = all.find((w) => w.id === ws2)!.root_path;
  writeFileSync(join(root2, 'OTHER.md'), '# other\n');
  await a.ctx.dispose();
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    // Only on the first load: later reloads must see what the page persisted.
    if (sessionStorage.getItem('rp-seeded')) return;
    sessionStorage.setItem('rp-seeded', '1');
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_right_open', '0');
    localStorage.setItem('otto_right_tab', 'git');
    localStorage.setItem('otto_right_width', '300');
  }, wsA);
});

const rail = (page: Page) => page.locator('.rstrip');
const railBtn = (page: Page, name: string) => rail(page).getByRole('button', { name, exact: true });

test('off a session: rail on every page, workspace tabs only, open and closed look like one component', async ({ page }) => {
  await openPage(page, 'home');
  await expect(rail(page)).toBeVisible();
  for (const name of ['Git', 'Files', 'Notes', 'Browser', 'API']) await expect(railBtn(page, name)).toBeVisible();
  for (const name of ['Activity', 'Outputs', 'Canvas', 'Info']) await expect(railBtn(page, name)).toHaveCount(0);
  await expect(page.locator('.rpanel')).toHaveCount(0);

  const closedRail = (await rail(page).boundingBox())!;
  await railBtn(page, 'Files').click();
  const panel = page.locator('.rpanel');
  await expect(panel).toBeVisible();
  // One header: the tab's title + Expand + Collapse — no second (text) tab bar.
  await expect(panel.locator('.rpanel-title')).toHaveText('Files');
  await expect(panel.getByRole('tab')).toHaveCount(0);
  await expect(panel.locator('.rpanel-head').getByRole('button')).toHaveCount(2);
  await expectFullyInViewport(page, panel.getByRole('button', { name: 'Expand panel' }));
  await expectFullyInViewport(page, panel.getByRole('button', { name: 'Collapse panel' }));
  // The rail is the same element in the same place, the shown tab pressed.
  const openRail = (await rail(page).boundingBox())!;
  expect(Math.round(openRail.x)).toBe(Math.round(closedRail.x));
  expect(Math.round(openRail.width)).toBe(Math.round(closedRail.width));
  await expect(railBtn(page, 'Files')).toHaveAttribute('aria-pressed', 'true');
  // Off a session the folder is the workspace's.
  const folderBtn = panel.locator('.folder-btn');
  await expect(folderBtn).toContainText(root.split('/').at(-1)!);
  await expect(folderBtn).toContainText('workspace');
  await expectNoHorizontalOverflow(page);

  // Another icon switches the content; the shown one collapses to the rail.
  await railBtn(page, 'Notes').click();
  await expect(panel.locator('.rpanel-title')).toHaveText('Notes');
  await expect(page.locator('textarea.notes')).toBeVisible();
  await railBtn(page, 'Notes').click();
  await expect(page.locator('.rpanel')).toHaveCount(0);
  await expect(rail(page)).toBeVisible();

  // State persists: open + tab + width survive a reload.
  await railBtn(page, 'Files').click();
  await expect(page.locator('.rpanel')).toBeVisible();
  await page.reload();
  await expect(page.locator('.rpanel .rpanel-title')).toHaveText('Files');
  expect(await page.evaluate(() => [localStorage.getItem('otto_right_open'), localStorage.getItem('otto_right_tab')])).toEqual(['1', 'files']);
  await page.screenshot({ path: test.info().outputPath('side-panel-files-home.png') });

  // Pages that already ARE a tab don't repeat it.
  await openPage(page, 'api');
  await expect(railBtn(page, 'API')).toHaveCount(0);
  await expect(railBtn(page, 'Git')).toBeVisible();
});

test('Files: folder navigation (up, crumbs, typed path, hidden toggle, open folder)', async ({ page }) => {
  await openPage(page, 'home');
  await railBtn(page, 'Files').click();
  const panel = page.locator('.rpanel');
  const crumbs = panel.getByRole('navigation', { name: 'Folder path' });
  const current = crumbs.locator('[aria-current="location"]');
  const rootName = root.split('/').at(-1)!;
  await expect(current).toHaveText(rootName);
  await expect(panel.locator('.tree-row', { hasText: 'README.md' })).toBeVisible();
  await expect(panel.locator('.tree-row', { hasText: '.env.example' })).toBeVisible();

  // Hidden toggle.
  const hidden = panel.getByRole('button', { name: 'Show hidden files' });
  await expect(hidden).toHaveAttribute('aria-pressed', 'true');
  await hidden.click();
  await expect(panel.locator('.tree-row', { hasText: '.env.example' })).toHaveCount(0);
  await hidden.click();
  await expect(panel.locator('.tree-row', { hasText: '.env.example' })).toBeVisible();

  // Open a folder as the root, then Up and Back/Forward.
  await panel.locator('.tree-item', { hasText: 'src' }).hover();
  await panel.getByRole('button', { name: 'Open folder src' }).click();
  await expect(current).toHaveText('src');
  await expect(panel.locator('.tree-row', { hasText: 'lib' })).toBeVisible();
  await panel.getByRole('button', { name: 'Up one folder' }).click();
  await expect(current).toHaveText(rootName);
  await panel.getByRole('button', { name: 'Back', exact: true }).click();
  await expect(current).toHaveText('src');
  await panel.getByRole('button', { name: 'Forward', exact: true }).click();
  await expect(current).toHaveText(rootName);

  // Typed path (relative to the shown folder).
  await panel.getByRole('button', { name: 'Type a path to go to' }).click();
  const input = panel.getByRole('textbox', { name: /Go to folder/ });
  await input.fill('src/lib');
  await input.press('Enter');
  await expect(current).toHaveText('lib');
  await expect(panel.locator('.tree-row', { hasText: 'util.ts' })).toBeVisible();

  // A breadcrumb jumps back up; "Back to <root>" follows the panel's folder again.
  await crumbs.getByRole('button', { name: 'src', exact: true }).click();
  await expect(current).toHaveText('src');
  await panel.getByRole('button', { name: `Back to ${rootName}` }).click();
  await expect(current).toHaveText(rootName);

  // The preview still opens beside the tree.
  await panel.locator('.tree-row', { hasText: 'README.md' }).click();
  await expect(panel.locator('.viewer-name-text')).toHaveText('README.md');
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: test.info().outputPath('side-panel-files-nav.png') });
});

test('agent session: the session tabs join the rail and Files/Git follow the session folder', async ({ page }) => {
  await openPage(page, 'agents');
  await page.getByRole('button', { name: /E2E Shell/ }).first().click();
  for (const name of ['Activity', 'Outputs', 'Canvas', 'Info']) await expect(railBtn(page, name)).toBeVisible();
  await railBtn(page, 'Files').click();
  const panel = page.locator('.rpanel');
  await expect(panel.locator('.folder-label')).toContainText('session');
  await expect(panel.locator('.folder-label')).toContainText('tmp');
  await railBtn(page, 'Activity').click();
  await expect(panel.locator('.rpanel-title')).toHaveText('Activity');
  // Leaving the session keeps the choice but shows a tab that exists there.
  await openPage(page, 'home');
  await expect(page.locator('.rpanel .rpanel-title')).toHaveText('Git');
  await expect(railBtn(page, 'Activity')).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem('otto_right_tab'))).toBe('activity');
});

test('API tab (narrow): full URL preview, params rows keep × on the row, light and dark', async ({ page }) => {
  await openPage(page, 'home');
  await railBtn(page, 'API').click();
  const panel = page.locator('.rpanel');
  await expect(panel.locator('.rpanel-title')).toHaveText('API');
  await panel.getByRole('textbox', { name: 'Request URL' }).fill('https://gwsinatra-stg.groovegaming.com/games/attribute/get/1.0/list');
  const params: [string, string][] = [['type', 'FEATURE'], ['page', '0'], ['size', '100'], ['sort', 'name'], ['order', 'asc'], ['q', 'a b&c']];
  for (const [k, v] of params) {
    await panel.getByRole('button', { name: /Add param/ }).click();
    await panel.getByRole('textbox', { name: 'Parameter name' }).last().fill(k);
    await panel.getByRole('textbox', { name: 'Value' }).last().fill(v);
  }
  // Untick the last one: it drops out of the URL.
  await panel.getByRole('checkbox', { name: 'Send q' }).uncheck();
  const full = panel.getByTestId('api-full-url');
  await expect(full).toHaveText('https://gwsinatra-stg.groovegaming.com/games/attribute/get/1.0/list?type=FEATURE&page=0&size=100&sort=name&order=asc');
  await panel.getByRole('checkbox', { name: 'Send q' }).check();
  await expect(full).toHaveText(/&order=asc&q=a\+b%26c$/);
  await expectFullyInViewport(page, panel.getByRole('button', { name: 'Copy the full URL' }));
  // The URL box wraps inside the panel instead of widening it.
  const box = (await full.boundingBox())!;
  const panelBox = (await panel.boundingBox())!;
  expect(box.x + box.width).toBeLessThanOrEqual(panelBox.x + panelBox.width + 0.5);
  // Every row keeps its checkbox, key, value and × on one line.
  const rows = panel.locator('.kv-row');
  for (let i = 0; i < (await rows.count()); i++) {
    const row = rows.nth(i);
    const key = (await row.getByRole('textbox', { name: 'Parameter name' }).boundingBox())!;
    const x = (await row.getByRole('button', { name: /^Remove parameter / }).boundingBox())!;
    expect(Math.abs(x.y + x.height / 2 - (key.y + key.height / 2))).toBeLessThan(4);
  }
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: test.info().outputPath('side-panel-api-light.png') });
  await page.evaluate(() => {
    localStorage.setItem('otto_scheme', 'dark');
    window.dispatchEvent(new StorageEvent('storage', { key: 'otto_scheme' }));
  });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: test.info().outputPath('side-panel-api-dark.png') });
});

test('Files follows the panel folder: a pick re-roots a browsed tree; a vanished pick offers the workspace folder', async ({ page }) => {
  await openPage(page, 'home');
  await railBtn(page, 'Files').click();
  const panel = page.locator('.rpanel');
  const current = panel.getByRole('navigation', { name: 'Folder path' }).locator('[aria-current="location"]');
  const rootName = root.split('/').at(-1)!;
  const otherName = root2.split('/').at(-1)!;
  await expect(current).toHaveText(rootName);
  // Browse away (Up), then pick another folder in the folder bar.
  await panel.getByRole('button', { name: 'Up one folder' }).click();
  await expect(current).not.toHaveText(rootName);
  await panel.locator('.folder-btn').click();
  // A long, filterable menu (the shared test daemon holds many workspaces).
  const menu = page.locator('.ctx-menu');
  const find = menu.getByPlaceholder('Find a repository or workspace');
  if (await find.count()) await find.fill(otherWs);
  await menu.getByRole('menuitemcheckbox', { name: new RegExp(otherWs) }).click();
  await expect(panel.locator('.folder-btn')).toContainText(otherName);
  await expect(current).toHaveText(otherName);
  await expect(panel.locator('.tree-row', { hasText: 'OTHER.md' })).toBeVisible();
  // A fresh Back/Forward history for the new folder.
  await expect(panel.getByRole('button', { name: 'Back', exact: true })).toBeDisabled();
  // The picked folder disappears: the bar says so and offers the workspace's.
  rmSync(root2, { recursive: true, force: true });
  await page.reload();
  await expect(panel.locator('.folder-missing')).toContainText('Not found');
  await panel.getByRole('button', { name: 'Use workspace folder' }).click();
  await expect(panel.locator('.folder-btn')).toContainText(rootName);
  await expect(panel.locator('.folder-missing')).toHaveCount(0);
  await expect(current).toHaveText(rootName);
});

test('pop-out windows keep the Side panel', async ({ page }) => {
  await page.goto('/?popout=1#/home');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  await expect(rail(page)).toBeVisible();
  await railBtn(page, 'Notes').click();
  await expect(page.locator('.rpanel .rpanel-title')).toHaveText('Notes');
  await expectNoHorizontalOverflow(page);
});
