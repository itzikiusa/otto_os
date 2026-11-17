import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — editing MongoDB documents over the live Docker Mongo.
//
// The documents carry STRING `_id`s with spaces and dots (the reported
// collection's shape), booleans and nulls. `_id` is immutable in MongoDB, so a
// changed `_id` used to be dropped from the document editor's update (Save did
// nothing, silently) and the grid's `_id` cell would not edit at all. Both now
// review a rename: `$merge` copies the stored document under the new `_id`,
// then the old one is deleted. A write that matches no document is an error,
// not "Applied". ObjectId / numeric `_id`s and typed grid literals ride along.
//
// Desktop-browser project only. Skips cleanly when the Mongo container is down.
// ─────────────────────────────────────────────────────────────────────────────

const SLEIPNIR = 'SLEIPNIR INTERACTIVE d.o.o';
let workspaceId = '';
let mongoConnId: string | null = null;

function mongosh(js: string): string {
  return execFileSync(
    'docker',
    ['exec', 'otto-dbv-mongo', 'mongosh', '-u', 'otto', '-p', 'ottopw', '--quiet', '--eval', `db = db.getSiblingDB("shopdb"); ${js}`],
    { stdio: 'pipe', timeout: 60_000 },
  ).toString();
}
/** A collection's documents, `_id`-sorted, as relaxed Extended JSON. */
function docs(coll: string): Record<string, unknown>[] {
  return JSON.parse(mongosh(`print(EJSON.stringify(db.${coll}.find().sort({_id: 1}).toArray()))`)) as Record<string, unknown>[];
}

const PROVIDERS = [
  { _id: SLEIPNIR, checkAdditionalFields: true, supportOnlySinglePlayer: false, supportOnlySingleGame: null },
  { _id: 'SoftSwiss', checkAdditionalFields: false, supportOnlySinglePlayer: true, supportOnlySingleGame: null },
];
function seed(): void {
  mongosh(`
    db.e2e_edit_str.drop(); db.e2e_edit_str.insertMany(${JSON.stringify(PROVIDERS)});
    db.e2e_edit_oid.drop(); db.e2e_edit_oid.insertMany([{name: "a", flag: true}, {name: "b", flag: false}]);
    db.e2e_edit_num.drop(); db.e2e_edit_num.insertMany([{_id: 1, name: "one", flag: true}, {_id: 2, name: "two", flag: null}]);
    db.e2e_edit_nested.drop(); db.e2e_edit_nested.insertMany([{_id: "n1", name: "x", meta: {a: 1, b: {c: true}}, tags: ["t1"]}]);
  `);
}

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  test.setTimeout(180_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  try {
    mongoConnId = await seedDockerConnection(ctx, base, workspaceId, 'mongodb');
  } catch {
    mongoConnId = null;
  }
  await ctx.dispose().catch(() => {});
});

