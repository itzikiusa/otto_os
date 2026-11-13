// After a publish is refused because the Jira description changed (409) and
// the person Refreshes, the suggestion written against the OLD text must not be
// re-offered as if nothing happened: it is marked "Based on an older version
// of the story", and its publish review says it replaces the current Jira
// description — with the site + issue key and the FULL text that is sent.
// A seeded draft story is presented as a Jira story through route fixtures; no
// publish ever reaches a daemon or Jira.
import {test, expect} from '@playwright/test';
import {apiCtx, seedWorkspace} from './seed';
import type {ProductStoryDetail, ProductStoryVersion} from '../src/modules/product/types';

test.use({serviceWorkers: 'block', viewport: {width: 1440, height: 1000}});

const SITE = 'acme.fixture.invalid';
const KEY = 'PROJ-7';
const suggestedBody = Array.from({length: 30}, (_, i) => `Suggested line ${i + 1}`).join('\n');

test('a suggestion older than the refreshed source is marked stale and its review shows everything sent', async ({page}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop mounted acceptance');
  const {ctx, base} = await apiCtx();
  let detail: ProductStoryDetail;
  let workspaceId: string;
  const title = `Stale suggestion ${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
  try {
    workspaceId = await seedWorkspace(ctx, base);
    const created = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/product/drafts`, {data: {title}});
    expect(created.ok()).toBe(true);
    detail = await created.json();
  } finally { await ctx.dispose(); }
  const id = detail.story.id;
  expect(detail.source).not.toBeNull();
  const story = {...detail.story, source_kind: 'jira', source_key: KEY, url: `https://${SITE}/browse/${KEY}`};
  const row = (vid: string, version_no: number, kind: string, body_md: string): ProductStoryVersion =>
    ({...detail.source!, id: vid, version_no, kind, body_md, title});
  const src1 = row('fx-src-1', 1, 'source', 'The original story.');
  const sug2 = row('fx-sug-2', 2, 'suggested', suggestedBody);
  const src3 = row('fx-src-3', 3, 'source', 'The original story.\nA teammate edited it in Jira.');
  let refreshed = false;
  const byId: Record<string, ProductStoryVersion> = {[src1.id]: src1, [sug2.id]: sug2, [src3.id]: src3};
  const snapshot = () => ({...detail, story, source: refreshed ? src3 : src1});

  await page.addInitScript(ws => {
    localStorage.setItem('otto_workspace', ws);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, workspaceId);
  const ctxRoute = page.context();
  await ctxRoute.route(new RegExp(`/api/v1/product/stories/${id}$`), route => route.fulfill({json: snapshot()}));
  await ctxRoute.route(new RegExp(`/api/v1/product/stories/${id}/versions$`), route =>
    route.fulfill({json: refreshed ? [src1, sug2, src3] : [src1, sug2]}));
  await ctxRoute.route(/\/api\/v1\/product\/versions\/fx-[a-z0-9-]+$/, route =>
    route.fulfill({json: byId[new URL(route.request().url()).pathname.split('/').pop()!]}));
  let publishes = 0;
  await ctxRoute.route(/\/api\/v1\/product\/versions\/fx-[a-z0-9-]+\/publish$/, route => {
    publishes++;
    return route.fulfill({status: 409, json: {code: 'conflict',
      message: 'the issue changed in Jira since Otto last synced it; refresh the story, review the changes, then publish again'}});
  });
  await ctxRoute.route(new RegExp(`/api/v1/product/stories/${id}/refresh$`), route => {
    refreshed = true;
    return route.fulfill({json: snapshot()});
  });

  await page.goto('/#/product');
  await page.locator('.story-row').and(page.getByTitle(title, {exact: true})).click();
  await page.getByRole('tab', {name: 'Rewrite', exact: true}).click();
  const publishBtn = page.getByTestId('rw-publish-btn');
  await expect(publishBtn).toBeVisible();
  await expect(page.getByTestId('rewrite-stale')).toHaveCount(0);

  // First publish: the review names the site + issue and carries every line.
  await publishBtn.click();
  const review = page.getByRole('dialog', {name: `Overwrite ${KEY}?`, exact: true});
  await expect(review.getByTestId('publish-review-where')).toContainText(KEY);
  await expect(review.getByTestId('publish-review-where')).toContainText(SITE);
  await expect(review.getByTestId('publish-review-body')).toHaveText(suggestedBody);
  await expect(review.getByTestId('publish-review-note')).toHaveCount(0);
  await review.getByRole('button', {name: `Overwrite ${KEY}`, exact: true}).click();

  // Refused (Jira changed): Refresh pulls the teammate's edit in.
  const conflict = page.getByRole('dialog', {name: 'Changed since the last sync', exact: true});
  await expect(conflict).toBeVisible();
  await conflict.getByRole('button', {name: 'Refresh', exact: true}).click();
  await expect(conflict).toHaveCount(0);
  expect(publishes).toBe(1);

  // The old suggestion is now marked, and its review says what it replaces.
  await expect(page.getByTestId('rewrite-stale')).toHaveText('Based on an older version of the story');
  await publishBtn.click();
  await expect(review.getByTestId('publish-review-note')).toContainText('Based on an older version of the story');
  await expect(review.getByTestId('publish-review-note')).toContainText(`replaces the current description of ${KEY}`);
  await expect(review.getByTestId('publish-review-body')).toHaveText(suggestedBody);
  await review.getByRole('button', {name: 'Cancel', exact: true}).click();
  await expect(review).toHaveCount(0);
  expect(publishes).toBe(1);
});
