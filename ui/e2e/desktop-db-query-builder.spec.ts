import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — the visual query builder (Docker-free: mocked engine routes,
// see e2e/db-mock.ts). Drives the real UI end to end:
//   • add a table → its FK partner joins itself; columns ticked on the cards;
//   • an aggregate groups by the other columns (visible, editable);
//   • WHERE with an OR group, HAVING, ORDER BY … NULLS LAST, LIMIT;
//   • the live SQL is MySQL-dialect, quoted and escaped; Run sends exactly it;
//   • "Open in Builder" on a query tab parses a statement back (round trip),
//     and a statement it can't hold is refused inline with the reason.
// The SQL generator itself is unit-tested per dialect (unit/dbQueryBuilder).
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let connId = '';
// Unique per worker: connection profiles are global, and a retried worker re-seeds.
const CONN = `mock-qb-${Math.random().toString(36).slice(2, 8)}`;

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  connId = await seedMockDbConnection(ctx, base, workspaceId, CONN);
  await ctx.dispose().catch(() => {});
});

const seen: string[] = [];

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  seen.length = 0;
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
  await mockDbRoutes(page, connId, { seen });
});

async function openBuilder(page: Page): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const c = page.locator('.conn-list .conn-name', { hasText: CONN });
  await expect(c.first()).toBeVisible({ timeout: 30_000 });
  await c.first().click();
  await page.locator('.main-tabs .mt', { hasText: 'Builder' }).click();
  await expect(page.locator('.builder')).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTitle('Add orders to the canvas', { exact: true })).toBeVisible({ timeout: 15_000 });
}

const sqlText = (page: Page) => page.getByTestId('qb-sql');

test('build a GROUP BY report with filters, HAVING and sort — and run it', async ({ page }) => {
  await openBuilder(page);
  await page.getByTitle('Add orders to the canvas', { exact: true }).click();
  await expect(page.locator('.builder .content > .node')).toHaveCount(1);
  await expect(sqlText(page)).toContainText('SELECT *');
  await expect(sqlText(page)).toContainText('FROM `shop`.`orders`');

  // Customers joins itself on the FK (one unambiguous suggestion).
  await page.getByTitle('Add customers to the canvas', { exact: true }).click();
  await expect(page.locator('.builder .content > .node')).toHaveCount(2);
  await expect(sqlText(page)).toContainText('INNER JOIN `shop`.`customers` ON `orders`.`customer_id` = `customers`.`id`');

  // Tick orders.region on its card, then add SUM(total) → GROUP BY region appears.
  await page.getByLabel('Select orders.region').check();
  await page.getByRole('button', { name: 'Aggregate' }).click();
  const agg = page.locator('.sel-row').nth(1);
  await agg.getByLabel('Aggregate', { exact: true }).selectOption('SUM');
  await agg.getByLabel('Aggregate of').selectOption('orders.total');
  await expect(sqlText(page)).toContainText('SUM(`orders`.`total`) AS `sum_total`');
  await expect(sqlText(page)).toContainText('GROUP BY `orders`.`region`');

  // WHERE: status = 'paid' OR status contains "ship" (escaped value).
  await page.getByRole('button', { name: 'Group', exact: true }).click();
  const grp = page.locator('.cgroup.nested').first();
  const c1 = grp.locator('.row.cond').first();
  await c1.getByLabel('Column').selectOption('orders.status');
  await c1.getByLabel('Operator').selectOption('=');
  await c1.getByLabel('Value').fill("pa'id");
  await grp.getByRole('button', { name: 'Condition', exact: true }).click();
  const c2 = grp.locator('.row.cond').nth(1);
  await c2.getByLabel('Column').selectOption('orders.status');
  await c2.getByLabel('Operator').selectOption('CONTAINS');
  await c2.getByLabel('Value').fill('50%');
  await expect(sqlText(page)).toContainText("WHERE (`orders`.`status` = 'pa''id' OR `orders`.`status` LIKE '%50\\\\%%')");

  // HAVING COUNT(*) > 2, ORDER BY the aggregate DESC NULLS LAST (emulated on MySQL).
  await page.locator('section[aria-labelledby="qb-having"]').getByRole('button', { name: 'Condition', exact: true }).click();
  const h = page.locator('section[aria-labelledby="qb-having"] .row.cond').first();
  await h.getByLabel('Value').fill('2');
  await expect(sqlText(page)).toContainText('HAVING COUNT(*) > 2');
  await page.getByRole('button', { name: 'Sort', exact: true }).click();
  const o = page.locator('section[aria-labelledby="qb-order"] .row').first();
  await o.getByLabel('NULLs position').selectOption('last');
  await expect(sqlText(page)).toContainText('ORDER BY `sum_total` IS NULL ASC, `sum_total` DESC');
  await expect(sqlText(page)).toContainText('LIMIT 100;');

  // No validation errors; Run opens a query tab with exactly this SQL.
  await expect(page.locator('.issues li.err')).toHaveCount(0);
  const generated = (await sqlText(page).innerText()).trim();
  await page.locator('.sql-actions .btn.primary', { hasText: 'Run' }).click();
  await expect(page.locator('.query-editor')).toBeVisible({ timeout: 10_000 });
  await expect.poll(() => seen.at(-1)?.trim()).toBe(generated);
});

