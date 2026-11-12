import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// ── E2E: a story's designs live in Design Hall (design-hall.md §4.1) ─────────
//
// The story Design tab (StoryDesigns) is the Hall's grid scoped to the story:
// ONE primary New design (the Hall sheet, story preset), an Ask Otto line, and
// cards that open `#/design/a/<id>?from=story/<storyId>`. A story attachment —
// what an agent's mockup writes — is mirrored in by the live sync and appears
// badged "From a story", read-only until Edit here. The Overview has one line,
// "N designs → Design". ⋯ → "Open the classic arena" mounts the old arena.

test.use({ serviceWorkers: 'block', viewport: { width: 1440, height: 900 } });

let workspaceId = '';
const SUFFIX = `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
const STORY = `Story designs ${SUFFIX}`;
const EPIC = `Story designs epic ${SUFFIX}`;
const CHILD = `Story designs child ${SUFFIX}`;
let storyId = '';
let epicId = '';
let childId = '';

async function must(r: { ok(): boolean; status(): number; text(): Promise<string>; json(): Promise<any> }, what: string) {
  if (!r.ok()) throw new Error(`${what} → ${r.status()} ${await r.text()}`);
  return r.json();
}

async function hallDesign(ctx: APIRequestContext, base: string, title: string, story: string): Promise<string> {
  const r = await must(
    await ctx.post(`${base}/api/v1/design/artifacts`, {
      data: { workspace_id: workspaceId, format: 'html', title, content: `<h1>${title}</h1>`, story_id: story },
    }),
    `create ${title}`,
  );
  return r.artifact.id as string;
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  storyId = (await must(await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/product/drafts`, { data: { title: STORY } }), 'story')).story.id;
  epicId = (await must(await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/product/drafts`, { data: { title: EPIC } }), 'epic')).story.id;
  await must(await ctx.patch(`${base}/api/v1/product/stories/${epicId}`, { data: { tree_kind: 'epic' } }), 'mark epic');
  childId = (await must(
    await ctx.post(`${base}/api/v1/product/stories/${epicId}/children`, { data: { title: CHILD, tree_kind: 'story', folder: 'Design' } }),
    'child',
  )).story.id;
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

async function openStory(page: Page, title: string): Promise<void> {
  await page.goto('/#/product');
  await expect(page.locator('.product-page')).toBeVisible({ timeout: 30_000 });
  const row = page.locator('.story-row', { hasText: title }).first();
  await expect(row).toBeVisible({ timeout: 20_000 });
  await row.click();
  await expect(page.locator('.overview')).toBeVisible({ timeout: 20_000 });
}

async function openDesignTab(page: Page): Promise<void> {
  await page.getByRole('tab', { name: 'Story', exact: true }).click();
  await page.locator('.tab-strip .st', { hasText: 'Design' }).first().click();
  await expect(page.getByTestId('story-designs')).toBeVisible({ timeout: 15_000 });
}

test('Overview shows one designs line that opens the Design tab; the tab has one primary action and no editor', async ({ page }) => {
  await openStory(page, STORY);
  const line = page.getByTestId('story-designs-line');
  await expect(line).toBeVisible({ timeout: 15_000 });
  await expect(line).toContainText('Design');
  // The old Overview canvas panel is gone.
  await expect(page.getByText('Linked canvases')).toHaveCount(0);
  await line.getByRole('button').click();
  await expect(page.getByTestId('story-designs')).toBeVisible({ timeout: 15_000 });
  // No second editor on the story page: no arena, no graph strip.
  await expect(page.locator('.design-arena')).toHaveCount(0);
  await expect(page.locator('.product-page .btn.primary', { hasText: 'New design' })).toHaveCount(1);
  await expect(page.getByTestId('story-designs-ask')).toBeVisible();
  await expectNoHorizontalOverflow(page);
});

test('New design files a draft that implements the story and opens it with a way back', async ({ page }) => {
  await openStory(page, STORY);
  await openDesignTab(page);
  await page.getByTestId('story-designs-new').click();
  const sheet = page.getByRole('dialog', { name: 'New design' });
  await expect(sheet).toBeVisible();
  // The story is preset in "Implements story".
  await expect(sheet).toContainText(STORY);
  await sheet.locator('#dh-new-title').fill(`Checkout screen ${SUFFIX}`);
  await page.getByTestId('design-new-create').click();
  await expect(page).toHaveURL(new RegExp(`#/design/a/[^/?]+\\?from=story/${storyId}$`), { timeout: 20_000 });
  const id = /#\/design\/a\/([^/?]+)/.exec(page.url())![1];
  const { ctx, base } = await apiCtx();
  const rows = await must(await ctx.get(`${base}/api/v1/design/artifacts?story_id=${storyId}`), 'list');
  expect((rows as { id: string }[]).some((a) => a.id === id)).toBe(true);
  await ctx.dispose();
});

