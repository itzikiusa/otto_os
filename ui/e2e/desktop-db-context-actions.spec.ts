import { test, expect, type Page, type Locator } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — cell "Query by value" / "Add to query" + per-row JSON view.
//
// Drives the REAL UI against the seeded Docker stack (mysql/mongodb/clickhouse).
// For each engine it proves the cell context-menu actions:
//   • "Query by value"  → rebuilds the active query filtered by the cell, writes
//     it into the editor AND the clipboard, and does NOT auto-run.
//   • "Add to query"    → ANDs / merges a second condition onto the first.
// Plus: the JSON view renders ONE object per row (numbered, bordered blocks),
// not a single array.
//
// Desktop-browser project only (the 3-pane layout + a real right-click). Each
// engine test.skips cleanly when its container isn't reachable.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
const conn: Record<'mysql' | 'mongodb' | 'clickhouse', string | null> = {
  mysql: null,
  mongodb: null,
  clickhouse: null,
};

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  for (const k of ['mysql', 'mongodb', 'clickhouse'] as const) {
    try {
      conn[k] = await seedDockerConnection(ctx, base, workspaceId, k);
    } catch {
      conn[k] = null;
    }
  }
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page, context }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  // Reading the clipboard back (to prove "copied to clipboard") needs the read
  // permission; write is allowed by default in Chromium.
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

async function openConn(page: Page, name: string): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const c = page.locator('.conn-list .conn-name', { hasText: name });
  await expect(c.first()).toBeVisible({ timeout: 30_000 });
  await c.first().click();
  await expect(page.locator('.main-tabs')).toBeVisible({ timeout: 20_000 });
}

async function setEditor(page: Page, sql: string): Promise<void> {
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.press('Delete');
  await content.pressSequentially(sql, { delay: 6 });
  await page.keyboard.press('Escape'); // dismiss the autocomplete popup
}

async function clickRun(page: Page): Promise<void> {
  await page.getByRole('button', { name: /^Run/ }).first().click();
  await expect(page.locator('.grid tbody tr:not(.spacer)').first()).toBeVisible({ timeout: 20_000 });
}

/** The first-data-row cell under the column whose header starts with `colName`.
 *  Header/body columns align 1:1 (incl. the leading #/checkbox column), so the
 *  header index is the body cell index. */
async function cellByColumn(page: Page, colName: string, rowIdx = 0): Promise<Locator> {
  const headers = page.locator('.grid thead th');
  const n = await headers.count();
  let idx = -1;
  for (let i = 0; i < n; i++) {
    const t = ((await headers.nth(i).textContent()) ?? '').trim();
    if (t === colName || t.startsWith(colName)) {
      idx = i;
      break;
    }
  }
  expect(idx, `column "${colName}" present in header`).toBeGreaterThan(0);
  return page.locator('.grid tbody tr:not(.spacer)').nth(rowIdx).locator('td').nth(idx);
}

async function rightClick(cell: Locator, page: Page): Promise<void> {
  await cell.scrollIntoViewIfNeeded();
  await cell.click({ button: 'right' });
  await expect(page.locator('.ctx-menu')).toBeVisible();
}

async function clickMenu(page: Page, label: RegExp): Promise<void> {
  await page.locator('.ctx-item', { hasText: label }).first().click();
  await expect(page.locator('.ctx-menu')).toBeHidden();
}

function clipboard(page: Page): Promise<string> {
  return page.evaluate(() => navigator.clipboard.readText());
}

function dataRowCount(page: Page): Promise<number> {
  return page.locator('.grid tbody tr:not(.spacer)').count();
}

