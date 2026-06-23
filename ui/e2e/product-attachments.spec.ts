import { test, expect, type Page } from '@playwright/test';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { seedProductStory } from './seed-product';

// ── E2E: story attachments (upload + preview + persist + delete) ───────────────
//
// Seeds one real story into the isolated throwaway daemon, opens its Overview,
// uploads a tiny PNG through the AttachmentsPanel file input, asserts a thumbnail
// renders, reloads to prove persistence, then deletes it and asserts it's gone.
//
// These flows involve a file picker, image preview and a confirm dialog that are
// awkward inside the phone accordion, so every test forces a desktop-width
// viewport (the page's mobile breakpoint is ≤640px) regardless of the device
// project it runs under — giving the stable side-by-side Overview layout. We
// override ONLY the viewport: the mobile projects run on WebKit, where the
// `isMobile` context option is unsupported (and the layout is width-driven, so
// 1280px is enough to get the desktop layout).
// Bound each action so a non-actionable element fails fast instead of hanging.
test.use({ viewport: { width: 1280, height: 900 }, actionTimeout: 12_000 });

const PNG_FIXTURE = join(process.cwd(), 'e2e', 'fixtures', 'tiny.png');

let workspaceId = '';
// Product stories are GLOBAL across workspaces (the daemon's list_stories is not
// workspace-filtered — see otto-state/src/product.rs), and the suite runs many
// specs in parallel against ONE shared throwaway daemon. So we rename our seeded
// story to a unique title and always select it BY TITLE — never `.first()`.
const STORY_TITLE = `E2E Attachments ${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const { storyId } = await seedProductStory(ctx, base, workspaceId);
  // Give this story a unique, selectable title. For a draft the story title is
  // driven by its draft version, so rename via the /draft endpoint (the plain
  // story PATCH does not change a draft's title).
  const r = await ctx.patch(`${base}/api/v1/product/stories/${storyId}/draft`, {
    data: { title: STORY_TITLE, body_md: `# ${STORY_TITLE}\n\nSeeded for the attachments E2E.` },
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

/** Open Product, select OUR seeded story by its unique title, land on Overview. */
async function openStoryOverview(page: Page): Promise<void> {
  await page.goto('/#/product');
  await expect(page.locator('.product-page')).toBeVisible({ timeout: 30_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  // Select OUR story row by unique title (the list is shared/global).
  const row = page.locator('.story-row', { hasText: STORY_TITLE }).first();
  await expect(row).toBeVisible({ timeout: 20_000 });
  await row.click();
  // Overview is the default tab on selection.
  await expect(page.locator('.overview')).toBeVisible({ timeout: 20_000 });
  // The AttachmentsPanel is mounted in the draft Overview right column.
  await expect(page.locator('.att-panel')).toBeVisible({ timeout: 15_000 });
}

test('attachments: upload a PNG → thumbnail renders → persists across reload → delete', async ({
  page,
}) => {
  await openStoryOverview(page);

  const panel = page.locator('.att-panel');

  // ── Upload via the file input (hidden behind the "+ File" label). ──────────
  await panel.locator('input[type="file"]').setInputFiles(PNG_FIXTURE);

  // The uploaded image renders as a thumbnail (<img class="att-img">) once the
  // authed blob URL resolves.
  const thumb = panel.locator('img.att-img');
  await expect(thumb).toBeVisible({ timeout: 15_000 });
  // The attachment row carries the fixture filename.
  await expect(panel.locator('.att-fname', { hasText: 'tiny.png' })).toBeVisible();
  // Exactly one attachment item is present.
  await expect(panel.locator('.att-item')).toHaveCount(1);

  // ── Reload → the attachment persists (server round-trip, not optimistic). ──
  await page.reload();
  await expect(page.locator('.product-page')).toBeVisible({ timeout: 30_000 });
  // After a reload no story is selected; re-open the Overview.
  await openStoryOverview(page);

  const panel2 = page.locator('.att-panel');
  await expect(panel2.locator('.att-fname', { hasText: 'tiny.png' })).toBeVisible({ timeout: 15_000 });
  await expect(panel2.locator('img.att-img')).toBeVisible({ timeout: 15_000 });

  // ── Delete it → confirm dialog → gone. ────────────────────────────────────
  await panel2.locator('.att-delete-btn').first().click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible({ timeout: 8_000 });
  await dialog.getByRole('button', { name: 'Delete' }).click();

  // The item disappears and the empty drop-hint returns.
  await expect(panel2.locator('.att-item')).toHaveCount(0, { timeout: 10_000 });
  await expect(panel2.locator('.att-drop-hint')).toBeVisible({ timeout: 10_000 });
});
