import { test, expect, type Page } from '@playwright/test';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { seedProductStory } from './seed-product';

// ── E2E: mockup viewer + pinned annotations ────────────────────────────────────
//
// Attaches a small HTML mockup on the Overview (file input), marks it as a
// mockup, opens the Mockups tab, selects it, and asserts it renders inside a
// SANDBOXED <iframe> (the security-critical isolation). Then it switches to
// Annotate mode, clicks the overlay to drop a pin, adds note text, reloads, and
// asserts the annotation persists (server round-trip, not optimistic state).
//
// Desktop-width viewport so the Mockups two-pane layout (list + viewer) renders
// rather than the ≤640px stacked variant, and the overlay has a large, stable
// click target. Only `viewport` is overridden — the mobile projects run on
// WebKit, where `isMobile` is unsupported, and the layout is width-driven anyway.
test.use({ viewport: { width: 1280, height: 900 }, actionTimeout: 12_000 });

const HTML_FIXTURE = join(process.cwd(), 'e2e', 'fixtures', 'mockup.html');
const NOTE_TEXT = 'E2E annotation: align the CTA to the grid';

let workspaceId = '';
// Product stories are GLOBAL across workspaces and the suite runs in parallel
// against ONE shared daemon, so we always select our story BY a unique title.
const STORY_TITLE = `E2E Mockups ${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const { storyId } = await seedProductStory(ctx, base, workspaceId);
  // A draft's title is driven by its draft version — rename via /draft.
  const r = await ctx.patch(`${base}/api/v1/product/stories/${storyId}/draft`, {
    data: { title: STORY_TITLE, body_md: `# ${STORY_TITLE}\n\nSeeded for the mockups E2E.` },
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

async function selectStory(page: Page): Promise<void> {
  await page.goto('/#/product');
  await expect(page.locator('.product-page')).toBeVisible({ timeout: 30_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  const row = page.locator('.story-row', { hasText: STORY_TITLE }).first();
  await expect(row).toBeVisible({ timeout: 20_000 });
  await row.click();
  await expect(page.locator('.overview')).toBeVisible({ timeout: 20_000 });
}

/** Switch to one of the per-story tabs by its visible label. */
async function openTab(page: Page, label: string): Promise<void> {
  await page.locator('.tab-strip .st', { hasText: label }).first().click();
}

test('mockups: attach HTML → mark as mockup → renders in sandboxed iframe → annotate persists', async ({
  page,
}) => {
  test.setTimeout(90_000);

  await selectStory(page);

  // ── 1. Attach the HTML mockup via the Overview attachments panel. ──────────
  const panel = page.locator('.att-panel');
  await expect(panel).toBeVisible({ timeout: 15_000 });
  await panel.locator('input[type="file"]').setInputFiles(HTML_FIXTURE);

  // The attachment row appears (HTML → file chip, not an image thumbnail).
  await expect(panel.locator('.att-fname', { hasText: 'mockup.html' })).toBeVisible({
    timeout: 15_000,
  });

  // ── 2. Mark it as a mockup. ────────────────────────────────────────────────
  await panel.getByRole('button', { name: 'Mark as mockup' }).click();
  // The row flips to a "mockup" badge.
  await expect(panel.locator('.mockup-badge')).toBeVisible({ timeout: 10_000 });

  // ── 3. Open the Mockups tab and select the mockup. ─────────────────────────
  await openTab(page, 'Mockups');
  await expect(page.locator('.mockups-tab')).toBeVisible({ timeout: 15_000 });

  const row = page.locator('.mockup-row', { hasText: 'mockup.html' }).first();
  await expect(row).toBeVisible({ timeout: 15_000 });
  await row.click();

  // ── 4. Assert the mockup renders inside a SANDBOXED iframe. ────────────────
  const frame = page.locator('.mockup-stage iframe.mockup-frame');
  await expect(frame).toBeVisible({ timeout: 15_000 });
  // The security-critical isolation: the iframe must carry a `sandbox` attribute
  // (HTML mockups default to the most-restrictive empty sandbox).
  await expect(frame).toHaveAttribute('sandbox');

  // ── 5. Annotate: drop a pin on the overlay, add note text, save. ───────────
  // Annotate is the default mode; the overlay captures clicks (pointer-events).
  const overlay = page.locator('.overlay.annotate');
  await expect(overlay).toBeVisible({ timeout: 10_000 });
  // Click offset from the corner so the inline editor has room and we don't land
  // exactly on the (0,0) edge.
  await overlay.click({ position: { x: 120, y: 90 } });

  const editor = page.locator('.editor');
  await expect(editor).toBeVisible({ timeout: 8_000 });
  await editor.locator('textarea').fill(NOTE_TEXT);
  await editor.getByRole('button', { name: 'Add' }).click();

  // A numbered pin renders on the overlay and the note appears in the side list.
  await expect(page.locator('.overlay .pin').first()).toBeVisible({ timeout: 10_000 });
  await expect(page.locator('.note-body', { hasText: NOTE_TEXT })).toBeVisible({ timeout: 10_000 });

  // ── 6. Reload → re-select the mockup → annotation persisted server-side. ───
  await page.reload();
  await expect(page.locator('.product-page')).toBeVisible({ timeout: 30_000 });
  await selectStory(page);
  await openTab(page, 'Mockups');
  await expect(page.locator('.mockups-tab')).toBeVisible({ timeout: 15_000 });
  await page.locator('.mockup-row', { hasText: 'mockup.html' }).first().click();
  await expect(page.locator('.mockup-stage iframe.mockup-frame')).toBeVisible({ timeout: 15_000 });

  // The persisted note re-appears in the list (and a pin on the overlay).
  await expect(page.locator('.note-body', { hasText: NOTE_TEXT })).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.overlay .pin').first()).toBeVisible({ timeout: 10_000 });
});
