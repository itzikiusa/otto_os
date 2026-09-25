import { test, expect, type Page, type FrameLocator } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Side by side (desktop-browser only): any two sidebar sections split the
// content column — the main pane + a side pane, which is the app itself in a
// same-origin iframe at `?embed=1#/<route>` (lib/sidePane.ts).
//
//   • Agents + Connections via the sidebar row's context menu; both render;
//   • the divider resizes from the keyboard (role="separator", aria-valuenow)
//     and Enter / double-click reset it to 50/50;
//   • Swap flips the panes' places WITHOUT reloading the pane's document;
//   • the pane's route, split and placement survive a reload;
//   • navigating inside the pane never touches the main window's hash, and a
//     navigation to the main pane's module is handed back to it;
//   • the pane's module can't open twice: ⌥-click / ⌘\ picker / close.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

let workspaceId = '';

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  if (!workspaceId) {
    const { ctx, base } = await apiCtx();
    workspaceId = await seedWorkspace(ctx, base);
    await ctx.dispose();
  }
  // One-shot reset (init scripts also run in the pane's iframe and on reload:
  // the sessionStorage flag — shared with the iframe — keeps it to the first
  // document of the test).
  await page.addInitScript((w) => {
    if (sessionStorage.getItem('side-reset')) return;
    sessionStorage.setItem('side-reset', '1');
    localStorage.removeItem('otto_side_pane');
    localStorage.setItem('otto_rail_expanded', '1');
    localStorage.setItem('otto_workspace', w as string);
  }, workspaceId);
});

const pane = (page: Page) => page.getByTestId('side-pane');
const frame = (page: Page): FrameLocator => page.frameLocator('[data-testid="side-pane-frame"]');
const divider = (page: Page) => page.getByTestId('split-divider');
const menu = (page: Page) => page.locator('.ctx-menu');
const navRow = (page: Page, id: string) =>
  page.locator(`.navigator [data-testid="sidebar-modules"] [data-nav-id="${id}"]`).first();
const mainHash = (page: Page) => page.evaluate(() => window.location.hash);
const saved = (page: Page) =>
  page.evaluate(() => JSON.parse(localStorage.getItem('otto_side_pane') || 'null') as {
    route: string | null;
    share: number;
    placement: string;
  } | null);

async function openAgents(page: Page): Promise<void> {
  await page.goto('/#/agents');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.navigator')).toBeVisible({ timeout: 15_000 });
}

/** Right-click Connections → "Open side by side", and wait for the pane's
 *  document to boot (the host's loading cover goes away). */
async function openConnectionsBeside(page: Page): Promise<void> {
  await navRow(page, 'connections').click({ button: 'right' });
  await expectFullyInViewport(page, menu(page), 'sidebar row menu');
  await menu(page).getByRole('menuitem', { name: 'Open side by side' }).click();
  await expect(pane(page)).toBeVisible();
  await expect(pane(page)).toHaveAttribute('data-module', 'connections');
  await expect(page.getByTestId('side-pane-cover')).toHaveCount(0, { timeout: 30_000 });
  await expect(frame(page).getByTestId('page-header')).toBeVisible({ timeout: 15_000 });
}

test('Agents + Connections side by side: both render, resize from the keyboard, swap, close', async ({ page }) => {
  await openAgents(page);
  const before = await mainHash(page);
  await openConnectionsBeside(page);

  // Both panes render: the main pane is still Agents (its tab row), the side
  // pane is the Connections page — inside the iframe, with the pane controls
  // at the end of its own header row.
  await expect(page.locator('.primary-pane .center')).toBeVisible();
  await expect(page.locator('.primary-pane .tabbar')).toBeVisible();
  await expect(frame(page).getByTestId('page-header')).toContainText('Connections');
  await expect(frame(page).getByTestId('pane-controls')).toBeVisible();
  // The embedded document is chrome-less: no sidebar, status bar or floating bar.
  await expect(frame(page).locator('.navigator, .rail, .statusbar, .fb')).toHaveCount(0);
  // The sidebar marks the module the side pane shows.
  await expect(page.getByTestId('side-mark-connections')).toBeVisible();
  expect(await mainHash(page)).toBe(before);
  await expectNoHorizontalOverflow(page);

  // Keyboard resize: the divider is a focusable separator.
  const div = divider(page);
  await expect(div).toHaveAttribute('role', 'separator');
  await expect(div).toHaveAttribute('aria-valuenow', '50');
  const paneW0 = (await pane(page).boundingBox())!.width;
  await div.focus();
  await page.keyboard.press('ArrowLeft');
  await page.keyboard.press('ArrowLeft');
  await expect(div).toHaveAttribute('aria-valuenow', '46');
  await page.keyboard.press('Shift+ArrowRight');
  await expect(div).toHaveAttribute('aria-valuenow', '56');
  // The main pane grew → the side pane shrank.
  expect((await pane(page).boundingBox())!.width).toBeLessThan(paneW0);
  await expect.poll(async () => (await saved(page))?.share).toBeCloseTo(0.44, 2);
  await page.keyboard.press('Enter');
  await expect(div).toHaveAttribute('aria-valuenow', '50');

  // Swap: the side pane moves to the leading edge; its document is NOT
  // reloaded (a marker set inside it survives).
  const iframe = page.frames().find((f) => f.url().includes('embed=1'))!;
  await iframe.evaluate(() => ((window as unknown as { __sideMarker: number }).__sideMarker = 7));
  const xBefore = (await pane(page).boundingBox())!.x;
  await frame(page).getByTestId('side-pane-swap').click();
  await expect.poll(async () => (await pane(page).boundingBox())!.x).toBeLessThan(xBefore);
  expect(await iframe.evaluate(() => (window as unknown as { __sideMarker?: number }).__sideMarker)).toBe(7);
  await expect.poll(async () => (await saved(page))?.placement).toBe('leading');

  // Close from the pane's own header.
  await frame(page).getByTestId('side-pane-close').click();
  await expect(pane(page)).toHaveCount(0);
  await expect(page.getByTestId('side-mark-connections')).toHaveCount(0);
  await expect.poll(async () => (await saved(page))?.route ?? null).toBeNull();
  expect(await mainHash(page)).toBe(before);
});

