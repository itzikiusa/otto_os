import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — POSTGRES engine sweep (mobile + tablet, portrait + landscape).
//
// The Postgres clone of the MySQL sweep: proves the Database Explorer is fully
// VISIBLE and USABLE for PostgreSQL against the live seeded Docker stack
// (127.0.0.1:15432 · otto/ottopw · shopdb), across the device projects. Postgres
// browses a connection's single database BY SCHEMA, so the tree's top level is a
// schema (`public`) rather than a database — the one structural difference from
// the MySQL sweep (mysql: shopdb→Tables→orders; postgres: public→Tables→orders).
//
// Same seven checks as the MySQL sweep, share the same content selectors
// (`.conn-list .conn-name`, `.main-tabs`, `.qe-edit .cm-content`, `.grid`,
// `.grid-scroll`); the phone layout adds accordion headers we expand first.
// Each project uses a UNIQUE scratch sku (`e2e-scratch-postgres-<project>`) so
// the write flows never race across the parallel device projects.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let connId: string | null = null;

const PHONE_MAX = 640;

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  try {
    connId = await seedDockerConnection(ctx, base, workspaceId, 'postgres');
  } catch {
    connId = null;
  }
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

function isPhone(page: Page): boolean {
  const w = page.viewportSize()?.width ?? 0;
  return w > 0 && w <= PHONE_MAX;
}

/** Open #/database and select the seeded Postgres connection. */
async function openPostgres(page: Page): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });

  const conn = page.locator('.conn-list .conn-name', { hasText: 'e2e-postgres' });
  await expect(conn.first()).toBeVisible({ timeout: 30_000 });
  await conn.first().click();

  await expect(page.locator('.main-tabs')).toBeVisible({ timeout: 20_000 });
}

async function ensureEditorOpen(page: Page): Promise<void> {
  if (!isPhone(page)) return;
  const editor = page.locator('.qe-edit');
  if (!(await editor.isVisible().catch(() => false))) {
    await page.locator('.qe-acc-head', { hasText: 'Editor' }).click();
  }
  await expect(editor).toBeVisible();
}

async function ensureResultsOpen(page: Page): Promise<void> {
  if (!isPhone(page)) return;
  const results = page.locator('.qe-results');
  if (!(await results.isVisible().catch(() => false))) {
    await page.locator('.qe-acc-head', { hasText: 'Results' }).click();
  }
  await expect(results).toBeVisible();
}

/** Type a statement into the CodeMirror editor (replacing whatever's there) and
 *  press Run, then wait for the run to settle. Mirrors the MySQL sweep's
 *  CodeMirror-fragility retry. */
async function typeStatement(page: Page, sql: string): Promise<void> {
  await ensureEditorOpen(page);
  const content = page.locator('.qe-edit .cm-content');
  const mod = process.platform === 'darwin' ? 'Meta' : 'Control';

  const want = sql.replace(/\s+/g, ' ').trim();
  for (let attempt = 0; attempt < 3; attempt++) {
    await content.click();
    await expect(content).toBeFocused({ timeout: 5_000 });
    await page.keyboard.press(`${mod}+A`);
    await page.keyboard.press('Delete');
    await content.pressSequentially(sql, { delay: 8 });
    await page.keyboard.press('Escape');

    const got = ((await content.textContent()) ?? '').replace(/\s+/g, ' ').trim();
    if (got.startsWith(want)) return;
  }
  const got = ((await content.textContent()) ?? '').replace(/\s+/g, ' ').trim();
  expect(got, `editor should hold the statement (got: "${got}")`).toContain(want);
}

async function runStatement(page: Page, sql: string): Promise<void> {
  await typeStatement(page, sql);
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
  await expect(page.locator('.btn.small.primary', { hasText: 'Run' }).first()).toBeVisible({
    timeout: 20_000,
  });
}

async function runRead(page: Page, sql: string): Promise<void> {
  await runStatement(page, sql);
  await ensureResultsOpen(page);
  await expect(page.locator('.grid tbody tr:not(.spacer)').first()).toBeVisible({
    timeout: 20_000,
  });
}

async function docOverflow(page: Page): Promise<{ vw: number; docScrollW: number }> {
  return page.evaluate(() => {
    const de = document.documentElement;
    return { vw: de.clientWidth, docScrollW: de.scrollWidth };
  });
}

