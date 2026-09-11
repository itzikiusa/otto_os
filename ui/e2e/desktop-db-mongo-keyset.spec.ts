import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — MongoDB keyset pagination over the live Docker Mongo.
//
// An unconstrained `find` is auto-limited to the store's default row cap
// (1,000). The server pages such a find by KEYSET: every page is sorted
// `{_id: 1}` (page 1 included) and a truncated page carries `next_cursor` — the
// last `_id` — which the pager's **Next** echoes back as `cursor` so the server
// filters `{_id: {$gt: cursor}}` instead of `skip`-scanning every row before the
// page. **Prev** stays offset-based (no cursor) and must land on exactly the
// page Next produced, because both paths walk the same forced order.
//
// The wire is observed through `page.route('**/db/query')`: request bodies
// (`cursor` / `offset`) and response bodies (`next_cursor`, every `_id` of every
// page — the grid is windowed, so the DOM only ever holds a slice).
//
// NOTE: this spec passes only once the store sends `cursor` from `runPage`
// (the `database.svelte.ts` handoff of the Mongo-backend work package): until
// then Next pages by offset alone and the `cursor` assertions fail.
//
// Desktop-browser project only. Skips cleanly when the Mongo container is down.
// ─────────────────────────────────────────────────────────────────────────────

const COLL = 'e2e_keyset';
/** Enough documents for three pages at the 1,000-row default cap (2 full + 1 short). */
const DOCS = 2_500;
const PAGE = 1_000;

interface WireResult {
  columns: { name: string }[];
  rows: unknown[][];
  truncated: boolean;
  auto_limited?: number | null;
  next_cursor?: unknown;
}
interface Exchange {
  req: Record<string, unknown>;
  res: WireResult;
}

let workspaceId = '';
let mongoConnId: string | null = null;

