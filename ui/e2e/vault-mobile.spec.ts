import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedVaultNotes } from './seed';

// Vault page — PHONE usability.
//
// The Vault (workspace knowledge store: notes with [[backlinks]], hybrid search,
// and an SVG knowledge graph) ships a desktop two-column grid (300px sidebar |
// reader). On a phone that 300px sidebar squeezes the reader/graph into a ~120px
// sliver — the note title wraps one word per line and the SVG graph spills off
// the right edge, unreachable. The mobile (≤640px) layout turns the page into a
// single full-width column that swaps between the INDEX (search + filters + note
// list) and the OPEN note / graph, with a "‹ Index" back button to return. Each
// pane scrolls independently and nothing overflows the viewport horizontally.
//
// These specs exercise REAL behavior against seeded notes + a seeded graph:
//   • the page fits the viewport width in both orientations (no h-overflow),
//   • the note list is reachable and scrollable,
//   • opening a note shows a scrollable reader with a back button,
//   • search filters the list,
//   • the knowledge graph fits the viewport.
//
// Phone-only cases skip on the >640px (tablet/desktop) projects; the
// no-overflow + reachability cases run on EVERY project (incl. landscape 814).

let workspaceId = '';
let noteIds: string[] = [];

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  noteIds = await seedVaultNotes(ctx, base, workspaceId);
  await ctx.dispose();
});

// Activate the seeded workspace (so the vault loads) and collapse the nav rail
// (which defaults expanded on a fresh phone profile and would cover the page).
test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

function isPhone(page: Page): boolean {
  return (page.viewportSize()?.width ?? 0) <= 640;
}

