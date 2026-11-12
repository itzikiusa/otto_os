import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Design Hall lobby (desktop-browser) — against the REAL /design/* routes
// (OTTO_E2E_BIN=<repo>/target/debug/ottod).
//
//   • All designs is server-paged: "Load more" goes past the first page;
//   • Archived finds an archived design (badge, not colour) and Unarchive
//     puts it back to Draft through the status route;
//   • Group by Story heads a section with the story, linking to its page;
//   • Needs review lists a design in review, and "Show all in review" sets
//     the Status chip;
//   • the New design sheet's "Implements story" is a search-as-you-type menu.
// ─────────────────────────────────────────────────────────────────────────────

const V1 = '/api/v1';
const stamp = `${Date.now().toString(36)}${process.pid.toString(36)}`;
const ARCHIVED = `Archived poster ${stamp}`;
const REVIEW = `Review screen ${stamp}`;
const STORY_TITLE = `Lobby story ${stamp}`;
let wsId = '';
let archivedId = '';
let storyId = '';

async function postJson(ctx: APIRequestContext, url: string, data: unknown): Promise<any> {
  const r = await ctx.post(url, { data });
  expect(r.ok(), `${url} → ${r.status()} ${await r.text()}`).toBeTruthy();
  return r.json();
}
async function frame(ctx: APIRequestContext, base: string, title: string, extra: Record<string, unknown> = {}): Promise<string> {
  const res = await postJson(ctx, `${base}${V1}/design/artifacts`, {
    workspace_id: wsId,
    format: 'html',
    studio: 'frames',
    title,
    content: `<!doctype html><html><body><h1>${title}</h1></body></html>`,
    ...extra,
  });
  return res.artifact.id;
}

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  // More than one lobby page (60) so "Load more" has somewhere to go.
  for (let i = 0; i < 65; i++) await frame(ctx, base, `Paged frame ${String(i).padStart(2, '0')} ${stamp}`);
  archivedId = await frame(ctx, base, ARCHIVED);
  expect((await ctx.delete(`${base}${V1}/design/artifacts/${archivedId}`)).ok()).toBeTruthy();
  const inReview = await frame(ctx, base, REVIEW);
  expect((await ctx.patch(`${base}${V1}/design/artifacts/${inReview}`, { data: { status: 'review' } })).ok()).toBeTruthy();
  const draft = await postJson(ctx, `${base}${V1}/workspaces/${wsId}/product/drafts`, { title: STORY_TITLE });
  storyId = draft.story.id;
  await frame(ctx, base, `Story frame ${stamp}`, { story_id: storyId });
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
  }, wsId);
});

async function openLobby(page: Page): Promise<void> {
  await page.goto('/#/design');
  await expect(page.getByTestId('design-all')).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId('design-artifact-card').first()).toBeVisible({ timeout: 15_000 });
}

test('All designs pages past the first page — no silent cap', async ({ page }) => {
  await openLobby(page);
  const cards = page.getByTestId('design-grid').getByTestId('design-artifact-card');
  await expect(cards).toHaveCount(60);
  await page.getByTestId('design-collection-more').click();
  await expect.poll(() => cards.count()).toBeGreaterThan(60);
  await expectNoHorizontalOverflow(page);
});

test('Archived finds an archived design and Unarchive moves it back to Draft', async ({ page }) => {
  await openLobby(page);
  // Archived designs never show in the default grid.
  await expect(page.getByTestId('design-grid').getByText(ARCHIVED)).toHaveCount(0);
  await page.getByTestId('design-filter-archived').click();
  await expect(page.getByTestId('design-filter-archived')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('heading', { name: 'Archived designs' })).toBeVisible();
  const card = page.getByTestId('design-artifact-card').filter({ hasText: ARCHIVED });
  await expect(card).toBeVisible({ timeout: 15_000 });
  await expect(card.getByTestId('design-card-archived')).toHaveText(/Archived/);
  await card.getByTestId('design-card-unarchive').click();
  await expect(card).toHaveCount(0);
  const { ctx, base } = await apiCtx();
  const d = await (await ctx.get(`${base}${V1}/design/artifacts/${archivedId}`)).json();
  expect(d.artifact.status).toBe('draft');
  await ctx.dispose();
});

test('Group by Story heads a section that links to the story page', async ({ page }) => {
  await openLobby(page);
  await page.getByTestId('design-group-story').click();
  await expect(page.getByTestId('design-group-story')).toHaveAttribute('aria-pressed', 'true');
  const head = page.locator(`a[href="#/design/story/${storyId}"]`);
  await expect(head).toBeVisible({ timeout: 15_000 });
  await head.click();
  await expect(page).toHaveURL(new RegExp(`#/design/story/${storyId}$`));
  await expect(page.getByTestId('design-artifact-card').filter({ hasText: `Story frame ${stamp}` })).toBeVisible();
});

test('Needs review lists a design in review and opens the filtered grid', async ({ page }) => {
  await openLobby(page);
  const rail = page.getByTestId('design-needs-review');
  await expect(rail.getByText(REVIEW)).toBeVisible({ timeout: 15_000 });
  await rail.getByTestId('design-review-all').click();
  await expect(page.getByTestId('design-filter-status')).toContainText('In review');
  const cards = page.getByTestId('design-grid').getByTestId('design-artifact-card');
  await expect(cards.filter({ hasText: REVIEW })).toBeVisible();
  await expect(cards.filter({ hasText: 'Paged frame' })).toHaveCount(0);
  // Clear brings everything back.
  await page.getByTestId('design-filter-clear').click();
  await expect(cards.filter({ hasText: 'Paged frame' }).first()).toBeVisible();
});

test('the New design sheet finds a story by typing', async ({ page }) => {
  await openLobby(page);
  await page.getByTestId('design-new').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Frame screen', exact: true }).click();
  const sheet = page.getByRole('dialog', { name: 'New design' });
  await expect(sheet).toBeVisible();
  await sheet.getByTestId('design-new-story').click();
  const menu = page.locator('.ctx-menu');
  await expectFullyInViewport(page, menu, 'story picker');
  await menu.locator('input').first().fill(stamp);
  await menu.getByRole('menuitemcheckbox', { name: new RegExp(STORY_TITLE) }).click();
  await expect(sheet.getByTestId('design-new-story')).toContainText(STORY_TITLE);
});
