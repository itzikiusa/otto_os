import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';
import { expectFullyInViewport } from './helpers';

// DB workbench chrome (desktop): one way to the side pane, the header ⋯ on a
// narrow window. Engine calls are the repo's db-mock; the daemon is real.
test.use({ serviceWorkers: 'block' });

let wsId = '';
let connId = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  connId = await seedMockDbConnection(ctx, base, wsId, 'chrome-mysql');
  await ctx.dispose();
});

async function openWorkbench(page: Page): Promise<void> {
  await page.addInitScript((w) => { localStorage.setItem('otto_workspace', w as string); localStorage.setItem('otto_firstrun_dismissed', '1'); }, wsId);
  await mockDbRoutes(page, connId);
  await page.goto(`/#/database/${connId}`);
  await expect(page.getByTestId('db-conn-state')).toContainText('Connected');
}

test('closing the only tab, or a link to a connection that is gone, lands on the connection list', async ({ page }) => {
  await openWorkbench(page);
  await page.getByTestId('page-header').getByRole('button', { name: 'More actions' }).click();
  await page.getByRole('menuitem', { name: /^Close connection/ }).click();
  await expect(page).toHaveURL(/#\/connections$/);
  await page.goto('/#/database/01NOSUCHCONNECTION0000000000');
  await expect(page).toHaveURL(/#\/connections$/);
});

test('the Database pane beside Agents keeps the window on Agents when its last tab closes', async ({ page }) => {
  await openWorkbench(page);
  await page.getByTestId('page-header').getByRole('button', { name: 'More actions' }).click();
  await page.getByRole('menuitem', { name: /^Open beside agents/ }).click();
  await expect(page).toHaveURL(/#\/agents/);
  const pane = page.locator('.db-root');
  await expect(pane.getByTestId('db-conn-state')).toBeVisible();
  await pane.getByTestId('page-header').getByRole('button', { name: 'More actions' }).click();
  await page.getByRole('menuitem', { name: /^Close connection/ }).click();
  await expect(pane.getByRole('button', { name: 'Open connections' })).toBeVisible();
  await expect(page).toHaveURL(/#\/agents/);
});

test('Tables and History: the open pane is the only way; hidden, the header opens it on that tab', async ({ page }) => {
  await openWorkbench(page);
  const header = page.getByTestId('page-header');
  const side = page.getByRole('tablist', { name: 'Sidebar view' });
  // The pane is open: its tabs are the controls, the header has no duplicates.
  await expect(side.getByRole('tab', { name: 'Tables', exact: true })).toBeVisible();
  await expect(header.getByRole('button', { name: 'Tables', exact: true })).toHaveCount(0);
  await expect(header.getByRole('button', { name: 'History', exact: true })).toHaveCount(0);
  // Hide it from the ⋯; now the header offers both, one click each.
  await header.getByRole('button', { name: 'More actions' }).click();
  await page.getByRole('menuitem', { name: /^Hide tables/ }).click();
  await expect(side).toBeHidden();
  await header.getByRole('button', { name: 'History', exact: true }).click();
  await expect(side.getByRole('tab', { name: 'History', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expect(header.getByRole('button', { name: 'Tables', exact: true })).toHaveCount(0);
});

test('375 px: the header ⋯ opens a menu that fits the window and scrolls to its last row', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 667 });
  await openWorkbench(page);
  const header = page.getByTestId('page-header');
  await expect(header.getByRole('button', { name: 'Tables', exact: true })).toBeVisible();
  const more = header.getByRole('button', { name: 'More actions' });
  await expectFullyInViewport(page, more, 'the header ⋯');
  await more.click();
  const menu = page.getByRole('menu');
  await expectFullyInViewport(page, menu, 'the workbench ⋯ menu');
  const last = menu.getByRole('menuitem', { name: /^Close connection/ });
  await last.scrollIntoViewIfNeeded();
  await expectFullyInViewport(page, last, 'its last row');
});

test('opening from the list adds only what the workbench needs: no second cluster list, saved queries when shown', async ({ page }) => {
  await page.addInitScript((w) => { localStorage.setItem('otto_workspace', w as string); localStorage.setItem('otto_firstrun_dismissed', '1'); }, wsId);
  await mockDbRoutes(page, connId);
  const seen: string[] = [];
  page.on('request', (r) => seen.push(new URL(r.url()).pathname));
  const count = (s: string) => seen.filter((p) => p.endsWith(s)).length;
  await page.goto('/#/connections');
  await expect(page.locator(`.conn-row[data-connection-id="${connId}"]`)).toBeVisible();
  await expect.poll(() => count('/brokers/clusters')).toBe(1);
  seen.length = 0;
  await page.locator(`.conn-row[data-connection-id="${connId}"]`).click();
  await expect(page.getByTestId('db-conn-state')).toContainText('Connected');
  await page.waitForTimeout(1500); // ui-guards: allow — proves no late request arrives
  expect(count('/brokers/clusters')).toBe(0);
  expect(count('/saved-queries')).toBe(0);
  await page.getByRole('tablist', { name: 'Sidebar view' }).getByRole('tab', { name: 'Saved', exact: true }).click();
  await expect.poll(() => count('/saved-queries')).toBe(1);
});
