import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Nested split layouts (desktop-browser only).
//
// The split view used to be ONE grid on ONE axis with ONE fraction per axis
// (3–4 panes were always a 2×2 with an empty cell at 3). It is a binary tree
// now: any pane splits left/right/up/down, every split node owns its fraction,
// and `otto_panes_<ws>` carries a `{v:2, tree, focused}` payload — a v1
// `{panes, axis}` payload still restores, through the old window fractions.
//
// The desktop project is 1280×800 and NO test resizes below 1025px (that is the
// tablet shell) — narrow panes come from pane COUNT, never from the viewport.
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsId = '';
const TITLES = ['Pirlo', 'Seedorf', 'Rui Costa'];
const idByTitle: Record<string, string> = {};

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  wsId = await seedWorkspace(ctx, base);
  for (const title of TITLES) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'e2e' } },
    });
    if (!r.ok()) throw new Error(`seed ${title} → ${r.status()} ${await r.text()}`);
    idByTitle[title] = (await r.json()).id as string;
  }
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, wsId);
  await page.goto('/#/agents');
  await expect(page.getByText(TITLES[0]).first()).toBeVisible({ timeout: 20_000 });
});

test.afterEach(async () => {
  await ctx?.dispose();
});

/** Open a seeded session from the sidebar and wait for ITS pane — waiting for
 *  any `.pane` races the route→store hop once one is already open. */
async function openSession(page: Page, title: string): Promise<void> {
  await page.locator('.nav-item.nested-item', { hasText: title }).first().click();
  await expect(
    page.locator(`[data-pane-key][data-session="${idByTitle[title]}"]`).first(),
  ).toBeVisible({ timeout: 15_000 });
}

const leaves = (page: Page) => page.locator('[data-pane-key]');

/** Bounding boxes of every leaf, in reading order. */
async function leafBoxes(page: Page): Promise<{ x: number; y: number; w: number; h: number }[]> {
  return leaves(page).evaluateAll((els) =>
    els.map((el) => {
      const r = el.getBoundingClientRect();
      return { x: r.left, y: r.top, w: r.width, h: r.height };
    }),
  );
}

/** Run a ⌘K command by its exact palette title. */
async function runCommand(page: Page, title: string): Promise<void> {
  await page.keyboard.press('Meta+k');
  await expect(page.locator('.palette')).toBeVisible({ timeout: 10_000 });
  await page.keyboard.type(title);
  await page.locator('.pal-item', { hasText: title }).first().click();
  await expect(page.locator('.palette')).toBeHidden({ timeout: 10_000 });
}

test('⌘D then ⌘⇧D gives one full-height pane beside two stacked', async ({ page }) => {
  await openSession(page, 'Pirlo');
  await page.keyboard.press('Meta+d');
  await expect(leaves(page)).toHaveCount(2);
  await page.keyboard.press('Meta+Shift+d');
  await expect(leaves(page)).toHaveCount(3);

  // The user's example: the first pane spans the full height on the left; the
  // other two share its right-hand neighbour's column, one above the other.
  const [a, b, c] = await leafBoxes(page);
  expect(a.x, 'first pane is leftmost').toBeLessThan(b.x);
  expect(b.x, 'the stacked pair shares one column').toBeCloseTo(c.x, 0);
  expect(b.w).toBeCloseTo(c.w, 0);
  expect(c.y, 'the third pane sits under the second').toBeGreaterThan(b.y + b.h - 1);
  expect(a.h, 'the first pane is as tall as both of them').toBeGreaterThan(b.h + c.h - 1);
});

test('dragging a row gutter changes the fraction and survives a reload', async ({ page }) => {
  await openSession(page, 'Pirlo');
  await page.keyboard.press('Meta+d');
  await page.keyboard.press('Meta+Shift+d');
  await expect(leaves(page)).toHaveCount(3);

  const gutter = page.locator('.gutter[aria-orientation="horizontal"]').first();
  await expect(gutter).toBeVisible();
  const before = Number(await gutter.getAttribute('aria-valuenow'));
  const g = (await gutter.boundingBox())!;
  await page.mouse.move(g.x + g.width / 2, g.y + g.height / 2);
  await page.mouse.down();
  await page.mouse.move(g.x + g.width / 2, g.y - 120, { steps: 10 });
  await page.mouse.up();

  const after = Number(await gutter.getAttribute('aria-valuenow'));
  expect(after, 'the row split moved').toBeLessThan(before);

  // Persisted as v2 under the SAME key the v1 payload used (winKey is the bare
  // key in a browser context). The gutter persist is debounced — poll for it.
  await expect
    .poll(
      async () =>
        page.evaluate((id) => {
          const raw = localStorage.getItem('otto_panes_' + id);
          return raw ? (JSON.parse(raw) as { v?: number }).v : null;
        }, wsId),
      { timeout: 5_000 },
    )
    .toBe(2);

  await page.reload();
  await expect(leaves(page)).toHaveCount(3, { timeout: 20_000 });
  const restored = Number(
    await page.locator('.gutter[aria-orientation="horizontal"]').first().getAttribute('aria-valuenow'),
  );
  expect(restored).toBe(after);
});

