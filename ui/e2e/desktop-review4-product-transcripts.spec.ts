import {test, expect} from '@playwright/test';
import {apiCtx, seedWorkspace} from './seed';

test.use({serviceWorkers: 'block', viewport: {width: 1440, height: 1000}});

test('Product collapsed transcripts use summaries and history search reveals an older body without losing page navigation', async ({page}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop mounted acceptance');
  const {ctx, base} = await apiCtx();
  const title = `Transcript review ${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
  let workspaceId: string, storyId: string, createdBy: string;
  try {
    workspaceId = await seedWorkspace(ctx, base);
    const response = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/product/drafts`, {data: {title}});
    expect(response.ok()).toBe(true);
    const detail = await response.json();
    storyId = detail.story.id; createdBy = detail.story.created_by;
  } finally {await ctx.dispose();}
  await page.addInitScript(ws => {
    localStorage.setItem('otto_workspace', ws);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, workspaceId);
  const body = (i: number) => i === 0 ? 'Archived decision ζ keeps the complete historical context.' : `Chosen body ${i}. ${'Recorded context. '.repeat(512)}`;
  const summaries = Array.from({length: 105}, (_, index) => {
    const i = 104 - index;
    return {id: `review-transcript-${i}`, story_id: storyId, title: `Recorded conversation ${i}`,
      created_by: createdBy, created_at: new Date(Date.UTC(2026, 8, 1, 0, i)).toISOString(), body_bytes: Buffer.byteLength(body(i))};
  });
  const pages: (string | null)[] = [], bodyReads: string[] = [], searchPages: (string | null)[] = [];
  await page.context().route(new RegExp(`/api/v1/product/stories/${storyId}/transcripts\\?`), route => {
    const q = new URL(route.request().url()).searchParams;
    expect(q.get('summary')).toBe('true'); expect(q.get('limit')).toBe('50');
    const cursor = q.get('cursor'); pages.push(cursor);
    const start = cursor === null ? 0 : cursor === 'older-50' ? 50 : 100;
    return route.fulfill({json: {items: summaries.slice(start, start + 50), next_cursor: start === 0 ? 'older-50' : start === 50 ? 'older-100' : null}});
  });
  await page.context().route('**/api/v1/product/transcripts/review-transcript-*', route => {
    const id = new URL(route.request().url()).pathname.split('/').at(-1)!;
    const row = summaries.find(summary => summary.id === id)!;
    expect(row).toBeDefined(); bodyReads.push(id);
    const {body_bytes: _bytes, ...metadata} = row;
    return route.fulfill({json: {...metadata, body: body(Number(id.split('-').at(-1)))}});
  });
  await page.context().route(new RegExp(`/api/v1/product/stories/${storyId}/transcripts/search\\?`), route => {
    const q = new URL(route.request().url()).searchParams;
    expect(q.get('q')).toBe('archived decision ζ');
    expect(q.get('limit')).toBe('100');
    const cursor = q.get('cursor'); searchPages.push(cursor);
    // A page with zero matches still has more history to scan.
    return route.fulfill({json: cursor === null ? {items: [], next_cursor: 'search-100'}
      : {items: [{...summaries[104], match_count: 1}], next_cursor: null}});
  });
  await page.goto('/#/product');
  await page.locator('.story-row', {hasText: title}).click();
  const rows = page.locator('.transcript-item');
  await expect(rows).toHaveCount(50);
  expect(bodyReads).toEqual([]);
  await expect(page.locator('.transcript-body')).toHaveCount(0);
  const latest = page.locator('[data-transcript-id="review-transcript-104"]');
  await latest.locator('.transcript-toggle').click();
  await expect(latest.locator('.transcript-body')).toHaveText(body(104));
  expect(bodyReads).toEqual(['review-transcript-104']);
  await latest.locator('.transcript-toggle').click();
  await expect(latest.locator('.transcript-body')).toHaveCount(0);
  await page.getByRole('button', {name: 'Next transcripts', exact: true}).click();
  await expect(page.locator('[data-transcript-id="review-transcript-54"]')).toBeVisible();
  await expect(rows).toHaveCount(50);
  expect(bodyReads).toEqual(['review-transcript-104']);
  await page.getByRole('button', {name: 'Previous transcripts', exact: true}).click();
  await expect(latest).toBeVisible();
  await expect(page.getByRole('button', {name: 'Previous transcripts', exact: true})).toBeDisabled();
  const beforeSearch = pages.length;
  await page.keyboard.press('Meta+f');
  const find = page.getByRole('search', {name: 'Find in page'});
  await find.getByRole('textbox', {name: 'Search query'}).fill('Archived decision ζ');
  const oldest = page.locator('[data-transcript-id="review-transcript-0"]');
  await expect(oldest.locator('.transcript-body')).toHaveText(body(0));
  await expect(oldest).toBeInViewport();
  expect(searchPages).toEqual([null, 'search-100']);
  expect(bodyReads).toEqual(['review-transcript-104', 'review-transcript-0']);
  expect(pages.length).toBe(beforeSearch);
  await expect(rows).toHaveCount(50);
  await find.getByRole('button', {name: 'Close find bar', exact: true}).click();
  await expect(oldest).toHaveCount(0);
  await expect(page.locator('[data-transcript-id="review-transcript-55"]')).toBeAttached();
  await page.getByRole('button', {name: 'Next transcripts', exact: true}).click();
  await expect(page.locator('[data-transcript-id="review-transcript-54"]')).toBeVisible();
  await page.getByRole('button', {name: 'Next transcripts', exact: true}).click();
  await expect(rows).toHaveCount(5);
  await expect(oldest).toBeAttached();
  await expect(page.getByRole('button', {name: 'Next transcripts', exact: true})).toBeDisabled();
  expect(pages).toEqual([null, 'older-50', null, 'older-50', 'older-100']);
});
