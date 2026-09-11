import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ── JSON editing in the results grid (desktop-browser; needs the docker Mongo) ──
//
// Regression: complex (object/array) cells only opened a READ-ONLY viewer — no
// edit path at all — and the JSON view mode had no way to edit a document.
// Proves both new paths end-to-end against the seeded docker MongoDB:
//   1. Grid: click a JSON cell → viewer → Edit → change → Save… → review shows
//      updateOne → Run → re-query shows the new value.
//   2. JSON view: per-document Edit → change a field → Save… → review shows
//      replaceOne → Run → re-query shows the new value.
//   3. Vertical view: nested fields are rows — double-click `items.0.qty` →
//      typed editor → pending → review shows the path diff + a dotted-path
//      `$set` → Run → re-query shows the new value; the field menu's Delete
//      parks a `$unset`; the record menu's Insert document… reviews an
//      insertOne.
// Works on a scratch collection so the shared seed data other specs assert on
// (customers/orders/…) is never mutated.

let workspaceId = '';
let mongoConnId: string | null = null;

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  try {
    mongoConnId = await seedDockerConnection(ctx, base, workspaceId, 'mongodb');
  } catch {
    mongoConnId = null;
  }
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

async function openConn(page: Page): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const conn = page.locator('.conn-list .conn-name', { hasText: 'e2e-mongodb' });
  await expect(conn.first()).toBeVisible({ timeout: 30_000 });
  await conn.first().click();
  await expect(page.locator('.main-tabs')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.cap-chip', { hasText: 'mongodb' })).toBeVisible({ timeout: 20_000 });
}

// Same CodeMirror quirks as db-sweep-mongodb.spec.ts: one-shot insertText so the
// server-backed autocomplete can't corrupt dotted commands, then settle.
async function setEditorText(page: Page, statement: string): Promise<void> {
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.waitForTimeout(60);
  await page.keyboard.press('ControlOrMeta+A');
  await page.waitForTimeout(40);
  await page.keyboard.insertText(statement);
  await page.waitForTimeout(300);
}

async function runStatement(page: Page, statement: string): Promise<void> {
  await setEditorText(page, statement);
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
}

/** Confirm the review modal shows the expected statement, run it, wait out. */
async function runReviewModal(page: Page, mustContain: string[]): Promise<void> {
  const modal = page.locator('.review-modal');
  await expect(modal).toBeVisible();
  const sql = await modal.locator('.review-sql').inputValue();
  for (const frag of mustContain) expect(sql, `review statement should contain ${frag}`).toContain(frag);
  await modal.locator('.tb-btn.primary', { hasText: 'Run' }).click();
  await expect(modal).toBeHidden({ timeout: 20_000 });
}

