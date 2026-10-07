import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';

// Exercise native input → grid key bubbling through the production DOM. Engine
// requests are mocked; this regression needs no Docker database or real writes.
test.use({ serviceWorkers: 'block' });

test('Enter commits a cell draft once and Escape cancels a later edit', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  const { ctx, base } = await apiCtx();
  const workspaceId = await seedWorkspace(ctx, base);
  const name = `grid-keys-${Date.now()}`;
  const connId = await seedMockDbConnection(ctx, base, workspaceId, name);
  await ctx.dispose();
  const seen: string[] = [];
  await mockDbRoutes(page, connId, { seen });
  await page.addInitScript(id => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
  await page.goto('/#/database');
  await page.locator('.conn-list .conn-name', { hasText: name }).click();
  const editor = page.locator('.qe-edit .cm-content');
  await expect(editor).toBeVisible();
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText('SELECT * FROM shop.orders LIMIT 1');
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
  const cell = page.locator('.grid tbody .cell').filter({ hasText: /^shipped$/ });
  await expect(cell).toBeVisible();
  await expect(cell).toHaveClass(/editable/);
  await cell.dblclick();
  const input = page.locator('.grid .cell-input');
  await expect(input).toBeVisible();
  await input.fill('reviewed');
  await input.press('Enter');
  // The same Enter must not reopen editing as it bubbles to the grid cursor.
  await expect(input).toHaveCount(0);
  const dirty = page.locator('.grid .cell.dirty');
  await expect(dirty).toHaveCount(1);
  await expect(dirty).toHaveText('reviewed');
  const pending = page.getByTestId('pending-edits-bar');
  await expect(pending).toBeVisible();
  await dirty.dblclick();
  await expect(input).toHaveValue('reviewed');
  await input.fill('discard this edit');
  await input.press('Escape');
  await expect(input).toHaveCount(0);
  await expect(dirty).toHaveText('reviewed');
  await pending.getByRole('button', { name: /Review & apply/ }).click();
  const review = page.locator('.review-modal .review-sql');
  await expect(review).toBeVisible();
  await expect(review).toHaveValue(/UPDATE[\s\S]*status[\s\S]*reviewed/);
  expect(await review.inputValue()).not.toContain('discard this edit');
  expect(seen.filter(statement => /^\s*UPDATE\b/i.test(statement))).toEqual([]);
});
