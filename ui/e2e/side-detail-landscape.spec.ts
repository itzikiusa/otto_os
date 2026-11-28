import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// A phone on its side (viewport.navDrawer: a short touch screen) follows the
// phone rule for a Detail pane (design.md §3): a sheet, closed until asked for,
// full width when it opens — never inline and already open beside the content.

test.use({ serviceWorkers: 'block' });

test('a phone on its side: Design details is a closed sheet, full width when asked', async ({ page }, info) => {
  test.skip(!/phone-landscape/.test(info.project.name), 'a phone on its side');
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base, 'Landscape details');
  const r = await ctx.post(`${base}/api/v1/design/artifacts`, {
    data: { workspace_id: wsId, format: 'html', studio: 'frames', title: `Landscape ${Date.now()}`, content: '<h1>One</h1>' },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  const id = ((await r.json()) as { artifact: { id: string } }).artifact.id;
  await ctx.dispose();
  await page.addInitScript((ws) => localStorage.setItem('otto_workspace', ws), wsId);
  await openPage(page, `design/a/${encodeURIComponent(id)}`);
  const toggle = page.getByTestId('design-details-toggle');
  await expect(toggle).toBeVisible({ timeout: 20_000 });
  const details = page.getByTestId('design-details');
  await expect(details).toBeHidden();
  await expect(toggle).toHaveAttribute('aria-pressed', 'false');
  await toggle.tap();
  await expect(details).toBeVisible();
  const box = (await details.boundingBox())!;
  expect(box.width, 'full width, like a phone').toBeGreaterThan(page.viewportSize()!.width - 2);
});
