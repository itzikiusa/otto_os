import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, runBarCommand } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// The pane header never clips a control (desktop-browser only).
//
// With three or more panes the header used to push the view toggle, restart, ⋯
// and ✕ past the edge: `.pane` is overflow:hidden and half those controls are
// flex-shrink:0, so they simply vanished. The header is now one compact row
// adapted to the PANE width by container queries (lib/paneHeader.ts tiers:
// full ≥720 · compact 420–719 · minimal 200–419 · micro <200); the title is the
// one flexible item, and every control a tier hides comes back as a row in the
// clamped global ctxMenu.
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

/** Run a ⌘K command by its exact title (⌘K focuses the floating bar). */
const runCommand = runBarCommand;

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
    await runCommand(page, 'Layout: equal columns');
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

/** Only what is actually rendered — tiers hide controls with `display: none`. */
const visibleHeaderButtons = (page: Page) => page.locator('.pane-head button:visible');

test('minimal panes keep dot + title + view flip + ⋯ and every button in the viewport', async ({ page }) => {
  test.slow();
  await columnsUntil(page, 400);
  const n = await page.locator('[data-pane-key]').count();
  await expect(page.locator('.pane-head[data-tier="minimal"]')).toHaveCount(n);

  // The Terminal · Chat tabs are hidden; one flip button per pane replaces them.
  await expect(page.getByRole('tablist', { name: 'Session view' })).toHaveCount(0);
  await expect(page.locator('[data-view-toggle]:visible')).toHaveCount(n);
  await expect(page.locator('button[title="More…"]:visible')).toHaveCount(n);
  // The title stays readable — it is the one thing that gets the space.
  const titleW = await page.locator('.pane-title').first().evaluate((el) => el.getBoundingClientRect().width);
  expect(titleW, 'title width in a minimal pane').toBeGreaterThan(40);
  await expect(page.locator('.pane-title').first()).toHaveText('Baresi');

  await page.locator('[data-view-toggle]').first().click();
  await expect(page.locator('.pane-body[data-view="chat"]').first()).toBeVisible({ timeout: 15_000 });

  // Restart, and the ✕ this tier hid, are ⋯ rows.
  await page.locator('button[title="More…"]').first().click();
  const menu = page.locator('.ctx-menu');
  await expectFullyInViewport(page, menu, 'pane overflow menu');
  await expect(menu.getByRole('menuitem', { name: 'Restart session' })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: 'Close session' })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: /^Agent: shell/ })).toBeDisabled();
  await page.keyboard.press('Escape');

  // Nothing in any header is clipped or off-screen.
  await expectHeadersFit(page);
  const buttons = visibleHeaderButtons(page);
  const count = await buttons.count();
  for (let i = 0; i < count; i++) await expectFullyInViewport(page, buttons.nth(i), 'pane header button');
});

test('micro panes keep the title and fold the view switch and close into ⋯', async ({ page }) => {
  test.slow();
  await columnsUntil(page, 190);
  const n = await page.locator('[data-pane-key]').count();
  await expect(page.locator('.pane-head[data-tier="micro"]')).toHaveCount(n);

  // The title never leaves the header; the flip button does.
  await expect(page.locator('.pane-title')).toHaveCount(n);
  await expect(page.locator('.pane-title').first()).toBeVisible();
  await expect(page.locator('[data-view-toggle]:visible')).toHaveCount(0);
  await expectHeadersFit(page);

  await page.locator('button[title="More…"]').first().click();
  const menu = page.locator('.ctx-menu');
  await expectFullyInViewport(page, menu, 'pane overflow menu');
  // The view switch is the first group of rows.
  await expect(menu.getByRole('menuitemcheckbox', { name: /^Terminal view/ })).toBeVisible();
  await menu.getByRole('menuitemcheckbox', { name: /^Chat view/ }).click();
  await expect(page.locator('.pane-body[data-view="chat"]').first()).toBeVisible({ timeout: 15_000 });

  // …and the ✕ is reachable from the same menu. Every pane here holds the SAME
  // session (⌘D clones the focused one), so closing its tab empties the split.
  await page.locator('button[title="More…"]').first().click();
  await menu.getByRole('menuitem', { name: 'Close session' }).click();
  await page.getByRole('button', { name: 'Delete session' }).click();
  await expect(page.locator('[data-pane-key]')).toHaveCount(0, { timeout: 15_000 });
});
