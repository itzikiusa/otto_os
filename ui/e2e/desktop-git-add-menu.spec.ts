import { test, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Git "+" repo picker with MANY registered repos (desktop-browser only).
//
// Regression: the picker is a context menu listing every not-yet-open repo.
// With enough registered repos the menu grew taller than the window; the
// clamp logic "flipped" it above the cursor (top = y - height → negative), so
// the whole menu rendered above the viewport — only its last item peeked out
// at the very top and nothing was clickable. The menu must instead be clamped
// INSIDE the viewport and scroll internally, keeping every entry reachable.
// ─────────────────────────────────────────────────────────────────────────────

const REPO_COUNT = 30; // enough rows to exceed the 800px viewport height

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

  const box = await menu.boundingBox();
  const viewport = page.viewportSize()!;
  expect(box, 'menu should render').not.toBeNull();
  expect(box!.y, 'menu top must be inside the viewport').toBeGreaterThanOrEqual(0);
  expect(box!.y + box!.height, 'menu bottom must be inside the viewport').toBeLessThanOrEqual(
    viewport.height,
  );

  // The static "add" actions at the top must be visible…
  await expect(menu.getByRole('menuitem', { name: 'Clone a repository…' })).toBeVisible();

  // …and the LAST repo entry must be reachable (menu scrolls internally) and
  // actually work: clicking it opens that repo as a tab.
  const last = menu.getByRole('menuitem', { name: `e2e-menu-${REPO_COUNT - 1}` });
  await last.scrollIntoViewIfNeeded();
  await last.click();
  await expect(
    page.locator('.git-tab-name', { hasText: `e2e-menu-${REPO_COUNT - 1}` }),
  ).toBeVisible();
});
