import {test, expect, type APIRequestContext} from '@playwright/test';
import {apiCtx, seedWorkspace} from './seed';
import {withUsage} from './chat-fixture';

// (The child-history Chat cases went with the session Chat view.)
test.use({serviceWorkers: 'block'});
let ctx: APIRequestContext, base: string, wsId: string, sessionId: string;

test.beforeEach(async ({page}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser acceptance');
  ({ctx, base} = await apiCtx());
  wsId = await seedWorkspace(ctx, base);
  const response = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: {kind: 'agent', provider: 'shell', title: 'Recovery fixture', cwd: '/tmp', meta: {nested_provider: 'claude', origin: 'e2e'}},
  });
  expect(response.ok()).toBe(true);
  sessionId = (await response.json()).id;
  await withUsage(page);
  await page.addInitScript(({wsId, sessionId}) => {
    localStorage.setItem('otto_workspace', wsId);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
  }, {wsId, sessionId});
});
test.afterEach(async () => { await ctx?.dispose(); });

test('mixed session batch retains submitted settings and retries only its failed member', async ({page}) => {
  const submitted: Record<string, unknown>[] = [];
  await page.route(`**/workspaces/${wsId}/sessions`, async route => {
    if (route.request().method() !== 'POST') return route.continue();
    submitted.push(route.request().postDataJSON());
    if (submitted.length === 2) return route.fulfill({status: 503, json: {code: 'unavailable', message: 'Synthetic second-member failure'}});
    return route.continue();
  });
  await page.goto(`/#/agents/${sessionId}`);
  await page.getByRole('button', {name: 'New session', exact: true}).click();
  const dialog = page.getByRole('dialog', {name: 'New session', exact: true});
  await dialog.locator('.provider-card', {hasText: 'shell'}).locator('.card-main').click();
  await dialog.getByLabel('One more shell session').click();
  await dialog.locator('#ns-title').fill('Recovery batch');
  await dialog.locator('#ns-cwd').fill('/tmp');
  await dialog.getByRole('button', {name: 'Start 2 sessions', exact: true}).click();
  await expect(dialog).toBeVisible();
  // The batch-result region (the dialog also keeps a session-count status).
  await expect(dialog.getByRole('status').filter({ hasText: /started;/ })).toContainText('1 started; 1 still need to start');
  expect(submitted).toHaveLength(2);
  await dialog.locator('#ns-title').fill('Changed after failure');
  await dialog.locator('#ns-cwd').fill('/');
  await dialog.getByRole('button', {name: 'Retry 1 failed', exact: true}).click();
  await expect(dialog).toHaveCount(0);
  expect(submitted).toHaveLength(3);
  expect(submitted[2]).toEqual(submitted[1]);
  const rows = await (await ctx.get(`${base}/api/v1/workspaces/${wsId}/sessions`)).json() as {title: string}[];
  expect(rows.filter(row => row.title === 'Recovery batch 1')).toHaveLength(1);
  expect(rows.filter(row => row.title === 'Recovery batch 2')).toHaveLength(1);
  expect(rows).toHaveLength(3);
});
