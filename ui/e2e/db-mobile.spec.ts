import { test, expect, type Page } from '@playwright/test';
import type { ChildProcess } from 'node:child_process';
import { createServer } from 'node:net';
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
// Each worker owns a Redis process/port and selects its connection by ID: the
// connection list is global, including profiles left by other specs.

let workspaceId = '';
let redisConnId: string | null = null;
let redisProc: ChildProcess | null = null;

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  try {
    const reservation = createServer();
    await new Promise<void>((resolve, reject) => {
      reservation.once('error', reject);
      reservation.listen(0, '127.0.0.1', resolve);
    });
    const address = reservation.address();
    if (!address || typeof address === 'string') throw new Error('No Redis fixture port');
    await new Promise<void>((resolve, reject) => reservation.close((e) => e ? reject(e) : resolve()));
    const r = await seedRedis(ctx, base, workspaceId, address.port);
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

  const conn = page.locator(`.conn-row[data-connection-id="${redisConnId}"] .tr-title`);
  await expect(conn).toBeVisible({ timeout: 30_000 });
  await conn.click();

  await expect(page.getByTestId('db-conn-state')).toBeVisible({ timeout: 20_000 });

  const editor = page.locator('.qe-edit .cm-content');
  await editor.click();
  await page.keyboard.type('KEYS *');
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();

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

  test('the rows fill the screen: the page stays put, the grid scrolls', async ({ page }) => {
    test.skip(redisConnId == null, 'redis-server unavailable');
    await openAndQuery(page);

    // The workbench fills the height under the header and does not scroll as
    // a document; the results reach the bottom of it (the grid scrolls inside).
    const m = await page.evaluate(() => {
      const box = (s: string) => document.querySelector(s)!.getBoundingClientRect();
      const pageEl = document.querySelector('.db-page')!;
      return { over: pageEl.scrollHeight - pageEl.clientHeight, gap: box('.db-main').bottom - box('.qe-results').bottom };
    });
    expect(m.over).toBeLessThanOrEqual(1);
    expect(m.gap).toBeLessThan(40);
  });

  // The sticky row numbers cover what scrolls under them: a striped row's
  // number cell was a 7 % tint, so a wide value read through it.
  test('a striped row\'s number stays opaque over the cells scrolling under it', async ({ page }) => {
    test.skip(redisConnId == null, 'redis-server unavailable');
    await openAndQuery(page);
    const alphas = await page.locator('.grid tbody tr:not(.spacer) td.rownum').evaluateAll((cells) =>
      cells.slice(0, 4).map((c) => {
        const bg = getComputedStyle(c).backgroundColor;
        const a = /\/\s*([\d.]+)\)|rgba\([^)]*,\s*([\d.]+)\)/.exec(bg);
        return { bg, image: getComputedStyle(c).backgroundImage, alpha: a ? Number(a[1] ?? a[2]) : 1 };
      }),
    );
    for (const a of alphas) expect(a.alpha === 1 || a.image !== 'none', JSON.stringify(a)).toBe(true);
  });

  test('running a query shows result rows in the grid', async ({ page }) => {
    test.skip(redisConnId == null, 'redis-server unavailable');
    await openAndQuery(page);

    const rows = await page.locator('.grid tbody tr:not(.spacer)').count();
    expect(rows).toBeGreaterThanOrEqual(1);
  });

  test('after a run the editor folds to one line and toggles back', async ({ page }) => {
    test.skip(redisConnId == null, 'redis-server unavailable');
    await openAndQuery(page);
    // The rows get the screen: the editor folded to its head on Run.
    await expect(page.locator('.qe-edit')).toBeHidden();
    await page.getByRole('button', { name: 'Edit the query' }).click();
    await expect(page.locator('.qe-edit')).toBeVisible();
    await page.getByRole('button', { name: 'Fold the editor' }).click();
    await expect(page.locator('.qe-edit')).toBeHidden();
  });

  test('the results block scrolls internally when rows overflow', async ({ page }) => {
    test.skip(redisConnId == null, 'redis-server unavailable');
    await openAndQuery(page);

    // The editor folded on Run; confirm the results' scroll container caps
    // (clientH) below its content (scrollH).
    await page.locator('.grid-scroll').scrollIntoViewIfNeeded();
    const info = await page.locator('.grid-scroll').evaluate((el) => ({
      clientH: el.clientHeight,
      scrollH: el.scrollHeight,
    }));
    // The seeded redis has 40 keys → the grid overflows its bounded block.
    expect(info.scrollH).toBeGreaterThan(info.clientH + 20);
  });
});
