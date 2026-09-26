import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage, openApiEditor, expectFullyInViewport } from './helpers';

async function workspace(page: Page) {
  const { ctx, base } = await apiCtx();
  const id = await seedWorkspace(ctx, base);
  await page.addInitScript(id => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, id);
  return { ctx, base, id };
}

async function files(page: Page) {
  const { ctx, base, id } = await workspace(page);
  const name = `UX files ${id}`;
  const response = await ctx.post(`${base}/api/v1/workspaces/${id}/connections`, {
    data: { name, kind: 'ssh', params: { host: 'fixture.invalid' } },
  });
  expect(response.ok()).toBeTruthy();
  const conn = await response.json();
  await ctx.dispose();
  await page.route(`**/connections/${conn.id}/sftp/list**`, route => {
    const path = new URL(route.request().url()).searchParams.get('path') ?? '/home';
    return route.fulfill({ json: { path, entries: path === '/home' ? [
      { name: 'reports', kind: 'dir', size: 0, mtime: '2026-09-25', perms: 'drwxr-xr-x', symlink_target: null },
    ] : [] } });
  });
  await page.route(`**/connections/${conn.id}/sftp/transfers`, route => route.fulfill({ json: [] }));
  await openPage(page, 'connections');
  await page.locator('.conn-row', { hasText: name }).click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Browse files (SFTP)', exact: true }).click();
  await expect(page.locator('.sftp .nav')).toHaveText('reports');
}

for (const input of ['click', 'Space'] as const) {
  test(`SFTP opens a directory with ${input}`, async ({ page }) => {
    await files(page);
    const directory = page.locator('.sftp .nav');
    if (input === 'click') await directory.click();
    else { await directory.focus(); await directory.press('Space'); }
    await expect(page.locator('.sftp .crumbs')).toContainText('reports');
    await expect(page.locator('.sftp .list')).toContainText('Empty directory.');
  });
}

test('SFTP phone keeps filter, directory name and file actions visible', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await files(page);
  await expectFullyInViewport(page, page.getByLabel('Filter files in this directory'));
  await expectFullyInViewport(page, page.locator('.sftp .nav'));
  expect((await page.locator('.sftp .nav').boundingBox())!.width).toBeGreaterThan(100);
  await expectFullyInViewport(page, page.locator('.sftp').getByRole('button', { name: 'Delete', exact: true }));
});

test('API phone exposes response tabs and JSON body modes', async ({ page }) => {
  const { ctx } = await workspace(page);
  await ctx.dispose();
  await page.route('**/api-client/execute', route => route.fulfill({ json: {
    status: 200, status_text: 'OK', headers: [{ key: 'Content-Type', value: 'application/json' }, { key: 'Set-Cookie', value: 'session=fixture; Secure' }],
    body: '{"message":"hello"}', body_base64: '', truncated: false, too_large: false,
    duration_ms: 10, size_bytes: 19, content_type: 'application/json', trace: [{ label: 'Response', detail: 'Fixture response', ms: 10, level: 'success' }],
  } }));
  await page.setViewportSize({ width: 375, height: 812 });
  await openApiEditor(page);
  await page.getByLabel('Request URL', { exact: true }).fill('https://fixture.invalid/response');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  const responseTabs = page.getByRole('tablist', { name: 'Response', exact: true });
  await expect(responseTabs).toBeVisible();
  await responseTabs.scrollIntoViewIfNeeded();
  await expectFullyInViewport(page, responseTabs.getByRole('tab', { name: 'Timeline' }));
  await expectFullyInViewport(page, page.getByRole('button', { name: 'Tree', exact: true }));
});
