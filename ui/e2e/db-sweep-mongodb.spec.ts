import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';

// ── DB Explorer sweep — MongoDB engine, mobile + tablet, both orientations ──────
//
// Proves the Database Explorer is fully VISIBLE and USABLE for a live MongoDB
// connection on a phone AND a tablet, in BOTH portrait and landscape, by running
// real read (find) and write (insertOne / updateOne / deleteMany) queries against
// the seeded Docker MongoDB (`shopdb`, authSource admin) and asserting the results
// render in a grid that scrolls both directions.
//
// LAYOUT NOTE — the page has two distinct layouts keyed off width (≤640 = phone):
//   • iphone-portrait (430) and iphone-se (320)  → PHONE layout: the page scrolls
//     vertically and the Connections / Schema / Editor / Results sections are
//     collapsible accordions that scroll independently.
//   • iphone-landscape (814), ipad-portrait (834), ipad-landscape (1194)
//                                                  → WIDE layout: the sidebar +
//     main area sit side-by-side at full height (no page scroll, no accordions);
//     the results grid scrolls internally.
// Both layouts are exercised here (we branch on width, never skip by it).
//
// The 7 checks, per project/orientation:
//   1. seed workspace + mongodb connection (beforeAll); workspace active (beforeEach)
//   2. open #/database and pick `e2e-mongodb`
//   3. READ: db.customers.find({}) → ≥1 document row in the grid
//   4. WRITE: insertOne → updateOne → find shows "updated" → cleanup (deleteMany)
//   5. no horizontal overflow (document scrollWidth ≤ vw+2; nothing under .db-page juts past)
//   6. grid scrolls horizontally (wide docs) AND vertically (many docs)
//   7. schema tree (databases → collections) visible + usable

let workspaceId = '';
let mongoConnId: string | null = null;

// The first /test against Mongo does topology discovery, so the seed can be slow
// under load — give beforeAll a generous budget.
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

// Activate the seeded workspace + close the nav drawer (defaults open on a fresh
// phone profile and would cover the page) before each test.
test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

// The connection must seed + /test green — a null is a real bug (driver
// connect/authSource handling), never a silent skip.
function requireConn(): void {
  expect(
    mongoConnId,
    'seedDockerConnection(mongodb) returned null — the Mongo driver could not connect/test ' +
      'against 127.0.0.1:17017 (check authSource=admin handling)',
  ).not.toBeNull();
}

const isPhone = (page: Page): boolean => (page.viewportSize()?.width ?? 0) <= 640;

// Open the DB page and select the seeded MongoDB connection. Leaves the page on
// the Query tab with the connection's capabilities + schema loaded.
async function openConn(page: Page): Promise<void> {
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });

  // On a phone the Connections accordion is open by default; on wide layouts the
  // conn list is always visible. Either way the row is in `.conn-list`.
  const conn = page.locator('.conn-list .conn-name', { hasText: 'e2e-mongodb' });
  await expect(conn.first()).toBeVisible({ timeout: 30_000 });
  await conn.first().click();

  // The main tab strip appears once a connection is open.
  await expect(page.locator('.main-tabs')).toBeVisible({ timeout: 20_000 });
  // Engine chip confirms the driver loaded its capabilities (query_language=mongo).
  await expect(page.locator('.cap-chip', { hasText: 'mongodb' })).toBeVisible({ timeout: 20_000 });
}

// On a phone the Editor block is an accordion — make sure it's expanded so the
// CodeMirror surface is interactable. No-op on wide layouts (always visible).
async function ensureEditorOpen(page: Page): Promise<void> {
  if (!isPhone(page)) return;
  const editHead = page.locator('.qe-acc-head', { hasText: 'Editor' });
  const editor = page.locator('.qe-edit');
  if (await editor.isHidden().catch(() => false)) {
    await editHead.click();
  }
  await expect(editor).toBeVisible();
}

async function ensureResultsOpen(page: Page): Promise<void> {
  if (!isPhone(page)) return;
  const resHead = page.locator('.qe-acc-head', { hasText: 'Results' });
  // The Results block hosts the grid/empty/error; expand it if collapsed.
  const collapsed = await page.locator('.qe-results.qe-collapsed').count();
  if (collapsed > 0) await resHead.click();
}

