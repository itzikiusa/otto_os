import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow, openPage } from './helpers';

let workspaceId = '';
const sceneTitle = `Content audit flow ${Date.now()}`;
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const response = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/canvas/scenes`, {
    data: { title: sceneTitle, doc: { type: 'otto-canvas', version: 1, format: 'mermaid', source: 'flowchart LR\n A[Start] --> B[Finish]' } },
  });
  expect(response.ok()).toBeTruthy();
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id), workspaceId);
});

test('Product phone empty state exposes the import action immediately', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await openPage(page, 'product');
  const action = page.getByRole('button', { name: 'Import story', exact: true });
  await expectFullyInViewport(page, action, 'Import story onboarding');
  await page.screenshot({ path: '/tmp/otto-ux-content-product-phone.png' });
  await action.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog')).toBeVisible();
  await expectNoHorizontalOverflow(page);
});

for (const scheme of ['light', 'dark'] as const) {
  test(`Canvas phone assistant fits and closes in ${scheme}`, async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await page.addInitScript((s) => localStorage.setItem('otto_scheme', s), scheme);
    await openPage(page, 'canvas');
    await page.locator('.scene-list .row', { hasText: sceneTitle }).getByRole('button').first().click();
    await expect(page.locator('.board svg').first()).toBeVisible();
    await page.getByRole('button', { name: 'Ask AI', exact: true }).click();
    const panel = page.locator('.convo-panel');
    await expectFullyInViewport(page, panel, 'Canvas assistant');
    await expectFullyInViewport(page, panel.getByRole('button', { name: 'Close assistant' }));
    await page.screenshot({ path: `/tmp/otto-ux-content-canvas-${scheme}.png` });
    await panel.getByRole('button', { name: 'Close assistant' }).click();
    await expect(panel).toHaveCount(0);
    await expect(page.locator('.board svg').first()).toBeVisible();
    await expectNoHorizontalOverflow(page);
  });
}

test('Browser URL field preserves left-to-right editing in RTL', async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 768 });
  await page.addInitScript(() => localStorage.setItem('otto_direction', 'rtl'));
  await openPage(page, 'browser');
  const address = page.getByPlaceholder('Enter URL');
  await address.fill('https://example.invalid/docs?query=hello#intro');
  await expect(address).toHaveCSS('direction', 'ltr');
  await page.screenshot({ path: '/tmp/otto-ux-content-browser-rtl.png' });
  await expectNoHorizontalOverflow(page);
});

for (const [module, endpoint, emptyTitle] of [
  ['product', '**/product/stories', 'No stories yet'],
  ['vault', '**/vault/vaults', 'The docs home'],
  ['canvas', '**/api/v1/canvas/scenes', 'Start a new canvas'],
]) {
  test(`${module} phone list failure offers a reachable Retry and recovers`, async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    let failed = true;
    await page.route(endpoint, (route) => route.fulfill({
      status: failed ? 503 : 200,
      contentType: 'application/json',
      body: failed ? JSON.stringify({ code: 'unavailable', message: 'Temporarily unavailable' }) : '[]',
    }));
    await openPage(page, module);
    const retry = page.getByRole('button', { name: 'Retry', exact: true }).first();
    await expectFullyInViewport(page, retry, `${module} Retry`);
    failed = false;
    await retry.click();
    await expect(page.getByRole('heading', { name: emptyTitle, exact: true })).toBeVisible();
    await expect(page.getByTestId('load-error')).toHaveCount(0);
    await expectNoHorizontalOverflow(page);
  });
}
