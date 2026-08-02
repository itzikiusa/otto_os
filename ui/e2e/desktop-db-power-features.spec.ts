import { test, expect, type Page, type Locator } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — "power" feature batch, driven against the live Docker stack:
//   • Filter chips actually narrow the grid (client-side over loaded rows)
//   • BSON type indicators (ObjectId("…")) + JSON key highlighting
//   • Undo (Cmd+Z) after a "Query by value" rewrite
//   • Auto-close quotes in the editor
//   • Cmd+F opens the in-EDITOR search panel (not the page-wide overlay)
//   • Format unwraps Java-style string concatenation into clean SQL
//   • Schema-tree "Copy create statement" (MySQL DDL → clipboard)
//   • Table Designer can ADD indexes + foreign keys (ALTER preview)
//   • Mongo index builder offers NESTED field paths (players.playerId)
//   • Index rows can EDIT (drop + recreate) / DROP — prepared, never applied
//   • Index CONDITIONS (partial indexes): Mongo partialFilterExpression,
//     Postgres WHERE, MySQL's CASE functional-key-part emulation
//
// Desktop-browser project only; each test.skips when its container is down.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
const conn: Record<'mysql' | 'mongodb' | 'postgres', string | null> = {
  mysql: null,
  mongodb: null,
  postgres: null,
};

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  for (const k of ['mysql', 'mongodb', 'postgres'] as const) {
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
  // insertText injects the whole string in ONE input event — char-by-char typing
  // would let the new auto-close-brackets feature double up `)`/`}` and corrupt a
  // `db.coll.find({})`.
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.waitForTimeout(60);
  await page.keyboard.press('ControlOrMeta+a');
  await page.waitForTimeout(40);
  await page.keyboard.insertText(sql);
  await page.waitForTimeout(300);
}

const editorText = (page: Page) =>
  page.locator('.qe-edit .cm-content').textContent().then((t) => t ?? '');

// Find a table/collection's label in the tree, expanding collapsed nodes until it
// appears (right-click bubbles to the row's oncontextmenu; left-click opens its
// structure). Expanding every collapsed caret is the robust way to surface a
// table regardless of which database holds it.
async function objectLabel(page: Page, label: string): Promise<Locator> {
  await expect(page.locator('.node-label').first()).toBeVisible({ timeout: 20_000 });
  // EXACT label match on the node's `.nl-text`, scoped to OBJECT nodes
  // (table/view/collection icons) so `orders` matches the table — not an index
  // (`idx_orders_status`) or a column named `orders` (daily_sales.orders).
  const exact = new RegExp(`^${label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`);
  const lbl = page
    .locator('.node')
    .filter({ has: page.locator('.node-icon.table, .node-icon.collection, .node-icon.view') })
    .filter({ has: page.locator('.nl-text').filter({ hasText: exact }) })
    .locator('.node-label')
    .first();
  for (let attempt = 0; attempt < 3; attempt++) {
    if (await lbl.isVisible().catch(() => false)) return lbl;
    const carets = page.locator('.node .caret');
    const n = await carets.count();
    for (let i = 0; i < n; i++) {
      await carets.nth(i).click().catch(() => {});
      await page.waitForTimeout(110);
    }
    await page.waitForTimeout(600);
  }
  await expect(lbl).toBeVisible({ timeout: 10_000 });
  return lbl;
}

// ── MySQL ───────────────────────────────────────────────────────────────────

test('MySQL: Format unwraps Java string-concatenation into clean SQL', async ({ page }) => {
  test.skip(!conn.mysql, 'mysql docker not reachable');
  await openConn(page, 'e2e-mysql');
  await setEditor(page, '"SELECT * FROM " + products + " WHERE id = " + 1');
  await page.getByRole('button', { name: /Format/ }).click();
  await expect(page.locator('.toast', { hasText: 'Format failed' })).toHaveCount(0);
  const t = await editorText(page);
  expect(t).toContain('SELECT');
  expect(t).toContain('FROM');
  expect(t).not.toContain('"SELECT');
});

test('MySQL: editor auto-closes a typed quote', async ({ page }) => {
  test.skip(!conn.mysql, 'mysql docker not reachable');
  await openConn(page, 'e2e-mysql');
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.press('Delete');
  await page.keyboard.type("SELECT 'a");
  await expect.poll(() => editorText(page)).toContain("'a'");
  await page.keyboard.press('Escape');
});

