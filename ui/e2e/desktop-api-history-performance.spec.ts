import { expect, test, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// Mounted production components; only history transport is mocked. No requests
// are sent to an upstream server and no provider/session is launched.
async function historyFixture(page: Page, deferred = false) {
  const { ctx, base } = await apiCtx();
  let workspaceId: string;
  try { workspaceId = await seedWorkspace(ctx, base); }
  finally { await ctx.dispose(); }
  const endpoint = `/api/v1/workspaces/${workspaceId}/api-client/history`;
  const summary = {
    id: 'history-selected', workspace_id: workspaceId, method: 'GET',
    url: 'https://summary.fixture.invalid/list-only', status: 201, duration_ms: 17,
    executed_at: '2026-09-13T10:00:00Z', request_id: null,
    source: { kind: 'user', session_id: null, via: null },
  };
  const snapshot = {
    method: 'POST', url: 'https://replay.fixture.invalid/exact-detail',
    headers: [{ key: 'X-Replay', value: 'detail-header', enabled: true }],
    query: [{ key: 'origin', value: 'detail-query', enabled: true }],
    body_mode: 'json', body: '{"origin":"detail-only","sequence":73}', auth: { type: 'none' },
  };
  const detail = { ...summary, request: snapshot, response: { body: 'stored-detail-only-response' } };
  const calls: string[] = [];
  let release!: () => void;
  let started!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const detailStarted = new Promise<void>(resolve => { started = resolve; });
  let delivered = 0;
  await page.route(`**${endpoint}**`, async route => {
    const path = new URL(route.request().url()).pathname;
    calls.push(path.slice(endpoint.length));
    if (path === `${endpoint}/summaries`) return route.fulfill({ json: [summary] });
    if (path === `${endpoint}/${summary.id}`) {
      started();
      if (deferred) await gate;
      await route.fulfill({ json: detail });
      delivered++;
      return;
    }
    // An accidental legacy full-history fetch is a failure, not hidden by a
    // catch-all mock returning valid full entries.
    return route.fulfill({ status: 500, json: { code: 'unexpected_history_fetch', message: path } });
  });
  await page.addInitScript(id => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, workspaceId);
  await openPage(page, 'api');
  await page.getByRole('tab', { name: 'History', exact: true }).click();
  await expect(page.locator(`.hist-row[data-url="${summary.url}"]`)).toBeVisible();
  return {
    calls, summary, snapshot, release, detailStarted,
    delivered: () => delivered,
    select: () => page.locator(`.hist-row[data-url="${summary.url}"]`).click(),
  };
}

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser only');
});

test('sidebar uses summaries and selects one exact detail before restoring replay fields', async ({ page }) => {
  const f = await historyFixture(page);
  expect(f.calls.length).toBeGreaterThan(0);
  expect(f.calls.every(path => path === '/summaries')).toBe(true);
  expect(f.summary).not.toHaveProperty('request');
  expect(f.summary).not.toHaveProperty('response');
  const detailResponse = page.waitForResponse(response => new URL(response.url()).pathname.endsWith(`/history/${f.summary.id}`));
  await f.select();
  const payload = await (await detailResponse).json();
  expect(payload.request).toEqual(f.snapshot);
  expect(payload.response.body).toBe('stored-detail-only-response');
  await expect(page.getByLabel('Request URL', { exact: true })).toHaveValue(f.snapshot.url);
  await expect(page.getByLabel('HTTP method', { exact: true })).toHaveValue('POST');
  const builder = page.locator('.builder');
  await builder.getByRole('tab', { name: 'Params', exact: true }).click();
  await expect(builder.getByPlaceholder('key', { exact: true }).first()).toHaveValue('origin');
  await expect(builder.getByPlaceholder('value', { exact: true }).first()).toHaveValue('detail-query');
  await builder.getByRole('tab', { name: 'Headers', exact: true }).click();
  await expect(builder.getByPlaceholder('key', { exact: true }).first()).toHaveValue('X-Replay');
  await expect(builder.getByPlaceholder('value', { exact: true }).first()).toHaveValue('detail-header');
  await builder.getByRole('tab', { name: 'Body', exact: true }).click();
  await expect(builder.locator('.cm-content')).toContainText(f.snapshot.body);
  expect(f.calls.filter(path => path !== '/summaries')).toEqual([`/${f.summary.id}`]);
});

test('late detail cannot overwrite a newly selected tab or its originating draft', async ({ page }) => {
  const f = await historyFixture(page, true);
  const url = page.getByLabel('Request URL', { exact: true });
  await url.fill('https://original.fixture.invalid/kept');
  await f.select();
  await f.detailStarted;
  await expect(page.getByText('Loading request…', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'New request tab', exact: true }).click();
  await url.fill('https://other-tab.fixture.invalid/kept');
  f.release();
  await expect.poll(f.delivered).toBe(1);
  await expect(page.getByText('Loading request…', { exact: true })).toBeHidden();
  await expect(url).toHaveValue('https://other-tab.fixture.invalid/kept');
  await page.locator('.req-tab').first().click();
  await expect(url).toHaveValue('https://original.fixture.invalid/kept');
});

test('late detail cannot replace edits made while its request is pending', async ({ page }) => {
  const f = await historyFixture(page, true);
  await f.select();
  await f.detailStarted;
  await page.getByLabel('Request URL', { exact: true }).fill('https://edited.fixture.invalid/kept');
  await page.getByLabel('HTTP method', { exact: true }).selectOption('PATCH');
  f.release();
  await expect.poll(f.delivered).toBe(1);
  await expect(page.getByText('Loading request…', { exact: true })).toBeHidden();
  await expect(page.getByLabel('Request URL', { exact: true })).toHaveValue('https://edited.fixture.invalid/kept');
  await expect(page.getByLabel('HTTP method', { exact: true })).toHaveValue('PATCH');
  expect(f.calls.filter(path => path === `/${f.summary.id}`)).toHaveLength(1);
});
