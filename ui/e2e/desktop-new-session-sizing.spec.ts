import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';
import { openNewSessionSheet, openPage, expectFullyInViewport } from './helpers';

let workspaceId = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  await seedShellSession(ctx, base, workspaceId);
  await ctx.dispose();
});

for (const viewport of [{ width: 1440, height: 1200 }, { width: 390, height: 844 }]) {
  for (const scheme of ['light', 'dark']) {
    test(`provider switches keep the sheet stable at ${viewport.width}px in ${scheme}`, async ({ page }, testInfo) => {
      await page.setViewportSize(viewport);
      await page.addInitScript(({ id, scheme }) => {
        localStorage.setItem('otto_workspace', id);
        localStorage.setItem('otto_firstrun_dismissed', '1');
        localStorage.setItem('otto_scheme', scheme);
      }, { id: workspaceId, scheme });
      await openPage(page, 'agents');
      const dialog = await openNewSessionSheet(page);
      const cards = dialog.locator('.card-main:not([disabled])');
      expect(await cards.count()).toBeGreaterThan(1);
      // Shell has fewer fields; starting there catches growth as well as shrink.
      await dialog.locator('.provider-card', { hasText: 'shell' }).locator('.card-main').click();
      await expect(dialog.locator('#ns-prompt')).toHaveCount(0);
      const initial = await dialog.boundingBox();
      expect(initial).not.toBeNull();
      for (let i = 0; i < await cards.count(); i++) {
        await cards.nth(i).click();
        await expect.poll(async () => {
          const box = await dialog.boundingBox();
          return box && Math.max(...(['x', 'y', 'width', 'height'] as const).map(key => Math.abs(box[key] - initial![key])));
        }).toBeLessThan(1);
        await expectFullyInViewport(page, dialog);
        await expectFullyInViewport(page, dialog.locator('footer'));
      }
      await dialog.locator('#ns-extra-dir').scrollIntoViewIfNeeded();
      await expect(dialog.locator('#ns-extra-dir')).toBeInViewport();
      await testInfo.attach(`new-session-${viewport.width}-${scheme}`, { body: await page.screenshot(), contentType: 'image/png' });
    });
  }
}
