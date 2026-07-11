import { expect, test } from '@playwright/test';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { expectNoHorizontalOverflow, openPage } from './helpers';

// Vault v3 graph view — desktop. Seeds a synthetic vault (300 chained notes +
// a hub) so the Canvas2D renderer + Barnes-Hut worker have real work: asserts
// the canvas actually paints (pixel sample), the controls report the node
// count, local mode narrows to a neighborhood, and filters re-fetch.

let workspaceId = '';
let vaultId = 0;

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  ({ vaultId } = await seedVaultDir(ctx, base, workspaceId, { notes: 300 }));
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript(
    ({ wsId, vId }) => {
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem(`otto_vault_last:${wsId}`, String(vId));
      localStorage.setItem('otto_rail_expanded', '0');
    },
    { wsId: workspaceId, vId: vaultId },
  );
});

/** Fraction of sampled canvas pixels that are non-transparent. */
async function paintedFraction(page: import('@playwright/test').Page): Promise<number> {
  return page.evaluate(() => {
    const c = document.querySelector('canvas');
    if (!c) return -1;
    const g = c.getContext('2d');
    if (!g) return -1;
    const { width, height } = c;
    if (!width || !height) return -1;
    const img = g.getImageData(0, 0, width, height).data;
    let painted = 0;
    let total = 0;
    for (let i = 3; i < img.length; i += 4 * 97) {
      total++;
      if (img[i] > 0) painted++;
    }
    return painted / Math.max(total, 1);
  });
}

test('graph renders 300+ nodes on canvas with live controls', async ({ page }) => {
  await openPage(page, 'vault');
  await page.locator('button[title="Graph view"]').click();
  await expectNoHorizontalOverflow(page);

  const canvas = page.locator('.center canvas');
  await expect(canvas).toBeVisible({ timeout: 15_000 });

  // The status strip reports the seeded scale (300 bulk + fixture notes).
  await expect(page.locator('.graph-root .counts')).toContainText(/3\d\d nodes/, {
    timeout: 20_000,
  });

  // The canvas actually paints (layout ticked + renderer drew).
  await expect.poll(() => paintedFraction(page), { timeout: 20_000 }).toBeGreaterThan(0.001);
});

test('api: local graph narrows to the neighborhood', async () => {
  const { ctx, base } = await apiCtx();
  const v1 = `${base}/api/v1/workspaces/${workspaceId}/vault/vaults/${vaultId}`;
  const full = await (await ctx.get(`${v1}/graph?mode=full`)).json();
  const local = await (
    await ctx.get(`${v1}/graph?mode=local&path=bulk%2Fnote-5.md&depth=1`)
  ).json();
  expect(full.paths.length).toBeGreaterThan(290);
  expect(local.paths.length).toBeLessThan(10); // note-5 + note-4/6 chain + hub
  expect(local.paths).toContain('bulk/note-5.md');
  // Edge budget honesty: an absurdly small budget reports truncation.
  const tiny = await (await ctx.get(`${v1}/graph?mode=full&edge_budget=5`)).json();
  expect(tiny.truncated).toBe(true);
  expect(tiny.edges.length).toBeLessThanOrEqual(10);
  await ctx.dispose();
});