test('the side pane survives a reload, and its navigation never moves the main hash', async ({ page }) => {
  await openAgents(page);
  await openConnectionsBeside(page);
  const before = await mainHash(page);

  // Navigate INSIDE the pane (a hash link in its document): the main window's
  // hash stays put; the pane's route is remembered.
  const iframe = page.frames().find((f) => f.url().includes('embed=1'))!;
  await iframe.evaluate(() => {
    window.location.hash = '#/git';
  });
  await expect(pane(page)).toHaveAttribute('data-module', 'git');
  await expect(frame(page).getByTestId('page-header')).toContainText('Git');
  expect(await mainHash(page)).toBe(before);
  await expect.poll(async () => (await saved(page))?.route).toBe('git');

  // A navigation in the pane to the MAIN pane's module is handed to the main
  // pane — the pane stays on Git, Agents is never open twice.
  await iframe.evaluate(() => {
    window.location.hash = '#/agents';
  });
  await expect.poll(() => iframe.evaluate(() => window.location.hash)).toBe('#/git');
  await expect(pane(page)).toHaveAttribute('data-module', 'git');

  // Reload: the pane comes back on its last route, beside the same main page.
  await page.reload();
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  await expect(pane(page)).toBeVisible({ timeout: 15_000 });
  await expect(pane(page)).toHaveAttribute('data-module', 'git');
  await expect(page.getByTestId('side-pane-cover')).toHaveCount(0, { timeout: 30_000 });
  await expect(frame(page).getByTestId('page-header')).toContainText('Git');
  expect(await mainHash(page)).toBe(before);
});

test('one module per pane: sidebar clicks go to the pane that shows it; ⌥-click and ⌘\\ open and close', async ({ page }) => {
  await openAgents(page);
  const before = await mainHash(page);

  // ⌥-click opens a module side by side.
  await navRow(page, 'connections').click({ modifiers: ['Alt'] });
  await expect(pane(page)).toHaveAttribute('data-module', 'connections');
  await expect(page.getByTestId('side-pane-cover')).toHaveCount(0, { timeout: 30_000 });

  // A plain click on the pane's module keeps the main pane where it is.
  await navRow(page, 'connections').click();
  expect(await mainHash(page)).toBe(before);
  await expect(pane(page)).toHaveAttribute('data-module', 'connections');

  // The main pane's own module is not offered for the side pane.
  await navRow(page, 'agents').click({ button: 'right' });
  await expect(menu(page)).toBeVisible();
  await expect(menu(page).getByRole('menuitem', { name: /side by side|side pane/i })).toHaveCount(0);
  await page.keyboard.press('Escape');

  // The pane's module row offers to promote / close it.
  await navRow(page, 'connections').click({ button: 'right' });
  await expect(menu(page).getByRole('menuitem', { name: 'Close side pane' })).toBeVisible();
  await page.keyboard.press('Escape');

  // ⌘\ closes an open pane (focus is back on the sidebar row, in the main
  // document)…
  await page.keyboard.press('Meta+Backslash');
  await expect(pane(page)).toHaveCount(0);

  // …and opens the module picker when there is none (filterable menu).
  await page.keyboard.press('Meta+Backslash');
  await expect(menu(page)).toBeVisible();
  await expectFullyInViewport(page, menu(page), 'side pane picker');
  // Picker rows are checkable (the pane's current module is ticked), so they
  // carry role=menuitemcheckbox.
  await expect(menu(page).getByRole('menuitemcheckbox', { name: 'Git', exact: true })).toBeVisible();
  await expect(menu(page).getByRole('menuitemcheckbox', { name: 'Agents', exact: true })).toHaveCount(0);
  await menu(page).locator('.ctx-search-input').fill('Git');
  await menu(page).getByRole('menuitemcheckbox', { name: 'Git', exact: true }).click();
  await expect(pane(page)).toHaveAttribute('data-module', 'git');

  // "Open in main pane": the page moves to the main pane, the pane closes.
  await expect(page.getByTestId('side-pane-cover')).toHaveCount(0, { timeout: 30_000 });
  await frame(page).getByTestId('side-pane-promote').click();
  await expect(pane(page)).toHaveCount(0);
  await expect.poll(() => mainHash(page)).toBe('#/git');
});