test('MySQL: Cmd+F opens the in-editor search panel, not the page overlay', async ({ page }) => {
  test.skip(!conn.mysql, 'mysql docker not reachable');
  await openConn(page, 'e2e-mysql');
  await page.locator('.qe-edit .cm-content').click();
  await page.keyboard.press('ControlOrMeta+f');
  await expect(page.locator('.qe-edit .cm-search')).toBeVisible({ timeout: 5_000 });
  await expect(page.locator('.otto-find-bar')).toHaveCount(0);
});

test('MySQL: schema tree "Copy create statement" copies the DDL', async ({ page }) => {
  test.skip(!conn.mysql, 'mysql docker not reachable');
  await openConn(page, 'e2e-mysql');
  const lbl = await objectLabel(page, 'products');
  await lbl.click({ button: 'right' });
  await expect(page.locator('.ctx-menu')).toBeVisible();
  await page.locator('.ctx-item', { hasText: /Copy create statement/ }).first().click();
  await expect
    .poll(() => page.evaluate(() => navigator.clipboard.readText()))
    .toContain('CREATE TABLE');
});

test('MySQL: Table Designer can add an index and a foreign key', async ({ page }) => {
  test.skip(!conn.mysql, 'mysql docker not reachable');
  await openConn(page, 'e2e-mysql');
  const lbl = await objectLabel(page, 'orders');
  await lbl.click(); // open structure
  await page.getByRole('button', { name: /Design/ }).first().click();
  await expect(page.locator('.td-modal')).toBeVisible({ timeout: 10_000 });

  await page.getByRole('button', { name: /Add index/ }).click();
  const ixRow = page.locator('.td-ix-row').last();
  await ixRow.locator('input').nth(0).fill('idx_status');
  await ixRow.locator('input').nth(1).fill('status');

  await page.getByRole('button', { name: /Add foreign key/ }).click();
  const fkRow = page.locator('.td-fk-row').last();
  await fkRow.locator('input').nth(0).fill('customer_id');
  await fkRow.locator('input').nth(1).fill('customers');
  await fkRow.locator('input').nth(2).fill('id');

  const preview = page.locator('.td-preview');
  await expect(preview).toContainText('ADD INDEX `idx_status` (`status`)');
  await expect(preview).toContainText('FOREIGN KEY (`customer_id`) REFERENCES `customers` (`id`)');
});

// ── MongoDB ─────────────────────────────────────────────────────────────────
//
// NOTE: BSON type indicators (ObjectId("…")/ISODate("…")) + JSON-view key
// highlighting render in the RESULTS pane, which the headless wide web shell
// collapses to zero height (the Tauri app shows it fine). The backend typing is
// covered by the `bson_to_json_typed_*` Rust unit test, and the UI humanizing /
// highlighting are pure functions — so they're not asserted here.

test('Mongo: Cmd+Z undoes an external statement rewrite (Format)', async ({ page }) => {
  test.skip(!conn.mongodb, 'mongodb docker not reachable');
  await openConn(page, 'e2e-mongodb');
  // Format rewrites the statement via the SAME setStatement path as "Query by
  // value"; the editor now applies external rewrites as an undoable transaction
  // (not a view rebuild), so Cmd+Z restores the prior text.
  await setEditor(page, 'db.products.find({sku:"SKU-1",inStock:true})');
  const before = await editorText(page);
  // Let the typed insert close its undo group (CodeMirror coalesces edits within
  // ~0.5s) so Cmd+Z undoes ONLY the Format, not the insert too.
  await page.waitForTimeout(800);
  await page.getByRole('button', { name: /Format/ }).click();
  await expect.poll(() => editorText(page)).not.toBe(before); // reflowed
  await page.locator('.qe-edit .cm-content').click();
  await page.keyboard.press('ControlOrMeta+z');
  await expect.poll(() => editorText(page).then((t) => t.replace(/\s+/g, ''))).toBe(
    before.replace(/\s+/g, ''),
  );
});

test('Mongo: index builder offers NESTED field paths, and filters the list', async ({ page }) => {
  test.skip(!conn.mongodb, 'mongodb docker not reachable');
  await openConn(page, 'e2e-mongodb');
  const lbl = await objectLabel(page, 'profiles');
  await lbl.click(); // open structure
  await page.getByRole('button', { name: /New index/ }).click();
  // profiles documents have an embedded `address.city` — it must be offered as a
  // row, not just the top-level `address`.
  await expect(page.locator('.ib-row', { hasText: 'address.city' }).first()).toBeVisible({
    timeout: 15_000,
  });
  // The field picker is a searchable LIST (60+ sampled paths make chips
  // unscannable) — typing narrows it.
  await page.locator('.ib-search').fill('city');
  await expect(page.locator('.ib-row', { hasText: 'address.city' })).toHaveCount(1);
  await expect(page.locator('.ib-row', { hasText: 'userId' })).toHaveCount(0);
});