// Open the Vault and wait for the seeded notes to appear in the list.
async function openVault(page: Page): Promise<void> {
  await page.goto('/#/vault');
  await expect(page.locator('.vault')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.vault-item').first()).toBeVisible({ timeout: 30_000 });
}

// Widest right-edge of any element under .vault vs the viewport width.
async function overflow(page: Page): Promise<{ vw: number; widest: number; docScrollW: number }> {
  return page.evaluate(() => {
    const de = document.documentElement;
    let widest = 0;
    document.querySelectorAll<HTMLElement>('.vault *').forEach((el) => {
      const r = el.getBoundingClientRect();
      if (r.right > widest) widest = r.right;
    });
    return { vw: de.clientWidth, widest: Math.round(widest), docScrollW: de.scrollWidth };
  });
}

// Return to the INDEX (phone uses the back button; on wider layouts both panes
// already show, so this is a no-op there).
async function backToIndex(page: Page): Promise<void> {
  const back = page.locator('.mobile-back');
  if (await back.isVisible().catch(() => false)) {
    await back.click();
    await page.waitForTimeout(150);
  }
}

test.describe('vault mobile', () => {
  // --- run on EVERY project (portrait, landscape, se, ipad) ----------------

  test('index/list is reachable with seeded notes', async ({ page }) => {
    await openVault(page);
    const items = page.locator('.vault-item');
    // The 4 seeded memories (+ any graph entity rows) are listed.
    expect(await items.count()).toBeGreaterThanOrEqual(4);
    await expect(page.locator('.vault-item', { hasText: 'Architecture overview' })).toBeVisible();
  });

  test('no horizontal overflow on the index', async ({ page }) => {
    await openVault(page);
    const { vw, widest, docScrollW } = await overflow(page);
    expect(docScrollW).toBeLessThanOrEqual(vw + 1);
    expect(widest).toBeLessThanOrEqual(vw + 2);
  });

  test('opening a note shows the reader (title + body)', async ({ page }) => {
    await openVault(page);
    await page.locator('.vault-item', { hasText: 'Architecture overview' }).first().click();
    await expect(page.locator('.note-head h1')).toHaveText('Architecture overview', {
      timeout: 15_000,
    });
    await expect(page.locator('.note-body')).toBeVisible();
    await expect(page.locator('.note-body')).toContainText('Otto is composed of');
  });

  test('no horizontal overflow with a note open', async ({ page }) => {
    await openVault(page);
    await page.locator('.vault-item', { hasText: 'Architecture overview' }).first().click();
    await expect(page.locator('.note-head h1')).toBeVisible({ timeout: 15_000 });
    const { vw, widest, docScrollW } = await overflow(page);
    expect(docScrollW).toBeLessThanOrEqual(vw + 1);
    expect(widest).toBeLessThanOrEqual(vw + 2);
  });

  test('search filters the note list', async ({ page }) => {
    await openVault(page);
    await backToIndex(page);
    const search = page.locator('.vault-search input');
    await expect(search).toBeVisible();
    await search.fill('daemon');
    // Hybrid search debounces (~250ms); the daemon note should rank in.
    await expect(page.locator('.vault-item', { hasText: 'Daemon design' })).toBeVisible({
      timeout: 15_000,
    });
  });

  test('knowledge graph fits the viewport width', async ({ page }) => {
    await openVault(page);
    await backToIndex(page);
    await page.getByRole('button', { name: 'Graph' }).click();
    const svg = page.locator('.vault-graph');
    await expect(svg).toBeVisible({ timeout: 15_000 });
    // The SVG renders the seeded graph nodes (memories + imported entities).
    await expect(page.locator('.vault-graph .g-node').first()).toBeVisible();
    // The SVG itself must not be wider than the viewport.
    const fit = await page.evaluate(() => {
      const el = document.querySelector<SVGElement>('.vault-graph');
      const de = document.documentElement;
      const r = el?.getBoundingClientRect();
      return { vw: de.clientWidth, w: Math.round(r?.width ?? 0), right: Math.round(r?.right ?? 0) };
    });
    expect(fit.w).toBeLessThanOrEqual(fit.vw + 2);
    expect(fit.right).toBeLessThanOrEqual(fit.vw + 2);
    // And the whole page still doesn't scroll horizontally.
    const { vw, docScrollW } = await overflow(page);
    expect(docScrollW).toBeLessThanOrEqual(vw + 1);
  });

  // --- phone-only single-column behavior (≤640px) --------------------------

  test('phone: a back button returns from the reader to the index', async ({ page }) => {
    test.skip(!isPhone(page), 'phone single-column only');
    await openVault(page);

    // Index visible, reader hidden initially.
    await expect(page.locator('.vault-side')).toBeVisible();
    await expect(page.locator('.note-head h1')).toBeHidden();

    // Open a note → reader covers the index, back button appears.
    await page.locator('.vault-item', { hasText: 'Architecture overview' }).first().click();
    await expect(page.locator('.note-head h1')).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('.vault-side')).toBeHidden();
    const back = page.locator('.mobile-back');
    await expect(back).toBeVisible();

    // Back → index again, reader gone.
    await back.click();
    await expect(page.locator('.vault-side')).toBeVisible();
    await expect(page.locator('.note-head h1')).toBeHidden();
  });

  test('phone: the note list scrolls independently', async ({ page }) => {
    test.skip(!isPhone(page), 'phone single-column only');
    await openVault(page);
    // The list is a bounded scroll container; with several notes it overflows.
    const info = await page.locator('.vault-list').evaluate((el) => ({
      clientH: el.clientHeight,
      scrollH: el.scrollHeight,
      overflowY: getComputedStyle(el).overflowY,
    }));
    expect(['auto', 'scroll']).toContain(info.overflowY);
    // It can be scrolled (content taller than the box, or at least scrollable).
    expect(info.scrollH).toBeGreaterThanOrEqual(info.clientH);
  });

  test('phone: the reader scrolls vertically for a long note', async ({ page }) => {
    test.skip(!isPhone(page), 'phone single-column only');
    await openVault(page);
    await page.locator('.vault-item', { hasText: 'Architecture overview' }).first().click();
    await expect(page.locator('.note-head h1')).toBeVisible({ timeout: 15_000 });
    // The reader pane (.vault-main) is the scroll container; the long hub note
    // overflows it.
    const info = await page.locator('.vault-main').evaluate((el) => ({
      clientH: el.clientHeight,
      scrollH: el.scrollHeight,
    }));
    expect(info.scrollH).toBeGreaterThan(info.clientH + 20);
  });

  test('phone: search box is full-width and usable', async ({ page }) => {
    test.skip(!isPhone(page), 'phone single-column only');
    await openVault(page);
    const box = page.locator('.vault-search input');
    const fit = await box.evaluate((el) => {
      const r = el.getBoundingClientRect();
      const fs = parseFloat(getComputedStyle(el).fontSize);
      return { w: Math.round(r.width), vw: document.documentElement.clientWidth, fs };
    });
    // Spans (most of) the viewport width…
    expect(fit.w).toBeGreaterThan(fit.vw * 0.8);
    // …and uses ≥16px text (so iOS Safari doesn't zoom-on-focus).
    expect(fit.fs).toBeGreaterThanOrEqual(16);
  });

  test('phone: graph view exposes a back button to the index', async ({ page }) => {
    test.skip(!isPhone(page), 'phone single-column only');
    await openVault(page);
    await page.getByRole('button', { name: 'Graph' }).click();
    await expect(page.locator('.vault-graph')).toBeVisible({ timeout: 15_000 });
    // The index sidebar is covered by the graph on a phone…
    await expect(page.locator('.vault-side')).toBeHidden();
    // …and the back button returns to it.
    await page.locator('.mobile-back').click();
    await expect(page.locator('.vault-side')).toBeVisible();
  });
});
