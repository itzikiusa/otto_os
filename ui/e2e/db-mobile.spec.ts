import { test, expect, type Page } from '@playwright/test';
import type { ChildProcess } from 'node:child_process';
import { apiCtx, seedWorkspace, seedRedis } from './seed';

// Database page — PHONE usability.
//
// The desktop DB Explorer crams the connection tree + schema + query tabs +
// toolbar + editor + results into ONE fixed viewport height; on a phone that's
// unreadable, doesn't scroll, and the RESULTS are unreachable. The mobile
// (≤640px) layout turns the page into a scrollable single column whose major
// sections (Connections / Schema / Editor / Results) are collapsible accordions
// that scroll independently when expanded.
//
// These specs exercise REAL behavior against a seeded redis connection:
//   • the page fits the viewport width (no element overflows horizontally),
//   • the page scrolls vertically when content overflows,
//   • picking a connection → schema → running a query → results grid shows rows,
//   • the section accordions collapse/expand,
//   • the results block scrolls internally when its rows overflow.
//
// Run with --workers=1 (single shared redis on a fixed port).

let workspaceId = '';
let redisConnId: string | null = null;
let redisProc: ChildProcess | null = null;

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  try {
    const r = await seedRedis(ctx, base, workspaceId);
    if (r) {
      redisProc = r.proc;
      redisConnId = r.connId;
    }
  } catch {
    redisConnId = null;
  }
  await ctx.dispose();
});

test.afterAll(() => {
  try {
    redisProc?.kill('SIGKILL');
  } catch {
    /* already gone */
  }
});

// Activate the seeded workspace (so connections load) and close the nav drawer
// (which defaults open on a fresh phone profile and would cover the page).
test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

// Drive: open the DB page, pick the seeded redis connection, run `KEYS *`,
// and wait for the results grid to populate. Returns once rows are present.
async function openAndQuery(page: Page): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });

  const conn = page.locator('.conn-list .conn-name', { hasText: 'e2e-redis' });
  await expect(conn.first()).toBeVisible({ timeout: 30_000 });
  await conn.first().click();

  await expect(page.locator('.main-tabs')).toBeVisible({ timeout: 20_000 });

  const editor = page.locator('.qe-edit .cm-content');
  await editor.click();
  await page.keyboard.type('KEYS *');
  await page.getByRole('button', { name: /Run/ }).first().click();

  // The grid renders ≥1 data row once the query returns.
  await expect(page.locator('.grid tbody tr:not(.spacer)').first()).toBeVisible({
    timeout: 20_000,
  });
}

// True if any element under .db-page extends past the document's client width
// (i.e. the page is wider than the viewport → horizontal overflow).
async function maxRight(page: Page): Promise<{ vw: number; widest: number; docScrollW: number }> {
  return page.evaluate(() => {
    const de = document.documentElement;
    let widest = 0;
    document.querySelectorAll<HTMLElement>('.db-page *').forEach((el) => {
      const r = el.getBoundingClientRect();
      if (r.right > widest) widest = r.right;
    });
    return { vw: de.clientWidth, widest: Math.round(widest), docScrollW: de.scrollWidth };
  });
}

test.describe('database mobile (phone)', () => {
  test.skip(({ viewport }) => (viewport?.width ?? 0) > 640, 'phone-only layout');

  test('content fits the viewport width — no horizontal overflow', async ({ page }) => {
    test.skip(redisConnId == null, 'redis-server unavailable');
    await openAndQuery(page);

    const { vw, widest, docScrollW } = await maxRight(page);
    // The document itself must not scroll horizontally.
    expect(docScrollW).toBeLessThanOrEqual(vw + 1);
    // No element should jut past the viewport (allow 2px for sub-pixel rounding).
    expect(widest).toBeLessThanOrEqual(vw + 2);
  });

  test('the page scrolls vertically when content overflows', async ({ page }) => {
    test.skip(redisConnId == null, 'redis-server unavailable');
    await openAndQuery(page);

    const scroll = await page.locator('.db-page').evaluate((el) => ({
      clientH: el.clientHeight,
      scrollH: el.scrollHeight,
    }));
    // With a connection + query open the stacked sections overflow the viewport,
    // and .db-page is the (vertical) scroll container.
    expect(scroll.scrollH).toBeGreaterThan(scroll.clientH + 20);
  });

  test('running a query shows result rows in the grid', async ({ page }) => {
    test.skip(redisConnId == null, 'redis-server unavailable');
    await openAndQuery(page);

    const rows = await page.locator('.grid tbody tr:not(.spacer)').count();
    expect(rows).toBeGreaterThanOrEqual(1);
  });

  test('sections are collapsible accordions', async ({ page }) => {
    test.skip(redisConnId == null, 'redis-server unavailable');
    await openAndQuery(page);

    // Editor is visible while expanded; collapsing its accordion hides it.
    await expect(page.locator('.qe-edit')).toBeVisible();
    await page.locator('.qe-acc-head', { hasText: 'Editor' }).click();
    await expect(page.locator('.qe-edit')).toBeHidden();
    // Expand again.
    await page.locator('.qe-acc-head', { hasText: 'Editor' }).click();
    await expect(page.locator('.qe-edit')).toBeVisible();

    // The connection list collapses too.
    await expect(page.locator('.conn-list')).toBeVisible();
    await page.locator('.acc-toggle', { hasText: 'Connections' }).click();
    await expect(page.locator('.conn-list')).toBeHidden();
  });

  test('the results block scrolls internally when rows overflow', async ({ page }) => {
    test.skip(redisConnId == null, 'redis-server unavailable');
    await openAndQuery(page);

    // Collapse the editor so the results block gets the room, then confirm its
    // scroll container caps (clientH) below its content (scrollH).
    await page.locator('.qe-acc-head', { hasText: 'Editor' }).click();
    await page.locator('.grid-scroll').scrollIntoViewIfNeeded();
    const info = await page.locator('.grid-scroll').evaluate((el) => ({
      clientH: el.clientHeight,
      scrollH: el.scrollHeight,
    }));
    // The seeded redis has 40 keys → the grid overflows its bounded block.
    expect(info.scrollH).toBeGreaterThan(info.clientH + 20);
  });
});