// Put a statement into the CodeMirror editor, replacing any prior text.
//
// Two CodeMirror quirks drive this approach:
//   • `keyboard.type` (char-by-char) lets the debounced, server-backed
//     autocomplete accept a collection suggestion mid-string and corrupt a dotted
//     Mongo command (e.g. `db.customers.find(...)` → `unsupported method
//     'customers.find'`). `insertText` injects the whole string in ONE input
//     event — no per-key completion sequence — and is also fast enough for the
//     large insertMany the scroll test uses.
//   • the editor's value propagates to the store asynchronously, so clicking Run
//     immediately after insertText reads the OLD (cleared) statement and no-ops.
// So: focus, clear, insertText, then WAIT (via a polling assertion) for the
// editor to actually hold the statement before returning.
async function setEditorText(page: Page, statement: string): Promise<void> {
  const content = page.locator('.qe-edit .cm-content');
  // Click to focus, select-all, then insertText — which REPLACES the selection in
  // one input event. This is the robust path:
  //   • select-all + insertText (no separate Delete keystroke — Delete races the
  //     contenteditable focus and intermittently swallows the following insert);
  //   • insertText injects the whole string at once, so the debounced server
  //     autocomplete can't accept a suggestion mid-string and corrupt a dotted
  //     Mongo command, and it's fast enough for the large insertMany;
  //   • the final settle lets the value propagate to the store before Run reads it
  //     (clicking too early would read the prior statement).
  // We deliberately do NOT read back .cm-content to verify — Playwright's text
  // getters return empty for this CodeMirror view even when it holds text, so such
  // an assertion is a false negative; the proof the right command ran is the
  // result the calling test asserts on.
  await content.click();
  await page.waitForTimeout(60);
  await page.keyboard.press('ControlOrMeta+A');
  await page.waitForTimeout(40);
  await page.keyboard.insertText(statement);
  await page.waitForTimeout(300);
}

async function runStatement(page: Page, statement: string): Promise<void> {
  await ensureEditorOpen(page);
  await setEditorText(page, statement);
  await page.locator('.btn.small.primary', { hasText: 'Run' }).first().click();
}

// Geometry probe: viewport width, the document's own horizontal scroll extent,
// and the widest right-edge of anything under .db-page — EXCLUDING elements that
// live inside a horizontal scroll container (e.g. the results `.grid-scroll`,
// whose table is intentionally wider than the viewport so it can scroll). Such
// elements are correctly contained and must NOT count as page overflow; only an
// element that juts past the viewport WITHOUT a scroll parent is a real bug.
async function overflow(
  page: Page,
): Promise<{ vw: number; widest: number; docScrollW: number; worst: string }> {
  return page.evaluate(() => {
    const de = document.documentElement;
    const insideScroller = (el: HTMLElement): boolean => {
      let p: HTMLElement | null = el.parentElement;
      while (p && p !== document.body) {
        const ox = getComputedStyle(p).overflowX;
        if ((ox === 'auto' || ox === 'scroll') && p.scrollWidth > p.clientWidth + 1) return true;
        p = p.parentElement;
      }
      return false;
    };
    let widest = 0;
    let worst = '';
    document.querySelectorAll<HTMLElement>('.db-page *').forEach((el) => {
      if (insideScroller(el)) return; // contained by a horizontal scroller — fine
      const r = el.getBoundingClientRect();
      if (r.right > widest) {
        widest = r.right;
        worst = (el.className && typeof el.className === 'string' ? el.className : el.tagName)
          .toString()
          .split(' ')
          .filter((c) => c && !c.startsWith('svelte-'))
          .join('.');
      }
    });
    return { vw: de.clientWidth, widest: Math.round(widest), docScrollW: de.scrollWidth, worst };
  });
}