// ── Index edit / drop ───────────────────────────────────────────────────────
// Both actions PREPARE a statement in a query tab; nothing is applied until the
// user presses Run — so these assert the generated text, and the seeded indexes
// survive the run.

/** Open `object`'s structure and return the row for index `name`. */
async function indexRow(page: Page, object: string, name: string): Promise<Locator> {
  const lbl = await objectLabel(page, object);
  await lbl.click();
  const row = page.locator('.idx-row').filter({ hasText: name }).first();
  await expect(row).toBeVisible({ timeout: 20_000 });
  await row.hover(); // actions are hover-revealed
  return row;
}

test('Mongo: Drop index prepares a dropIndex for review', async ({ page }) => {
  test.skip(!conn.mongodb, 'mongodb docker not reachable');
  await openConn(page, 'e2e-mongodb');
  const row = await indexRow(page, 'profiles', 'address.city_1');
  await row.getByRole('button', { name: 'Drop index address.city_1' }).click();
  await expect
    .poll(() => editorText(page), { timeout: 15_000 })
    .toContain('db.profiles.dropIndex("address.city_1")');
});

test('Mongo: Edit index prepares a drop + recreate that keeps unique', async ({ page }) => {
  test.skip(!conn.mongodb, 'mongodb docker not reachable');
  await openConn(page, 'e2e-mongodb');
  // `userId_1` is seeded UNIQUE — editing must carry that (and the name) over.
  const row = await indexRow(page, 'profiles', 'userId_1');
  await row.getByRole('button', { name: 'Edit index userId_1' }).click();
  await expect(page.locator('.ib-editing')).toContainText('userId_1');
  await expect(page.locator('.ib-name input')).toHaveValue('userId_1');
  await expect(page.locator('.ib-unique input')).toBeChecked();
  await expect(page.locator('.ib-row.on', { hasText: 'userId' }).first()).toBeVisible();

  await page.getByRole('button', { name: /Prepare drop \+ recreate/ }).click();
  await expect
    .poll(() => editorText(page), { timeout: 15_000 })
    .toContain('dropIndex("userId_1")');
  const stmt = await editorText(page);
  expect(stmt).toContain('db.profiles.createIndex');
  expect(stmt.replace(/\s+/g, '')).toContain('"unique":true');
  expect(stmt.replace(/\s+/g, '')).toContain('"name":"userId_1"');
});

test("Mongo: the _id_ index can't be edited or dropped", async ({ page }) => {
  test.skip(!conn.mongodb, 'mongodb docker not reachable');
  await openConn(page, 'e2e-mongodb');
  const row = await indexRow(page, 'profiles', '_id_');
  await expect(row.getByRole('button', { name: 'Edit index _id_' })).toBeDisabled();
  await expect(row.getByRole('button', { name: 'Drop index _id_' })).toBeDisabled();
});

test('MySQL: Drop index prepares ALTER TABLE … DROP INDEX (and DROP PRIMARY KEY)', async ({
  page,
}) => {
  test.skip(!conn.mysql, 'mysql docker not reachable');
  await openConn(page, 'e2e-mysql');
  const row = await indexRow(page, 'orders', 'idx_orders_status');
  await row.getByRole('button', { name: 'Drop index idx_orders_status' }).click();
  await expect
    .poll(() => editorText(page), { timeout: 15_000 })
    .toContain('ALTER TABLE `orders` DROP INDEX `idx_orders_status`;');

  // MySQL names the PK index PRIMARY, which DROP INDEX can't take.
  const pk = await indexRow(page, 'orders', 'PRIMARY');
  await pk.getByRole('button', { name: 'Drop index PRIMARY' }).click();
  await expect
    .poll(() => editorText(page), { timeout: 15_000 })
    .toContain('ALTER TABLE `orders` DROP PRIMARY KEY;');
});

// ── Index conditions (partial indexes) ──────────────────────────────────────
// One operator pair (`exists` / `in`) mapped onto three very different engines.

