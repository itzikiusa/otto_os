import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// The pane header never clips a control (desktop-browser only).
//
// With three or more panes the header used to push the view toggle, restart, ⋯
// and ✕ past the edge: `.pane` is overflow:hidden and half those controls are
// flex-shrink:0, so they simply vanished. The header now folds in TIERS keyed
// to its own inline size, and everything a tier drops comes back as a row in
// the clamped global ctxMenu.
//
// A tier is reached by pane COUNT at the project's 1280×800 — never by shrinking
// the viewport (below 1025px that is the tablet shell, a different layout).
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsId = '';

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  wsId = await seedWorkspace(ctx, base);
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: 'Baresi', cwd: '/tmp', meta: { origin: 'e2e' } },
  });
  if (!r.ok()) throw new Error(`seed → ${r.status()} ${await r.text()}`);
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, wsId);
  await page.goto('/#/agents');
  await expect(page.getByText('Baresi').first()).toBeVisible({ timeout: 20_000 });
});

test.afterEach(async () => {
  await ctx?.dispose();
});

/** Run a ⌘K command by its exact palette title. */
async function runCommand(page: Page, title: string): Promise<void> {
  await page.keyboard.press('Meta+k');
  await expect(page.locator('.palette')).toBeVisible({ timeout: 10_000 });
  await page.keyboard.type(title);
  await page.locator('.pal-item', { hasText: title }).first().click();
  await expect(page.locator('.palette')).toBeHidden({ timeout: 10_000 });
}

/**
 * Split into equal COLUMNS until a pane header is at most `maxW` wide, and
 * return that width. 1280px minus the shell leaves ≈ 970px for the tree, so 4
 * columns ≈ 240px and 8 ≈ 115px — the 15-pane cap is never reached here.
 */
async function columnsUntil(page: Page, maxW: number): Promise<number> {
  await page.locator('.nav-item.nested-item', { hasText: 'Baresi' }).first().click();
  await expect(page.locator('.pane').first()).toBeVisible({ timeout: 15_000 });
  let w = await page.locator('.pane-head').first().evaluate((el) => el.clientWidth);
  for (let i = 0; i < 14 && w > maxW; i++) {
    await page.keyboard.press('Meta+d');
    await expect(page.locator('[data-pane-key]')).toHaveCount(i + 2, { timeout: 15_000 });
    await runCommand(page, 'Layout: Equal Columns');
    w = await page.locator('.pane-head').first().evaluate((el) => el.clientWidth);
  }
  expect(w, `could not reach a ${maxW}px header before the pane cap`).toBeLessThanOrEqual(maxW);
  return w;
}

/** Every header must genuinely FIT — `overflow: clip` does not hide overflow
 *  from scrollWidth, so this is the real proof the tier set was sized right. */
async function expectHeadersFit(page: Page): Promise<void> {
  const over = await page.locator('.pane-head').evaluateAll((els) =>
    els.map((el) => el.scrollWidth - el.clientWidth),
  );
  expect(over.length).toBeGreaterThan(0);
  for (const o of over) expect(o, 'pane header horizontal overflow (px)').toBeLessThanOrEqual(2);
}

test('narrow column panes shed the segmented control and keep every button in the viewport', async ({ page }) => {
  test.slow();
  const w = await columnsUntil(page, 260);
  const n = await page.locator('[data-pane-key]').count();

  // Tier 5+: the three-tab segmented control is gone from the DOM.
  await expect(page.locator('.view-seg')).toHaveCount(0);
  await expect(page.locator('button[title="More…"]')).toHaveCount(n);

  if (w > 200) {
    // Tier 5: one view icon per pane, its menu switches the view.
    await expect(page.locator('[data-view-mini]')).toHaveCount(n);
    await page.locator('[data-view-mini]').first().click();
    const menu = page.locator('.ctx-menu');
    await expectFullyInViewport(page, menu, 'view menu');
    await menu.getByRole('menuitem', { name: 'Chat', exact: true }).click();
  } else {
    // Tier 6: the view rows live in the ⋯ menu.
    const more = page.locator('button[title="More…"]').first();
    await more.click();
    const menu = page.locator('.ctx-menu');
    await expectFullyInViewport(page, menu, 'pane overflow menu');
    await menu.getByRole('menuitem', { name: 'View: Chat' }).click();
  }
  await expect(page.locator('.pane-body[data-view="chat"]').first()).toBeVisible({ timeout: 15_000 });

  // The restart button folded away — its action is a menu row now.
  const more = page.locator('button[title="More…"]').first();
  await more.click();
  await expect(page.locator('.ctx-menu').getByRole('menuitem', { name: 'Restart session' })).toBeVisible();
  await page.keyboard.press('Escape');

  // Nothing in any header is clipped or off-screen.
  await expectHeadersFit(page);
  const buttons = page.locator('.pane-head button');
  const count = await buttons.count();
  for (let i = 0; i < count; i++) await expectFullyInViewport(page, buttons.nth(i), 'pane header button');
});

test('tier 7 folds the title and close into ⋯', async ({ page }) => {
  test.slow();
  await columnsUntil(page, 140);

  // THE tier-7 proof: the title left the header and every header still fits.
  await expect(page.locator('.pane-title')).toHaveCount(0);
  await expectHeadersFit(page);

  await page.locator('button[title="More…"]').first().click();
  const menu = page.locator('.ctx-menu');
  await expectFullyInViewport(page, menu, 'pane overflow menu');
  // The first row is a disabled header carrying the session title.
  const first = menu.getByRole('menuitem').first();
  await expect(first).toContainText('Baresi');
  await expect(first).toBeDisabled();

  // …and the ✕ is reachable from the same menu. Every pane here holds the SAME
  // session (⌘D clones the focused one), so closing its tab empties the split.
  await menu.getByRole('menuitem', { name: 'Close pane' }).click();
  await page.getByRole('button', { name: 'Delete session' }).click();
  await expect(page.locator('[data-pane-key]')).toHaveCount(0, { timeout: 15_000 });
});
