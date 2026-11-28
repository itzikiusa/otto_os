import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { STARTER_KITS } from '../src/modules/design-hall/brand/starters.ts';

test.use({ serviceWorkers: 'block', viewport: { width: 390, height: 844 } });

// The Canvas page folded into Design Hall: a Canvas board opens on a phone as
// its Hall mirror (whiteboard), read-only with a reachable "Edit here" that
// takes it over (adopt). Editing the adopted whiteboard is the Hall's artifact
// view, covered by the Design Hall specs.
for (const theme of ['light', 'dark']) {
  test(`phone Canvas board opens as its Design Hall mirror with a reachable Edit here (${theme})`, async ({ page }, testInfo) => {
    test.setTimeout(60_000);
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const response = await ctx.post(`${base}/api/v1/workspaces/${ws}/canvas/scenes`, {
      data: { title: `Editable phone diagram ${theme}`, doc: { type: 'otto-canvas', version: 1, format: 'mermaid', source: 'flowchart LR\n A --> B' } },
    });
    expect(response.ok()).toBeTruthy();
    const scene = await response.json();
    let mirrorId = '';
    await expect
      .poll(async () => {
        const r = await ctx.get(`${base}/api/v1/design/artifacts?source_kind=canvas_scene&source_id=${scene.id}`);
        mirrorId = r.ok() ? (((await r.json()) as { id: string }[])[0]?.id ?? '') : '';
        return mirrorId;
      }, { timeout: 20_000 })
      .not.toBe('');
    await page.addInitScript(({ ws, theme }) => {
      localStorage.setItem('otto_workspace', ws);
      localStorage.setItem('otto_scheme', theme);
    }, { ws, theme });
    await page.goto(`/#/canvas/${scene.id}`);
    await expect(page).toHaveURL(new RegExp(`#/design/a/${mirrorId}`), { timeout: 20_000 });
    const banner = page.getByTestId('design-mirror-banner');
    await expect(banner).toBeVisible({ timeout: 20_000 });
    const adopt = page.getByTestId('design-adopt');
    await expectFullyInViewport(page, adopt, 'phone Edit here');
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: testInfo.outputPath(`canvas-mirror-phone-${theme}.png`) });
    await adopt.click();
    await expect(banner).toHaveCount(0, { timeout: 15_000 });
    await expect
      .poll(async () => {
        const r = await ctx.get(`${base}/api/v1/design/artifacts/${mirrorId}`);
        const d = (await r.json()) as { artifact?: { meta?: { adopted?: boolean } } };
        return d.artifact?.meta?.adopted === true;
      })
      .toBe(true);
    await expectNoHorizontalOverflow(page);
    await ctx.dispose();
  });
}

test('phone Site Studio exposes each editing panel without squeezing the page', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  const response = await ctx.post(`${base}/api/v1/design/artifacts`, {
    data: { workspace_id: ws, format: 'otto-site', studio: 'site', title: 'Phone site', content: JSON.stringify({ type: 'otto-site', version: 1, title: 'Phone site', pages: [] }) },
  });
  expect(response.ok()).toBeTruthy();
  const artifact = await response.json();
  await ctx.dispose();
  await page.addInitScript(ws => localStorage.setItem('otto_workspace', ws), ws);
  await page.goto(`/#/design/a/${artifact.artifact.id}`);
  await page.getByTestId('site-template').filter({ hasText: 'Landing page' }).click();
  // A phone: Pages · Layers · Blocks is the list pane (the page); a layer
  // pushes the canvas, the standard Back returns; the Design inspector is the
  // details sheet, opened from the bar.
  const views = page.getByRole('tablist', { name: 'Site panels' });
  await expect(views).toBeVisible();
  await views.getByRole('tab', { name: 'Blocks' }).click();
  await expect(page.getByTestId('site-left')).toBeVisible();
  await expect(page.getByRole('region', { name: 'Page canvas', exact: true })).not.toBeVisible();
  await page.getByTestId('site-tab-layers').click();
  await page.getByTestId('site-layer').filter({ hasText: 'Hero' }).click();
  await expect(page.getByRole('region', { name: 'Page canvas', exact: true })).toBeVisible();
  await page.getByTestId('site-details-toggle').click();
  const details = page.getByRole('dialog', { name: 'Design details', exact: true });
  await expect(details).toBeVisible();
  await expect(page.getByTestId('design-tab-design')).toHaveAttribute('aria-selected', 'true');
  await details.getByRole('button', { name: 'Close Design details', exact: true }).click();
  const actions = page.getByTestId('site-section-toolbar');
  await expect(actions).toBeVisible();
  await expectFullyInViewport(page, actions, 'phone section actions');
  const more = actions.getByRole('button', { name: 'More section actions' });
  expect((await more.boundingBox())!.height).toBeGreaterThanOrEqual(36);
  const stage = await page.getByRole('region', { name: 'Page canvas', exact: true }).boundingBox();
  const bar = await actions.boundingBox();
  expect(bar!.x + bar!.width).toBeLessThanOrEqual(stage!.x + stage!.width);
  await expectNoHorizontalOverflow(page);
});

test('phone brand kit has a reachable Back and touch-sized presets', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  const response = await ctx.post(`${base}/api/v1/design/artifacts`, {
    data: { workspace_id: ws, format: 'otto-brand', studio: 'brand', title: 'Phone brand', content: JSON.stringify(STARTER_KITS[0].build('Phone brand')) },
  });
  expect(response.ok()).toBeTruthy();
  const artifact = await response.json();
  await ctx.dispose();
  await page.addInitScript(ws => localStorage.setItem('otto_workspace', ws), ws);
  await page.goto(`/#/design/brand/${artifact.artifact.id}`);
  const back = page.getByRole('button', { name: 'Back to Design Hall' });
  await expect(back).toBeVisible();
  const preset = page.locator('.pchip').first();
  await expect(preset).toBeVisible();
  expect((await preset.boundingBox())!.height).toBeGreaterThanOrEqual(36);
  await expectNoHorizontalOverflow(page);
  await back.click();
  await expect(page).toHaveURL(/#\/design$/);
});

test('narrow 3D studio keeps scene tools reachable', async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 900 });
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  const response = await ctx.post(`${base}/api/v1/design/artifacts`, {
    data: { workspace_id: ws, format: 'scene3d', studio: '3d', title: 'Narrow scene', content: JSON.stringify({ type: 'otto-scene3d', version: 2, objects: [], lights: [], groups: [], states: [] }) },
  });
  expect(response.ok()).toBeTruthy();
  const artifact = await response.json();
  await ctx.dispose();
  await page.addInitScript(ws => localStorage.setItem('otto_workspace', ws), ws);
  await page.goto(`/#/design/a/${artifact.artifact.id}`);
  // A narrow window keeps the Scene tools as the list pane beside the viewport.
  await expect(page.getByTestId('s3d-left')).toBeVisible();
  await expectNoHorizontalOverflow(page);
});
