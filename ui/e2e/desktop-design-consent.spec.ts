import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { STARTER_KITS } from '../src/modules/design-hall/brand/starters';
import { expectNoHorizontalOverflow } from './helpers';
import { mkdir } from 'node:fs/promises';

for (const brand of [false, true]) {
  test(`${brand ? 'brand kit' : 'design'} approval stays on the version shown before a live update`, async ({ page }) => {
    const { ctx, base } = await apiCtx();
    const workspace = await seedWorkspace(ctx, base);
    const content = brand ? JSON.stringify(STARTER_KITS[0].build('Consent brand')) : '<h1>Confirmed version</h1>';
    const response = await ctx.post(`${base}/api/v1/design/artifacts`, { data: {
      workspace_id: workspace, format: brand ? 'otto-brand' : 'html', title: brand ? 'Consent brand' : 'Consent design', content,
    } });
    expect(response.ok()).toBeTruthy();
    const first = await response.json();
    const id = first.artifact.id;
    await page.addInitScript((workspace) => { localStorage.setItem('otto_workspace', workspace); localStorage.setItem('otto_scheme', 'light'); }, workspace);
    await page.goto(`/#/design/${brand ? 'brand' : 'a'}/${id}`);
    if (brand) {
      await expect(page.getByTestId('brand-editor')).toBeVisible();
      await page.getByTestId('brand-approve').click();
    } else {
      await expect(page.getByTestId('design-stage')).toBeVisible();
      await page.getByTestId('design-status').click();
      await page.getByRole('menuitem', { name: /Approve v1/ }).click();
    }
    const dialog = page.getByRole('dialog', { name: brand ? 'Accept brand kit' : 'Approve design', exact: true });
    await expect(dialog).toContainText('v1');
    const changed = brand ? JSON.stringify({ ...JSON.parse(content), name: 'Updated brand' }) : '<h1>Unseen later version</h1>';
    const update = await ctx.put(`${base}/api/v1/design/artifacts/${id}/content`, { data: { content: changed, base_version: first.version.id } });
    expect(update.ok(), await update.text()).toBeTruthy();
    // Wait for the real websocket event to refresh the component while its
    // confirmation remains open. The request below must still approve v1.
    if (brand) await expect(page.getByTestId('brand-approve')).toContainText('v2');
    else await expect(page.getByTestId('design-version-chip').filter({ hasText: 'v2' })).toBeVisible();
    const sent = page.waitForRequest((r) => r.url().endsWith(`/design/artifacts/${id}/approve`) && r.method() === 'POST');
    await dialog.getByRole('button', { name: brand ? 'Accept v1' : 'Approve v1', exact: true }).click();
    expect((await sent).postDataJSON()).toEqual({ version_id: first.version.id });
    await expect(dialog).toHaveCount(0);
    await expectNoHorizontalOverflow(page);
    if (!brand) {
      await expect(page.getByTestId('design-status')).toContainText('Draft');
      await expect(page.getByTestId('design-status')).not.toContainText('Approved');
    }
    if (brand) {
      await expect(page.getByTestId('brand-editor')).toContainText('still use v1');
      await expect(page.locator('.page-header-bar .ph-badge')).toContainText('Draft');
      await expect(page.locator('.page-header-bar .ph-badge')).not.toContainText('Approved');
      const dir = '../docs/reviews/quality-20261008/evidence/R16';
      await mkdir(dir, { recursive: true });
      await page.setViewportSize({ width: 1440, height: 900 });
      await page.screenshot({ path: `${dir}/brand-light.png`, animations: 'disabled' });
      await page.evaluate(() => { localStorage.setItem('otto_scheme', 'dark'); document.documentElement.dataset.scheme = 'dark'; });
      await page.screenshot({ path: `${dir}/brand-dark.png`, animations: 'disabled' });
      await page.setViewportSize({ width: 390, height: 844 });
      await expectNoHorizontalOverflow(page);
      await page.screenshot({ path: `${dir}/brand-phone.png`, animations: 'disabled' });
    }
    await ctx.dispose();
  });
}