test.describe('cell Query-by-value / Add-to-query', () => {
  test('MySQL: query by value + add to query, into editor & clipboard, no auto-run', async ({ page }) => {
    test.skip(!conn.mysql, 'mysql docker not reachable');
    await openConn(page, 'e2e-mysql');
    await setEditor(page, 'SELECT id, name FROM products ORDER BY id LIMIT 5');
    await clickRun(page);
    const before = await dataRowCount(page);
    expect(before).toBeGreaterThan(1);

    // Query by value on the `name` cell.
    const nameCell = await cellByColumn(page, 'name');
    const nameVal = ((await nameCell.textContent()) ?? '').trim();
    expect(nameVal.length).toBeGreaterThan(0);
    await rightClick(nameCell, page);
    await clickMenu(page, /Query by value/);

    const clip1 = await clipboard(page);
    expect(clip1).toContain('WHERE `name` =');
    expect(clip1).toContain(nameVal);
    // Shown in the editor too.
    const ed1 = ((await page.locator('.qe-edit .cm-content').textContent()) ?? '');
    expect(ed1).toContain('`name`');
    expect(ed1).toContain(nameVal);
    // Did NOT auto-run: the grid still shows the original rows.
    expect(await dataRowCount(page)).toBe(before);

    // Add to query on the `id` cell → ANDed onto the existing WHERE.
    const idCell = await cellByColumn(page, 'id');
    const idVal = ((await idCell.textContent()) ?? '').trim();
    await rightClick(idCell, page);
    await clickMenu(page, /Add to query/);
    const clip2 = await clipboard(page);
    expect(clip2).toContain('WHERE `name` =');
    expect(clip2).toMatch(/AND\s+`id`\s*=\s*/);
    expect(clip2).toContain(idVal);
    expect(await dataRowCount(page)).toBe(before); // still no auto-run
  });

  test('ClickHouse: query by value writes a WHERE into editor & clipboard', async ({ page }) => {
    test.skip(!conn.clickhouse, 'clickhouse docker not reachable');
    await openConn(page, 'e2e-clickhouse');
    await setEditor(page, 'SELECT event_type, user_id FROM analytics.events ORDER BY event_id LIMIT 5');
    await clickRun(page);

    const cell = await cellByColumn(page, 'event_type');
    const val = ((await cell.textContent()) ?? '').trim();
    await rightClick(cell, page);
    await clickMenu(page, /Query by value/);

    const clip = await clipboard(page);
    expect(clip).toContain('WHERE `event_type` =');
    expect(clip).toContain(val);
    const ed = ((await page.locator('.qe-edit .cm-content').textContent()) ?? '');
    expect(ed).toContain('`event_type`');
  });

  test('MongoDB: query by value sets the find filter; add to query merges it', async ({ page }) => {
    test.skip(!conn.mongodb, 'mongodb docker not reachable');
    await openConn(page, 'e2e-mongodb');
    await setEditor(page, 'db.products.find({})');
    await clickRun(page);

    const nameCell = await cellByColumn(page, 'name');
    const nameVal = ((await nameCell.textContent()) ?? '').trim();
    await rightClick(nameCell, page);
    await clickMenu(page, /Query by value/);
    const clip1 = await clipboard(page);
    expect(clip1).toContain('.find({ "name":');
    expect(clip1).toContain(nameVal);

    // Add to query on `sku` → merged into the same filter object.
    const skuCell = await cellByColumn(page, 'sku');
    const skuVal = ((await skuCell.textContent()) ?? '').trim();
    await rightClick(skuCell, page);
    await clickMenu(page, /Add to query/);
    const clip2 = await clipboard(page);
    expect(clip2).toContain('"name":');
    expect(clip2).toContain('"sku":');
    expect(clip2).toContain(skuVal);
    // Both keys live in one merged object: "name" appears before "sku".
    expect(clip2.indexOf('"name"')).toBeLessThan(clip2.indexOf('"sku"'));
  });
});

test.describe('JSON view renders one object per row', () => {
  test('MySQL: JSON view shows numbered per-row blocks, not one array', async ({ page }) => {
    test.skip(!conn.mysql, 'mysql docker not reachable');
    await openConn(page, 'e2e-mysql');
    await setEditor(page, 'SELECT id, name FROM products ORDER BY id LIMIT 5');
    await clickRun(page);
    const rows = await dataRowCount(page);
    expect(rows).toBeGreaterThan(1);

    await page.locator('.view-seg .vs', { hasText: 'JSON' }).click();
    // One bordered, numbered block per row (no single big array).
    await expect(page.locator('.jrec').first()).toBeVisible({ timeout: 10_000 });
    expect(await page.locator('.jrec').count()).toBe(rows);
    await expect(page.locator('.jrec-head', { hasText: '#1' })).toBeVisible();
    await expect(page.locator('.jrec-head', { hasText: '#2' })).toBeVisible();
    // Each block is a standalone JSON object.
    const first = ((await page.locator('.jrec .alt-json').first().textContent()) ?? '').trim();
    expect(first.startsWith('{')).toBe(true);
    expect(first.endsWith('}')).toBe(true);
  });
});