test('JSON cell + JSON-view document editing round-trips through review', async ({ page }) => {
  test.setTimeout(180_000);
  expect(mongoConnId, 'docker Mongo must seed (127.0.0.1:17017 up?)').not.toBeNull();

  await openConn(page);

  // Idempotence: a failed prior run leaves mutated docs behind — start clean.
  await runStatement(page, 'db.e2e_json_edit.deleteMany({})');
  await page.waitForTimeout(500);

  // Scratch doc with a complex (array-of-objects) field.
  await runStatement(
    page,
    'db.e2e_json_edit.insertOne({ k: "doc1", items: [{ productId: 1, qty: 1 }], status: "pending" })',
  );
  await runStatement(page, 'db.e2e_json_edit.find({})');
  // Mongo opens in Vertical by default — this half of the spec is about the grid.
  await page.locator('.view-seg .vs', { hasText: 'Grid' }).click();
  await expect(page.locator('.view-seg .vs.on')).toHaveText('Grid');
  await expect(page.locator('.grid tbody tr').first()).toBeVisible({ timeout: 20_000 });

  // ── 1. Grid: edit the complex `items` cell through the viewer ──
  await page.locator('.cell.json').first().click();
  const viewer = page.locator('.cell-viewer');
  await expect(viewer).toBeVisible();
  await viewer.locator('.tb-btn', { hasText: 'Edit' }).click();
  const cellEditor = viewer.locator('.cv-edit');
  await expect(cellEditor).toBeVisible();
  await cellEditor.fill('[{ "productId": 1, "qty": 5 }]');
  await viewer.locator('.btn.primary', { hasText: 'Save' }).click();
  // Saving PARKS the cell as a pending change (multi-field batching): the
  // dirty cell + the pending bar appear, and "Review & apply" prepares ONE
  // updateOne carrying every parked column of the row.
  const pendingBar = page.locator('[data-testid="pending-edits-bar"]');
  await expect(pendingBar).toBeVisible();
  await expect(page.locator('.cell.dirty').first()).toBeVisible();
  await pendingBar.locator('.btn.primary', { hasText: 'Review & apply' }).click();
  await runReviewModal(page, ['updateOne', '"items"', '"qty":5']);

  await runStatement(page, 'db.e2e_json_edit.find({})');
  await expect(page.locator('.cell.json').first()).toContainText('"qty":5', { timeout: 20_000 });

  // ── 2. JSON view: whole-document edit → replaceOne ──
  await page.locator('.vs', { hasText: 'JSON' }).click();
  const rec = page.locator('.jrec').first();
  await expect(rec).toBeVisible();
  await rec.locator('[aria-label="Edit document"]').click();
  const docEd = page.locator('.cell-viewer .cv-edit');
  await expect(docEd).toBeVisible();
  const draft = JSON.parse(await docEd.inputValue()) as Record<string, unknown>;
  expect(draft.status).toBe('pending');
  draft.status = 'paid';
  await docEd.fill(JSON.stringify(draft, null, 2));
  await page.locator('.cell-viewer .btn.primary', { hasText: 'Save' }).click();
  await runReviewModal(page, ['replaceOne', '"status":"paid"']);

  await runStatement(page, 'db.e2e_json_edit.find({})');
  await page.locator('.vs', { hasText: 'JSON' }).click();
  await expect(page.locator('.alt-json').first()).toContainText('paid', { timeout: 20_000 });

  // Bad JSON is rejected inline (no review modal, error shown).
  await page.locator('.jrec').first().locator('[aria-label="Edit document"]').click();
  await expect(docEd).toBeVisible();
  await docEd.fill('{ not json');
  await page.locator('.cell-viewer .btn.primary', { hasText: 'Save' }).click();
  await expect(page.locator('.cv-err')).toBeVisible();
  await expect(page.locator('.review-modal')).toBeHidden();
  await page.locator('.cell-viewer .btn.ghost', { hasText: 'Cancel' }).click();

  // Cleanup the scratch collection.
  await runStatement(page, 'db.e2e_json_edit.deleteMany({})');
});

/** Reset the scratch collection to ONE known document and show it in Vertical. */
async function seedAndOpenVertical(page: Page): Promise<void> {
  await runStatement(page, 'db.e2e_json_edit.deleteMany({})');
  await page.waitForTimeout(500);
  await runStatement(
    page,
    'db.e2e_json_edit.insertOne({ k: "doc1", items: [{ productId: 1, qty: 1 }], status: "pending" })',
  );
  await runStatement(page, 'db.e2e_json_edit.find({})');
  await page.locator('.view-seg .vs', { hasText: 'Vertical' }).click();
  await expect(page.locator('.view-seg .vs.on')).toHaveText('Vertical');
  await expect(page.locator('.vrec').first()).toBeVisible({ timeout: 20_000 });
}

/** The `.vrow` of record `rec` whose key label is exactly `key`. */
function fieldRow(page: Page, rec: ReturnType<Page['locator']>, key: string): ReturnType<Page['locator']> {
  return rec.locator('.vrow', { has: page.locator('.vk', { hasText: new RegExp(`^${key}$`) }) });
}

