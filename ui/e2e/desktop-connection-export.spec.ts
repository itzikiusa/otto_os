import { test, expect, type Page, type Locator } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { apiCtx, seedWorkspace } from './seed';

async function download(page: Page, button: Locator) {
  const pending = page.waitForEvent('download');
  await button.click();
  const file = await pending;
  return readFileSync((await file.path())!, 'utf8');
}

test('connection export covers all workspaces and includes passwords only by opt-in', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser only');
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  try {
    const a = await seedWorkspace(ctx, base), b = await seedWorkspace(ctx, base);
    await ctx.patch(`${base}/api/v1/workspaces/${a}`, { data: { name: 'Export A' } });
    await ctx.patch(`${base}/api/v1/workspaces/${b}`, { data: { name: 'Export B' } });
    for (const [ws, kind, name, params] of [
      [a, 'mysql', 'Export MySQL', { host: 'mysql.invalid', port: 3306, user: 'fixture', db: 'saved' }],
      [b, 'mongodb', 'Export Mongo', { conn_string: 'mongodb://fixture@mongo.invalid:27017/saved' }],
      [a, 'redis', 'Export Redis', { host: 'redis.invalid', port: 6379, user: 'default' }],
      [a, 'ssh', 'Export SSH', { host: 'ssh.invalid', user: 'fixture' }],
    ] as const) {
      const r = await ctx.post(`${base}/api/v1/workspaces/${ws}/connections`, { data: { name, kind, params, secret: 'fixture-export-password-7821' } });
      expect(r.ok(), await r.text()).toBeTruthy();
    }
    await page.goto('/#/settings/backup');
    const card = page.getByRole('region', { name: 'Connection export', exact: true });
    await expect(card.getByLabel('Include saved passwords and credentials')).not.toBeChecked();
    await card.getByRole('button', { name: 'Prepare export', exact: true }).click();
    let text = await download(page, card.getByRole('button', { name: /^Download / }).first());
    for (const name of ['Export MySQL', 'Export Mongo', 'Export Redis', 'Export SSH']) expect(text).toContain(name);
    expect(text).not.toContain('fixture-export-password-7821');
    await card.getByLabel('Include saved passwords and credentials').check();
    await card.getByRole('button', { name: 'Prepare export with passwords', exact: true }).click();
    text = await download(page, card.getByRole('button', { name: /^Download / }).first());
    expect(text).toContain('fixture-export-password-7821');
    await card.getByLabel('Export format', { exact: true }).selectOption('mysql_workbench');
    await card.getByRole('button', { name: 'Prepare export with passwords', exact: true }).click();
    await expect(card.getByRole('button', { name: /^Download / })).toHaveCount(2);
    const xml = await download(page, card.getByRole('button', { name: /Download .*\.xml/ }));
    expect(xml).toContain('Export MySQL'); expect(xml).not.toContain('fixture-export-password-7821');
    const credentials = await download(page, card.getByRole('button', { name: /Download .*\.json/ }));
    expect(credentials).toContain('fixture-export-password-7821');
    await card.getByLabel('Connections to export').selectOption('workspaces');
    await card.getByRole('group', { name: 'Export workspaces' }).getByLabel('Export B', { exact: true }).check();
    await card.getByLabel('Export format', { exact: true }).selectOption('nosqlbooster');
    await card.getByRole('button', { name: 'Prepare export with passwords', exact: true }).click();
    const uris = await download(page, card.getByRole('button', { name: /^Download / }).first());
    expect(uris).toContain('mongodb://'); expect(uris).toContain('mongo.invalid'); expect(uris).not.toContain('mysql.invalid');
    await page.setViewportSize({ width: 390, height: 844 });
    // Phone: the Navigator is an off-canvas drawer with its own state, closed by
    // default (never the desktop sidebar's expanded preference) — so the page is
    // not covered and nothing needs dismissing. Assert that rather than assume it.
    await expect(page.getByRole('button', { name: 'Open navigator', exact: true })).toBeVisible();
    await expect(page.getByRole('dialog', { name: 'Navigator', exact: true })).toHaveCount(0);
    await card.scrollIntoViewIfNeeded();
    await expect(card).toBeVisible();
    await card.getByLabel('Export format', { exact: true }).selectOption('json');
    await card.getByLabel('Connections to export').selectOption('all');
    await card.getByLabel('Include saved passwords and credentials').uncheck();
    await card.getByRole('button', { name: 'Prepare export', exact: true }).click();
    const mobileExport = await download(page, card.getByRole('button', { name: /^Download / }).first());
    expect(mobileExport).toContain('Export MySQL');
    expect(mobileExport).not.toContain('fixture-export-password-7821');
    const bounds = await card.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(391);
    await card.screenshot({ path: '/tmp/otto-connection-export-mobile.png' });
  } finally { await ctx.dispose(); }
});
