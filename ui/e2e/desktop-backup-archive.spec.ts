import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import type { StateArchive } from '../src/lib/api/types';

test('data archives include feature records and Vault files with reviewed additive restore', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser only');
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  const post = async (path: string, data: unknown) => {
    const response = await ctx.post(`${base}/api/v1${path}`, { data });
    expect(response.ok(), await response.text()).toBeTruthy();
    return response.json();
  };
  try {
    const workflow = await post(`/workspaces/${ws}/workflows`, { name: 'Archive workflow', graph: { nodes: [{ id: 'start', kind: 'start', name: 'Start', params: {} }], edges: [] } });
    const task = await post(`/workspaces/${ws}/scheduled-tasks`, { name: 'Archive task', prompt: 'Saved task text', schedule: { cadence: 'interval', every_min: 60 }, destination: { type: 'none' }, enabled: false });
    const connection = await post(`/workspaces/${ws}/connections`, { name: 'Archive database', kind: 'redis', params: { host: 'archive.invalid' }, secret: 'fixture-archive-password-4938' });
    await seedVaultDir(ctx, base, ws);
    await page.goto('/#/settings/backup');
    const card = page.getByRole('region', { name: 'Otto data backup', exact: true });
    const downloadPromise = page.waitForEvent('download');
    await card.getByRole('button', { name: 'Download data archive', exact: true }).click();
    const downloaded = await downloadPromise;
    const path = await downloaded.path();
    expect(path).toBeTruthy();
    const text = readFileSync(path!, 'utf8');
    const archive = JSON.parse(text) as StateArchive;
    // Match the server's encoded size limit; pretty printing can make a valid archive too large to import.
    expect(text).toBe(JSON.stringify(archive));
    expect(archive.archive_format).toBe(2);
    expect(archive.records.workflows?.some(row => row.id === workflow.id)).toBeTruthy();
    expect(archive.records.scheduled_tasks?.some(row => row.id === task.id)).toBeTruthy();
    expect(archive.records.connections?.some(row => row.id === connection.id)).toBeTruthy();
    expect(text).not.toContain('fixture-archive-password-4938');
    expect(archive.files.some(file => file.path.endsWith('auth-api.md'))).toBeTruthy();
    await card.locator('input[type=file]').setInputFiles(path!);
    await card.getByLabel('When an item already exists').selectOption('abort');
    await card.getByRole('button', { name: 'Preview restore', exact: true }).click();
    await expect(card.getByText('This archive cannot be restored with the selected policy.', { exact: false })).toBeVisible();
    await expect(card.getByRole('button', { name: 'Restore new items', exact: true })).toBeDisabled();
    await card.getByLabel('When an item already exists').selectOption('skip_existing');
    await card.getByRole('button', { name: 'Preview restore', exact: true }).click();
    await card.getByLabel('I reviewed the contents, conflicts, and reconnect requirements.').check();
    await card.getByRole('button', { name: 'Restore new items', exact: true }).click();
    await expect(card.getByRole('status').last()).toContainText('Restored');
    expect((await ctx.get(`${base}/api/v1/workflows/${workflow.id}`)).ok()).toBeTruthy();
    const connections = await (await ctx.get(`${base}/api/v1/workspaces/${ws}/connections`)).json();
    expect(connections.find((row: { id: string }) => row.id === connection.id).secret_ref).toBe(connection.secret_ref);
    // Archive contents must be checked again after tampering; no restore button should remain enabled.
    archive.files[0].content_base64 = Buffer.from('tampered').toString('base64');
    await card.locator('input[type=file]').setInputFiles({ name: 'tampered.json', mimeType: 'application/json', buffer: Buffer.from(JSON.stringify(archive)) });
    await card.getByRole('button', { name: 'Preview restore', exact: true }).click();
    await expect(card.getByRole('alert')).toBeVisible();
    await expect(card.getByRole('button', { name: 'Restore new items', exact: true })).toHaveCount(0);
  } finally { await ctx.dispose(); }
});
