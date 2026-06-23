import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — MYSQL engine sweep (mobile + tablet, portrait + landscape).
//
// Proves the Database Explorer is fully VISIBLE and USABLE for MySQL against the
// live seeded Docker stack (127.0.0.1:13306 · otto/ottopw · shopdb), across every
// device project the harness defines:
//   • iphone-portrait  430×932  → phone accordion layout  (≤640px)
//   • iphone-se        375×667  → phone accordion layout  (≤640px)
//   • iphone-landscape 932×430  → tablet layout (mobile shell, NOT accordions)
//   • ipad-portrait    834×1194 → tablet layout
//   • ipad-landscape   1194×834 → desktop 3-pane layout (≥1025px)
//
// The three layouts share the SAME content selectors (`.conn-list .conn-name`,
// `.main-tabs`, `.qe-edit .cm-content`, `.grid`, `.grid-scroll`); only the phone
// layout adds tappable accordion headers (`.qe-acc-head`, `.acc-toggle`) that
// must be expanded first. We branch on viewport width (≤640 = phone) for those,
// and assert the SAME seven checks everywhere:
//   1. open #/database + pick the e2e-mysql connection
//   2. SELECT (read) → ≥1 data row in the grid
//   3. INSERT→UPDATE→SELECT (write) → updated value visible + rows-affected msg
//   4. cleanup DELETE
//   5. no horizontal overflow (document + no .db-page child juts past the edge)
//   6. wide result scrolls HORIZONTALLY; tall result scrolls VERTICALLY
//   7. schema tree / connection list reachable + usable in each orientation
//
// Each project uses a UNIQUE scratch sku (`e2e-scratch-mysql-<project>`) so the
// write flows never race across the parallel device projects.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let connId: string | null = null;

const PHONE_MAX = 640;

// Seeding pools a real MySQL connection + runs /test; on a freshly-spawned
// daemon (still warming the usage tailer / cli-update) the first connect can be
// slow, so give the hook generous headroom beyond the default test timeout.
test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  try {
    connId = await seedDockerConnection(ctx, base, workspaceId, 'mysql');
  } catch {
    connId = null;
  }
  // Disposing a traced APIRequestContext from a hook can race the trace-artifact
  // writer (a harmless ENOENT on the network-copy file); seeding is already done,
  // so swallow it rather than fail the whole suite's beforeAll.
  await ctx.dispose().catch(() => {});
});

// Activate the seeded workspace (so connections load) and close the nav drawer
// (which defaults open on a fresh phone profile and would cover the page).
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

/** Open #/database and select the seeded MySQL connection. Returns once the main
 *  tab strip is up (a connection is active). */
async function openMysql(page: Page): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });

  const conn = page.locator('.conn-list .conn-name', { hasText: 'e2e-mysql' });
  await expect(conn.first()).toBeVisible({ timeout: 30_000 });
  await conn.first().click();

  await expect(page.locator('.main-tabs')).toBeVisible({ timeout: 20_000 });
}

/** On a phone the Editor block is gated behind a collapsible accordion header —
 *  make sure it's expanded so the CodeMirror surface is interactable. No-op on
 *  tablet/desktop (the header doesn't render there). */
async function ensureEditorOpen(page: Page): Promise<void> {
  if (!isPhone(page)) return;
  const editor = page.locator('.qe-edit');
  if (!(await editor.isVisible().catch(() => false))) {
    await page.locator('.qe-acc-head', { hasText: 'Editor' }).click();
  }
  await expect(editor).toBeVisible();
}

/** On a phone the Results block is also accordion-gated — expand it so the grid
 *  is on screen. No-op on tablet/desktop. */
async function ensureResultsOpen(page: Page): Promise<void> {
  if (!isPhone(page)) return;
  const results = page.locator('.qe-results');
  if (!(await results.isVisible().catch(() => false))) {
    await page.locator('.qe-acc-head', { hasText: 'Results' }).click();
  }
  await expect(results).toBeVisible();
}

/** Type a statement into the CodeMirror editor (replacing whatever's there) and
 *  press Run, then wait for the run to settle (running dot clears). */