/** Open the New-index builder on `object`, select `field` as the key. */
async function newIndexOn(page: Page, object: string, field: string): Promise<void> {
  const lbl = await objectLabel(page, object);
  await lbl.click();
  await page.getByRole('button', { name: /New index/ }).click();
  await page.locator('.ib-search').fill(field);
  await page.locator('.ib-row', { hasText: field }).first().click();
  await expect(page.locator('.ib-row.on', { hasText: field }).first()).toBeVisible();
}

/** Add a condition row: pick field + operator (+ values for `in`). */
async function addCondition(
  page: Page,
  field: string,
  op: 'exists' | 'in',
  values?: string,
): Promise<void> {
  await page.getByRole('button', { name: /Add condition/ }).click();
  const row = page.locator('.ib-cond').last();
  await row.locator('select').first().selectOption(field);
  await row.locator('select').nth(1).selectOption(op);
  if (op === 'in' && values !== undefined) await row.locator('input').fill(values);
}

test('Mongo: an `exists` condition becomes a partialFilterExpression', async ({ page }) => {
  test.skip(!conn.mongodb, 'mongodb docker not reachable');
  await openConn(page, 'e2e-mongodb');
  await newIndexOn(page, 'profiles', 'userId');
  await addCondition(page, 'userId', 'exists');
  await page.getByRole('button', { name: /Prepare createIndex/ }).click();
  const stmt = (await editorText(page)).replace(/\s+/g, '');
  expect(stmt).toContain('db.profiles.createIndex');
  expect(stmt).toContain('"partialFilterExpression"');
  expect(stmt).toContain('"userId":{"$exists":true}');
});

test('Mongo: an `in` condition types its values ($in with a number)', async ({ page }) => {
  test.skip(!conn.mongodb, 'mongodb docker not reachable');
  await openConn(page, 'e2e-mongodb');
  await newIndexOn(page, 'profiles', 'userId');
  await addCondition(page, 'userId', 'in', 'a, 3, true');
  await page.getByRole('button', { name: /Prepare createIndex/ }).click();
  // Bare numbers/bools must NOT be quoted — a stringified 3 would never match.
  expect((await editorText(page)).replace(/\s+/g, '')).toContain('"userId":{"$in":["a",3,true]}');
});

test('MySQL: a condition emulates a partial index with a CASE key part', async ({ page }) => {
  test.skip(!conn.mysql, 'mysql docker not reachable');
  await openConn(page, 'e2e-mysql');
  await newIndexOn(page, 'orders', 'status');
  await addCondition(page, 'status', 'in', 'paid, shipped');
  await expect(page.locator('.ib-warn')).toContainText('no partial index');
  await page.getByRole('button', { name: /Prepare CREATE INDEX/ }).click();
  const stmt = await editorText(page);
  expect(stmt).toContain('MySQL has no partial indexes');
  // Expression key parts need their OWN parens inside the key list.
  expect(stmt).toContain(
    "((CASE WHEN `status` IN ('paid', 'shipped') THEN `status` END))",
  );
});

test('Postgres: a condition becomes a native WHERE clause', async ({ page }) => {
  test.skip(!conn.postgres, 'postgres docker not reachable');
  await openConn(page, 'e2e-postgres');
  await newIndexOn(page, 'orders', 'status');
  await addCondition(page, 'status', 'exists');
  await page.getByRole('button', { name: /Prepare CREATE INDEX/ }).click();
  const stmt = await editorText(page);
  expect(stmt).toContain('ON "orders" ("status") WHERE "status" IS NOT NULL;');
  expect(stmt).not.toContain('CASE'); // native support — no emulation
});

test('Postgres: Drop index uses DROP INDEX, but DROP CONSTRAINT for the PK', async ({ page }) => {
  test.skip(!conn.postgres, 'postgres docker not reachable');
  await openConn(page, 'e2e-postgres');
  const row = await indexRow(page, 'orders', 'idx_orders_status');
  await row.getByRole('button', { name: 'Drop index idx_orders_status' }).click();
  await expect
    .poll(() => editorText(page), { timeout: 15_000 })
    .toContain('DROP INDEX "idx_orders_status";');

  // `orders_pkey` BACKS the primary-key constraint — Postgres rejects DROP INDEX
  // on it and points at the constraint, so we must emit the constraint form.
  const pk = await indexRow(page, 'orders', 'orders_pkey');
  await pk.getByRole('button', { name: 'Drop index orders_pkey' }).click();
  await expect
    .poll(() => editorText(page), { timeout: 15_000 })
    .toContain('ALTER TABLE "orders" DROP CONSTRAINT "orders_pkey";');
});
