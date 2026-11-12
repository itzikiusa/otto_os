import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ── E2E (phone / tablet): the story Design tab is Design Hall's grid ─────────
//
// design-hall.md §4.1: on a phone the story's Design tab shows the story's
// designs (cards stack in one column), the ONE primary "New design", the Ask
// Otto line and ⋯ — all reachable without horizontal page scroll, in light and
// dark. A card opens the Hall artifact with `?from=story/<id>`; ⋯ → "Open the
// classic arena" still mounts the old arena (its own segmented phone layout).

test.use({ serviceWorkers: 'block' });

let workspaceId = '';
let storyId = '';
const SUFFIX = `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
const STORY = `Phone story designs ${SUFFIX}`;

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const draft = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/product/drafts`, { data: { title: STORY } });
  expect(draft.ok()).toBeTruthy();
  storyId = (await draft.json()).story.id;
  const r = await ctx.post(`${base}/api/v1/design/artifacts`, {
    data: { workspace_id: workspaceId, format: 'html', title: `Phone card ${SUFFIX}`, content: '<h1>Phone</h1>', story_id: storyId },
  });
  expect(r.ok()).toBeTruthy();
  await ctx.dispose();
});

async function openDesignTab(page: Page, scheme: 'light' | 'dark'): Promise<void> {
  await page.addInitScript(
    ({ ws, s }) => {
      localStorage.setItem('otto_workspace', ws);
      localStorage.setItem('otto_scheme', s);
    },
    { ws: workspaceId, s: scheme },
  );
  // `#/product/<id>` opens the story from the route (phone: its content panel).
  await page.goto(`/#/product/${storyId}`);
  await expect(page.locator('.product-page')).toBeVisible({ timeout: 30_000 });
  const cls = (await page.locator('.product-page').getAttribute('class')) ?? '';
  if (cls.includes('m-list-open') && !cls.includes('m-content-open')) {
    await page.locator('.story-row', { hasText: STORY }).first().click();
  }
  const group = page.locator('.product-header-row2 .tab-strip:not(.sub-tab-strip) .st', { hasText: 'Story' }).first();
  await group.scrollIntoViewIfNeeded();
  await group.click();
  const sub = page.locator('.sub-tab-strip .st', { hasText: 'Design' }).first();
  await sub.scrollIntoViewIfNeeded();
  await sub.click();
  await expect(page.getByTestId('story-designs')).toBeVisible({ timeout: 15_000 });
}

for (const scheme of ['light', 'dark'] as const) {
  test(`phone story Design tab: grid, New design, Ask Otto and ⋯ fit (${scheme})`, async ({ page }, info) => {
    await openDesignTab(page, scheme);
    const card = page.getByTestId('design-artifact-card').filter({ hasText: `Phone card ${SUFFIX}` });
    await expect(card).toBeVisible({ timeout: 15_000 });
    await expectFullyInViewport(page, page.getByTestId('story-designs-new'), 'New design');
    await expectFullyInViewport(page, page.getByTestId('story-designs-more'), 'More design actions');
    const ask = page.getByTestId('story-designs-ask');
    await ask.scrollIntoViewIfNeeded();
    await expect(ask.getByRole('textbox')).toBeVisible();
    await expect(ask.getByRole('button', { name: /Ask Otto/ })).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath(`story-designs-phone-${scheme}.png`), fullPage: true });
  });
}

test('phone: a card opens the Hall artifact with ?from=story', async ({ page }) => {
  await openDesignTab(page, 'light');
  const card = page.getByTestId('design-artifact-card').filter({ hasText: `Phone card ${SUFFIX}` });
  await card.locator('a').first().click();
  await expect(page).toHaveURL(new RegExp(`#/design/a/[^/?]+\\?from=story/${storyId}$`), { timeout: 15_000 });
  await expectNoHorizontalOverflow(page);
});

test('phone: ⋯ → Open the classic arena, then Back to designs', async ({ page }) => {
  await openDesignTab(page, 'dark');
  await page.getByTestId('story-designs-more').click();
  const item = page.getByRole('menuitem', { name: /Open the classic arena/ });
  await expectFullyInViewport(page, item, 'classic arena menu item');
  await item.click();
  await expect(page.locator('.mockups-tab')).toBeVisible({ timeout: 15_000 });
  const back = page.getByTestId('story-designs-back');
  await expectFullyInViewport(page, back, 'Back to designs');
  await expectNoHorizontalOverflow(page);
  await back.click();
  await expect(page.getByTestId('story-designs')).toBeVisible();
});
