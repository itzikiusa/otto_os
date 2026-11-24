import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';

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
