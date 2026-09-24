import { test, expect, type Locator, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — the results view must not JUMP while a table loads.
//
// Docker-free: a real connection profile on the isolated daemon, every engine
// call answered by the in-memory "shop" mock (e2e/db-mock.ts) with latency on
// both the query and the object probe, so every async step is observable:
//   Run → loading frame → rows → editability probe lands (PK badge, checkbox).
// The grid's frame (top/left/size) and the first data cell must stay put from
// the loading frame to the settled, editable grid.
//
// Also pins the per-engine auto-Vertical default: a 12-column MySQL result
// opens in the GRID (auto-Vertical is MongoDB-only unless the user opts in).
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let connId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  connId = await seedMockDbConnection(ctx, base, workspaceId, 'mock-shop');
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
  await mockDbRoutes(page, connId, { queryDelay: 600, objectDelay: 700 });
});

async function openMock(page: Page): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const c = page.locator('.conn-list .conn-name', { hasText: 'mock-shop' });
  await expect(c.first()).toBeVisible({ timeout: 30_000 });
  await c.first().click();
  await expect(page.locator('.query-editor')).toBeVisible({ timeout: 20_000 });
}

async function typeStatement(page: Page, stmt: string): Promise<void> {
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText(stmt);
  await page.waitForTimeout(250);
}

async function box(l: Locator): Promise<{ x: number; y: number; width: number; height: number }> {
  const b = await l.boundingBox();
  expect(b, 'element should be laid out').not.toBeNull();
  return b!;
}

function expectSameBox(
  a: { x: number; y: number; width: number; height: number },
  b: { x: number; y: number; width: number; height: number },
  what: string,
): void {
  expect(Math.abs(a.x - b.x), `${what}: left moved`).toBeLessThanOrEqual(1);
  expect(Math.abs(a.y - b.y), `${what}: top moved`).toBeLessThanOrEqual(1);
  expect(Math.abs(a.width - b.width), `${what}: width changed`).toBeLessThanOrEqual(1);
  expect(Math.abs(a.height - b.height), `${what}: height changed`).toBeLessThanOrEqual(1);
}

test('opening a table: grid frame and first row stay put from loading to settled', async ({ page }) => {
  await openMock(page);
  await typeStatement(page, 'SELECT * FROM shop.orders');
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();

  // 1) Loading frame — drawn at the final geometry before any row exists.
  const frame = page.locator('[data-grid-frame]');
  await expect(page.locator('.grid-wrap[data-state="loading"]')).toBeVisible({ timeout: 5_000 });
  const loading = await box(frame);

  // 2) Rows land in the same frame.
  const firstCell = page.locator('.grid tbody tr:not(.spacer)').first().locator('td').nth(1);
  await expect(firstCell).toBeVisible({ timeout: 15_000 });
  const loaded = await box(frame);
  const cellLoaded = await box(firstCell);
  expectSameBox(loading, loaded, 'grid frame (loading → rows)');
  // First data row sits right under the 40px header band.
  expect(Math.abs(cellLoaded.y - (loaded.y + 1 + 40)), 'first row under the header').toBeLessThanOrEqual(2);

  // 3) The editability probe lands (PK badge + checkboxes) — nothing moves.
  await expect(page.locator('.gt-edit-hint')).toBeVisible({ timeout: 10_000 });
  await expect(page.locator('.grid thead .th-pk')).toBeVisible();
  const settled = await box(frame);
  const cellSettled = await box(firstCell);
  expectSameBox(loaded, settled, 'grid frame (rows → editable)');
  expectSameBox(cellLoaded, cellSettled, 'first data cell (rows → editable)');

  // Toolbar + status bar are single fixed-height lines.
  expect((await box(page.locator('.grid-toolbar'))).height).toBeLessThanOrEqual(40);
  expect((await box(page.locator('.grid-foot'))).height).toBeLessThanOrEqual(26);
});

