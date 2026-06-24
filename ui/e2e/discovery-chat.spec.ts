import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { seedProductStory } from './seed-product';

// ── E2E: Discovery Chat ────────────────────────────────────────────────────────
//
// Opens a seeded story's Chat tab, starts a chat, sends a message via a starter
// prompt, and asserts the (stubbed) agent reply renders with action cards — then
// applies the "add questions" card and confirms the applied state.
//
// The throwaway daemon runs with OTTO_E2E=1, so the discovery-chat turn returns
// the deterministic stub (prose + an `actions` JSON block with apply_draft +
// add_questions) instead of spawning a real CLI. Deterministic + network-free.

test.use({ viewport: { width: 1360, height: 900 }, actionTimeout: 12_000 });

let workspaceId = '';
const STORY_TITLE = `E2E Chat ${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const { storyId } = await seedProductStory(ctx, base, workspaceId);
  const r = await ctx.patch(`${base}/api/v1/product/stories/${storyId}/draft`, {
    data: { title: STORY_TITLE, body_md: `# ${STORY_TITLE}\n\nSeeded for the discovery-chat E2E.` },
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

async function openChatTab(page: Page): Promise<void> {
  await page.goto('/#/product');
  await expect(page.locator('.product-page')).toBeVisible({ timeout: 30_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  const row = page.locator('.story-row', { hasText: STORY_TITLE }).first();
  await expect(row).toBeVisible({ timeout: 20_000 });
  await row.click();
  await expect(page.locator('.overview')).toBeVisible({ timeout: 20_000 });
  // Chat lives under the "Discover" workflow group: pick the group, then the sub.
  await page.getByRole('tab', { name: 'Discover', exact: true }).click();
  await page.getByRole('tab', { name: 'Chat' }).click();
  await expect(page.locator('.chat-tab')).toBeVisible({ timeout: 15_000 });
}

test('discovery-chat: new chat → starter prompt → agent reply with action cards → apply questions', async ({
  page,
}) => {
  test.setTimeout(90_000);
  await openChatTab(page);

  // Start a chat.
  await page.getByRole('button', { name: '+ New chat' }).click();

  // Empty state offers starter chips that prefill (not auto-send) the composer.
  const chip = page.locator('.starter-chip').first();
  await expect(chip).toBeVisible({ timeout: 15_000 });
  await chip.click();
  const input = page.locator('.msg-input');
  await expect(input).not.toHaveValue('', { timeout: 5_000 });

  // Send the turn (the stub answers deterministically).
  await page.locator('.send-btn').click();

  // The agent bubble appears…
  await expect(page.locator('.bubble-agent').last()).toBeVisible({ timeout: 20_000 });

  // …carrying action cards (apply_draft + add_questions from the stub).
  const cards = page.locator('.action-card');
  await expect.poll(() => cards.count(), { timeout: 15_000 }).toBeGreaterThanOrEqual(2);

  // Apply the "Questions" card (no confirm dialog, unlike apply_draft).
  const questionsCard = page.locator('.action-card', { hasText: 'Questions' }).first();
  await expect(questionsCard).toBeVisible();
  await questionsCard.getByRole('button', { name: /Add \d+ question/ }).click();

  // The card collapses to a sticky applied row with an Undo affordance.
  await expect(questionsCard.locator('.applied-row, .applied-text')).toBeVisible({ timeout: 12_000 });
});

test.describe('discovery-chat on a phone', () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test('discovery-chat: starter chips render on a phone', async ({ page }) => {
    test.setTimeout(60_000);
    await openChatTab(page);
    await page.getByRole('button', { name: '+ New chat' }).click();
    // Chips are present (as a horizontal scroll row on a phone).
    await expect(page.locator('.starter-chip').first()).toBeVisible({ timeout: 15_000 });
  });
});
