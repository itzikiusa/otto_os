import {test, expect, type Page} from '@playwright/test';
import {createHash} from 'node:crypto';
import {apiCtx, seedWorkspace} from './seed';
import type {ProductStoryDetail} from '../src/modules/product/types';

test.use({serviceWorkers: 'block', viewport: {width: 1440, height: 1000}});
const originalBody = 'Reviewed café.\r\n  Keep these spaces.  \nThird\nFourth\nFifth\nSixth\nOriginal final line.\n';
const revisedBody = 'Revised café.\r\n  New exact spaces.  \nThird\nFourth\nFifth\nSixth\nRevised final line.\n';
const digest = (body: string) => createHash('sha256').update(body, 'utf8').digest('hex');

async function fixture(page: Page) {
  const {ctx, base} = await apiCtx();
  let detail: ProductStoryDetail;
  let workspaceId: string;
  const title = `Publication review ${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
  try {
    workspaceId = await seedWorkspace(ctx, base);
    const created = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/product/drafts`, {data: {title}});
    expect(created.ok()).toBe(true);
    detail = await created.json();
    const saved = await ctx.patch(`${base}/api/v1/product/stories/${detail.story.id}/draft`, {data: {title, body_md: originalBody}});
    expect(saved.ok()).toBe(true);
    detail = await saved.json();
  } finally { await ctx.dispose(); }
  const id = detail.story.id;
  expect(detail.source).not.toBeNull();
  const content = {title, body: originalBody};
  const snapshot = () => ({...detail, story: {...detail.story, title: content.title},
    source: {...detail.source!, title: content.title, body_md: content.body}});
  await page.addInitScript(ws => {
    localStorage.setItem('otto_workspace', ws);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, workspaceId);
  await page.context().route(new RegExp(`/api/v1/product/stories/${id}$`), route => route.fulfill({json: snapshot()}));
  await page.context().route(new RegExp(`/api/v1/product/stories/${id}/versions$`), route => route.fulfill({json: [snapshot().source]}));
  const accounts = ['A', 'B'].map(id => ({id, label: `Account ${id}`, base_url: `https://${id.toLowerCase()}.invalid`,
    provider: 'jira', email: 'fixture@example.invalid', user_id: detail.story.created_by, token_expires_at: null, created_at: detail.story.created_at}));
  await page.context().route('**/api/v1/issue/accounts', route => route.fulfill({json: accounts}));
  await page.context().route('**/api/v1/issue/projects?*', route => route.fulfill({json: [{key: 'REVIEW', name: 'Reviewed project'}]}));
  await page.context().route('**/api/v1/issue/*/*/issue-types', route => route.fulfill({json: ['Story', 'Task']}));
  await page.context().route('**/api/v1/issue/confluence/spaces?*', route => route.fulfill({json: [{key: 'RFC', name: 'Reviewed space'}]}));
  // Catch both outward-facing operations before opening the module. Individual
  // cases override their target; no unmatched publication can reach a daemon.
  await page.context().route(`**/api/v1/product/stories/${id}/publish-as-*`, route => route.fulfill({status: 500, json: {code: 'fixture', message: 'Unexpected publication route'}}));
  await page.goto('/#/product');
  await page.locator('.story-row', {hasText: title}).click();
  await expect(page.locator('.overview')).toBeVisible();
  return {id, content, snapshot, versionId: detail.source!.id};
}