test('a wide SQL result opens in the grid — auto-Vertical is MongoDB-only by default', async ({ page }) => {
  await openMock(page);
  await typeStatement(page, 'SELECT * FROM shop.orders');
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
  await expect(page.locator('.view-seg')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.view-seg .vs.on')).toHaveText('Grid');
  await expect(page.locator('.view-seg')).toHaveAttribute('title', 'engine default');
  // 12 columns, well past the (Mongo) threshold of 10.
  await expect(page.locator('.grid thead th:not(.rownum)')).toHaveCount(12);
});

test('opting MySQL in re-enables auto-Vertical for wide results', async ({ page }) => {
  await page.addInitScript(() =>
    localStorage.setItem('otto_db_auto_vertical_by_engine', JSON.stringify({ mysql: 10 })),
  );
  await openMock(page);
  await typeStatement(page, 'SELECT * FROM shop.orders');
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
  await expect(page.locator('.view-seg .vs.on')).toHaveText('Vertical', { timeout: 15_000 });
  await expect(page.locator('.view-seg')).toHaveAttribute('title', /auto: 12 columns > 10/);
});

test('grid: filter row, row detail, numeric alignment and NULL styling', async ({ page }) => {
  await openMock(page);
  await typeStatement(page, 'SELECT * FROM shop.orders');
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
  await expect(page.locator('.grid tbody tr:not(.spacer)').first()).toBeVisible({ timeout: 15_000 });

  // Numbers right-aligned; NULL is a word, not an empty cell.
  const totalHead = page.locator('.grid thead th', { hasText: 'total' });
  await expect(totalHead).toHaveClass(/num/);
  await expect(page.locator('.grid tbody .cell.null .null-glyph').first()).toHaveText('NULL');

  // Filter row: `status` = refunded narrows the view client-side.
  await page.getByRole('button', { name: 'Filter row' }).click();
  await page.getByLabel('Filter status').fill('=refunded');
  await expect(page.locator('.grid-foot')).toContainText('of 240 rows');
  const statuses = await page
    .locator('.grid tbody tr:not(.spacer)')
    .evaluateAll((trs) => trs.map((tr) => tr.querySelectorAll('td')[3]?.textContent?.trim()));
  expect(statuses.length).toBeGreaterThan(0);
  expect(new Set(statuses)).toEqual(new Set(['refunded']));

  // Row detail follows the grid cursor.
  await page.getByRole('button', { name: 'Row detail' }).click();
  const detail = page.locator('.row-detail');
  await expect(detail).toBeVisible();
  await page.locator('.grid tbody tr:not(.spacer)').nth(1).locator('td.cell').first().click();
  await expect(detail.locator('.rd-title')).toHaveText(/Row \d+/);
  await expect(detail.locator('.rd-field', { hasText: 'status' })).toContainText('refunded');
});

test('Copy / Export / More menus stay inside the viewport', async ({ page }) => {
  await openMock(page);
  await typeStatement(page, 'SELECT * FROM shop.orders');
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
  await expect(page.locator('.grid tbody tr:not(.spacer)').first()).toBeVisible({ timeout: 15_000 });
  for (const name of ['Copy', 'Export']) {
    await page.locator('.grid-toolbar .tb-btn', { hasText: name }).click();
    const menu = page.locator('.ctx-menu');
    await expect(menu).toBeVisible();
    const b = await box(menu);
    const vp = page.viewportSize()!;
    expect(b.x + b.width).toBeLessThanOrEqual(vp.width);
    expect(b.y + b.height).toBeLessThanOrEqual(vp.height);
    await page.keyboard.press('Escape');
    await expect(menu).toBeHidden();
  }
  await page.getByRole('button', { name: 'More result actions' }).click();
  await expect(page.locator('.ctx-item', { hasText: 'Examine with AI' })).toBeVisible();
  await page.keyboard.press('Escape');
});