test('a v1 payload restores as the legacy 3-pane shape', async ({ page }) => {
  const ids = TITLES.map((t) => idByTitle[t]);
  await page.addInitScript(
    (arg) => {
      const a = arg as { ws: string; ids: string[] };
      localStorage.setItem('otto_tabs_' + a.ws, JSON.stringify(a.ids));
      localStorage.setItem('otto_panes_' + a.ws, JSON.stringify({ panes: a.ids, axis: 'col' }));
      localStorage.setItem('otto_split_col_frac', '0.3');
    },
    { ws: wsId, ids },
  );
  await page.reload();
  await expect(leaves(page)).toHaveCount(3, { timeout: 20_000 });

  // v1 n=3 → top row (p0 | p1) at colFrac, p2 full width below at rowFrac.
  const [a, b, c] = await leafBoxes(page);
  const row = a.w + 8 + b.w;
  expect(a.w / row, 'the old column fraction became the root fraction').toBeGreaterThan(0.24);
  expect(a.w / row).toBeLessThan(0.36);
  expect(b.y).toBeCloseTo(a.y, 0);
  expect(c.y, 'the third pane spans the full width below').toBeGreaterThan(a.y + a.h - 1);
  expect(c.w).toBeGreaterThan(row - 9);

  // …and it was migrated to v2 on restore (one-way).
  const v = await page.evaluate((id) => {
    const raw = localStorage.getItem('otto_panes_' + id);
    return raw ? (JSON.parse(raw) as { v?: number }).v : null;
  }, wsId);
  expect(v).toBe(2);
});

test('⋯ → "Layout: One beside two" rebuilds the tree', async ({ page }) => {
  await openSession(page, 'Pirlo');
  await page.keyboard.press('Meta+d');
  await page.keyboard.press('Meta+d');
  await expect(leaves(page)).toHaveCount(3);

  await page.locator('button[title="More…"]').first().click();
  const menu = page.locator('.ctx-menu');
  await menu.getByRole('menuitem', { name: 'Layout: One beside two' }).click();
  await expect(menu).toBeHidden();

  const [a, b, c] = await leafBoxes(page);
  expect(a.x).toBeLessThan(b.x);
  expect(b.x).toBeCloseTo(c.x, 0);
  expect(c.y).toBeGreaterThan(b.y + b.h - 1);
  expect(a.h).toBeGreaterThan(b.h + c.h - 1);
});

test('closing a session whose neighbour is already on screen collapses its split', async ({ page }) => {
  await openSession(page, 'Pirlo');
  await page.keyboard.press('Meta+d'); // [Pirlo, Pirlo]
  await openSession(page, 'Seedorf'); // the focused leaf is replaced → [Pirlo, Seedorf]
  await page.keyboard.press('Meta+d'); // [Pirlo, Seedorf, Seedorf]
  await openSession(page, 'Rui Costa'); // → [Pirlo, Seedorf, Rui Costa]
  await expect(leaves(page)).toHaveCount(3);

  // The header ✕ closes the TAB (Archive / Delete), and closeTab maps every
  // leaf of that session onto the fallback — which is already on screen, so the
  // replaced leaf is deduped away and its split node collapses.
  const rui = page.locator('.pane', { hasText: 'Rui Costa' }).first();
  await rui.locator('button[aria-label="Close session (⌘W)"]').click();
  await page.getByRole('button', { name: 'Delete session' }).click();

  await expect(leaves(page)).toHaveCount(2, { timeout: 15_000 });
  const sessions = await leaves(page).evaluateAll((els) =>
    els.map((el) => (el as HTMLElement).dataset.session),
  );
  expect(new Set(sessions).size, 'two distinct sessions remain').toBe(2);
  await expect(page.locator('.gutter'), 'the collapsed node took its gutter with it').toHaveCount(1);
});

test('⌘D stops at 15 panes and Layout: Grid re-equalises them', async ({ page }) => {
  test.slow();
  await openSession(page, 'Pirlo');
  for (let i = 0; i < 14; i++) await page.keyboard.press('Meta+d');
  await expect(leaves(page)).toHaveCount(15, { timeout: 30_000 });
  // At the cap the split is a silent no-op (no toast, like every tiling WM).
  await page.keyboard.press('Meta+d');
  await expect(leaves(page)).toHaveCount(15);

  await runCommand(page, 'Layout: Grid');
  const boxes = await leafBoxes(page);
  expect(boxes).toHaveLength(15);
  // 15 tiles ⇒ 4 columns (the same rule the tiled view uses): the top row holds
  // four panes at four distinct x positions.
  const top = Math.min(...boxes.map((b) => b.y));
  const firstRow = boxes.filter((b) => Math.abs(b.y - top) < 2);
  expect(firstRow, 'first row is four columns wide').toHaveLength(4);
  expect(new Set(firstRow.map((b) => Math.round(b.x))).size).toBe(4);

  // …and at that width every header still FITS (nothing clipped past the edge).
  const overflow = await page.locator('.pane-head').evaluateAll((els) =>
    els.map((el) => el.scrollWidth - el.clientWidth),
  );
  for (const o of overflow) expect(o, 'pane header horizontal overflow (px)').toBeLessThanOrEqual(2);
});