async function rightEdge(
  page: Page,
  selector: string,
): Promise<{ right: number; vw: number; left: number }> {
  return page.locator(selector).first().evaluate((el) => {
    const r = el.getBoundingClientRect();
    return {
      right: Math.round(r.right),
      left: Math.round(r.left),
      vw: document.documentElement.clientWidth,
    };
  });
}

test.describe('DB Explorer · Postgres sweep', () => {
  test.describe.configure({ mode: 'serial' });

  test('connection seeds & is reachable (driver health)', () => {
    // A null here is a real bug (driver/daemon couldn't reach the seeded Postgres,
    // or the /test probe regressed to the psql CLI), not a reason to skip — fail
    // loudly so it's investigated.
    expect(
      connId,
      'seedDockerConnection(postgres) returned null — Postgres driver/daemon could not connect (or /test) to 127.0.0.1:15432',
    ).not.toBeNull();
  });

  test('SELECT (read) shows rows in the grid', async ({ page }) => {
    test.skip(connId == null, 'postgres connection unavailable');
    await openPostgres(page);
    await runRead(page, 'SELECT * FROM customers ORDER BY id');

    const rows = await page.locator('.grid tbody tr:not(.spacer)').count();
    expect(rows, 'customers SELECT should return ≥1 data row').toBeGreaterThanOrEqual(1);

    await ensureResultsOpen(page);
    await expect(page.locator('.grid tbody').getByText('ada@example.com').first()).toBeVisible({
      timeout: 10_000,
    });
  });

  test('UPDATE (write) applies and the new value is visible', async ({ page }, info) => {
    test.skip(connId == null, 'postgres connection unavailable');
    const sku = `e2e-scratch-postgres-${info.project.name}`;
    await openPostgres(page);

    // Best-effort pre-clean in case a prior aborted run left the scratch row.
    await runStatement(page, `DELETE FROM products WHERE sku='${sku}'`);

    // INSERT the scratch row (standard SQL — id is SERIAL, in_stock/metadata default).
    await runStatement(page, `INSERT INTO products (sku,name,price_cents) VALUES ('${sku}','E2E',1)`);
    await ensureResultsOpen(page);
    await expect(
      page.locator('.grid-empty, .grid-notice, .grid-foot').filter({ hasText: /affected|OK/i }).first(),
    ).toBeVisible({ timeout: 15_000 });

    // UPDATE it to a known value.
    await runStatement(page, `UPDATE products SET price_cents=2 WHERE sku='${sku}'`);
    await ensureResultsOpen(page);
    await expect(
      page.locator('.grid-empty, .grid-notice, .grid-foot').filter({ hasText: /affected|OK/i }).first(),
    ).toBeVisible({ timeout: 15_000 });

    // SELECT it back — the grid must show price_cents = 2 (bare SELECT is
    // auto-LIMITed; a single scratch row is well within the cap).
    await runRead(page, `SELECT id,sku,price_cents FROM products WHERE sku='${sku}'`);
    const scratchRow = page.locator('.grid tbody tr:not(.spacer)', { hasText: sku }).first();
    await scratchRow.scrollIntoViewIfNeeded();
    await expect(scratchRow).toBeVisible({ timeout: 15_000 });
    await expect(scratchRow, 'scratch row should show updated price_cents=2').toContainText('2', {
      timeout: 15_000,
    });

    // Cleanup.
    await runStatement(page, `DELETE FROM products WHERE sku='${sku}'`);
    await ensureResultsOpen(page);
    await expect(
      page.locator('.grid-empty, .grid-notice, .grid-foot').filter({ hasText: /affected|OK/i }).first(),
    ).toBeVisible({ timeout: 15_000 });
  });

  test('no horizontal overflow (page + core panes fit the viewport width)', async ({ page }) => {
    test.skip(connId == null, 'postgres connection unavailable');
    await openPostgres(page);
    await runRead(page, 'SELECT c.*, o.* FROM customers c JOIN orders o ON o.customer_id=c.id');
    await ensureResultsOpen(page);

    const { vw, docScrollW } = await docOverflow(page);
    expect(docScrollW, 'document must not scroll horizontally').toBeLessThanOrEqual(vw + 1);

    const ed = await rightEdge(page, '.qe-edit');
    expect(ed.left, 'editor starts within the viewport').toBeGreaterThanOrEqual(-2);
    expect(ed.right, 'editor box fits within the viewport').toBeLessThanOrEqual(ed.vw + 2);

    const gs = await rightEdge(page, '.grid-scroll');
    expect(gs.left, 'results grid starts within the viewport').toBeGreaterThanOrEqual(-2);
    expect(gs.right, 'results grid scroll container fits within the viewport').toBeLessThanOrEqual(gs.vw + 2);

    const toolbarOverflows = await page
      .locator('.grid-toolbar')
      .first()
      .evaluate((el) => el.scrollWidth > el.clientWidth + 1);
    expect(toolbarOverflows, 'results toolbar wraps instead of clipping its controls').toBe(false);
  });

  test('wide result scrolls HORIZONTALLY inside the grid', async ({ page }) => {
    test.skip(connId == null, 'postgres connection unavailable');
    await openPostgres(page);
    await runRead(page, 'SELECT c.*, o.* FROM customers c JOIN orders o ON o.customer_id=c.id');
    await ensureResultsOpen(page);

    const scroll = page.locator('.grid-scroll');
    await scroll.scrollIntoViewIfNeeded();
    const info = await scroll.evaluate((el) => ({ clientW: el.clientWidth, scrollW: el.scrollWidth }));
    expect(info.scrollW, 'wide JOIN result should overflow the grid horizontally').toBeGreaterThan(
      info.clientW + 10,
    );
  });

  test('tall result scrolls VERTICALLY inside the grid', async ({ page }) => {
    test.skip(connId == null, 'postgres connection unavailable');
    await openPostgres(page);
    await runRead(
      page,
      'SELECT c.id AS cid, o.id AS oid, oi.id AS iid FROM customers c, orders o, order_items oi LIMIT 500',
    );
    await ensureResultsOpen(page);

    const scroll = page.locator('.grid-scroll');
    await scroll.scrollIntoViewIfNeeded();
    const info = await scroll.evaluate((el) => ({ clientH: el.clientHeight, scrollH: el.scrollHeight }));
    expect(info.scrollH, 'many rows should overflow the bounded grid vertically').toBeGreaterThan(
      info.clientH + 20,
    );
  });

  test('schema tree is reachable & drills public → Tables → orders → customer_id', async ({ page }) => {
    test.skip(connId == null, 'postgres connection unavailable');
    await openPostgres(page);

    // Reveal the connection list, then the schema view (phone: accordions;
    // tablet/desktop: sidebar tabs) — same affordances as the MySQL sweep.
    if (isPhone(page)) {
      const list = page.locator('.conn-list');
      if (!(await list.isVisible().catch(() => false))) {
        await page.locator('.acc-toggle', { hasText: 'Connections' }).click();
      }
    } else {
      await page.locator('.side-switch .ss', { hasText: 'Connections' }).click();
    }
    await expect(page.locator('.conn-list')).toBeVisible();
    await expect(page.locator('.conn-list .conn-name', { hasText: 'e2e-postgres' }).first()).toBeVisible();

    if (isPhone(page)) {
      const sideBody = page.locator('.side-body');
      if (!(await sideBody.isVisible().catch(() => false))) {
        await page.locator('.acc-toggle', { hasText: 'Schema' }).click();
      }
    } else {
      await page.locator('.side-switch .ss', { hasText: 'Schema' }).click();
    }
    await expect(page.locator('.side-switch')).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('.schema-tree')).toBeVisible({ timeout: 15_000 });

    // Postgres browses by schema: the top level is `public` (not a database), then
    // Tables → orders → its columns include customer_id.
    await expandTreeNode(page, 'public');
    await expandTreeNode(page, 'Tables');
    await expect(treeNode(page, 'orders')).toBeVisible({ timeout: 15_000 });
    await expandTreeNode(page, 'orders');
    await expect(treeNode(page, 'customer_id')).toBeVisible({ timeout: 15_000 });
  });
});

function treeNode(page: Page, label: string) {
  return page.locator('.schema-tree .node .nl-text', { hasText: new RegExp(`^${label}$`) }).first();
}

async function expandTreeNode(page: Page, label: string): Promise<void> {
  const node = treeNode(page, label);
  await node.scrollIntoViewIfNeeded().catch(() => {});
  await expect(node).toBeVisible({ timeout: 15_000 });
  const caret = node.locator('xpath=ancestor::div[contains(@class,"node")][1]').locator('.caret');
  await caret.click();
}