test.afterAll(() => {
  if (mongoConnId) mongosh('["str", "oid", "num", "nested"].forEach((c) => db["e2e_edit_" + c].drop());');
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  test.skip(mongoConnId === null, 'docker MongoDB not reachable on 127.0.0.1:17017');
  seed();
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

async function openFind(page: Page, coll: string): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const conn = page.locator('.conn-list .conn-name', { hasText: 'e2e-mongodb' });
  await expect(conn.first()).toBeVisible({ timeout: 30_000 });
  await conn.first().click();
  await expect(page.locator('.cap-chip', { hasText: 'mongodb' })).toBeAttached({ timeout: 20_000 });
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText(`db.${coll}.find({})`);
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
  // Mongo results open in the Vertical (document) view.
  await expect(page.locator('.vrec').first()).toBeVisible({ timeout: 30_000 });
}

/** Run the open review modal and wait for it to close (a successful apply). */
async function applyReview(page: Page, title: RegExp): Promise<void> {
  const sheet = page.locator('.sheet').filter({ hasText: title });
  await expect(sheet).toBeVisible();
  await sheet.locator('.btn.primary').last().click();
  await expect(sheet).toBeHidden({ timeout: 15_000 });
}

async function editDocument(page: Page, record: number, doc: Record<string, unknown>): Promise<void> {
  await page.locator('.vrec').nth(record).getByRole('button', { name: 'Edit record' }).click();
  const editor = page.locator('.sheet').filter({ hasText: 'Edit document' }).locator('.cm-content');
  await expect(editor).toBeVisible();
  await editor.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.insertText(JSON.stringify(doc, null, 2));
  await page.locator('.sheet .btn.primary', { hasText: 'Save' }).click();
}

const gridRows = (page: Page) => page.locator('.grid tbody tr:not(.spacer)');
async function toGrid(page: Page): Promise<void> {
  await page.locator('.view-seg .vs', { hasText: 'Grid' }).click();
  await expect(gridRows(page).first()).toBeVisible({ timeout: 10_000 });
}
async function editCell(page: Page, row: number, col: number, value: string): Promise<void> {
  await gridRows(page).nth(row).locator('td.cell').nth(col).dblclick();
  const input = page.locator('.grid input.cell-input');
  await expect(input).toBeVisible();
  await input.fill(value);
  await input.press('Enter');
  await expect(input).toBeHidden();
}

test('document view: changing a string _id (spaces, dots) renames the document instead of doing nothing', async ({ page }) => {
  await openFind(page, 'e2e_edit_str');
  await editDocument(page, 0, { ...PROVIDERS[0], _id: `${SLEIPNIR}.`, supportOnlySinglePlayer: true });
  await applyReview(page, /Review _id change/);
  await expect.poll(() => docs('e2e_edit_str').map((d) => d._id)).toEqual([`${SLEIPNIR}.`, 'SoftSwiss']);
  expect(docs('e2e_edit_str')[0]).toEqual({
    _id: `${SLEIPNIR}.`, checkAdditionalFields: true, supportOnlySinglePlayer: true, supportOnlySingleGame: null,
  });
});

test('document view: a field save on a string _id writes booleans and null with their types', async ({ page }) => {
  await openFind(page, 'e2e_edit_str');
  await editDocument(page, 1, { ...PROVIDERS[1], checkAdditionalFields: true, supportOnlySingleGame: false });
  await applyReview(page, /Review updateOne/);
  await expect.poll(() => docs('e2e_edit_str')[1]).toEqual({ ...PROVIDERS[1], checkAdditionalFields: true, supportOnlySingleGame: false });
});

test('document view: a save whose document is gone is an error naming the _id, not "Applied"', async ({ page }) => {
  await openFind(page, 'e2e_edit_str');
  mongosh(`db.e2e_edit_str.deleteOne({_id: "SoftSwiss"})`);
  await editDocument(page, 1, { ...PROVIDERS[1], checkAdditionalFields: true });
  const sheet = page.locator('.sheet').filter({ hasText: /Review updateOne/ });
  await sheet.locator('.btn.primary').last().click();
  await expect(sheet).toContainText('No document matched _id "SoftSwiss"; it may have been changed or deleted.');
  await expect(page.locator('.toast', { hasText: 'Did not match any row' })).toBeVisible();
});

test('grid: the _id cell edits as a rename, and a null field typed true becomes a boolean', async ({ page }) => {
  await openFind(page, 'e2e_edit_str');
  await toGrid(page);
  // Row 1 = SoftSwiss; columns: _id, checkAdditionalFields, supportOnlySinglePlayer, supportOnlySingleGame.
  await editCell(page, 1, 0, 'SoftSwiss Ltd.');
  await editCell(page, 1, 3, 'true');
  await page.locator('.btn.small.primary', { hasText: 'Review & apply' }).click();
  await applyReview(page, /Review updateOne/);
  await expect.poll(() => docs('e2e_edit_str').map((d) => d._id)).toEqual([SLEIPNIR, 'SoftSwiss Ltd.']);
  expect(docs('e2e_edit_str')[1]).toEqual({ ...PROVIDERS[1], _id: 'SoftSwiss Ltd.', supportOnlySingleGame: true });
});

test('grid: an ObjectId _id collection edits two rows in one review', async ({ page }) => {
  await openFind(page, 'e2e_edit_oid');
  await toGrid(page);
  const before = docs('e2e_edit_oid');
  // Two rows in ONE review: a `;`-separated batch (a bare newline used to
  // reach mongosh as one expression and fail with a syntax error).
  await editCell(page, 0, 2, 'false'); // _id, name, flag
  await editCell(page, 1, 1, 'b2');
  await page.locator('.btn.small.primary', { hasText: 'Review & apply' }).click();
  await applyReview(page, /Review updateOne/);
  await expect.poll(() => docs('e2e_edit_oid')).toEqual([{ ...before[0], flag: false }, { ...before[1], name: 'b2' }]);
});

test('grid: a numeric _id renames as a number', async ({ page }) => {
  await openFind(page, 'e2e_edit_num');
  await toGrid(page);
  await editCell(page, 1, 0, '20');
  await editCell(page, 1, 2, 'true');
  await page.locator('.btn.small.primary', { hasText: 'Review & apply' }).click();
  await applyReview(page, /Review updateOne/);
  await expect.poll(() => docs('e2e_edit_num')).toEqual([
    { _id: 1, name: 'one', flag: true },
    { _id: 20, name: 'two', flag: true },
  ]);
});

test('grid: nested objects open in the value viewer, scalar siblings still edit inline', async ({ page }) => {
  await openFind(page, 'e2e_edit_nested');
  await toGrid(page);
  await editCell(page, 0, 1, 'y'); // _id, name, meta, tags
  await page.locator('.btn.small.primary', { hasText: 'Review & apply' }).click();
  await applyReview(page, /Review updateOne/);
  await expect.poll(() => docs('e2e_edit_nested')[0]).toEqual({ _id: 'n1', name: 'y', meta: { a: 1, b: { c: true } }, tags: ['t1'] });
});
