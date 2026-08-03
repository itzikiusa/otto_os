import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — MongoDB with FAT documents (the reported break).
//
// Reproduces the real shape that killed the UI: `lobby_api.lobby_format_history`,
// where ONE document is ~88KB (a huge embedded `structure` array) and the fields
// worth indexing live in a small embedded subdocument next to it. 100 such rows
// is ~9MB, and the non-grid views used to render every branch of every document
// eagerly — millions of DOM nodes, browser unresponsive.
//
// Guards three things, each of which was broken:
//   1. Structure tab exposes EMBEDDED field paths (`meta.brand_id`) as a Fields
//      table. A collection has no `columns`, so the tab previously showed no
//      field list at all — nested paths were undiscoverable.
//   2. One click from that table seeds the index builder with the nested path.
//   3. JSON + Vertical views stay BOUNDED on fat documents: batched records,
//      collapsed branches, and a DOM that stays small enough to stay responsive.
//
// Desktop-browser project only. Skips cleanly when the Mongo container is down.
// ─────────────────────────────────────────────────────────────────────────────

const COLL = 'fatdocs';
/** Ceiling for total DOM nodes with a fat result on screen. Normal app chrome is
 *  ~2k; the pre-fix eager render was six figures, so this catches a regression
 *  without being brittle about exact markup. */
const MAX_DOM_NODES = 12_000;

let workspaceId = '';
let mongoConnId: string | null = null;

/** Seed fat documents straight through mongosh — deterministic, and independent
 *  of Otto's own write path (which is what these specs are testing). */
function seedFatDocs(): void {
  const js = [
    'db = db.getSiblingDB("shopdb");',
    `db.${COLL}.drop();`,
    'var items = [];',
    'for (var i = 0; i < 300; i++) {',
    '  items.push({ name: "cat-" + i, category_id: "c" + i, icon: null,',
    '               games: ["g1","g2","g3"], spec: { provider: "p" + i, live: true } });',
    '}',
    'var docs = [];',
    'for (var d = 0; d < 60; d++) {',
    '  docs.push({ blob: { structure: items },',
    '              meta: { brand_id: 1000 + (d % 2), type: "lobby.guest",',
    '                      revision: d, whenUpdated: new Date(1700000000000 - d * 86400000) } });',
    '}',
    `db.${COLL}.insertMany(docs);`,
  ].join('\n');
  execFileSync(
    'docker',
    ['exec', 'otto-dbv-mongo', 'mongosh', '-u', 'otto', '-p', 'ottopw', '--quiet', '--eval', js],
    { stdio: 'pipe', timeout: 60_000 },
  );
}

