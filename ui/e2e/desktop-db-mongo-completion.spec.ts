import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — MongoDB smart autocompletion (the reported gaps).
//
// Drives the REAL UI against the seeded Docker MongoDB (`shopdb`) and proves the
// query editor's server-backed completion is context-aware for BOTH dialects the
// Mongo runner accepts:
//
//   NATIVE  `db.`                         → the collection list
//           `db.customers.find({ `        → the collection's FIELDS (index-first)
//           `db.customers.aggregate([{ $match: { ` → fields (pipeline stage)
//   SQL     `SELECT * FROM customers WHERE ` → the in-scope collection's FIELDS,
//                                              NOT the collection list (the bug)
//           `SELECT * FROM `              → the collection list
//
// Plus: the Mongo editor highlights as JavaScript (native queries are JS-like),
// asserted via the editor's data-lang hook.
//
// Desktop-browser project only (3-pane layout). Skips cleanly when the Mongo
// container isn't reachable.
// ─────────────────────────────────────────────────────────────────────────────

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
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  test.skip(mongoConnId === null, 'docker MongoDB not reachable on 127.0.0.1:17017');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

// Open the DB page and select the seeded MongoDB connection. Leaves the editor
// ready (capabilities loaded → query_language=mongo).
async function openMongo(page: Page): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const conn = page.locator('.conn-list .conn-name', { hasText: 'e2e-mongodb' });
  await expect(conn.first()).toBeVisible({ timeout: 30_000 });
  await conn.first().click();
  await expect(page.locator('.main-tabs')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.cap-chip', { hasText: 'mongodb' })).toBeVisible({ timeout: 20_000 });
}

// Type `text` into the editor (replacing any prior content), force-open the
// completion popup at the cursor, and return the offered option labels. Pressing
// Ctrl+Space (explicit trigger) works even at an empty-token position like
// `find({ ` or `WHERE `. Closes the popup before returning so calls don't bleed.
async function completionsFor(page: Page, text: string): Promise<string[]> {
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.press('Delete');
  await content.pressSequentially(text, { delay: 6 });
  // Dismiss any popup the typing opened, then force a fresh one at the cursor.
  await page.keyboard.press('Escape');
  await page.keyboard.press('Control+Space');
  const popup = page.locator('.cm-tooltip-autocomplete');
  await expect(popup).toBeVisible({ timeout: 10_000 });
  const labels = await popup.locator('.cm-completionLabel').allInnerTexts();
  await page.keyboard.press('Escape');
  return labels.map((s) => s.trim()).filter(Boolean);
}

test('native `db.` lists collections', async ({ page }) => {
  await openMongo(page);
  const labels = await completionsFor(page, 'db.');
  expect(labels).toContain('customers');
  expect(labels).toContain('orders');
  expect(labels).toContain('profiles');
});

test('native find({ }) offers the collection fields (index-first)', async ({ page }) => {
  await openMongo(page);
  const labels = await completionsFor(page, 'db.customers.find({ ');
  // Real document fields, not the collection list.
  expect(labels).toContain('email');
  expect(labels).toContain('country');
  expect(labels).not.toContain('orders');
});

test('native aggregate $match stage offers the collection fields', async ({ page }) => {
  await openMongo(page);
  const labels = await completionsFor(page, 'db.orders.aggregate([{ $match: { ');
  // orders has indexes on customerId / status — they must be offered as fields.
  expect(labels).toContain('customerId');
  expect(labels).toContain('status');
});

test('SQL `WHERE` offers FIELDS, never the collection list (the bug)', async ({ page }) => {
  await openMongo(page);
  const labels = await completionsFor(page, 'SELECT * FROM customers WHERE ');
  expect(labels).toContain('email');
  expect(labels).toContain('country');
  // The reported regression: collections must NOT appear in a WHERE slot.
  expect(labels).not.toContain('orders');
  expect(labels).not.toContain('products');
});

test('SQL `FROM` offers the collection list', async ({ page }) => {
  await openMongo(page);
  const labels = await completionsFor(page, 'SELECT * FROM ');
  expect(labels).toContain('customers');
  expect(labels).toContain('orders');
});

test('Mongo editor highlights as JavaScript', async ({ page }) => {
  await openMongo(page);
  // Native Mongo queries are JS-like, so the editor uses the JS highlighter.
  await expect(page.locator('.qe-edit .code-editor-outer')).toHaveAttribute('data-lang', 'js');
});
