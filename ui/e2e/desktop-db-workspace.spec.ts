import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDockerConnection } from './seed';
import { expectFullyInViewport, openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — query-workspace real estate + object search.
//
//   • ⌘B collapses the schema sidebar to its rail and restores it (the rail must
//     never be zero-width, or the sidebar is unrecoverable),
//   • the editor keeps a per-tab height instead of one shared value,
//   • the search scope picker exists and a scoped catalog lookup finds a table
//     inside a schema that was never expanded — the exact case the old
//     client-side filter could not do,
//   • the tree's option row stays inside the viewport.
//
// Desktop-browser only; skips when the Docker MySQL is down (matching the other
// db-* specs — see docs: the dev stack runs MySQL on 13306).
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let mysqlConn: string | null = null;

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  try {
    mysqlConn = await seedDockerConnection(ctx, base, workspaceId, 'mysql');
  } catch {
    mysqlConn = null;
  }
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }) => {
  test.skip(!mysqlConn, 'Docker MySQL (127.0.0.1:13306) is not up');
  await page.addInitScript(
    ({ wsId }) => {
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem('otto_rail_expanded', '0');
      // Both are sticky preferences — start from a known state.
      localStorage.setItem('db.sidebarCollapsed', '0');
      localStorage.setItem('db.showCounts', '0');
    },
    { wsId: workspaceId },
  );
});

async function openWorkbench(page: import('@playwright/test').Page): Promise<void> {
  await openPage(page, 'database');
  await page.locator('.db-side .conn-row, .db-side button', { hasText: 'e2e-mysql' })
    .first()
    .click();
  await expect(page.locator('.schema-tree')).toBeVisible({ timeout: 30_000 });
}

test('⌘B collapses the schema sidebar to a rail and back', async ({ page }) => {
  await openWorkbench(page);
  const side = page.locator('aside.db-side');
  await expect(side).toBeVisible();

  await page.keyboard.press('Meta+b');
  const rail = page.locator('.side-rail');
  await expect(rail).toBeVisible();
  // The rail must keep real width — a zero-width sidebar cannot be brought back.
  const box = await rail.boundingBox();
  expect(box!.width).toBeGreaterThan(10);
  await expectFullyInViewport(page, rail, 'collapsed sidebar rail');

  // Restoring works from the rail button as well as the chord.
  await rail.locator('button').click();
  await expect(page.locator('.schema-tree')).toBeVisible();
  await expect(rail).toHaveCount(0);
});

test('the editor keeps its own height per query tab', async ({ page }) => {
  await openWorkbench(page);
  const editor = page.locator('.qe-edit');
  await expect(editor).toBeVisible();
  const first = (await editor.boundingBox())!.height;

  // Drag the splitter up to shrink this tab's editor.
  const splitter = page.locator('.qe-splitter');
  const sb = (await splitter.boundingBox())!;
  await page.mouse.move(sb.x + sb.width / 2, sb.y + sb.height / 2);
  await page.mouse.down();
  await page.mouse.move(sb.x + sb.width / 2, sb.y - 120, { steps: 8 });
  await page.mouse.up();
  const shrunk = (await editor.boundingBox())!.height;
  expect(shrunk).toBeLessThan(first);

  // A new tab must NOT inherit this tab's hand-set height as its own state:
  // switching back has to restore what we dragged to.
  await page.keyboard.press('Alt+Meta+t');
  await page.keyboard.press('Alt+Meta+ArrowLeft');
  await expect.poll(async () => (await editor.boundingBox())!.height).toBeCloseTo(shrunk, -1);
});

test('scoped search finds a table without expanding its schema', async ({ page }) => {
  await openWorkbench(page);

  const scope = page.locator('.scope-pick');
  await expect(scope).toBeVisible();
  await scope.selectOption('all');

  // `shopdb` is the seeded Docker database; nothing has been expanded yet, so
  // this can only succeed via the server-side catalog lookup.
  await page.locator('.tree-search-input').fill('order');
  await expect(page.locator('.hit').first()).toBeVisible({ timeout: 20_000 });
  const hits = page.locator('.hit');
  expect(await hits.count()).toBeGreaterThan(0);
  await expect(hits.first()).toContainText(/order/i);
  // Every hit is labelled with its schema so same-named tables are tellable apart.
  await expect(page.locator('.hit-schema').first()).not.toBeEmpty();

  // Clearing returns the browsable tree.
  await page.locator('.tree-search-clear').click();
  await expect(page.locator('.hit')).toHaveCount(0);
});

test('the tree option row stays inside the viewport', async ({ page }) => {
  await openWorkbench(page);
  const opts = page.locator('.tree-opts');
  await expect(opts).toBeVisible();
  await expectFullyInViewport(page, opts, 'schema tree options row');
});

test('api: search-objects is scoped, capped and honest about cost', async () => {
  test.skip(!mysqlConn, 'Docker MySQL is not up');
  const { ctx, base } = await apiCtx();
  const url = `${base}/api/v1/connections/${mysqlConn}/db/search-objects`;

  const all = await (await ctx.post(url, { data: { q: 'a', scope: 'all' } })).json();
  expect(all.supported).toBe(true);
  expect(Array.isArray(all.hits)).toBe(true);
  // System catalogs must never leak into results.
  for (const h of all.hits) {
    expect(['information_schema', 'performance_schema', 'mysql', 'sys']).not.toContain(h.schema);
    expect(h.path).toContain(`db:${h.schema}`);
  }

  // A blank needle returns nothing rather than dumping the whole catalog.
  const blank = await (await ctx.post(url, { data: { q: '  ', scope: 'all' } })).json();
  expect(blank.hits).toHaveLength(0);

  // The cap is honored and reports itself.
  const capped = await (await ctx.post(url, { data: { q: 'a', scope: 'all', limit: 1 } })).json();
  expect(capped.hits.length).toBeLessThanOrEqual(1);
  await ctx.dispose();
});