test.beforeAll(async () => {
  test.setTimeout(180_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  try {
    mongoConnId = await seedDockerConnection(ctx, base, workspaceId, 'mongodb');
    if (mongoConnId) seedFatDocs();
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

async function openMongo(page: Page): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const conn = page.locator('.conn-list .conn-name', { hasText: 'e2e-mongodb' });
  await expect(conn.first()).toBeVisible({ timeout: 30_000 });
  await conn.first().click();
  await expect(page.locator('.cap-chip', { hasText: 'mongodb' })).toBeVisible({ timeout: 20_000 });
}

/** Open the fat collection's structure tab (expanding tree nodes as needed). */
async function openFatStructure(page: Page): Promise<void> {
  await openMongo(page);
  const exact = new RegExp(`^${COLL}$`);
  const lbl = page
    .locator('.node')
    .filter({ has: page.locator('.node-icon.collection') })
    .filter({ has: page.locator('.nl-text').filter({ hasText: exact }) })
    .locator('.node-label')
    .first();
  for (let attempt = 0; attempt < 3; attempt++) {
    if (await lbl.isVisible().catch(() => false)) break;
    const carets = page.locator('.node .caret');
    const n = await carets.count();
    for (let i = 0; i < n; i++) {
      await carets.nth(i).click().catch(() => {});
      await page.waitForTimeout(110);
    }
    await page.waitForTimeout(600);
  }
  await expect(lbl).toBeVisible({ timeout: 15_000 });
  await lbl.click();
}

/** Run a statement and wait for the results pane. */
async function runQuery(page: Page, stmt: string): Promise<void> {
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.press('Delete');
  await page.keyboard.insertText(stmt);
  await page.waitForTimeout(300);
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
  await expect(page.locator('.grid tbody tr').first()).toBeVisible({ timeout: 45_000 });
}

const domNodes = (page: Page) => page.evaluate(() => document.querySelectorAll('*').length);

// ── 1. Embedded paths are discoverable ──────────────────────────────────────

test('Structure tab lists EMBEDDED field paths for a collection', async ({ page }) => {
  await openFatStructure(page);
  const fields = page.locator('.block', { has: page.locator('.block-title', { hasText: 'Fields' }) });
  await expect(fields).toBeVisible({ timeout: 20_000 });
  // The reported gap: the nested metadata paths, not just the top-level objects.
  await expect(fields.locator('td.cn', { hasText: 'meta.brand_id' })).toBeVisible();
  await expect(fields.locator('td.cn', { hasText: 'meta.whenUpdated' })).toBeVisible();
  // …and they're marked as embedded rather than looking like top-level fields.
  await expect(
    fields.locator('tr', { has: page.locator('td.cn', { hasText: 'meta.brand_id' }) }).locator('.nested-tag'),
  ).toBeVisible();
});

test('Fields table indexes a nested path in one click', async ({ page }) => {
  await openFatStructure(page);
  const fields = page.locator('.block', { has: page.locator('.block-title', { hasText: 'Fields' }) });
  await fields
    .locator('tr', { has: page.locator('td.cn', { hasText: 'meta.brand_id' }) })
    .getByRole('button', { name: /Index/ })
    .click();
  // Builder opens with the nested path already selected as key #1.
  const selected = page.locator('.ib-row.on');
  await expect(selected).toHaveCount(1);
  await expect(selected.locator('.ib-fname')).toHaveText('meta.brand_id');
});

test('index builder can name a path the sampler never saw', async ({ page }) => {
  await openFatStructure(page);
  await page.getByRole('button', { name: /New index/ }).first().click();
  // A path absent from every sampled document — previously unreachable.
  await page.locator('.idx-builder .ib-search').fill('meta.absent.deep');
  const custom = page.locator('.ib-row.custom');
  await expect(custom).toBeVisible();
  await custom.click();
  await expect(page.locator('.ib-row.on .ib-fname')).toHaveText('meta.absent.deep');
});

test('index builder can set a DESCENDING key direction', async ({ page }) => {
  await openFatStructure(page);
  const fields = page.locator('.block', { has: page.locator('.block-title', { hasText: 'Fields' }) });
  await fields
    .locator('tr', { has: page.locator('td.cn', { hasText: 'meta.whenUpdated' }) })
    .getByRole('button', { name: /Index/ })
    .click();
  const dir = page.locator('.ib-row.on .ib-dir');
  await expect(dir).toHaveText(/↑ 1/);
  await dir.click();
  await expect(dir).toHaveText(/↓ -1/);
});

// ── 2. Fat results stay renderable ──────────────────────────────────────────

test('JSON view stays bounded on fat documents', async ({ page }) => {
  await openMongo(page);
  await runQuery(page, `db.${COLL}.find({})`);
  await page.locator('.vs', { hasText: 'JSON' }).click();

  const recs = page.locator('.jrec');
  await expect(recs.first()).toBeVisible({ timeout: 20_000 });
  // Batched, not "everything at once".
  expect(await recs.count()).toBeLessThanOrEqual(25);
  await expect(page.locator('.alt-more')).toBeVisible();

  // The fat branch renders as a COLLAPSED summary, not 300 expanded entries.
  const first = recs.first();
  await expect(first.getByText('blob', { exact: true })).toBeVisible();
  await expect(first.locator('.sum').first()).toBeVisible();
  await expect(first.getByText('cat-299')).toHaveCount(0);

  expect(await domNodes(page)).toBeLessThan(MAX_DOM_NODES);
});

test('JSON view expands a collapsed branch on demand', async ({ page }) => {
  await openMongo(page);
  await runQuery(page, `db.${COLL}.find({})`);
  await page.locator('.vs', { hasText: 'JSON' }).click();
  const first = page.locator('.jrec').first();
  await expect(first).toBeVisible({ timeout: 20_000 });

  // `meta` is small, so it auto-expands and its values are readable inline —
  // collapsing must not hide the fields the user actually came for.
  await expect(first.getByText('brand_id', { exact: true })).toBeVisible();

  // `blob` holds a single key so it auto-opens; the 300-element array beneath it
  // is what stays shut, showing only its size.
  const structure = first.locator('.toggle', { hasText: 'structure' });
  await expect(structure).toBeVisible();
  await expect(structure).toContainText('300 items');

  // Opening it is opt-in AND still chunked — not all 300 at once. The elements
  // themselves stay collapsed too (they're deeper than the auto-expand depth), so
  // opening a big array can't cascade into rendering its whole subtree.
  await structure.click();
  const more = first.locator('.more').first();
  await expect(more).toContainText(/show 50 more · 250 hidden/);
  await expect(first.getByText('cat-299', { exact: true })).toHaveCount(0);

  // The chunk grows only when asked.
  await more.click();
  await expect(first.locator('.more').first()).toContainText(/200 hidden/);
  expect(await domNodes(page)).toBeLessThan(MAX_DOM_NODES);
});

test('Vertical view stays bounded on fat documents', async ({ page }) => {
  await openMongo(page);
  await runQuery(page, `db.${COLL}.find({})`);
  await page.locator('.vs', { hasText: 'Vertical' }).click();

  const recs = page.locator('.vrec');
  await expect(recs.first()).toBeVisible({ timeout: 20_000 });
  expect(await recs.count()).toBeLessThanOrEqual(25);

  // The fat column is a collapsed tree, NOT a stringified 50KB blob.
  await expect(recs.first().locator('.vv.tree').first()).toBeVisible();
  await expect(recs.first().getByText('cat-299')).toHaveCount(0);

  expect(await domNodes(page)).toBeLessThan(MAX_DOM_NODES);
});

test('grid cells clip blob text instead of shipping it whole', async ({ page }) => {
  await openMongo(page);
  await runQuery(page, `db.${COLL}.find({})`);
  // The BLOB cell specifically (`td.cell.json` is the complex-value cell) — a
  // ~50KB embedded document must not land in the DOM verbatim. Targeting
  // `td.cell` generally would match the short `_id` and prove nothing.
  const cell = page.locator('.grid tbody tr').first().locator('td.cell.json').first();
  await expect(cell).toBeVisible();
  const len = await cell.evaluate((el) => (el.textContent ?? '').length);
  expect(len).toBeGreaterThan(0);
  expect(len).toBeLessThan(2_000);
  expect(await domNodes(page)).toBeLessThan(MAX_DOM_NODES);
});
