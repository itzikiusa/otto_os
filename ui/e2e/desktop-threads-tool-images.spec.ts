import { test, expect, type APIRequestContext } from '@playwright/test';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// Uses the real fake-Codex process → driver → persisted runtime → UI path.
let ctx: APIRequestContext;
let base: string;
let workspace: string;
let dir: string;
test.beforeAll(async () => {
  ({ ctx, base } = await apiCtx());
  dir = mkdtempSync(join(tmpdir(), 'otto-tool-images-'));
  const result = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Tool images', root_path: dir } });
  expect(result.ok(), await result.text()).toBeTruthy();
  workspace = (await result.json()).id;
});
test.afterAll(async () => {
  if (ctx && workspace) {
    const sessions = await (await ctx.get(`${base}/api/v1/workspaces/${workspace}/runtime-sessions`)).json() as { id: string }[];
    for (const session of sessions) await ctx.post(`${base}/api/v1/runtime-sessions/${session.id}/close`).catch(() => {});
  }
  await ctx?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
});
test('tool images retain text, enlarge with keyboard, survive reload and fit phone widths', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    if (!sessionStorage.getItem('tool-images-init')) {
      localStorage.removeItem('otto_threads_v1');
      sessionStorage.setItem('tool-images-init', '1');
    }
  }, workspace);
  await page.goto('/#/threads');
  const stage = page.getByTestId('new-thread-stage');
  await expect(stage).toBeVisible({ timeout: 30_000 });
  await stage.getByTestId('picker-agent').click();
  await page.getByTestId('model-picker-provider-codex').click();
  await page.keyboard.press('Escape');
  await stage.getByTestId('new-thread-input').fill('tool image');
  await stage.getByTestId('new-thread-send-btn').click();
  const row = page.locator('[data-item-type="dynamic_tool"]').filter({ hasText: 'preview.screenshot' });
  await expect(row).toBeVisible({ timeout: 30_000 });
  await row.locator('.rw-line').click();
  await expect(row).toContainText('Screenshot captured');
  await expect(row).toContainText('"width":1');
  const previews = row.getByTestId('runtime-tool-images');
  await expect(previews.locator('img')).toHaveCount(2);
  await expect.poll(() => previews.locator('img').first().evaluate((img: HTMLImageElement) => img.naturalWidth)).toBe(1);
  const opener = previews.getByRole('button', { name: 'Enlarge tool image 1', exact: true });
  await opener.focus();
  await page.keyboard.press('Enter');
  const enlarged = page.getByTestId('runtime-tool-image-enlarged');
  await expect(enlarged).toBeVisible();
  await expectFullyInViewport(enlarged);
  await page.keyboard.press('Escape');
  await expect(enlarged).toHaveCount(0);
  await expect(opener).toBeFocused();
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await opener.click();
    await page.screenshot({ path: info.outputPath(`tool-image-${scheme}.png`), animations: 'disabled' });
    await page.keyboard.press('Escape');
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await opener.click();
  await expectFullyInViewport(enlarged);
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('tool-image-phone.png'), animations: 'disabled' });
  await page.keyboard.press('Escape');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.reload();
  await expect(row).toBeVisible({ timeout: 30_000 });
  await row.locator('.rw-line').click();
  await expect(previews.locator('img')).toHaveCount(2);
});
