import { test, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Git "+" repo picker with MANY registered repos (desktop-browser only).
//
// Regression 1: the picker is a context menu listing every not-yet-open repo.
// With enough registered repos the menu grew taller than the window; the
// clamp logic "flipped" it above the cursor (top = y - height → negative), so
// the whole menu rendered above the viewport — only its last item peeked out
// at the very top and nothing was clickable. The menu must instead be clamped
// INSIDE the viewport and scroll internally.
//
// The picker is now also FILTERABLE: it shows at most 12 repo rows (a "+N
// more" hint collapses the rest) with a pinned search input that narrows the
// list — so every repo stays reachable by typing, not by scrolling a
// window-height menu.
// ─────────────────────────────────────────────────────────────────────────────

const REPO_COUNT = 30; // enough rows to exceed the 800px viewport height
// One worker for the file: the repos are seeded in beforeAll, and a second
// worker would re-run it and register every repo twice.
test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  for (let i = 0; i < REPO_COUNT; i++) {
    const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-menu-'));
    const git = (...a: string[]) => execFileSync('git', ['-C', dir, ...a], { stdio: 'ignore' });
    git('init', '-q');
    git('config', 'user.email', 'e2e@otto.local');
    git('config', 'user.name', 'E2E');
    git('config', 'commit.gpgsign', 'false');
    git('commit', '-q', '--allow-empty', '-m', 'init');
    const name = `e2e-menu-${String(i).padStart(2, '0')}`;
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, {
      data: { path: dir, name },
    });
    if (!r.ok()) throw new Error(`repo seed failed: ${r.status()} ${await r.text()}`);
  }
  await ctx.dispose();
});

test('add-repo picker stays inside the viewport and every entry is reachable', async ({ page }) => {
  await openPage(page, 'git');

  await page.locator('.git-tab-new').click();
  const menu = page.locator('.ctx-menu');
  await expect(menu).toBeVisible();
  // Give the post-render clamp (rAF) a beat to land before measuring.
  await page.waitForTimeout(100);

  await expectFullyInViewport(page, menu, 'add-repo picker menu');

  // The pinned "add" actions at the top must be visible…
  await expect(menu.getByRole('menuitem', { name: 'Clone a repository…' })).toBeVisible();

  // …the repo list is capped (12 rows + the 3 pinned actions) with a "+N more"
  // hint instead of a window-height scroll…
  const rows = menu.getByRole('menuitem');
  await expect(rows).toHaveCount(12 + 3);
  await expect(menu.locator('.ctx-more')).toContainText('more');

  // …and the LAST repo (hidden by the cap) is reachable via the search input
  // and actually works: narrowing to it and clicking opens that repo as a tab.
  const lastName = `e2e-menu-${REPO_COUNT - 1}`;
  await menu.locator('.ctx-search-input').fill(lastName);
  const last = menu.getByRole('menuitem', { name: lastName });
  await expect(last).toBeVisible();
  await last.click();
  await expect(page.locator('.git-tab-name', { hasText: lastName })).toBeVisible();
});

// Regression 2: with several repos open, the tab strip overflowed the header's
// inline-tabs slot and the trailing + (the last child of the SAME scroller) was
// clipped at its edge. Only the tab row scrolls now; + and auto-fetch are
// pinned after it and must be fully inside the slot, however many tabs.
test('+ stays fully visible when many repo tabs are open', async ({ page }) => {
  await openPage(page, 'git');
  for (let i = 0; i < 8; i++) {
    await page.locator('.git-tab-new').click();
    const menu = page.locator('.ctx-menu');
    await menu.locator('.ctx-search-input').fill(`e2e-menu-${String(i).padStart(2, '0')}`);
    await menu.getByRole('menuitem').filter({ hasText: 'e2e-menu-' }).first().click();
    await expect(menu).toBeHidden();
  }
  // ≥ 8: the previous test's opened tab may persist.
  await expect.poll(() => page.locator('.git-tab').count(), { timeout: 10_000 }).toBeGreaterThanOrEqual(8);
  // The tab row really does overflow (otherwise this proves nothing)…
  const overflows = await page
    .locator('.git-tablist')
    .evaluate((el) => el.scrollWidth > el.clientWidth + 1);
  expect(overflows).toBe(true);
  // …yet + sits wholly inside the header's inline-tabs slot.
  const within = await page.locator('.git-tab-new').evaluate((btn) => {
    const slot = btn.closest('.ph-tabs-inline') ?? btn.parentElement!;
    const b = btn.getBoundingClientRect();
    const s = slot.getBoundingClientRect();
    return b.left >= s.left - 0.5 && b.right <= s.right + 0.5 && b.width >= 20;
  });
  expect(within).toBe(true);
  await expectFullyInViewport(page, page.locator('.git-tab-new'), 'open-repo + button');
  // …and the strip USES the header's free width (the Git header has no
  // actions of its own): it used to stop at 50% of the row, ellipsizing
  // repo names beside an empty half-row. Its trailing edge must reach
  // close to the row's end.
  const gap = await page.locator('.git-tab-new').evaluate((btn) => {
    const row = btn.closest('.ph-row') as HTMLElement;
    return row.getBoundingClientRect().right - btn.getBoundingClientRect().right;
  });
  expect(gap).toBeLessThan(80);
});