/** Seed straight through mongosh, in batches so a single insertMany stays small. */
function seedKeysetDocs(): void {
  const js = [
    'db = db.getSiblingDB("shopdb");',
    `db.${COLL}.drop();`,
    `for (var b = 0; b < ${DOCS}; b += 500) {`,
    '  var docs = [];',
    `  for (var i = b; i < Math.min(b + 500, ${DOCS}); i++) docs.push({ n: i, tag: "k" + i });`,
    `  db.${COLL}.insertMany(docs);`,
    '}',
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
    if (mongoConnId) seedKeysetDocs();
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
    // Pin the page size to the default cap so the fixture paginates 1000/1000/500.
    localStorage.setItem('otto_db_row_limit', String(1000));
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

/** Run a statement and wait for the footer pager (an auto-limited find). */
async function runQuery(page: Page, stmt: string): Promise<void> {
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.waitForTimeout(60);
  await page.keyboard.press('ControlOrMeta+A');
  await page.waitForTimeout(40);
  await page.keyboard.insertText(stmt);
  await page.waitForTimeout(300);
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
  await expect(page.locator('.grid-foot .pager')).toBeVisible({ timeout: 45_000 });
}

/** Record every `/db/query` exchange (request body + parsed response) in order. */
function tapQueries(page: Page): Exchange[] {
  const log: Exchange[] = [];
  void page.route('**/db/query', async (route) => {
    const req = (route.request().postDataJSON() ?? {}) as Record<string, unknown>;
    const resp = await route.fetch();
    const body = await resp.text();
    if (resp.ok()) log.push({ req, res: JSON.parse(body) as WireResult });
    await route.fulfill({ response: resp, body });
  });
  return log;
}

const idsOf = (r: WireResult): string[] => {
  const idx = r.columns.findIndex((c) => c.name === '_id');
  expect(idx, 'result has an _id column').toBeGreaterThanOrEqual(0);
  return r.rows.map((row) => JSON.stringify(row[idx]));
};

/** The `_id` text of the first rendered grid row (the windowed DOM's top). */
async function firstGridId(page: Page): Promise<string> {
  // Mongo results default to the Vertical view; the grid is one segment away and
  // its first row is always inside the virtualization window.
  await page.locator('.view-seg .vs', { hasText: 'Grid' }).click();
  const cell = page.locator('.grid tbody tr:not(.spacer)').first().locator('td.cell').first();
  await expect(cell).toBeVisible({ timeout: 10_000 });
  return (await cell.innerText()).trim();
}

// Range text uses an en-dash and locale thousands separators; accept both dashes,
// with or without the comma.
const RANGE_1 = /rows\s*1[–-]1,?000/;
const RANGE_2 = /rows\s*1,?001[–-]2,?000/;
const RANGE_3 = /rows\s*2,?001[–-]2,?500/;

test('Next pages by keyset cursor, Prev by offset, no duplicate _id across the walk', async ({
  page,
}) => {
  const wire = tapQueries(page);
  await openMongo(page);
  await runQuery(page, `db.${COLL}.find({})`);

  // Page 1: a fresh run carries neither cursor nor offset; the server flags the
  // auto-limit, truncates at the cap, and offers the keyset cursor.
  await expect.poll(() => wire.length).toBe(1);
  const p1 = wire[0];
  expect(p1.req.cursor).toBeUndefined();
  expect(p1.req.offset).toBeUndefined();
  expect(p1.res.auto_limited).toBe(PAGE);
  expect(p1.res.rows).toHaveLength(PAGE);
  expect(p1.res.truncated).toBe(true);
  expect(p1.res.next_cursor, 'page 1 offers a keyset cursor').toBeDefined();
  expect(p1.res.next_cursor).toEqual(JSON.parse(idsOf(p1.res)[PAGE - 1]));
  await expect(page.locator('.pg-range')).toContainText(RANGE_1);
  const top1 = await firstGridId(page);

  // Next → the request echoes page 1's cursor back verbatim (offset may ride
  // along for the range display; the server ignores it when the cursor applies).
  await page.locator('.pg-btn', { hasText: 'Next' }).click();
  await expect.poll(() => wire.length).toBe(2);
  const p2 = wire[1];
  expect(p2.req.cursor).toEqual(p1.res.next_cursor);
  expect(p2.res.rows).toHaveLength(PAGE);
  expect(p2.res.next_cursor, 'page 2 offers a keyset cursor').toBeDefined();
  await expect(page.locator('.pg-range')).toContainText(RANGE_2, { timeout: 10_000 });
  const top2 = await firstGridId(page);
  expect(top2).not.toBe(top1);

  // Next → the short last page: no more rows, so no cursor and Next disables.
  await page.locator('.pg-btn', { hasText: 'Next' }).click();
  await expect.poll(() => wire.length).toBe(3);
  const p3 = wire[2];
  expect(p3.req.cursor).toEqual(p2.res.next_cursor);
  expect(p3.res.rows).toHaveLength(DOCS - 2 * PAGE);
  expect(p3.res.truncated).toBe(false);
  expect(p3.res.next_cursor).toBeUndefined();
  await expect(page.locator('.pg-range')).toContainText(RANGE_3, { timeout: 10_000 });
  await expect(page.locator('.pg-btn', { hasText: 'Next' })).toBeDisabled();

  // The whole walk: every document exactly once.
  const all = [...idsOf(p1.res), ...idsOf(p2.res), ...idsOf(p3.res)];
  expect(new Set(all).size).toBe(DOCS);
  expect(all).toHaveLength(DOCS);

  // Prev → offset-based (no cursor), and — because `{_id: 1}` is forced on every
  // page of the walk — it reproduces page 2 exactly, cursor included.
  await page.locator('.pg-btn', { hasText: 'Prev' }).click();
  await expect.poll(() => wire.length).toBe(4);
  const prev = wire[3];
  expect(prev.req.cursor).toBeUndefined();
  expect(prev.req.offset).toBe(PAGE);
  expect(idsOf(prev.res)).toEqual(idsOf(p2.res));
  expect(prev.res.next_cursor).toEqual(p2.res.next_cursor);
  await expect(page.locator('.pg-range')).toContainText(RANGE_2, { timeout: 10_000 });
  expect(await firstGridId(page)).toBe(top2);
});

test('a find that is not keyset-eligible pages by offset and offers no cursor', async ({ page }) => {
  const wire = tapQueries(page);
  await openMongo(page);
  // A sort on another field cannot walk by `_id` → offset/`skip` path.
  await runQuery(page, `db.${COLL}.find({}).sort({ n: -1 })`);
  await expect.poll(() => wire.length).toBe(1);
  expect(wire[0].res.next_cursor).toBeUndefined();
  expect(wire[0].res.auto_limited).toBe(PAGE);

  await page.locator('.pg-btn', { hasText: 'Next' }).click();
  await expect.poll(() => wire.length).toBe(2);
  expect(wire[1].req.cursor).toBeUndefined();
  expect(wire[1].req.offset).toBe(PAGE);
  expect(wire[1].res.rows).toHaveLength(PAGE);
  await expect(page.locator('.pg-range')).toContainText(RANGE_2, { timeout: 10_000 });
  // Descending `n` ⇒ page 2 starts at n = 1499.
  const nIdx = wire[1].res.columns.findIndex((c) => c.name === 'n');
  expect(wire[1].res.rows[0][nIdx]).toBe(DOCS - PAGE - 1);
});
