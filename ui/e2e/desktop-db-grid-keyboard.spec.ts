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

test('rerunning the same result columns preserves the grid scroll position', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.setViewportSize({ width: 1280, height: 800 });
  const { ctx, base } = await apiCtx();
  const workspaceId = await seedWorkspace(ctx, base);
  let connId: string;
  try {
    connId = await seedMockDbConnection(ctx, base, workspaceId, `grid-rerun-${Date.now()}`);
  } finally { await ctx.dispose(); }
  await mockDbRoutes(page, connId);
  const statement = 'SELECT id, status FROM shop.orders ORDER BY id';
  let generation = 0;
  await page.route(new RegExp(`/connections/${connId}/db/query$`), async route => {
    expect(route.request().postDataJSON().statement).toBe(statement);
    generation += 1;
    await route.fulfill({ json: {
      // Each response has fresh column objects but exactly the same shape.
      columns: [{ name: 'id', type_hint: 'BIGINT' }, { name: 'status', type_hint: 'VARCHAR(20)' }],
      rows: Array.from({ length: 100 }, (_, i) => [i + 1, `run-${generation}`]),
      stats: { row_count: 100, duration_ms: generation },
      truncated: false, auto_limited: 100,
    } });
  });
  await page.addInitScript(id => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
  await page.goto('/#/database');
  await page.locator(`.conn-row[data-connection-id="${connId}"] .conn-name`).click();
  const editor = page.locator('.qe-edit .cm-content');
  await expect(editor).toBeVisible();
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText(statement);
  await page.keyboard.press('Escape');
  const run = page.locator('.btn.small.primary', { hasText: 'Run' }).first();
  const grid = page.locator('.grid-scroll');
  let settledScrollTop = 0;
  for (const expectedGeneration of [1, 2]) {
    const responsePromise = page.waitForResponse(response =>
      response.request().method() === 'POST'
      && new URL(response.url()).pathname.endsWith(`/connections/${connId}/db/query`)
      && response.request().postDataJSON().statement === statement);
    await run.click();
    const response = await responsePromise;
    expect(response.ok()).toBe(true);
    expect(await response.finished()).toBeNull();
    // Prove the new result is rendered, rather than accepting the old rows
    // while its replacement is still in flight.
    await expect(page.locator('.grid tbody .cell').filter({ hasText: `run-${expectedGeneration}` }).first()).toBeVisible();
    await expect(page.locator('.qe-stop')).toHaveCount(0);
    await expect(page.locator('.rg-overlay')).toHaveCount(0);
    if (expectedGeneration === 1) {
      await grid.evaluate(el => { el.scrollTop = 400; });
      // Table/spacer geometry can round the requested position by one pixel.
      await expect.poll(() => grid.evaluate(el => Math.abs(el.scrollTop - 400))).toBeLessThanOrEqual(2);
      settledScrollTop = await grid.evaluate(el => el.scrollTop);
      expect(settledScrollTop).toBeGreaterThan(390);
    }
  }
  expect(generation).toBe(2);
  await expect.poll(() => grid.evaluate((el, before) => Math.abs(el.scrollTop - before), settledScrollTop)).toBeLessThanOrEqual(2);
});