async function typeStatement(page: Page, sql: string): Promise<void> {
  await ensureEditorOpen(page);
  const content = page.locator('.qe-edit .cm-content');
  const mod = process.platform === 'darwin' ? 'Meta' : 'Control';

  // CodeMirror occasionally drops the leading character if keys arrive before
  // the click-to-focus has settled. Retry the whole type until the editor's text
  // (whitespace-normalized) starts with our statement.
  const want = sql.replace(/\s+/g, ' ').trim();
  for (let attempt = 0; attempt < 3; attempt++) {
    await content.click();
    await expect(content).toBeFocused({ timeout: 5_000 });
    await page.keyboard.press(`${mod}+A`);
    await page.keyboard.press('Delete');
    await content.pressSequentially(sql, { delay: 8 });
    // Dismiss any server-driven autocomplete popup so Run/⌘↵ can't accept a
    // highlighted completion and corrupt the statement.
    await page.keyboard.press('Escape');

    const got = ((await content.textContent()) ?? '').replace(/\s+/g, ' ').trim();
    if (got.startsWith(want)) return;
  }
  // Surface the mismatch loudly rather than running a corrupted statement.
  const got = ((await content.textContent()) ?? '').replace(/\s+/g, ' ').trim();
  expect(got, `editor should hold the statement (got: "${got}")`).toContain(want);
}

async function runStatement(page: Page, sql: string): Promise<void> {
  await typeStatement(page, sql);
  await page.getByRole('button', { name: /^Run/ }).first().click();
  // The Run button flips to "Stop" while running; wait for it to come back.
  await expect(page.getByRole('button', { name: /^Run/ }).first()).toBeVisible({
    timeout: 20_000,
  });
}

/** Run a READ statement and wait until ≥1 data row is rendered in the grid. */
async function runRead(page: Page, sql: string): Promise<void> {
  await runStatement(page, sql);
  await ensureResultsOpen(page);
  await expect(page.locator('.grid tbody tr:not(.spacer)').first()).toBeVisible({
    timeout: 20_000,
  });
}

/** The document's horizontal scroll vs the viewport — the hard "the page never
 *  scrolls sideways / content is never lost off a scrolling page" gate. Holds in
 *  every layout (phone/tablet/desktop) because overflowing chrome is clipped by
 *  an `overflow:hidden` ancestor rather than widening the document. */
async function docOverflow(page: Page): Promise<{ vw: number; docScrollW: number }> {
  return page.evaluate(() => {
    const de = document.documentElement;
    return { vw: de.clientWidth, docScrollW: de.scrollWidth };
  });
}

/** Right edge + viewport width for a single element (rounded). */
async function rightEdge(page: Page, selector: string): Promise<{ right: number; vw: number; left: number }> {
  return page.locator(selector).first().evaluate((el) => {
    const r = el.getBoundingClientRect();
    return { right: Math.round(r.right), left: Math.round(r.left), vw: document.documentElement.clientWidth };
  });
}

