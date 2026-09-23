import { test, expect } from '@playwright/test';
import { mkdtempSync, writeFileSync, rmSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { apiCtx, seedWorkspace } from './seed';

test('Git backup preview commits only snapshot files and supports reviewed imports', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser only');
  test.setTimeout(120_000);
  const repo = mkdtempSync(join(tmpdir(), 'otto-backup-git-ui-'));
  const git = (...args: string[]) => execFileSync('git', args, { cwd: repo, encoding: 'utf8' }).trim();
  const { ctx, base } = await apiCtx();
  try {
    git('init', '--initial-branch=main');
    git('config', 'user.name', 'Backup fixture'); git('config', 'user.email', 'backup@example.test');
    writeFileSync(join(repo, 'README.md'), 'Backup fixture\n');
    git('add', 'README.md'); git('commit', '-m', 'test: seed backup repository');
    writeFileSync(join(repo, 'unrelated.txt'), 'Keep this staged user work\n'); git('add', 'unrelated.txt');
    const ws = await seedWorkspace(ctx, base);
    const created = await ctx.post(`${base}/api/v1/workspaces/${ws}/workflows`, { data: { name: 'Git archived workflow', graph: { nodes: [], edges: [] } } });
    expect(created.ok()).toBeTruthy();
    await page.goto('/#/settings/backup');
    const card = page.getByRole('region', { name: 'Git backup sync', exact: true });
    await card.getByLabel('Existing local repository').fill(repo);
    await card.getByRole('button', { name: 'Check repository', exact: true }).click();
    await expect(card.getByText('Local changes present', { exact: false })).toBeVisible();
    await card.getByRole('button', { name: 'Preview snapshot', exact: true }).click();
    await expect(card.getByLabel('Git snapshot preview')).toContainText('config/workflows.json');
    await card.getByRole('button', { name: 'Write snapshot', exact: true }).click();
    await expect(card.getByRole('button', { name: 'Snapshot written', exact: true })).toBeDisabled();
    await card.getByLabel('Snapshot commit message').fill('chore: save portable configuration');
    await card.getByRole('button', { name: 'Commit snapshot', exact: true }).click();
    await expect(card.getByRole('status')).toContainText('Snapshot committed');
    expect(git('diff', '--cached', '--name-only')).toBe('unrelated.txt');
    expect(git('show', 'HEAD:.otto-sync/config/workflows.json')).toContain('Git archived workflow');
    expect(readFileSync(join(repo, 'unrelated.txt'), 'utf8')).toBe('Keep this staged user work\n');
    await card.getByRole('button', { name: 'Preview snapshot', exact: true }).click();
    await expect(card.getByLabel('Git snapshot preview')).toContainText('0 changed files');
    await card.getByRole('button', { name: 'Preview Git restore', exact: true }).click();
    await expect(card.getByLabel('Restore preview')).toBeVisible();
    await card.getByLabel('I reviewed the contents, conflicts, and reconnect requirements.').check();
    await card.getByRole('button', { name: 'Restore new items', exact: true }).click();
    // A restore is one whole-state SQLite transaction: well under a second on
    // an idle daemon, but on the shared e2e daemon it queues behind other
    // workers' writes (e.g. desktop-backup-archive's own restore running in
    // parallel), so allow it more than the default 10 s to report.
    await expect(card.getByRole('status')).toContainText('Restored', { timeout: 60_000 });
    expect(git('diff', '--cached', '--name-only')).toBe('unrelated.txt');
  } finally { await ctx.dispose(); rmSync(repo, { recursive: true, force: true }); }
});
