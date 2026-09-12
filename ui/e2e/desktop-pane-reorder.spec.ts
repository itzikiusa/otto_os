import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Reordering sessions — panes, tiles and sidebar rows (desktop-browser only).
//
// Three surfaces, one gesture: drag the pane header grip onto another pane's
// CENTRE to swap the two sessions (the focused slot keeps focus) or onto an
// EDGE to move the pane into a new split there; drag a tile onto another to
// reorder the tiled grid; drag a sidebar row to switch the Agents list into
// Manual order. Everything also works from the keyboard (⌘⌥arrows / ⌘⌥S) and
// the palette, so it is reachable without a mouse.
//
// HTML5 DnD is driven through explicit dragstart/dragover/drop events sharing
// one DataTransfer — `locator.dragTo` is flaky for drags whose drop target only
// becomes hit-testable once the drag has STARTED (the pane drop veils).
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsId = '';
const TITLES = ['Costacurta', 'Albertini', 'Boban'];
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
    // The sidebar lists EVERY workspace's sessions by default, and the suite runs
    // 4 parallel workers against ONE daemon — pin the list to this workspace so
    // another worker's sessions can't shift the row counts below.
    localStorage.setItem('otto_nav_all_ws', '0');
  }, wsId);
  await page.goto('/#/agents');
  await expect(page.getByText(TITLES[0]).first()).toBeVisible({ timeout: 20_000 });
});

test.afterEach(async () => {
  await ctx?.dispose();
});

/** Click a sidebar row and wait for THAT session's pane. Waiting for any
 *  `.pane` is not enough once a pane is already open: the click lands, the
 *  route→store hop is still in flight, and the next ⌘D would clone the pane the
 *  PREVIOUS session still holds — leaving two slots on one session. */
async function openSession(page: Page, title: string): Promise<void> {
  await page.locator('.nav-item.nested-item', { hasText: title }).first().click();
  await expect(
    page.locator(`[data-pane-key][data-session="${idByTitle[title]}"]`).first(),
  ).toBeVisible({ timeout: 15_000 });
}

/** Two distinct sessions side by side (⌘D clones, then the sidebar replaces the
 *  focused leaf), then a third. Returns nothing — assert on `[data-session]`. */
async function threePanes(page: Page): Promise<void> {
  await openSession(page, 'Costacurta');
  await page.keyboard.press('Meta+d');
  await openSession(page, 'Albertini');
  await page.keyboard.press('Meta+d');
  await openSession(page, 'Boban');
  await expect(page.locator('[data-pane-key]')).toHaveCount(3, { timeout: 15_000 });
}

/** Focus a pane without touching a control. The header's leading padding is the
 *  one spot no button ever occupies — its CENTRE is the segmented view control
 *  in a roomy pane (which stops mousedown, so the click switches the view and
 *  focuses nothing). */
async function focusPane(page: Page, i: number): Promise<void> {
  const pane = page.locator('[data-pane-key]').nth(i);
  await pane.locator('.pane-head').click({ position: { x: 3, y: 3 } });
  await expect(pane.locator('.pane.focused')).toHaveCount(1);
}

const sessionOrderOf = (page: Page) =>
  page.locator('[data-pane-key]').evaluateAll((els) => els.map((el) => (el as HTMLElement).dataset.session));
const keyOrderOf = (page: Page) =>
  page.locator('[data-pane-key]').evaluateAll((els) => els.map((el) => (el as HTMLElement).dataset.paneKey));

/**
 * Drive one HTML5 drag from `source` to a point inside `target`, sharing a
 * single DataTransfer so `dataTransfer.setData`/`getData` behave as they do for
 * a real drag. `at` is a 0–1 fraction of the target box.
 */
async function dragOnto(
  page: Page,
  source: string,
  target: string,
  at: { x: number; y: number },
): Promise<void> {
  const dt = await page.evaluateHandle(() => new DataTransfer());
  await page.dispatchEvent(source, 'dragstart', { dataTransfer: dt });
  const box = (await page.locator(target).boundingBox())!;
  const point = { clientX: box.x + box.width * at.x, clientY: box.y + box.height * at.y };
  await page.dispatchEvent(target, 'dragover', { dataTransfer: dt, ...point });
  await page.dispatchEvent(target, 'drop', { dataTransfer: dt, ...point });
  await page.dispatchEvent(source, 'dragend', { dataTransfer: dt });
  await dt.dispose();
}

test("dropping a grip on a pane's centre swaps the two sessions", async ({ page }) => {
  await threePanes(page);
  const before = await sessionOrderOf(page);
  const keysBefore = await keyOrderOf(page);

  await dragOnto(page, '[data-pane-key] >> nth=0 >> [data-testid="pane-grip"]', '[data-pane-key] >> nth=1 >> .drop-veil', { x: 0.5, y: 0.5 });

  await expect.poll(() => sessionOrderOf(page)).toEqual([before[1], before[0], before[2]]);
  // A swap moves SESSIONS, never slots — the keys stay exactly where they were.
  expect(await keyOrderOf(page)).toEqual(keysBefore);
});

test('dropping on the right edge moves the pane beside it', async ({ page }) => {
  await threePanes(page);
  const before = await sessionOrderOf(page);

  await dragOnto(page, '[data-pane-key] >> nth=0 >> [data-testid="pane-grip"]', '[data-pane-key] >> nth=2 >> .drop-veil', { x: 0.92, y: 0.5 });

  await expect(page.locator('[data-pane-key]')).toHaveCount(3);
  // The dragged pane left position 0 and landed after its target.
  await expect.poll(() => sessionOrderOf(page)).toEqual([before[1], before[2], before[0]]);
});