test.describe('DB Explorer · MySQL sweep', () => {
  // Serial within a project so the per-project scratch row's INSERT→UPDATE→
  // SELECT→DELETE lifecycle isn't interleaved with the read/layout tests sharing
  // the same connection session.
  test.describe.configure({ mode: 'serial' });

  test('connection seeds & is reachable (driver health)', () => {
    // A null here is a real bug (driver/daemon couldn't reach the seeded MySQL),
    // not a reason to silently skip — fail loudly so it's investigated.
    expect(
      connId,
      'seedDockerConnection(mysql) returned null — MySQL driver/daemon could not connect to 127.0.0.1:13306',
    ).not.toBeNull();
  });

  test('SELECT (read) shows rows in the grid', async ({ page }) => {
    test.skip(connId == null, 'mysql connection unavailable');
    await openMysql(page);
    await runRead(page, 'SELECT * FROM customers ORDER BY id');

    const rows = await page.locator('.grid tbody tr:not(.spacer)').count();
    expect(rows, 'customers SELECT should return ≥1 data row').toBeGreaterThanOrEqual(1);

    // The seeded first customer is present somewhere in the grid.
    await ensureResultsOpen(page);
    await expect(page.locator('.grid tbody').getByText('ada@example.com').first()).toBeVisible({
      timeout: 10_000,
    });
  });

  test('UPDATE (write) applies and the new value is visible', async ({ page }, info) => {
    test.skip(connId == null, 'mysql connection unavailable');
    const sku = `e2e-scratch-mysql-${info.project.name}`;
    await openMysql(page);

    // Best-effort pre-clean in case a prior aborted run left the scratch row.
    await runStatement(page, `DELETE FROM products WHERE sku='${sku}'`);

    // INSERT the scratch row.
    await runStatement(page, `INSERT INTO products (sku,name,price_cents) VALUES ('${sku}','E2E',1)`);
    await ensureResultsOpen(page);
    // A write surfaces a rows-affected / OK message in the results pane.
    await expect(
      page.locator('.grid-empty, .grid-notice, .grid-foot').filter({ hasText: /affected|OK/i }).first(),
    ).toBeVisible({ timeout: 15_000 });

    // UPDATE it to a known value.
    await runStatement(page, `UPDATE products SET price_cents=2 WHERE sku='${sku}'`);
    await ensureResultsOpen(page);
    await expect(
      page.locator('.grid-empty, .grid-notice, .grid-foot').filter({ hasText: /affected|OK/i }).first(),
    ).toBeVisible({ timeout: 15_000 });

    // SELECT it back — the grid must show price_cents = 2 (proves the UPDATE took
    // effect and the result is visible).
    await runRead(page, `SELECT id,sku,price_cents FROM products WHERE sku='${sku}'`);
    // Locate the scratch row, scroll it into the virtualized window, then assert
    // it renders price_cents = 2. (innerText on an un-laid-out virtualized row can
    // read empty in a short landscape viewport, so wait for visibility first.)
    const scratchRow = page.locator('.grid tbody tr:not(.spacer)', { hasText: sku }).first();
    await scratchRow.scrollIntoViewIfNeeded();
    await expect(scratchRow).toBeVisible({ timeout: 15_000 });
    await expect(
      scratchRow,
      'scratch row should show updated price_cents=2',
    ).toContainText('2', { timeout: 15_000 });

    // Cleanup.
    await runStatement(page, `DELETE FROM products WHERE sku='${sku}'`);
    await ensureResultsOpen(page);
    await expect(
      page.locator('.grid-empty, .grid-notice, .grid-foot').filter({ hasText: /affected|OK/i }).first(),
    ).toBeVisible({ timeout: 15_000 });
  });

  test('no horizontal overflow (page + core panes fit the viewport width)', async ({ page }) => {
    test.skip(connId == null, 'mysql connection unavailable');
    await openMysql(page);
    // A wide result is the worst case for overflow — load one, then measure.
    await runRead(page, 'SELECT c.*, o.* FROM customers c JOIN orders o ON o.customer_id=c.id');
    await ensureResultsOpen(page);

    // Hard gate: the DOCUMENT never scrolls sideways (content is never lost off a
    // scrolling page). True in every layout — overflowing chrome is clipped, not
    // exposed via a horizontally-scrolling document.
    const { vw, docScrollW } = await docOverflow(page);
    expect(docScrollW, 'document must not scroll horizontally').toBeLessThanOrEqual(vw + 1);

    // The core data panes — the editor box and the results grid's scroll
    // container — fit inside the viewport (their own box; the grid's WIDE table
    // scrolls INSIDE .grid-scroll, asserted separately). This is the real
    // "visible + usable" guarantee. (The results toolbar now wraps rather than
    // overflowing — asserted below. The main-tab status row is a separate surface.)
    const ed = await rightEdge(page, '.qe-edit');
    expect(ed.left, 'editor starts within the viewport').toBeGreaterThanOrEqual(-2);
    expect(ed.right, 'editor box fits within the viewport').toBeLessThanOrEqual(ed.vw + 2);

    const gs = await rightEdge(page, '.grid-scroll');
    expect(gs.left, 'results grid starts within the viewport').toBeGreaterThanOrEqual(-2);
    expect(gs.right, 'results grid scroll container fits within the viewport').toBeLessThanOrEqual(gs.vw + 2);

    // The results toolbar must WRAP, not overflow its container. Before the fix
    // it clipped trailing controls (Download…/Full Export) in the narrow desktop
    // results pane (viewport > 1024, pane < viewport). scrollWidth > clientWidth
    // means content runs off the edge; with flex-wrap they fall to a new row.
    const toolbarOverflows = await page
      .locator('.grid-toolbar')
      .first()
      .evaluate((el) => el.scrollWidth > el.clientWidth + 1);
    expect(toolbarOverflows, 'results toolbar wraps instead of clipping its controls').toBe(false);
  });

  test('wide result scrolls HORIZONTALLY inside the grid', async ({ page }) => {
    test.skip(connId == null, 'mysql connection unavailable');
    await openMysql(page);
    // customers (5 cols) JOIN orders (5 cols) → ~10 cols, wider than any viewport.
    await runRead(page, 'SELECT c.*, o.* FROM customers c JOIN orders o ON o.customer_id=c.id');
    await ensureResultsOpen(page);

    const scroll = page.locator('.grid-scroll');
    await scroll.scrollIntoViewIfNeeded();
    const info = await scroll.evaluate((el) => ({
      clientW: el.clientWidth,
      scrollW: el.scrollWidth,
    }));
    expect(info.scrollW, 'wide JOIN result should overflow the grid horizontally').toBeGreaterThan(
      info.clientW + 10,
    );
  });

  test('tall result scrolls VERTICALLY inside the grid', async ({ page }) => {
    test.skip(connId == null, 'mysql connection unavailable');
    await openMysql(page);
    // A wide UNION cross of the seed tables makes enough rows to overflow the
    // bounded results block. order_items + a cross join gives plenty of rows.
    await runRead(
      page,
      'SELECT c.id AS cid, o.id AS oid, oi.id AS iid FROM customers c, orders o, order_items oi LIMIT 500',
    );
    await ensureResultsOpen(page);

    const scroll = page.locator('.grid-scroll');
    await scroll.scrollIntoViewIfNeeded();
    const info = await scroll.evaluate((el) => ({
      clientH: el.clientHeight,
      scrollH: el.scrollHeight,
    }));
    expect(info.scrollH, 'many rows should overflow the bounded grid vertically').toBeGreaterThan(
      info.clientH + 20,
    );
  });

  test('schema tree / connection list is reachable & usable', async ({ page }) => {
    test.skip(connId == null, 'mysql connection unavailable');
    await openMysql(page);

    // Reveal the connection list. On a phone it's behind the "Connections"
    // accordion; on tablet/desktop it's the "Connections" sidebar tab (opening a
    // connection switches the sidebar to Schema, so click back to Connections).
    if (isPhone(page)) {
      const list = page.locator('.conn-list');
      if (!(await list.isVisible().catch(() => false))) {
        await page.locator('.acc-toggle', { hasText: 'Connections' }).click();
      }
    } else {
      await page.locator('.side-switch .ss', { hasText: 'Connections' }).click();
    }
    await expect(page.locator('.conn-list')).toBeVisible();
    await expect(page.locator('.conn-list .conn-name', { hasText: 'e2e-mysql' }).first()).toBeVisible();

    // Open the schema view. Phone: the "Schema & saved" accordion gates it.
    // Tablet/desktop: click the "Schema" tab.
    if (isPhone(page)) {
      const sideBody = page.locator('.side-body');
      if (!(await sideBody.isVisible().catch(() => false))) {
        await page.locator('.acc-toggle', { hasText: 'Schema' }).click();
      }
    } else {
      await page.locator('.side-switch .ss', { hasText: 'Schema' }).click();
    }
    // The schema switch (Schema/Saved/History) and the tree are present.
    await expect(page.locator('.side-switch')).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('.schema-tree')).toBeVisible({ timeout: 15_000 });

    // Drill the tree: shopdb (database) → Tables (folder) → the `orders` table
    // node appears. The SchemaTree lazy-loads children, so each expand fetches
    // the next level; reaching `orders` proves the tree is interactive + usable.
    await expandTreeNode(page, 'shopdb');
    await expandTreeNode(page, 'Tables');
    await expect(treeNode(page, 'orders')).toBeVisible({ timeout: 15_000 });
  });
});

/** A schema-tree node located by its exact label text. */
function treeNode(page: Page, label: string) {
  return page.locator('.schema-tree .node .nl-text', { hasText: new RegExp(`^${label}$`) }).first();
}

/** Expand a schema-tree node by clicking its caret toggle (clicking a TABLE
 *  label opens the Structure view instead of expanding, so we always use the
 *  caret). Scrolls it into view first. */
async function expandTreeNode(page: Page, label: string): Promise<void> {
  const node = treeNode(page, label);
  await node.scrollIntoViewIfNeeded().catch(() => {});
  await expect(node).toBeVisible({ timeout: 15_000 });
  // The caret sits in the same `.node` row, before the label.
  const caret = node.locator('xpath=ancestor::div[contains(@class,"node")][1]').locator('.caret');
  await caret.click();
}
