import { test, expect, type Page } from '@playwright/test';
import { bootToolsIde, openTool, putRun, shot } from './ide-side-tools';

test.use({ serviceWorkers: 'block' });

// The IDE's tool-window lists on the shared side panes (rc6-side-b2), with REAL
// taps on a phone: each list (Tests, Changes, Frames, Devices) fills the tool;
// a tapped item pushes its content with the ONE Back — the page header's,
// naming the nearest list ("‹ Tests"; design.md §3) — and Back returns with the
// item still marked; another item opens. One Back per screen on the editor; the
// Run button fits the one header row. Tablet: the list sits beside its content.

const pane = (page: Page, name: string) => page.locator(`aside[aria-label="${name}"]`);
/** One Back on the screen: the page header's, naming the nearest list. */
async function back(page: Page, name: string) {
  const b = page.getByTestId('page-header').first().getByRole('button', { name: `Back to ${name}` });
  await expect(b).toBeVisible();
  await expect(page.locator('.phone-back:visible')).toHaveCount(1);
  return b;
}

test('phone: Tests, Changes, Frames and Devices push with the shared Back', async ({ page }, info) => {
  test.skip(info.project.name !== 'iphone-portrait', 'phone portrait');
  test.setTimeout(180_000);
  await page.setViewportSize({ width: 390, height: 844 });
  const { root } = await bootToolsIde(page, 'files');
  const header = page.getByTestId('page-header').first();
  await expect(header.getByTestId('open-navigator')).toBeVisible();

  // The editor page: one Back (the header's), no ‹ in the file row; Run fits the row.
  await page.getByTestId('ide-project-tree').locator('button.tree-item[title="go.mod"]').first().tap();
  await expect(header.getByTestId('phone-back')).toBeVisible();
  await expect(page.getByTestId('ide-phone-file-row')).toBeVisible();
  await expect(page.getByTestId('ide-nav-back')).toHaveCount(0);
  const run = header.getByTestId('ide-run-btn');
  await expect(run).toBeVisible();
  await expect(run).toHaveAccessibleName('Run');
  const rowBox = (await header.boundingBox())!;
  const runBox = (await run.boundingBox())!;
  expect(runBox.height).toBeLessThanOrEqual(44);
  expect(runBox.y).toBeGreaterThanOrEqual(rowBox.y);
  expect(runBox.y + runBox.height).toBeLessThanOrEqual(rowBox.y + rowBox.height);
  await shot(page, 'ide-phone-content');

  // Tests
  await putRun(page, root);
  await openTool(page, 'run');
  const tests = pane(page, 'Tests');
  await expect(tests).toBeVisible();
  await back(page, 'Files'); // the list is in the tool: the Back is still the explorer's
  await expect(page.getByTestId('ide-test-fix-agent')).toBeVisible();
  await shot(page, 'ide-phone-tests-list');
  await tests.getByRole('treeitem', { name: /TestComputeBonus/ }).tap();
  await expect(tests).toBeHidden();
  await back(page, 'Tests');
  await expect(page.getByRole('region', { name: 'Console output' })).toContainText('want 330, got 300');
  await shot(page, 'ide-phone-tests-content');
  await (await back(page, 'Tests')).tap();
  await expect(tests.getByRole('treeitem', { name: /TestComputeBonus/ })).toHaveAttribute('aria-selected', 'true');
  await tests.getByRole('treeitem', { name: /TestMarshal/ }).tap();
  await back(page, 'Tests');
  await (await back(page, 'Tests')).tap();

  // Changes
  await openTool(page, 'git');
  const changes = pane(page, 'Changes');
  await expect(changes).toBeVisible({ timeout: 20_000 });
  await expect(changes).toContainText('go.mod');
  await shot(page, 'ide-phone-changes-list');
  await changes.locator('[data-side-row]', { hasText: 'go.mod' }).tap();
  await back(page, 'Changes');
  await shot(page, 'ide-phone-changes-content');
  await (await back(page, 'Changes')).tap();
  await expect(changes.locator('[data-side-row]', { hasText: 'go.mod' })).toHaveAttribute('aria-current', 'true');
  await changes.locator('[data-side-row]', { hasText: 'notes.txt' }).tap();
  await back(page, 'Changes');
  await (await back(page, 'Changes')).tap();

  // Frames → Variables
  await openTool(page, 'debug');
  const frames = pane(page, 'Frames');
  await expect(frames).toBeVisible({ timeout: 20_000 });
  await shot(page, 'ide-phone-frames-list');
  await frames.locator('.dbg-frame', { hasText: 'main.main' }).tap();
  await back(page, 'Frames');
  await expect(pane(page, 'Variables')).toBeVisible();
  await shot(page, 'ide-phone-frames-content');
  await (await back(page, 'Frames')).tap();
  await expect(frames).toBeVisible();
  await frames.locator('.dbg-frame', { hasText: 'runtime.main' }).tap();
  await back(page, 'Frames');
  await (await back(page, 'Frames')).tap();

  // Devices
  await openTool(page, 'devices');
  const devices = pane(page, 'Devices');
  await expect(devices).toBeVisible({ timeout: 20_000 });
  await shot(page, 'ide-phone-devices-list');
  await devices.locator('[data-side-row]', { hasText: 'Pixel 7' }).tap();
  await back(page, 'Devices');
  await shot(page, 'ide-phone-devices-content');
  await (await back(page, 'Devices')).tap();
  await expect(devices.locator('[data-side-row]', { hasText: 'Pixel 7' })).toHaveAttribute('aria-current', 'true');
  await devices.locator('[data-side-row]', { hasText: 'iPhone 15' }).tap();
  await back(page, 'Devices');

  // The Side panel stays one tap away; ‹ Devices, then ‹ Files back to the list page with ☰.
  await expect(header.getByTestId('open-side-panel')).toBeVisible();
  await (await back(page, 'Devices')).tap();
  await (await back(page, 'Files')).tap();
  await expect(header.getByTestId('open-navigator')).toBeVisible();
});

test('tablet: each tool list sits beside its content', async ({ page }, info) => {
  test.skip(info.project.name !== 'ipad-portrait', 'tablet portrait');
  test.setTimeout(150_000);
  await page.setViewportSize({ width: 820, height: 1180 });
  const { root } = await bootToolsIde(page, 'files');
  await putRun(page, root);
  for (const [tab, name] of [['run', 'Tests'], ['git', 'Changes'], ['debug', 'Frames'], ['devices', 'Devices']] as const) {
    await openTool(page, tab);
    await expect(pane(page, name)).toBeVisible({ timeout: 20_000 });
    await expect(page.getByRole('button', { name: /^Back to (Tests|Changes|Frames|Devices)$/ })).toHaveCount(0);
    await shot(page, `ide-tablet-${tab}`);
  }
});
