import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport } from './helpers';

// Home: the personal dashboard — up to 4 views of up to 8 live boxes on a
// 12-column grid, sliding between views (arrows / dots / ←→) with a 30 s
// auto-rotation, per-box resize (grid units) and zoom-to-fill.
//
// Layout is per device (localStorage), so every test starts from a clean
// storage; the boxes themselves talk to the isolated daemon (empty workspace →
// empty states), which is enough to prove wiring without fixtures.

test.describe.configure({ mode: 'serial' });

let wsId = '';

async function boot(page: Page, route = 'home'): Promise<void> {
  // The init script re-runs on every navigation (incl. reload), so the layout
  // reset is one-shot per tab — reload-persistence assertions stay meaningful.
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    if (sessionStorage.getItem('otto_e2e_home_reset')) return;
    sessionStorage.setItem('otto_e2e_home_reset', '1');
    localStorage.removeItem('otto_home_views');
    localStorage.removeItem('otto_home_active');
    localStorage.removeItem('otto_home_rotate');
  }, wsId);
  await page.goto(`/#/${route}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
}

async function addView(page: Page, name: string): Promise<void> {
  await page.getByRole('button', { name: 'Add view' }).click();
  const input = page.locator('.cf-input');
  await input.fill(name);
  await page.getByRole('button', { name: 'Create' }).click();
}

test.beforeAll(async () => {
  const c = await apiCtx();
  wsId = await seedWorkspace(c.ctx, c.base);
});

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
});

test('Home is the first sidebar entry and seeds a default view of live boxes', async ({ page }) => {
  await boot(page);
  // The sidebar (collapsed rail or expanded navigator) carries a Home entry
  // and it routes here.
  await page.goto('/#/agents');
  await page.locator('.rail-btn[aria-label="Home"], .sidebar-material button:has-text("Home")').first().click();
  await expect(page).toHaveURL(/#\/home$/);
  const home = page.getByRole('region', { name: 'Home dashboard' });
  await expect(home).toBeVisible();
  // Root sees every kind → the seed places four boxes.
  const boxes = home.locator('section.hbox');
  await expect(boxes).toHaveCount(4);
  await expect(boxes.nth(0)).toHaveAttribute('data-kind', 'sessions');
  await expect(boxes.nth(1)).toHaveAttribute('data-kind', 'mission-control');
  // Every box body reaches a settled state (empty state or data), never a
  // permanent skeleton.
  await expect(boxes.nth(0)).toContainText(/open|No open agent sessions/);
  await expect(boxes.nth(1)).toContainText(/Nothing in flight|active/);
  await expect(boxes.nth(2)).toContainText(/No clusters yet|cluster/);
  await expect(boxes.nth(3)).toContainText(/No DB dashboards yet|Pick a dashboard/);
  // A single view: no rotation control, no arrows, one dot.
  await expect(page.getByRole('tab')).toHaveCount(1);
  await expect(page.getByRole('button', { name: 'Next view' })).toHaveCount(0);
});

test('views: add (max 4), slide with arrows / dots / keys, rename, delete, persist', async ({ page }) => {
  await boot(page);
  await addView(page, 'Second');
  await expect(page.getByRole('tab')).toHaveCount(2);
  await expect(page.locator('.vname')).toContainText('Second');
  // New view starts empty with the add-box empty state.
  await expect(page.locator('.stage')).toContainText('This view is empty');
  // Arrows wrap around.
  await page.getByRole('button', { name: 'Previous view' }).click();
  await expect(page.locator('.vname')).toContainText('Overview');
  await page.getByRole('button', { name: 'Next view' }).click();
  await expect(page.locator('.vname')).toContainText('Second');
  // Keyboard.
  await page.locator('.stage').click({ position: { x: 5, y: 5 } });
  await page.keyboard.press('ArrowRight');
  await expect(page.locator('.vname')).toContainText('Overview');
  // Dots.
  await page.getByRole('tab', { name: 'Second' }).click();
  await expect(page.locator('.vname')).toContainText('Second');
  // Rename via the view menu.
  await page.locator('.vname').click();
  await page.getByRole('menuitem', { name: /Rename view/ }).click();
  await page.locator('.cf-input').fill('Ops');
  await page.getByRole('button', { name: 'Rename' }).click();
  await expect(page.locator('.vname')).toContainText('Ops');
  // Cap at 4.
  await addView(page, 'Three');
  await addView(page, 'Four');
  await expect(page.getByRole('tab')).toHaveCount(4);
  await expect(page.getByRole('button', { name: 'Add view' })).toHaveCount(0);
  // Auto-rotate control appears with >1 views, defaults ON, toggles and persists.
  const rot = page.getByRole('button', { name: /Auto-rotate/ });
  await expect(rot).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('.progress i')).toBeVisible();
  await rot.click();
  await expect(rot).toHaveAttribute('aria-pressed', 'false');
  await expect(page.locator('.progress i')).toHaveCount(0);
  // Persistence across reload (no init-script reset this time).
  await page.reload();
  await expect(page.locator('.shell')).toBeVisible();
  await expect(page.getByRole('tab')).toHaveCount(4);
  await expect(page.locator('.vname')).toContainText('Four');
  await expect(page.getByRole('button', { name: /Auto-rotate/ })).toHaveAttribute('aria-pressed', 'false');
  // Delete the current view.
  await page.locator('.vname').click();
  await page.getByRole('menuitem', { name: /Delete view/ }).click();
  await page.getByRole('button', { name: 'Delete' }).click();
  await expect(page.getByRole('tab')).toHaveCount(3);
});

test('boxes: add from the picker (max 8), resize in grid units, zoom, remove', async ({ page }) => {
  await boot(page);
  const home = page.getByRole('region', { name: 'Home dashboard' });
  const boxes = home.locator('section.hbox');
  await expect(boxes).toHaveCount(4);

  // Add box picker lists every kind for root and adds one.
  await page.getByRole('button', { name: 'Add box' }).click();
  const sheet = page.getByRole('dialog');
  await expectFullyInViewport(page, sheet, 'add-box sheet');
  await expect(sheet.locator('.kind')).toHaveCount(6);
  await sheet.locator('.kind', { hasText: 'Usage' }).click();
  await expect(boxes).toHaveCount(5);
  await expect(boxes.nth(4)).toHaveAttribute('data-kind', 'usage');
  await expect(boxes.nth(4)).toContainText(/spend|Usage engine is off|Usage unavailable/);

  // Cap: fill to 8 then the button disables.
  for (const k of ['Insights', 'Agents', 'Kubernetes']) {
    await page.getByRole('button', { name: 'Add box' }).click();
    await page.getByRole('dialog').locator('.kind', { hasText: k }).click();
  }
  await expect(boxes).toHaveCount(8);
  await expect(page.getByRole('button', { name: 'Add box' })).toBeDisabled();

  // Resize with the keyboard on the corner handle: one grid unit per arrow.
  const first = boxes.nth(0);
  const before = await first.evaluate((el) => (el as HTMLElement).style.gridColumn);
  expect(before).toBe('span 4');
  const handle = first.getByRole('slider');
  await handle.focus();
  await page.keyboard.press('ArrowRight');
  await page.keyboard.press('ArrowRight');
  await page.keyboard.press('ArrowDown');
  await expect(first).toHaveAttribute('aria-label', 'Agents');
  expect(await first.evaluate((el) => (el as HTMLElement).style.gridColumn)).toBe('span 6');
  expect(await first.evaluate((el) => (el as HTMLElement).style.gridRow)).toBe('span 5');
  // Never below the minimum or above 12 columns.
  for (let i = 0; i < 20; i++) await page.keyboard.press('ArrowRight');
  expect(await first.evaluate((el) => (el as HTMLElement).style.gridColumn)).toBe('span 12');
  for (let i = 0; i < 20; i++) await page.keyboard.press('ArrowLeft');
  expect(await first.evaluate((el) => (el as HTMLElement).style.gridColumn)).toBe('span 3');

  // Pointer resize: drag the corner handle two columns to the right.
  const box = await first.boundingBox();
  const h = await handle.boundingBox();
  if (!box || !h) throw new Error('no box');
  const colPx = box.width / 3;
  await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2);
  await page.mouse.down();
  await page.mouse.move(h.x + h.width / 2 + colPx * 2, h.y + h.height / 2, { steps: 6 });
  await page.mouse.up();
  expect(await first.evaluate((el) => (el as HTMLElement).style.gridColumn)).toBe('span 5');

  // Zoom fills the stage; Esc exits; the grid comes back intact.
  await first.getByRole('button', { name: 'Zoom in' }).click();
  const zoomed = home.locator('section.hbox.zoomed');
  await expect(zoomed).toHaveCount(1);
  await expect(zoomed).toHaveAttribute('data-kind', 'sessions');
  await expect(home.locator('section.hbox')).toHaveCount(1);
  const stage = await home.locator('.stage').boundingBox();
  const z = await zoomed.boundingBox();
  if (!stage || !z) throw new Error('no stage');
  expect(z.width).toBeGreaterThan(stage.width - 40);
  expect(z.height).toBeGreaterThan(stage.height - 40);
  await page.keyboard.press('Escape');
  await expect(home.locator('section.hbox.zoomed')).toHaveCount(0);
  await expect(boxes).toHaveCount(8);

  // Remove via the box menu; the resize survives a reload.
  await boxes.nth(7).getByRole('button', { name: 'Box menu' }).click();
  await page.getByRole('menuitem', { name: /Remove box/ }).click();
  await expect(boxes).toHaveCount(7);
  await page.reload();
  await expect(page.locator('.shell')).toBeVisible();
  await expect(boxes).toHaveCount(7);
  expect(await boxes.nth(0).evaluate((el) => (el as HTMLElement).style.gridColumn)).toBe('span 5');
});

test('Go to Home is in the palette and the page never overflows horizontally', async ({ page }) => {
  await boot(page, 'agents');
  await page.keyboard.press('Meta+k');
  await page.keyboard.type('go to home');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('region', { name: 'Home dashboard' })).toBeVisible();
  const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(over).toBeLessThanOrEqual(1);
});
