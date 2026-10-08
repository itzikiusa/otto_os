import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';
import { expectFullyInViewport, expectNoHorizontalOverflow, openApiEditor } from './helpers';

// Real isolated workspace persistence; engine responses are deterministic and
// no database writes or outward API requests are sent by these journeys.
test.use({ serviceWorkers: 'block' });

for (const scheme of ['light', 'dark']) {
  test(`populated database keeps the reviewed edit through narrow layout and Cancel ${scheme}`, async ({ page }, info) => {
    const { ctx, base } = await apiCtx();
    const workspace = await seedWorkspace(ctx, base);
    const name = `populated-review-${scheme}`;
    const connection = await seedMockDbConnection(ctx, base, workspace, name);
    await ctx.dispose();
    const statements: string[] = [];
    await mockDbRoutes(page, connection, { seen: statements });
    await page.addInitScript(({ workspace, scheme }) => {
      localStorage.setItem('otto_workspace', workspace);
      localStorage.setItem('otto_scheme', scheme);
      localStorage.setItem('otto_theme', 'native');
      localStorage.setItem('otto_rail_expanded', '0');
    }, { workspace, scheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto('/#/database');
    await page.locator('.conn-list .conn-name', { hasText: name }).click();
    const editor = page.locator('.qe-edit .cm-content');
    await editor.click();
    await page.keyboard.press('ControlOrMeta+A');
    await page.keyboard.insertText('SELECT * FROM shop.orders LIMIT 1');
    await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
    const cell = page.locator('.grid tbody .cell').filter({ hasText: /^shipped$/ });
    await expect(cell).toBeVisible();
    await cell.dblclick();
    const input = page.locator('.grid .cell-input');
    await input.fill('reviewed without applying');
    await input.press('Enter');
    const pending = page.getByTestId('pending-edits-bar');
    await expect(pending).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath('database-populated-desktop.png') });
    await pending.getByRole('button', { name: /Review & apply/ }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog.locator('.review-sql')).toHaveValue(/reviewed without applying/);
    await page.setViewportSize({ width: 390, height: 844 });
    await expectFullyInViewport(page, dialog);
    await expectFullyInViewport(page, dialog.getByRole('button', { name: 'Cancel', exact: true }));
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath('database-review-phone.png') });
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).press('Enter');
    await expect(dialog).toHaveCount(0);
    await page.setViewportSize({ width: 1440, height: 900 });
    await expect(page.locator('.grid .cell.dirty')).toHaveText('reviewed without applying');
    await expect(pending).toBeVisible();
    expect(statements.filter(statement => /^\s*UPDATE\b/i.test(statement))).toEqual([]);
  });

  test(`populated API draft survives responsive unsaved-close cancellation ${scheme}`, async ({ page }, info) => {
    const { ctx, base } = await apiCtx();
    const workspace = await seedWorkspace(ctx, base);
    await ctx.dispose();
    await page.addInitScript(({ workspace, scheme }) => {
      localStorage.setItem('otto_workspace', workspace);
      localStorage.setItem('otto_scheme', scheme);
      localStorage.setItem('otto_theme', 'native');
      localStorage.setItem('otto_rail_expanded', '0');
    }, { workspace, scheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    await openApiEditor(page);
    await page.getByLabel('HTTP method').selectOption('POST');
    const url = page.getByLabel('Request URL');
    await url.fill('https://fixture.invalid/orders/draft');
    await page.getByRole('tab', { name: 'Headers', exact: true }).click();
    await page.getByRole('button', { name: 'Add header' }).click();
    await page.locator('.kv-row .kv-key').last().fill('X-Review');
    await page.locator('.kv-row .kv-val').last().fill('Keep this unsent value');
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath('api-populated-desktop.png') });
    await page.setViewportSize({ width: 390, height: 844 });
    await expectNoHorizontalOverflow(page);
    await page.locator('.req-tab-close').click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toContainText('unsaved changes');
    await expectFullyInViewport(page, dialog);
    await page.screenshot({ path: info.outputPath('api-draft-confirm-phone.png') });
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).press('Enter');
    await expect(dialog).toHaveCount(0);
    await expect(url).toHaveValue('https://fixture.invalid/orders/draft');
    await expect(page.getByLabel('HTTP method')).toHaveValue('POST');
    await expect(page.locator('.kv-row .kv-val').last()).toHaveValue('Keep this unsent value');
    await expectNoHorizontalOverflow(page);
  });
}