test('Vertical: nested dbl-click → $set dotted path with diff', async ({ page }) => {
  test.setTimeout(180_000);
  expect(mongoConnId, 'docker Mongo must seed (127.0.0.1:17017 up?)').not.toBeNull();

  await openConn(page);
  await seedAndOpenVertical(page);
  const rec = page.locator('.vrec').first();

  // Sub-documents are nested rows, open by default (the small doc fits the
  // node budget): `qty` inside `items.0` is on screen without a click.
  const qty = fieldRow(page, rec, 'qty');
  await expect(qty).toBeVisible();
  await expect(qty.locator('.vv')).toHaveText('1');

  // Double-click → inline typed editor (pre-selected: number) → 7 → Enter
  // PARKS the change: dirty row + the pending bar.
  await qty.dblclick();
  const input = rec.locator('.ve-input');
  await expect(input).toBeVisible();
  await expect(rec.locator('.ve-kind').first()).toHaveValue('number');
  await input.fill('7');
  await input.press('Enter');
  const pendingBar = page.locator('[data-testid="pending-edits-bar"]');
  await expect(pendingBar).toBeVisible();
  await expect(rec.locator('.vrow.dirty')).toHaveCount(1);
  await expect(fieldRow(page, rec, 'qty').locator('.vv')).toHaveText('7');

  // Review: the diff table names the dotted path; the statement is ONE
  // updateOne with a dotted-path $set (not a replace).
  await pendingBar.locator('.btn.primary', { hasText: 'Review & apply' }).click();
  const modal = page.locator('.review-modal');
  await expect(modal).toBeVisible();
  await expect(modal.locator('.review-diff tr.op-set .rd-path')).toHaveText('items.0.qty');
  await expect(modal.locator('.review-diff tr.op-set .rd-val').first()).toHaveText('1');
  await runReviewModal(page, ['updateOne', '"$set"', '"items.0.qty": 7']);

  await runStatement(page, 'db.e2e_json_edit.find({})');
  await page.locator('.view-seg .vs', { hasText: 'Vertical' }).click();
  await expect(fieldRow(page, page.locator('.vrec').first(), 'qty').locator('.vv')).toHaveText('7', { timeout: 20_000 });

  // Cleanup the scratch collection.
  await runStatement(page, 'db.e2e_json_edit.deleteMany({})');
});

test('Vertical: field menu → Delete field parks a $unset', async ({ page }) => {
  test.setTimeout(180_000);
  expect(mongoConnId, 'docker Mongo must seed (127.0.0.1:17017 up?)').not.toBeNull();

  await openConn(page);
  await seedAndOpenVertical(page);
  const rec = page.locator('.vrec').first();

  await fieldRow(page, rec, 'status').click({ button: 'right' });
  await page.locator('.ctx-item', { hasText: /Delete field \(\$unset\)/ }).first().click();
  const dlg = page.getByRole('dialog', { name: 'Delete field' });
  await expect(dlg).toBeVisible();
  await dlg.getByRole('button', { name: 'Delete', exact: true }).click();

  const pendingBar = page.locator('[data-testid="pending-edits-bar"]');
  await expect(pendingBar).toBeVisible();
  await expect(fieldRow(page, rec, 'status')).toHaveClass(/dirty/);
  await pendingBar.locator('.btn.primary', { hasText: 'Review & apply' }).click();
  const modal = page.locator('.review-modal');
  await expect(modal).toBeVisible();
  await expect(modal.locator('.review-diff tr.op-unset .rd-path')).toHaveText('status');
  await runReviewModal(page, ['updateOne', '"$unset"', '"status": ""']);

  await runStatement(page, 'db.e2e_json_edit.find({})');
  await page.locator('.view-seg .vs', { hasText: 'Vertical' }).click();
  // The field is gone from the document — and, columns being inferred from
  // the returned documents, from the result's columns too.
  await expect(page.locator('.vrec').first()).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.vrec').first().locator('.vk', { hasText: /^status$/ })).toHaveCount(0);

  await runStatement(page, 'db.e2e_json_edit.deleteMany({})');
});

test('Vertical: record menu → Insert document… reviews an insertOne', async ({ page }) => {
  test.setTimeout(180_000);
  expect(mongoConnId, 'docker Mongo must seed (127.0.0.1:17017 up?)').not.toBeNull();

  await openConn(page);
  await seedAndOpenVertical(page);
  const rec = page.locator('.vrec').first();

  await rec.locator('[aria-label="Record actions"]').click();
  await page.locator('.ctx-item', { hasText: /Insert document/ }).first().click();
  const docEd = page.locator('.cell-viewer .cv-edit');
  await expect(docEd).toBeVisible();
  await expect(page.getByRole('dialog', { name: 'Insert document' })).toBeVisible();
  await docEd.fill('{ "k": "doc2" }');
  await page.locator('.cell-viewer .btn.primary', { hasText: 'Save' }).click();
  const modal = page.locator('.review-modal');
  await expect(modal).toBeVisible();
  await expect(modal.locator('.review-diff tr.op-set .rd-path')).toHaveText('k');
  await runReviewModal(page, ['insertOne', '"k": "doc2"']);

  await runStatement(page, 'db.e2e_json_edit.find({})');
  await page.locator('.view-seg .vs', { hasText: 'Vertical' }).click();
  await expect(page.locator('.vrec')).toHaveCount(2, { timeout: 20_000 });

  await runStatement(page, 'db.e2e_json_edit.deleteMany({})');
});
