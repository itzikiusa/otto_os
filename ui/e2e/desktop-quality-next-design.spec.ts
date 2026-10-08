import { test, expect, type Route } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

for (const appearance of [
  { theme: 'native', scheme: 'light' },
  { theme: 'native', scheme: 'dark' },
  { theme: 'warm', scheme: 'dark' },
]) test(`header overflow preserves keyboard focus and action across viewport changes ${appearance.theme}-${appearance.scheme}`, async ({ page }, info) => {
  const { ctx, base } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript(id => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, workspace);
  await page.addInitScript(value => {
    localStorage.setItem('otto_theme', value.theme);
    localStorage.setItem('otto_scheme', value.scheme);
  }, appearance);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/api');
  // Open a request before resizing: the empty-state load removes the header's
  // New action asynchronously, and Linux fonts can then fit every action.
  // A populated editor gives this overflow test a stable, genuinely full row.
  await page.getByRole('button', { name: 'New request', exact: true }).click();
  await expect(page.getByLabel('Request URL', { exact: true })).toBeFocused();
  const header = page.getByTestId('page-header');
  const sync = header.getByRole('button', { name: 'Sync with Git…', exact: true });
  await expect(sync).toBeVisible();
  await sync.focus();
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(sync).toBeHidden();
  const more = header.getByRole('button', { name: 'More actions', exact: true });
  await expect(more).toBeFocused();
  await more.press('Enter');
  const menu = page.getByRole('menu', { name: 'Context menu' });
  await expectFullyInViewport(page, menu);
  const syncItem = menu.getByRole('menuitem', { name: 'Sync with Git…', exact: true });
  await expect(syncItem).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(more).toBeFocused();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('header-phone.png') });
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(sync).toBeVisible();
  await expect(sync).toBeFocused();
  await page.screenshot({ path: info.outputPath('header-desktop.png') });
  // An unrelated editor retains its focus when toolbar actions collapse.
  await page.getByRole('button', { name: 'New request', exact: true }).click();
  const url = page.getByLabel('Request URL', { exact: true });
  await url.fill('https://fixture.invalid/unsaved');
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(url).toBeFocused();
  await expect(url).toHaveValue('https://fixture.invalid/unsaved');
  await more.click();
  await syncItem.click();
  const dialog = page.getByRole('dialog', { name: 'Sync collections with Git' });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
});

test('workspace boot failure is reported inline and Retry restores the saved workspace', async ({ page }, info) => {
  const { ctx, base } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  await ctx.dispose();
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  let fail = true;
  await page.route('**/api/v1/workspaces', route => fail
    ? route.fulfill({ status: 503, json: { code: 'unavailable', message: 'Workspace list temporarily unavailable' } })
    : route.continue());
  await page.addInitScript(id => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_rail_expanded', '1');
  }, workspace);
  await page.goto('/#/api');
  const error = page.getByTestId('workspace-load-error');
  await expect(error).toContainText('Workspace list temporarily unavailable');
  expect(errors).toEqual([]);
  await page.screenshot({ path: info.outputPath('workspace-retry-desktop.png') });
  await page.setViewportSize({ width: 390, height: 844 });
  await expectFullyInViewport(page, error);
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('workspace-retry-phone.png') });
  await page.setViewportSize({ width: 1440, height: 900 });
  fail = false;
  await error.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(error).toHaveCount(0);
  await expect(page.locator('.navigator .active-ws')).toBeVisible();
  expect(errors).toEqual([]);
});

test('a failed older workspace load cannot publish into a newer identity', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  await ctx.dispose();
  let held!: (route: Route) => void;
  const pending = new Promise<Route>(resolve => { held = resolve; });
  let loads = 0;
  await page.route('**/api/v1/workspaces', route => {
    if (++loads === 1) { held(route); return; }
    return route.continue();
  });
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(id => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_rail_expanded', '1');
  }, workspace);
  await page.goto('/#/api');
  const older = await pending;
  await page.evaluate(async () => {
    const path = '/src/lib/stores/auth.svelte.ts';
    const { auth } = await import(/* @vite-ignore */ path);
    auth.me = { ...auth.me, id: 'synthetic-new-effective-identity' };
  });
  await expect(page.locator('.navigator .active-ws')).toBeVisible();
  await older.fulfill({ status: 503, json: { code: 'unavailable', message: 'Old identity workspace list failed' } });
  // Flush the rejected response and Svelte render before checking publication.
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  await expect(page.getByTestId('workspace-load-error')).toHaveCount(0);
  await expect(page.locator('.navigator .active-ws')).toBeVisible();
  expect(errors).toEqual([]);
});
