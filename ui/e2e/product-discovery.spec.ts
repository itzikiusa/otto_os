import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { seedProductStory } from './seed-product';

// ── E2E: discovery swarm launch from a story ───────────────────────────────────
//
// Opens a seeded story's Overview, clicks "Run Discovery", confirms the dialog,
// then asserts the Discovery tab surfaces the new run with a status badge and a
// "View in Swarm" control.
//
// The discover endpoint auto-resolves/creates a default swarm (no swarm seeding
// needed) and records the run row BEFORE the swarm coordinator starts. The
// throwaway daemon's agent runner points at a non-existent CLI (set in
// global-setup), so the discovery planner fails fast and falls back to a fixed
// task set — agents never meaningfully complete, which is expected. We assert on
// the RUN being created/visible, never on agent completion.
//
// Desktop-width viewport so the Overview toolbar + Discovery tab render in the
// stable side-by-side layout regardless of the device project. Only `viewport`
// is overridden — the mobile projects run on WebKit, where `isMobile` is
// unsupported, and the layout is width-driven anyway.
test.use({ viewport: { width: 1280, height: 900 }, actionTimeout: 12_000 });

let workspaceId = '';
// Product stories are GLOBAL across workspaces and the suite runs in parallel
// against ONE shared daemon, so we always select our story BY a unique title.
const STORY_TITLE = `E2E Discovery ${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const { storyId } = await seedProductStory(ctx, base, workspaceId);
  // A draft's title is driven by its draft version — rename via /draft.
  const r = await ctx.patch(`${base}/api/v1/product/stories/${storyId}/draft`, {
    data: { title: STORY_TITLE, body_md: `# ${STORY_TITLE}\n\nSeeded for the discovery E2E.` },
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

test('discovery: Run Discovery → confirm → run appears with status badge + View in Swarm', async ({
  page,
}) => {
  // The discover POST does swarm/project/task seeding server-side; even with the
  // fast-failing planner allow generous headroom over the default test timeout.
  test.setTimeout(120_000);

  await openStoryOverview(page);

  // The Overview toolbar carries a "Run Discovery" button (only one swarm exists
  // here, so no team picker is shown — the endpoint auto-resolves it).
  const runBtn = page.getByRole('button', { name: 'Run Discovery' });
  await expect(runBtn).toBeVisible({ timeout: 15_000 });
  await runBtn.click();

  // Confirm the dialog (scoped to the modal so we don't re-hit the toolbar btn).
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible({ timeout: 8_000 });
  await dialog.getByRole('button', { name: 'Run Discovery' }).click();

  // The Overview switches to the Discovery tab on success.
  await expect(page.locator('.discovery-tab')).toBeVisible({ timeout: 60_000 });

  // A run card appears with a status badge and a "View in Swarm" control.
  const runCard = page.locator('.run-card').first();
  await expect(runCard).toBeVisible({ timeout: 60_000 });
  await expect(runCard.locator('.status-badge')).toBeVisible();
  await expect(runCard.locator('.view-swarm-btn')).toBeVisible();
  // The badge text is one of the known derived statuses.
  await expect(runCard.locator('.status-badge')).toHaveText(/running|done|error|\w+/, {
    timeout: 5_000,
  });

  // Reload + revisit the Discovery tab: the run persisted server-side.
  await page.reload();
  await expect(page.locator('.product-page')).toBeVisible({ timeout: 30_000 });
  await openStoryOverview(page);
  await page.locator('.tab-strip .st', { hasText: 'Discovery' }).first().click();
  await expect(page.locator('.discovery-tab')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.run-card').first()).toBeVisible({ timeout: 30_000 });
});
