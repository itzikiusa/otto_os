import { test, expect, type Page, type BrowserContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { seedProductStory } from './seed-product';

// ── E2E: talk-to-agent story refinement (the Refine tab) ───────────────────────
//
// Opens a seeded story's Overview, switches to the Refine tab, creates a new
// refinement thread, sends a chat message, and asserts the request round-trip +
// persistence — never specific agent wording (the agent reply is
// non-deterministic on the throwaway daemon).
//
// Throwaway-daemon reality (verified against target/debug/ottod): global-setup
// points the agent runner at a non-existent CLI (CLAUDE_BIN=/nonexistent…), so a
// refinement turn's `run_agent` fails fast — the daemon returns HTTP 500 for the
// turn in ~2ms. CRUCIALLY the backend persists the USER message BEFORE invoking
// the agent, so it survives even when the turn errors. The UI sends optimistically
// (PO bubble + "thinking…" + disabled input appear immediately), then on the 500
// it rolls the optimistic bubble back and surfaces a toast. The durable proof of
// the round-trip is therefore the RELOAD: the user message was written
// server-side and re-renders after the daemon round-trip.
//
// The genuine turn is sub-2ms, far too fast to observe the in-flight UI. To make
// the optimistic + "thinking…" state observable we DELAY delivery of the real
// turn response to the browser via context.route + route.continue() (page.route
// does not intercept the UI's cross-origin fetch to the 127.0.0.1 daemon —
// context.route does). We do NOT fabricate the body: the genuine daemon response
// still drives the UI, and no client-side fetch timeout is introduced anywhere.
//
// Desktop-width viewport so the Overview toolbar + the (horizontally scrollable)
// tab strip render in the stable side-by-side layout regardless of the device
// project — mirrors product-discovery.spec.ts.
//
// serviceWorkers: 'block' is REQUIRED here: the UI registers a service worker
// that proxies fetches, which bypasses Playwright network interception; blocking
// it lets context.route below delay the turn-response delivery so the (otherwise
// sub-2ms) in-flight UI is observable. It does not affect the feature under test.
test.use({ viewport: { width: 1280, height: 900 }, actionTimeout: 12_000, serviceWorkers: 'block' });

let workspaceId = '';
// Product stories are GLOBAL across workspaces and the suite runs in parallel
// against ONE shared daemon, so we always select our story BY a unique title.
const STORY_TITLE = `E2E Refine ${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
// A unique message body so the persisted-message assertion can't match unrelated
// content from a sibling spec running against the same daemon.
const PO_MESSAGE = `Please sharpen the acceptance criteria ${Math.random()
  .toString(36)
  .slice(2, 8)}`;

// Glob for the turn POST only (no list/detail GET ends in /messages). The
// shorter `**`-prefixed form matches the UI's cross-origin daemon URL reliably;
// the fully-qualified `**/api/v1/product/...` form did not match in PW 1.61.
const MESSAGES_ROUTE = '**/refinement-threads/*/messages';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const { storyId } = await seedProductStory(ctx, base, workspaceId);
  // A draft's title is driven by its draft version — rename via /draft.
  const r = await ctx.patch(`${base}/api/v1/product/stories/${storyId}/draft`, {
    data: { title: STORY_TITLE, body_md: `# ${STORY_TITLE}\n\nSeeded for the refinement E2E.` },
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

async function openRefineTab(page: Page): Promise<void> {
  // The tab strip is horizontally scrollable; Playwright auto-scrolls the tab
  // into view before clicking. The accessible name is the tab label.
  const refineTab = page.getByRole('tab', { name: 'Refine' });
  await expect(refineTab).toBeVisible({ timeout: 15_000 });
  await refineTab.click();
  await expect(page.locator('.refine-tab')).toBeVisible({ timeout: 15_000 });
}

test('refine: new thread → send → optimistic PO bubble + thinking → settles → persists', async ({
  page,
  context,
}: {
  page: Page;
  context: BrowserContext;
}) => {
  // The send POST runs the agent turn synchronously; allow generous headroom over
  // the default test timeout (the artificial response delay also counts here).
  test.setTimeout(120_000);

  // Register the turn-response delay UP FRONT (before any navigation) so it is in
  // effect by the time we send. context.route (not page.route) is required to
  // intercept the UI's cross-origin fetch to the 127.0.0.1 daemon. The glob
  // matches ONLY the send POST (the list/detail GETs don't end in /messages), so
  // it never slows the other calls. route.continue() forwards the GENUINE request
  // to the daemon — the real response still drives the UI and the server persists
  // the user message before replying — we only delay delivery to the browser so
  // the optimistic + "thinking…" state is observable.
  await context.route(MESSAGES_ROUTE, async (route) => {
    await new Promise((r) => setTimeout(r, 2000));
    await route.continue();
  });

  await openStoryOverview(page);
  await openRefineTab(page);

  // ── 1. Create a new thread → it appears and is auto-selected (chat renders) ──
  const newThreadBtn = page.getByRole('button', { name: /New thread/ });
  await expect(newThreadBtn).toBeVisible({ timeout: 15_000 });
  await newThreadBtn.click();

  // A thread row appears in the list AND becomes the active (selected) one.
  await expect(page.locator('.thread-item').first()).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.thread-item.active')).toBeVisible({ timeout: 20_000 });
  // Selecting a thread renders the chat pane.
  await expect(page.locator('.refine-chat')).toBeVisible({ timeout: 15_000 });

  // ── 2. Type a message + press Enter → optimistic PO bubble + thinking state ──
  const input = page.locator('.msg-input');
  await expect(input).toBeVisible({ timeout: 10_000 });
  await input.click();
  await input.fill(PO_MESSAGE);
  await input.press('Enter');

  // The PO (user) bubble appears immediately (optimistic, before the turn ends).
  await expect(page.locator('.bubble-user', { hasText: PO_MESSAGE })).toBeVisible({
    timeout: 5_000,
  });
  // While the synchronous turn is in flight the input is disabled and a
  // "thinking…" indicator bubble is shown.
  await expect(input).toBeDisabled({ timeout: 5_000 });
  await expect(page.locator('.refine-chat .thinking')).toBeVisible({ timeout: 5_000 });

  // ── 3. The turn settles → input re-enables; the round-trip completed ────────
  // The store imposes no tight client fetch timeout, so the turn always
  // completes; the input returns to enabled once `sending` clears. (On the
  // throwaway daemon the turn 500s, so the optimistic bubble is rolled back here
  // and the durable persistence is asserted via the reload below — the real
  // proof, since the user message was written server-side before the agent call.)
  await expect(input).toBeEnabled({ timeout: 90_000 });
  await expect(page.locator('.refine-chat .thinking')).toHaveCount(0, { timeout: 10_000 });

  // Drop the delay so the reload's GETs aren't slowed.
  await context.unroute(MESSAGES_ROUTE);

  // ── 4. Reload → reopen Refine → reselect the thread → user message persisted ─
  // The backend persisted the user message BEFORE the agent call, so it survives
  // the round-trip regardless of how the turn itself resolved. This is the
  // durable proof that the thread + its messages live server-side.
  await page.reload();
  await expect(page.locator('.product-page')).toBeVisible({ timeout: 30_000 });
  await openStoryOverview(page);
  await openRefineTab(page);

  const persistedThread = page.locator('.thread-item').first();
  await expect(persistedThread).toBeVisible({ timeout: 20_000 });
  await persistedThread.locator('.thread-btn').click();
  await expect(page.locator('.refine-chat')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.bubble-user', { hasText: PO_MESSAGE })).toBeVisible({
    timeout: 20_000,
  });
});