test('⌘⌥→ moves the focused pane', async ({ page }) => {
  await threePanes(page);
  const before = await sessionOrderOf(page);
  // Focus the first pane, then push it right past its neighbour.
  await focusPane(page, 0);
  await page.keyboard.press('Meta+Alt+ArrowRight');

  await expect.poll(() => sessionOrderOf(page)).not.toEqual(before);
  // The route followed the pane: the navigator still highlights its session.
  const focused = (await sessionOrderOf(page)).find((id) => id === before[0]);
  expect(focused).toBe(before[0]);
  await expect(page.locator('.nav-item.nested-item.active')).toHaveCount(1);
});

test('⌘⌥S swaps the focused pane with the next', async ({ page }) => {
  await threePanes(page);
  const before = await sessionOrderOf(page);
  const keysBefore = await keyOrderOf(page);
  await focusPane(page, 0);
  // `Meta+Alt+KeyS`, not `Meta+Alt+s`: with ⌥ held macOS reports `e.key === 'ß'`,
  // which is exactly the bug this chord had — the handler matches `e.code`.
  await page.keyboard.press('Meta+Alt+KeyS');

  // A swap moves SESSIONS between slots; the slots themselves stay put.
  await expect.poll(() => sessionOrderOf(page)).toEqual([before[1], before[0], before[2]]);
  expect(await keyOrderOf(page)).toEqual(keysBefore);
});

test('tiled view: drag a tile onto another reorders and a reload keeps it', async ({ page }) => {
  await openSession(page, 'Costacurta');
  await page.locator('button[aria-label="Tiled view"]').click();
  await expect(page.locator('[data-tile-id]')).toHaveCount(3, { timeout: 20_000 });
  const before = await page.locator('[data-tile-id]').evaluateAll((els) =>
    els.map((el) => (el as HTMLElement).dataset.tileId),
  );

  // The drag source is whichever header the tile has (live pane grip, else the
  // placeholder's own header).
  const first = page.locator('[data-tile-id]').first();
  const grip = first.locator('[data-testid="pane-grip"]');
  const source = (await grip.count()) > 0 ? '[data-tile-id] >> nth=0 >> [data-testid="pane-grip"]' : '[data-tile-id] >> nth=0 >> .ph-head';
  await dragOnto(page, source, '[data-tile-id] >> nth=2', { x: 0.5, y: 0.5 });

  const expected = [before[1], before[2], before[0]];
  await expect
    .poll(() =>
      page.locator('[data-tile-id]').evaluateAll((els) => els.map((el) => (el as HTMLElement).dataset.tileId)),
    )
    .toEqual(expected);
  const stored = await page.evaluate((id) => localStorage.getItem('otto_tile_order_' + id), wsId);
  expect(JSON.parse(stored ?? '[]')).toEqual(expected);

  await page.reload();
  await expect(page.locator('[data-tile-id]')).toHaveCount(3, { timeout: 20_000 });
  await expect
    .poll(() =>
      page.locator('[data-tile-id]').evaluateAll((els) => els.map((el) => (el as HTMLElement).dataset.tileId)),
    )
    .toEqual(expected);
});

test('sidebar: drag a row onto another switches to manual and persists', async ({ page }) => {
  // Only the flat Agents list: the "No workspace" group renders `.nested-row`s
  // too, and scratch sessions are global to the daemon every worker shares.
  const rows = page.getByTestId('agents-list').locator('.nested-row');
  await expect(rows).toHaveCount(3, { timeout: 20_000 });
  const before = await rows.locator('.nested-item .ellipsis').allInnerTexts();

  await dragOnto(
    page,
    '[data-testid="agents-list"] .nested-row >> nth=2 >> .nested-item',
    '[data-testid="agents-list"] .nested-row >> nth=0',
    { x: 0.5, y: 0.5 },
  );

  const expected = [before[2], before[0], before[1]];
  await expect.poll(() => rows.locator('.nested-item .ellipsis').allInnerTexts()).toEqual(expected);
  // The first drag implies Manual — the header control says so.
  const sort = page.getByTestId('agents-sort-toggle');
  await expect(sort).toHaveAttribute('title', /Manual order/);
  const stored = await page.evaluate((id) => localStorage.getItem('otto_session_order_' + id), wsId);
  expect((JSON.parse(stored ?? '{}') as { mode?: string }).mode).toBe('manual');

  await page.reload();
  await expect(rows).toHaveCount(3, { timeout: 20_000 });
  await expect.poll(() => rows.locator('.nested-item .ellipsis').allInnerTexts()).toEqual(expected);

  // …and "Reset to recent" hands the list back to the daemon's order.
  await sort.click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Reset to recent' }).click();
  await expect.poll(() => rows.locator('.nested-item .ellipsis').allInnerTexts()).toEqual(before);
});

test('sidebar rows are not draggable while searching', async ({ page }) => {
  const rows = page.getByTestId('agents-list').locator('.nested-row');
  await expect(rows).toHaveCount(3, { timeout: 20_000 });
  await expect(page.locator("[data-testid='agents-list'] .nested-row[draggable='true']")).toHaveCount(3);
  await page.locator('.nav-search-input').fill('Boban');
  await expect(rows).toHaveCount(1);
  await expect(page.locator("[data-testid='agents-list'] .nested-row[draggable='true']")).toHaveCount(0);
});
