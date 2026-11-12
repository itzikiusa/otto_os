import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { seedProductStory } from './seed-product';
import { expectNoHorizontalOverflow } from './helpers';

// ── E2E: the merged story Chat (former Chat + Refine tabs) ─────────────────────
//
// One "Chat" sub-tab under Discover (no "Refine"). A turn on the throwaway
// daemon (OTTO_E2E=1) returns the deterministic discovery stub: its proposed
// story body becomes a SUGGESTED version — the reply links to Rewrite and no
// "Replace draft" card appears — and its questions arrive as an appliable card.
// A story's earlier Discovery Chat stays readable, read-only, under "Earlier
// chats". Run in light and dark, plus a phone-width overflow check.

test.use({ serviceWorkers: 'block' });

const STUB_BODY = 'As a user, I want X so that Y.';

async function seedStoryWithEarlierChat(title: string): Promise<{ ws: string; earlierTitle: string }> {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  const { storyId } = await seedProductStory(ctx, base, ws);
  const r = await ctx.patch(`${base}/api/v1/product/stories/${storyId}/draft`, {
    data: { title, body_md: `# ${title}\n\nSeeded for the merged-chat E2E.` },
  });
  if (!r.ok()) throw new Error(`rename story → ${r.status()} ${await r.text()}`);
  // An earlier Discovery Chat row (the old engine): it must stay listed and
  // open read-only. Creating the row is still allowed; sending to it is not.
  const earlierTitle = 'Kickoff discovery';
  const chat = await ctx.post(`${base}/api/v1/product/stories/${storyId}/discovery-chats`, {
    data: { title: earlierTitle },
  });
  if (!chat.ok()) throw new Error(`seed earlier chat → ${chat.status()} ${await chat.text()}`);
  const cid = (await chat.json()).id as string;
  const send = await ctx.post(`${base}/api/v1/product/discovery-chats/${cid}/messages`, {
    data: { body: 'still there?' },
  });
  expect(send.status(), 'an earlier chat is read-only').toBe(409);
  await ctx.dispose();
  return { ws, earlierTitle };
}

async function openChat(page: Page, title: string): Promise<void> {
  await page.goto('/#/product');
  await expect(page.locator('.product-page')).toBeVisible({ timeout: 30_000 });
  await page.locator('.story-row', { hasText: title }).first().click();
  await page.getByRole('tab', { name: 'Discover', exact: true }).click();
  // The merged tab: one "Chat", no "Refine" sub-tab any more.
  await expect(page.locator('.sub-tab-strip .st', { hasText: 'Refine' })).toHaveCount(0);
  await page.getByRole('tab', { name: 'Chat', exact: true }).click();
  await expect(page.locator('.chat-tab')).toBeVisible({ timeout: 15_000 });
}

for (const scheme of ['light', 'dark'] as const) {
  test(`merged Chat: suggestion → Rewrite, cards inline, earlier chats read-only (${scheme})`, async ({ page }) => {
    test.setTimeout(120_000);
    const title = `Merged chat ${scheme} ${Date.now()}`;
    const { ws, earlierTitle } = await seedStoryWithEarlierChat(title);
    await page.addInitScript(
      ({ ws, scheme }) => {
        localStorage.setItem('otto_workspace', ws);
        localStorage.setItem('otto_scheme', scheme);
        localStorage.setItem('otto_rail_expanded', '0');
      },
      { ws, scheme },
    );
    await openChat(page, title);

    // No conversation yet: the start state offers a chat; start one from the list.
    await page.locator('.list-pane').getByRole('button', { name: 'New chat' }).click();
    await expect(page.locator('.chat-item.active')).toBeVisible({ timeout: 15_000 });

    // One composer with a compact agent picker; a starter chip prefills it.
    const chat = page.locator('.refine-chat');
    await expect(chat.getByRole('combobox', { name: 'Chat agent' })).toBeVisible();
    await chat.locator('.starter-chip').first().click();
    await expect(chat.locator('.msg-input')).not.toHaveValue('');
    await chat.getByRole('button', { name: 'Send', exact: true }).click();

    // The reply: the proposed body is a suggestion linked to Rewrite …
    const review = chat.locator('.story-updated-chip');
    await expect(review).toContainText(/Suggestion v\d+ · Review in Rewrite/, { timeout: 30_000 });
    await expect(chat.getByRole('button', { name: /Replace draft/ })).toHaveCount(0);
    // … and the questions arrive as a card, applied explicitly.
    const questions = chat.locator('.action-card', { hasText: 'Questions' }).first();
    await expect(questions).toBeVisible();
    await questions.getByRole('button', { name: /Add \d+ question/ }).click();
    await expect(chat.locator('.applied-row')).toContainText('Added', { timeout: 12_000 });

    // Earlier chats: listed, collapsed by default, open read-only (no composer).
    const toggle = page.getByRole('button', { name: /Earlier chats \(1\)/ });
    await expect(toggle).toHaveAttribute('aria-expanded', 'false');
    await toggle.click();
    await page.locator('.chat-item', { hasText: earlierTitle }).locator('.chat-btn').click();
    const earlier = page.locator('.earlier-chat');
    await expect(earlier).toContainText('Earlier chat · read-only');
    await expect(earlier.locator('textarea')).toHaveCount(0);
    await expect(earlier.getByRole('button', { name: 'Continue in a new chat' })).toBeVisible();

    // Back to the conversation, then follow the suggestion into Rewrite.
    await page.locator('.chat-list').first().locator('.chat-btn').first().click();
    await page.locator('.refine-chat .story-updated-chip').first().click();
    await expect(page.getByRole('tab', { name: 'Rewrite', exact: true })).toHaveAttribute('aria-selected', 'true', { timeout: 15_000 });
    await expect(page.getByText(STUB_BODY).first()).toBeVisible({ timeout: 20_000 });
  });
}

test('merged Chat fits a phone without horizontal scroll', async ({ page }) => {
  test.setTimeout(90_000);
  await page.setViewportSize({ width: 390, height: 844 });
  const title = `Merged chat phone ${Date.now()}`;
  const { ws } = await seedStoryWithEarlierChat(title);
  await page.addInitScript((ws) => {
    localStorage.setItem('otto_workspace', ws);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
  await openChat(page, title);
  await page.locator('.list-pane').getByRole('button', { name: 'New chat' }).click();
  await expect(page.locator('.refine-chat .msg-input')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.refine-chat .starter-chip').first()).toBeVisible();
  await expectNoHorizontalOverflow(page);
});
