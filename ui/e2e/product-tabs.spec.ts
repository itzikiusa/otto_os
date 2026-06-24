import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { seedProductStory } from './seed-product';

// ── E2E: Product workflow-group navigation ─────────────────────────────────────
//
// The 13 per-story sub-views are bucketed into 4 workflow GROUPS (Story ·
// Discover · Deliver · Log). The top tab strip shows exactly those 4 group
// labels; picking a group reveals a secondary sub-nav of its sub-views. This
// spec seeds one story, opens it, and asserts (a) the top strip is exactly the
// 4 groups and (b) the two-step Discover → Chat navigation renders the chat tab.
//
// Desktop-width viewport so both tab strips render in the stable side-by-side
// layout regardless of the device project (mirrors product-discovery.spec.ts).
test.use({ viewport: { width: 1280, height: 900 }, actionTimeout: 12_000 });

let workspaceId = '';
// Product stories are GLOBAL across workspaces and the suite runs in parallel
// against ONE shared daemon, so we always select our story BY a unique title.
const STORY_TITLE = `E2E Tabs ${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const { storyId } = await seedProductStory(ctx, base, workspaceId);
  // A draft's title is driven by its draft version — rename via /draft.
  const r = await ctx.patch(`${base}/api/v1/product/stories/${storyId}/draft`, {
    data: { title: STORY_TITLE, body_md: `# ${STORY_TITLE}\n\nSeeded for the tab-groups E2E.` },
  });
  if (!r.ok()) throw new Error(`rename story → ${r.status()} ${await r.text()}`);
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

async function openStoryOverview(page: Page): Promise<void> {
  await page.goto('/#/product');
  await expect(page.locator('.product-page')).toBeVisible({ timeout: 30_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  const row = page.locator('.story-row', { hasText: STORY_TITLE }).first();
  await expect(row).toBeVisible({ timeout: 20_000 });
  await row.click();
  await expect(page.locator('.overview')).toBeVisible({ timeout: 20_000 });
}

test('product tabs: top strip is the 4 workflow groups; Discover → Chat renders chat', async ({
  page,
}) => {
  test.setTimeout(60_000);

  await openStoryOverview(page);

  // The TOP strip (row 2) shows exactly the 4 group labels, in order.
  const groupStrip = page.locator('.product-header-row2 .tab-strip');
  await expect(groupStrip).toBeVisible({ timeout: 15_000 });
  await expect(groupStrip.locator('.st')).toHaveText(['Story', 'Discover', 'Deliver', 'Log']);

  // Two-step navigation: pick the Discover group, then the Chat sub-tab.
  // Exact match on the group so 'Discover' doesn't grab the 'Discovery' sub.
  await page.getByRole('tab', { name: 'Discover', exact: true }).click();
  await page.getByRole('tab', { name: 'Chat' }).click();
  await expect(page.locator('.chat-tab')).toBeVisible({ timeout: 15_000 });
});