for (const mode of ['story', 'rfc'] as const) {
  test(`Product ${mode} conflict requires explicit review of changed bytes at the same version`, async ({page}, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop mounted acceptance');
    const f = await fixture(page);
    const sent: Record<string, any>[] = [];
    await page.context().route(`**/api/v1/product/stories/${f.id}/publish-as-${mode}`, route => {
      sent.push(route.request().postDataJSON());
      return sent.length === 1
        ? route.fulfill({status: 409, json: {code: 'conflict', message: 'Reviewed content changed; reload and review'}})
        : route.fulfill({json: f.snapshot()});
    });
    await page.getByRole('button', {name: mode === 'story' ? 'Publish as Jira story…' : 'Publish as Confluence RFC…', exact: true}).click();
    const dialog = page.getByRole('dialog', {name: mode === 'story' ? 'Publish as Jira story' : 'Publish as Confluence RFC', exact: true});
    const preview = dialog.getByTestId('publish-preview');
    await expect(preview).toContainText('Reviewed café.');
    await preview.locator('summary').click();
    await expect(preview.locator('details pre')).toHaveText(originalBody);
    await expect(preview.locator('.pd-preview-title')).toHaveText(f.content.title);
    const oldTitle = f.content.title;
    f.content.body = revisedBody; f.content.title = `${oldTitle} revised`;
    const publish = dialog.getByRole('button', {name: mode === 'story' ? 'Publish story' : 'Publish RFC', exact: true});
    await publish.click();
    await expect(dialog).toBeVisible();
    await expect(preview.getByRole('alert')).toContainText('Reload the preview');
    await expect(publish).toBeDisabled();
    expect(sent[0].reviewed_content).toEqual({version_id: f.versionId, body_sha256: digest(originalBody), title: oldTitle, source_kind: 'draft', url: f.snapshot().story.url});
    // The failed modal remains editable; source reload does not reset destination.
    if (mode === 'rfc') await dialog.locator('#pd-parent').fill('4242');
    else await dialog.locator('#pd-issuetype').selectOption('Task');
    await dialog.getByRole('button', {name: 'Retry preview', exact: true}).click();
    await expect(preview).toContainText('Revised café.');
    await expect(preview.locator('.pd-preview-title')).toHaveText(f.content.title);
    if (await preview.locator('details').getAttribute('open') === null) await preview.locator('summary').click();
    await expect(preview.locator('details pre')).toHaveText(revisedBody);
    await expect(publish).toBeEnabled();
    await publish.click();
    await expect(dialog).toHaveCount(0);
    expect(sent).toHaveLength(2);
    expect(sent[1].reviewed_content).toEqual({version_id: f.versionId, body_sha256: digest(revisedBody), title: f.content.title, source_kind: 'draft', url: f.snapshot().story.url});
    if (mode === 'rfc') expect(sent[1]).toMatchObject({account_id: 'A', space_key: 'RFC', parent_id: '4242'});
    else expect(sent[1]).toMatchObject({account_id: 'A', project_key: 'REVIEW', issue_type: 'Task'});
  });
}

test('Product RFC destination retry preserves title and parent and ignores a departed account response', async ({page}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop mounted acceptance');
  const f = await fixture(page);
  let bCalls = 0, release!: () => void, staleCompleted = false;
  const gate = new Promise<void>(resolve => {release = resolve;});
  await page.context().route('**/api/v1/issue/confluence/spaces?*', async route => {
    if (new URL(route.request().url()).searchParams.get('account_id') === 'B') {
      if (++bCalls === 1) return route.fulfill({status: 503, json: {code: 'unavailable', message: 'Synthetic spaces failure'}});
      await gate;
      await route.fulfill({json: [{key: 'OLD', name: 'Departed account space'}]}); staleCompleted = true;
      return;
    }
    return route.fulfill({json: [{key: 'CURRENT', name: 'Current account space'}]});
  });
  const sent: Record<string, any>[] = [];
  await page.context().route(`**/api/v1/product/stories/${f.id}/publish-as-rfc`, route => {
    sent.push(route.request().postDataJSON()); return route.fulfill({json: f.snapshot()});
  });
  try {
    await page.getByRole('button', {name: 'Publish as Confluence RFC…', exact: true}).click();
    const dialog = page.getByRole('dialog', {name: 'Publish as Confluence RFC', exact: true});
    await expect(dialog.locator('#pd-space')).toHaveValue('CURRENT');
    await dialog.locator('#pd-title').fill('My exact RFC title');
    await dialog.locator('#pd-parent').fill('123456');
    await dialog.locator('#pd-account').selectOption('B');
    await expect(dialog.getByRole('alert')).toContainText('Couldn’t load Confluence spaces');
    await dialog.getByRole('button', {name: 'Retry spaces', exact: true}).click();
    await expect.poll(() => bCalls).toBe(2);
    await dialog.locator('#pd-account').selectOption('A');
    await expect(dialog.locator('#pd-space')).toHaveValue('CURRENT');
    release(); await expect.poll(() => staleCompleted).toBe(true);
    await expect(dialog.locator('#pd-space')).toHaveValue('CURRENT');
    await expect(dialog.locator('#pd-space option')).toHaveText('Current account space (CURRENT)');
    await expect(dialog.locator('#pd-title')).toHaveValue('My exact RFC title');
    await expect(dialog.locator('#pd-parent')).toHaveValue('123456');
    await expect(dialog.getByTestId('publish-visibility')).toContainText('under page 123456');
    await dialog.getByRole('button', {name: 'Publish RFC', exact: true}).click();
    await expect(dialog).toHaveCount(0);
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({account_id: 'A', space_key: 'CURRENT', parent_id: '123456', title: 'My exact RFC title'});
  } finally {release();}
});