test.describe('DB Explorer — MongoDB sweep', () => {
  // 3 — READ: a plain find returns document rows into the grid.
  test('read: find returns document rows', async ({ page }) => {
    requireConn();
    await openConn(page);
    await runStatement(page, 'db.customers.find({})');

    await ensureResultsOpen(page);
    // Documents render as table rows; ≥1 customer (the seed has 4).
    await expect(page.locator('.grid tbody tr:not(.spacer)').first()).toBeVisible({
      timeout: 20_000,
    });
    const rows = await page.locator('.grid tbody tr:not(.spacer)').count();
    expect(rows, 'find should return ≥1 customer document').toBeGreaterThanOrEqual(1);
    // The grid should carry the customers' fields as columns (email is seeded).
    await expect(page.locator('.grid thead th', { hasText: 'email' })).toBeVisible();
  });

  // 4 — WRITE round-trip in a per-project scratch collection: insertOne →
  // updateOne → find shows "updated" → cleanup with deleteMany.
  //
  // A Mongo write op returns a single-cell status result (column "result"), so it
  // renders as a one-row grid whose cell holds the driver's summary, e.g.
  // "inserted 1" / "matched 1, modified 1" / "deleted 1". We assert on that cell.
  test('write: insertOne + updateOne round-trip', async ({ page }, testInfo) => {
    requireConn();
    // Unique scratch collection per project so the 5 projects never collide.
    const coll = `e2e_scratch_${testInfo.project.name.replace(/[^a-z0-9]/gi, '_')}`;
    await openConn(page);

    // Start clean (a prior aborted run may have left a doc behind).
    await runStatement(page, `db.${coll}.deleteMany({})`);
    await ensureResultsOpen(page);
    await expect(
      page.locator('.grid tbody td', { hasText: /^deleted \d+$/ }).first(),
    ).toBeVisible({ timeout: 20_000 });

    // insertOne → status cell "inserted 1".
    await runStatement(page, `db.${coll}.insertOne({k:1, note:"e2e"})`);
    await ensureResultsOpen(page);
    await expect(
      page.locator('.grid tbody td', { hasText: 'inserted 1' }).first(),
    ).toBeVisible({ timeout: 20_000 });

    // updateOne → status cell "matched 1, modified 1".
    await runStatement(page, `db.${coll}.updateOne({k:1},{$set:{note:"updated"}})`);
    await ensureResultsOpen(page);
    await expect(
      page.locator('.grid tbody td', { hasText: 'modified 1' }).first(),
    ).toBeVisible({ timeout: 20_000 });

    // find → the doc now shows note "updated".
    await runStatement(page, `db.${coll}.find({})`);
    await ensureResultsOpen(page);
    await expect(page.locator('.grid tbody tr:not(.spacer)').first()).toBeVisible({
      timeout: 20_000,
    });
    await expect(
      page.locator('.grid tbody tr:not(.spacer) td', { hasText: 'updated' }).first(),
    ).toBeVisible();

    // Cleanup — deleteMany (drop() is not in the shorthand parser's vocabulary;
    // deleteMany({}) is the documented fallback and IS supported).
    await runStatement(page, `db.${coll}.deleteMany({})`);
    await ensureResultsOpen(page);
    await expect(
      page.locator('.grid tbody td', { hasText: 'deleted 1' }).first(),
    ).toBeVisible({ timeout: 20_000 });
  });

  // 5 — No horizontal overflow, even with a wide multi-field document showing.
  test('layout: no horizontal overflow', async ({ page }) => {
    requireConn();
    await openConn(page);
    // customers have several fields → a good width stressor.
    await runStatement(page, 'db.customers.find({})');
    await ensureResultsOpen(page);
    await expect(page.locator('.grid tbody tr:not(.spacer)').first()).toBeVisible({
      timeout: 20_000,
    });

    const { vw, widest, docScrollW, worst } = await overflow(page);
    // The document itself must not scroll horizontally…
    expect(docScrollW, 'document should not scroll horizontally').toBeLessThanOrEqual(vw + 2);
    // …and nothing under .db-page should jut past the viewport (2px sub-pixel slack).
    // KNOWN SHARED-UI BUG (escalated, not fixable from the Mongo driver): on tablet
    // widths ~641–900px the DB Explorer keeps the desktop side-by-side layout, so
    // the 280px sidebar + the main toolbar overflow horizontally and are clipped by
    // `.content { overflow:hidden }` — the engine chip + Test button end up off
    // screen and unreachable. This assertion correctly RED-flags that on
    // iphone-landscape (814) and ipad-portrait (834); the fix belongs in the shared
    // ui/src/modules/database/DatabasePage.svelte (a tablet breakpoint).
    expect(
      widest,
      `no element under .db-page should exceed the viewport width — widest offender: ` +
        `.${worst} at right=${widest}px (vw=${vw})`,
    ).toBeLessThanOrEqual(vw + 2);
  });

  // 6 — The results grid scrolls HORIZONTALLY (wide documents / many fields) and
  // VERTICALLY (many docs). We widen the result with a synthetic many-field doc
  // and stack enough rows to overflow the bounded grid block.
  test('grid scrolls both directions', async ({ page }, testInfo) => {
    requireConn();
    const coll = `e2e_scrollcoll_${testInfo.project.name.replace(/[^a-z0-9]/gi, '_')}`;
    await openConn(page);

    // Seed many wide documents: 40 rows, each with 18 long fields → forces both
    // a tall result (vertical scroll) and a wide one (horizontal scroll). Write
    // ops render their status as a single-cell grid row (see test 4).
    await runStatement(page, `db.${coll}.deleteMany({})`);
    await ensureResultsOpen(page);
    await expect(
      page.locator('.grid tbody td', { hasText: /^deleted \d+$/ }).first(),
    ).toBeVisible({ timeout: 20_000 });

    const docs = Array.from({ length: 40 }, (_, r) => {
      const fields: string[] = [`row:${r}`];
      for (let c = 0; c < 18; c++) {
        fields.push(`field_${String(c).padStart(2, '0')}:"value-${r}-${c}-padding-to-be-wide"`);
      }
      return `{${fields.join(',')}}`;
    });
    await runStatement(page, `db.${coll}.insertMany([${docs.join(',')}])`);
    await ensureResultsOpen(page);
    await expect(
      page.locator('.grid tbody td', { hasText: 'inserted 40' }).first(),
    ).toBeVisible({ timeout: 20_000 });

    // Fetch them all back (an explicit .limit(40) pins the count above the
    // driver's default 50-row cap).
    await runStatement(page, `db.${coll}.find({}).limit(40)`);
    await ensureResultsOpen(page);
    await expect(page.locator('.grid tbody tr:not(.spacer)').first()).toBeVisible({
      timeout: 20_000,
    });

    const scroll = page.locator('.grid-scroll');
    await scroll.scrollIntoViewIfNeeded();
    const dims = await scroll.evaluate((el) => ({
      clientW: el.clientWidth,
      scrollW: el.scrollWidth,
      clientH: el.clientHeight,
      scrollH: el.scrollHeight,
    }));
    // Wide documents (~19 columns) overflow the grid's width → horizontal scroll.
    expect(dims.scrollW, 'grid content should overflow width (horizontal scroll)').toBeGreaterThan(
      dims.clientW + 20,
    );
    // 40 rows overflow the bounded grid block height → vertical scroll.
    expect(dims.scrollH, 'grid content should overflow height (vertical scroll)').toBeGreaterThan(
      dims.clientH + 20,
    );
    // Prove the horizontal scroll actually moves.
    await scroll.evaluate((el) => (el.scrollLeft = el.scrollWidth));
    const movedLeft = await scroll.evaluate((el) => el.scrollLeft);
    expect(movedLeft, 'grid should scroll horizontally').toBeGreaterThan(0);

    // Cleanup.
    await runStatement(page, `db.${coll}.deleteMany({})`);
    await ensureResultsOpen(page);
    await expect(
      page.locator('.grid tbody td', { hasText: 'deleted 40' }).first(),
    ).toBeVisible({ timeout: 20_000 });
  });

  // 7 — Schema tree: databases → collections is visible + usable in this layout.
  test('schema tree: databases and collections usable', async ({ page }) => {
    requireConn();
    await openConn(page);

    // On a phone the Schema accordion is open by default; ensure it (the side
    // panel only renders once a connection is selected).
    if (isPhone(page)) {
      const schemaHead = page.locator('.acc-toggle', { hasText: /Schema/ });
      const sideBody = page.locator('.side-body');
      if (await sideBody.isHidden().catch(() => false)) {
        await schemaHead.first().click();
      }
    }
    // The Schema side-tab is the default; the SchemaTree mounts inside .side-body.
    await expect(page.locator('.side-body')).toBeVisible({ timeout: 20_000 });

    // The database node `shopdb` should be listed at the schema root.
    const shopdb = page.locator('.side-body').getByText('shopdb', { exact: true });
    await expect(shopdb.first()).toBeVisible({ timeout: 20_000 });
    // Expand it → the customers collection becomes visible (databases → collections).
    await shopdb.first().click();
    await expect(
      page.locator('.side-body').getByText('customers', { exact: true }).first(),
    ).toBeVisible({ timeout: 20_000 });
  });
});
