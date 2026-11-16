import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Find / Replace in Path: a capped result list says so, and a replace preview never
// outlives the query, replacement or options it was made for.

const NEEDLES = Array.from({ length: 120 }, (_, i) => `needle ${i}`).join('\n') + '\n';

let ws = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  ws = await seedWorkspace(ctx, base, 'Find in path');
  const put = await ctx.put(`${base}/api/v1/scratches/file`, { data: { path: 'find-e2e-needles.txt', content: NEEDLES } });
  expect(put.ok()).toBe(true);
  await ctx.dispose();
});

async function openFindInPath(page: Page, replace: boolean): Promise<void> {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
  await page.goto('/#/ide?scratches=1');
  await expect(page.getByTestId('ide-target-switcher')).toContainText('Scratches');
  await page.getByTestId('ide-project-tree').getByText('find-e2e-needles.txt').dblclick();
  await expect(page.locator('.cm-editor').filter({ hasText: 'needle 0' })).toBeVisible();
  await page.keyboard.press(replace ? 'Meta+Shift+R' : 'Meta+Shift+F');
  await expect(page.locator('.se-input').first()).toBeVisible();
}

test('Find in Path says when it shows only the first matches', async ({ page }) => {
  await openFindInPath(page, false);
  await page.locator('.se-input').first().fill('needle');
  await expect(page.getByTestId('ide-search-truncated')).toContainText('Showing the first 80 matches', { timeout: 15_000 });
});

test('a replace preview is dropped when the replacement or an option changes', async ({ page }) => {
  await openFindInPath(page, true);
  await page.locator('.se-input').first().fill('needle 1');
  const replacement = page.getByTestId('ide-replace-input');
  await replacement.fill('pin');
  await page.getByRole('button', { name: 'Preview', exact: true }).click();
  const box = page.locator('.se-preview-box');
  await expect(box).toContainText('+ pin', { timeout: 15_000 });
  await expect(box).toContainText('matches)');
  await replacement.fill('nail');
  await expect(box).toHaveCount(0);
  await page.getByRole('button', { name: 'Preview', exact: true }).click();
  await expect(box).toContainText('+ nail', { timeout: 15_000 });
  await page.getByRole('button', { name: /match case/i }).first().click();
  await expect(box).toHaveCount(0);
});
