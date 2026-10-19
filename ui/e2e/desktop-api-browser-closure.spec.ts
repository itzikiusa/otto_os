import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedGitRepo } from './seed';
import { expectNoHorizontalOverflow, openApiEditor } from './helpers';

test.use({ serviceWorkers: 'block', viewport: { width: 390, height: 844 } });

test('phone request tabs name edits and completion reveals the response', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript(ws => localStorage.setItem('otto_workspace', ws), ws);
  await page.route('**/api-client/execute', r => r.fulfill({ json: {
    status: 200, status_text: 'OK', headers: [], body: 'Visible response', body_base64: '',
    truncated: false, too_large: false, duration_ms: 2, size_bytes: 16, content_type: 'text/plain', trace: [],
  } }));
  await openApiEditor(page);
  await page.getByLabel('Request URL', { exact: true }).fill('https://example.invalid/response');
  await expect(page.locator('.req-tab-main').first()).toHaveAccessibleName(/unsaved changes/);
  const close = page.locator('.req-tab-close').first();
  await expect(close).toHaveAccessibleName(/Close tab .+/);
  expect((await close.boundingBox())!.height).toBeGreaterThanOrEqual(36);
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.locator('.req-tab-main').first()).toHaveAccessibleName(/last send succeeded/);
  await expect(page.locator('.resp-pane')).toBeInViewport();
  await expectNoHorizontalOverflow(page);
});

test('site credentials are reachable and summaries identify degraded fetches', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript(ws => localStorage.setItem('otto_workspace', ws), ws);
  await page.route('**/browser/page?url=**', r => r.fulfill({ json: { url: 'https://example.invalid/page', title: 'Page', markdown: 'Body', html: '<p>Body</p>', engine: 'plain-fetch', degraded: true } }));
  await page.route('**/browser/summarize', r => r.fulfill({ json: { summary: 'A draft summary', engine: 'plain-fetch', degraded: true } }));
  await page.goto('/#/browser');
  await page.getByRole('button', { name: 'Site credentials', exact: true }).click();
  const modal = page.getByRole('dialog', { name: 'Site credentials', exact: true });
  await expect(modal.getByText('No saved site credentials', { exact: true })).toBeVisible();
  await expect(modal.getByRole('button', { name: 'Add credential', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(modal).not.toBeVisible();
  await page.getByPlaceholder('e.g. https://example.com').fill('https://example.invalid/page');
  await page.getByPlaceholder('e.g. https://example.com').press('Enter');
  await page.getByRole('button', { name: 'Summarize this page' }).click();
  await expect(page.locator('.summary')).toContainText('A draft summary');
  await expect(page.locator('.summary')).toContainText('plain-fetch');
  await expect(page.locator('.summary')).toContainText('No JavaScript ran');
  await expectNoHorizontalOverflow(page);
});

test('failed collection pull stays open and Retry closes only after success', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await seedGitRepo(ctx, base, ws);
  await ctx.dispose();
  await page.addInitScript(ws => localStorage.setItem('otto_workspace', ws), ws);
  let fail = true;
  await page.route('**/api-collections/pull', r => fail
    ? r.fulfill({ status: 503, json: { message: 'Repository unavailable' } })
    : r.fulfill({ json: { files: [] } }));
  await openApiEditor(page);
  await page.getByRole('button', { name: 'Sync with Git…', exact: true }).click();
  const modal = page.getByRole('dialog', { name: 'Sync collections with Git' });
  await modal.getByRole('button', { name: 'Pull', exact: true }).click();
  await expect(modal).toBeVisible();
  await expect(modal.getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
  fail = false;
  await modal.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(modal).not.toBeVisible();
});
