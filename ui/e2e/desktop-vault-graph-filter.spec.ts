import { expect, test } from '@playwright/test';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { expectFullyInViewport, openPage } from './helpers';

// Vault graph focus filters — desktop. Seeds a vault with 24 synthetic service
// bundles (Service + Flow notes each) plus bulk References, so the graph is a
// hairball worth narrowing. Asserts the client-side projection: type/service
// filtering shrinks the node set without a refetch, the service rollup collapses
// to one node per bundle, reset restores, and the facet lists stay inside the
// viewport instead of growing the panel past the window.

let workspaceId = '';
let vaultId = 0;

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  ({ vaultId } = await seedVaultDir(ctx, base, workspaceId, { notes: 40, services: 24 }));
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript(
    ({ wsId, vId }) => {
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem(`otto_vault_last:${wsId}`, String(vId));
      localStorage.setItem('otto_rail_expanded', '0');
      // Start every test from a clean focus — but ONCE. addInitScript re-runs on
      // every navigation, so an unguarded remove would wipe the sticky filter
      // the reload test is asserting. sessionStorage is per-context, so each
      // test still starts clean.
      if (!sessionStorage.getItem('e2e-focus-cleared')) {
        localStorage.removeItem(`otto.vault.graph.filter.v1.${vId}`);
        sessionStorage.setItem('e2e-focus-cleared', '1');
      }
    },
    { wsId: workspaceId, vId: vaultId },
  );
});

/** Nodes currently rendered, read off the status strip ("312 / 9,179 nodes"). */
async function nodeCount(page: import('@playwright/test').Page): Promise<number> {
  const text = (await page.locator('.graph-root .counts').innerText()).replace(/,/g, '');
  const m = text.match(/([\d]+)\s*(?:\/\s*[\d]+\s*)?nodes/);
  return m ? Number(m[1]) : -1;
}

async function openGraph(page: import('@playwright/test').Page): Promise<void> {
  await openPage(page, 'vault');
  await page.locator('button[title="Graph view"]').click();
  await expect(page.locator('.center canvas')).toBeVisible({ timeout: 15_000 });
  await expect.poll(() => nodeCount(page), { timeout: 20_000 }).toBeGreaterThan(60);
}

test('type filter narrows the graph without refetching', async ({ page }) => {
  const requests: string[] = [];
  page.on('request', (r) => {
    if (r.url().includes('/graph?')) requests.push(r.url());
  });

  await openGraph(page);
  const before = await nodeCount(page);
  const fetchesBefore = requests.length;

  // Keep only Flow notes: 24 seeded flows out of ~90 notes.
  await page.locator('.facets label:has(.fl[title="Flow"]) input').check();
  await expect.poll(() => nodeCount(page), { timeout: 10_000 }).toBeLessThan(before);
  const filtered = await nodeCount(page);
  expect(filtered).toBeGreaterThan(0);

  // The counts switch to "shown / total" once a focus is active.
  await expect(page.locator('.graph-root .counts')).toContainText(`/ ${before}`);

  // Projection is client-side: no new /graph request was issued.
  expect(requests.length).toBe(fetchesBefore);

  // Reset restores the full graph.
  await page.locator('.panel .link', { hasText: 'reset' }).first().click();
  await expect.poll(() => nodeCount(page), { timeout: 10_000 }).toBe(before);
  expect(requests.length).toBe(fetchesBefore);
});

test('service rollup collapses to one node per bundle and drills back open', async ({ page }) => {
  await openGraph(page);
  const before = await nodeCount(page);

  await page.locator('.panel select').first().selectOption('service');
  await expect.poll(() => nodeCount(page), { timeout: 10_000 }).toBeLessThan(before / 2);
  const rolled = await nodeCount(page);
  // 24 seeded services + services/ + runbooks/ + bulk/ (+ tag/ghost buckets).
  expect(rolled).toBeGreaterThan(20);
  expect(rolled).toBeLessThan(40);

  await expect(page.locator('.panel .hint')).toContainText('expand that group');
});

test('facet lists scroll instead of pushing the panel off-screen', async ({ page }) => {
  await openGraph(page);

  const panel = page.locator('.graph-root .panel');
  await expectFullyInViewport(page, panel, 'graph controls panel');

  // 27 services is more than fits — the list must cap and scroll, not grow.
  const facets = page.locator('.panel .facets').first();
  await expect(facets).toBeVisible();
  const overflows = await facets.evaluate((el) => el.scrollHeight > el.clientHeight + 1);
  expect(overflows, 'seeded services should overflow the facet list').toBe(true);
  await expectFullyInViewport(page, facets, 'service facet list');
});

test('focus survives a reload (sticky per vault)', async ({ page }) => {
  await openGraph(page);
  const before = await nodeCount(page);
  await page.locator('.facets label:has(.fl[title="Flow"]) input').check();
  await expect.poll(() => nodeCount(page), { timeout: 10_000 }).toBeLessThan(before);
  const filtered = await nodeCount(page);

  await page.reload();
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  // The view toggle may already have restored to graph mode — clicking again
  // would turn it back off.
  if (!(await page.locator('.center canvas').isVisible())) {
    await page.locator('button[title="Graph view"]').click();
  }
  await expect(page.locator('.center canvas')).toBeVisible({ timeout: 15_000 });
  await expect.poll(() => nodeCount(page), { timeout: 20_000 }).toBe(filtered);
});

test('api: graph payload carries the filterable node attributes', async () => {
  const { ctx, base } = await apiCtx();
  const v1 = `${base}/api/v1/workspaces/${workspaceId}/vault/vaults/${vaultId}`;
  const g = await (await ctx.get(`${v1}/graph?mode=full&tags=true`)).json();

  // Parallel arrays stay in lockstep with `paths`.
  expect(g.types.length).toBe(g.paths.length);
  expect(g.services.length).toBe(g.paths.length);
  expect(g.tag_off.length).toBe(g.paths.length + 1);
  expect(g.tag_off[g.paths.length]).toBe(g.tag_ids.length);

  // Ids index their label tables.
  expect(Math.max(...g.types)).toBeLessThan(g.type_labels.length);
  expect(Math.max(...g.services)).toBeLessThan(g.service_labels.length);
  expect(Math.max(...g.tag_ids)).toBeLessThan(g.tag_labels.length);

  // The seeded bundles show up as services, and Flow/Service as types.
  expect(g.service_labels).toContain('svc-00');
  expect(g.type_labels).toContain('Flow');
  expect(g.type_labels).toContain('Service');
  await ctx.dispose();
});