test('a loose column under GROUP BY is flagged, with a one-click fix', async ({ page }) => {
  await openBuilder(page);
  await page.getByTitle('Add orders to the canvas', { exact: true }).click();
  await page.getByLabel('Select orders.status').check();
  await page.getByLabel('Select orders.region').check();
  await page.getByRole('button', { name: 'Aggregate' }).click();
  // The auto GROUP BY took both; remove one to provoke the error.
  const chips = page.locator('section[aria-labelledby="qb-group"] .chip');
  await expect(chips).toHaveCount(2);
  await chips.first().getByRole('button').click();
  const err = page.locator('.issues li.err');
  await expect(err).toContainText('neither grouped nor aggregated');
  await expect(page.locator('.sql-actions .btn.primary')).toBeDisabled();
  await err.getByRole('button', { name: /Group by it/ }).click();
  await expect(page.locator('.issues li.err')).toHaveCount(0);
});

test('round trip: Open in Builder parses a query tab; unsupported SQL is refused inline', async ({ page }) => {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  await page.locator('.conn-list .conn-name', { hasText: CONN }).first().click();
  await expect(page.locator('.query-editor')).toBeVisible({ timeout: 20_000 });
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText(
    "SELECT o.status, COUNT(*) AS n FROM shop.orders o WHERE o.region IN ('eu-west', 'us-east') GROUP BY o.status ORDER BY n DESC LIMIT 5",
  );
  await page.waitForTimeout(300);
  await page.locator('.qe-tab').first().click({ button: 'right' });
  await page.locator('.ctx-item', { hasText: 'Open in Builder' }).click();
  await expect(page.locator('.builder')).toBeVisible({ timeout: 10_000 });
  await expect(page.locator('.builder .content > .node')).toHaveCount(1, { timeout: 10_000 });
  await expect(page.locator('.builder .alias-input')).toHaveValue('o');
  await expect(sqlText(page)).toContainText("WHERE `region` IN ('eu-west', 'us-east')");
  await expect(sqlText(page)).toContainText('GROUP BY `status`');
  await expect(sqlText(page)).toContainText('ORDER BY `n` DESC');

  // Something the builder can't hold: refused with a reason, canvas untouched.
  await page.locator('.main-tabs .mt', { hasText: 'Query' }).click();
  await content.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText('SELECT * FROM shop.orders UNION SELECT * FROM shop.orders');
  await page.waitForTimeout(300);
  await page.locator('.qe-tab').first().click({ button: 'right' });
  await page.locator('.ctx-item', { hasText: 'Open in Builder' }).click();
  await expect(page.locator('.notice.err')).toContainText("can't be edited in the builder");
  await expect(page.locator('.builder .content > .node')).toHaveCount(1);
});