test('cards open the Hall artifact with ?from=story; a story attachment appears as a read-only mirror', async ({ page }) => {
  test.setTimeout(60_000);
  const { ctx, base } = await apiCtx();
  await hallDesign(ctx, base, `Hall card ${SUFFIX}`, storyId);
  await openStory(page, STORY);
  await openDesignTab(page);
  const card = page.getByTestId('design-artifact-card').filter({ hasText: `Hall card ${SUFFIX}` });
  await expect(card).toBeVisible({ timeout: 15_000 });

  // What an agent's mockup (or an upload) writes: a design attachment on the story.
  await must(
    await ctx.post(`${base}/api/v1/product/stories/${storyId}/attachments`, {
      data: { filename: `agent-mockup-${SUFFIX}.html`, mime: 'text/html', kind: 'design', data_b64: Buffer.from('<h1>Agent mockup</h1>').toString('base64') },
    }),
    'attachment',
  );
  await ctx.dispose();
  // The live sync mirrors it within seconds and the grid reloads on the event.
  const mirror = page.getByTestId('design-artifact-card').filter({ hasText: `agent-mockup-${SUFFIX}` });
  await expect(mirror).toBeVisible({ timeout: 20_000 });
  await expect(mirror.getByTestId('design-card-mirror')).toContainText('From a story');

  await mirror.locator('a').first().click();
  await expect(page).toHaveURL(new RegExp(`\\?from=story/${storyId}$`), { timeout: 15_000 });
  await expect(page.getByTestId('design-mirror-banner')).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId('design-adopt')).toBeVisible();
});

test('Ask Otto creates a draft for the story and opens it on the Otto tab', async ({ page }) => {
  await openStory(page, STORY);
  await openDesignTab(page);
  const ask = page.getByTestId('story-designs-ask');
  await ask.getByRole('textbox').fill(`A calm confirmation screen ${SUFFIX}`);
  await ask.getByRole('button', { name: /Ask Otto/ }).click();
  await expect(page).toHaveURL(new RegExp(`#/design/a/[^/?]+/otto\\?from=story/${storyId}$`), { timeout: 20_000 });
  const id = /#\/design\/a\/([^/?]+)/.exec(page.url())![1];
  const { ctx, base } = await apiCtx();
  const rows = (await must(await ctx.get(`${base}/api/v1/design/artifacts?story_id=${storyId}`), 'list')) as { id: string; meta?: { brief?: string } }[];
  const made = rows.find((a) => a.id === id);
  expect(made, 'the Ask Otto draft implements the story').toBeTruthy();
  await ctx.dispose();
});

test('⋯ → Open the classic arena mounts the old arena and Back returns to the designs', async ({ page }) => {
  await openStory(page, STORY);
  await openDesignTab(page);
  await page.getByTestId('story-designs-more').click();
  await expect(page.getByRole('menuitem', { name: /Open in Design Hall/ })).toBeVisible();
  await page.getByRole('menuitem', { name: /Open the classic arena/ }).click();
  await expect(page.getByTestId('story-designs-classic')).toBeVisible();
  await expect(page.locator('.mockups-tab')).toBeVisible({ timeout: 15_000 });
  await page.getByTestId('story-designs-back').click();
  await expect(page.getByTestId('story-designs')).toBeVisible();
  await expect(page.locator('.design-arena')).toHaveCount(0);
});

test('an epic lists its children’s designs, grouped by story', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  await hallDesign(ctx, base, `Child design ${SUFFIX}`, childId);
  await ctx.dispose();
  await openStory(page, EPIC);
  await openDesignTab(page);
  const card = page.getByTestId('design-artifact-card').filter({ hasText: `Child design ${SUFFIX}` });
  await expect(card).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('section.group', { has: card }).locator('.g-head')).toContainText(CHILD);
});

for (const scheme of ['light', 'dark'] as const) {
  test(`story Design tab renders in ${scheme}`, async ({ page }, info) => {
    await page.addInitScript((s) => localStorage.setItem('otto_scheme', s), scheme);
    await openStory(page, STORY);
    await openDesignTab(page);
    await expect(page.getByTestId('design-grid').first()).toBeVisible({ timeout: 15_000 });
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath(`story-designs-${scheme}.png`) });
  });
}
