import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { STARTER_KITS } from '../src/modules/design-hall/brand/starters.ts';

test.use({ serviceWorkers: 'block', viewport: { width: 390, height: 844 } });

for (const theme of ['light', 'dark']) {
  test(`phone Canvas remains editable (${theme})`, async ({ page }, testInfo) => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const response = await ctx.post(`${base}/api/v1/workspaces/${ws}/canvas/scenes`, {
      data: { title: `Editable phone diagram ${theme}`, doc: { type: 'otto-canvas', version: 1, format: 'mermaid', source: 'flowchart LR\n A --> B' } },
    });
    expect(response.ok()).toBeTruthy();
    const scene = await response.json();
    await page.addInitScript(({ ws, theme }) => {
      localStorage.setItem('otto_workspace', ws);
      localStorage.setItem('otto_scheme', theme);
    }, { ws, theme });
    await page.goto('/#/canvas');
    await page.locator('.scene-list .row', { hasText: `Editable phone diagram ${theme}` }).click();
    await page.getByRole('button', { name: 'Code', exact: true }).click();
    const editor = page.locator('.code-pane .cm-content');
    await expect(editor).toBeVisible();
    await editor.fill('flowchart LR\n Phone --> Saved');
    await expect.poll(async () => {
      const result = await ctx.get(`${base}/api/v1/canvas/scenes/${scene.id}`);
      return JSON.stringify(await result.json());
    }).toContain('Phone --> Saved');
    const zoom = page.getByRole('button', { name: 'Zoom in', exact: true });
    expect((await zoom.boundingBox())!.height).toBeGreaterThanOrEqual(36);
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: testInfo.outputPath(`canvas-phone-${theme}.png`) });
    await page.getByRole('button', { name: 'Ask Otto', exact: true }).click();
    const close = page.getByRole('button', { name: 'Close assistant', exact: true });
    await expectFullyInViewport(page, close, 'phone assistant Close');
    const title = await page.locator('.assistant .head .title').boundingBox();
    const closeBox = await close.boundingBox();
    expect(Math.abs(closeBox!.y - title!.y)).toBeLessThan(18);
    await expectNoHorizontalOverflow(page);
    await close.click();
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
  const panels = page.getByRole('group', { name: 'Site panels' });
  await expect(panels).toBeVisible();
  await expect(page.getByRole('region', { name: 'Page canvas', exact: true })).toBeVisible();
  await panels.getByRole('button', { name: 'Blocks', exact: true }).click();
  await expect(page.getByRole('complementary', { name: 'Pages, layers and blocks' })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Page canvas', exact: true })).not.toBeVisible();
  await page.getByTestId('site-tab-layers').click();
  await page.getByTestId('site-layer').filter({ hasText: 'Hero' }).click();
  await expect(panels.getByRole('button', { name: 'Design', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await panels.getByRole('button', { name: 'Design', exact: true }).click();
  await expect(page.getByRole('complementary', { name: 'Design', exact: true })).toBeVisible();
  await panels.getByRole('button', { name: 'Page', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Page canvas', exact: true })).toBeVisible();
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
  const toggle = page.locator('button[aria-controls="s3d-scene-tools"]');
  await expect(toggle).toBeVisible();
  await expect(page.getByTestId('s3d-left')).not.toBeVisible();
  await toggle.click();
  await expect(page.getByTestId('s3d-left')).toBeVisible();
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  await expectNoHorizontalOverflow(page);
});
