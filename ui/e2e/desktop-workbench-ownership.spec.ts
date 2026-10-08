import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';
import { mkdir } from 'node:fs/promises';
let workspace = '', docId = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspace = await seedWorkspace(ctx, base, 'Workbench recovery review');
  const r = await ctx.post(`${base}/api/v1/workspaces/${workspace}/workbench/docs`, { data: { name: 'kept.txt', content: 'Original saved text', language: 'text' } });
  expect(r.ok()).toBeTruthy(); docId = (await r.json()).id; await ctx.dispose();
});
test('initial failure and Retry preserve selected file; failed save prevents trash', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(({ workspace, docId }) => {
    localStorage.setItem('otto_workspace', workspace); localStorage.setItem('otto_scheme', 'light');
    localStorage.setItem(`otto.wb.tabs.${workspace}`, JSON.stringify({ tabs: [docId], active: docId }));
  }, { workspace, docId });
  let failList = true, failSave = true, trashRequests = 0;
  await page.route(`**/api/v1/workspaces/${workspace}/workbench/docs`, async route => {
    if (failList && route.request().method() === 'GET') return route.fulfill({ status: 503, json: { error: 'Temporary fixture failure' } });
    return route.continue();
  });
  await page.route(`**/api/v1/workspaces/${workspace}/workbench/docs/${docId}`, async route => {
    if (route.request().method() === 'DELETE') trashRequests++;
    if (failSave && route.request().method() === 'PATCH') return route.fulfill({ status: 503, json: { error: 'Save fixture offline' } });
    return route.continue();
  });
  await page.goto('/#/workbench');
  await expect(page.getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
  expect(await page.evaluate(id => JSON.parse(localStorage.getItem(`otto.wb.tabs.${id}`) ?? '{}').active, workspace)).toBe(docId);
  failList = false; await page.getByRole('button', { name: 'Retry', exact: true }).click();
  const editor = page.getByTestId('wb-editor').locator('.cm-content');
  await expect(editor).toContainText('Original saved text');
  await editor.fill('My unsaved text must survive');
  await expect(page.getByTestId('wb-save-state')).toContainText('Not saved');
  await page.getByRole('button', { name: 'More actions', exact: true }).first().click();
  await page.getByRole('menuitem', { name: 'Move to trash', exact: true }).click();
  await expect(page.getByText('Couldn’t move to trash', { exact: true })).toBeVisible();
  await expect(editor).toContainText('My unsaved text must survive'); expect(trashRequests).toBe(0);
  failSave = false; await page.getByTestId('wb-save-state').getByRole('button', { name: 'Retry' }).click();
  await expect(page.getByTestId('wb-save-state')).toContainText('Saved'); await expectNoHorizontalOverflow(page);
  if (process.env.OTTO_REVIEW_ARTIFACT_DIR) {
    await mkdir(process.env.OTTO_REVIEW_ARTIFACT_DIR, { recursive: true });
    for (const scheme of ['light', 'dark']) {
      await page.evaluate(scheme => { localStorage.setItem('otto_scheme', scheme); document.documentElement.dataset.scheme = scheme; }, scheme);
      await page.screenshot({ path: `${process.env.OTTO_REVIEW_ARTIFACT_DIR}/workbench-${scheme}.png`, animations: 'disabled' });
    }
    await page.setViewportSize({ width: 390, height: 844 }); await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: `${process.env.OTTO_REVIEW_ARTIFACT_DIR}/workbench-phone-dark.png`, animations: 'disabled' });
  }
});
