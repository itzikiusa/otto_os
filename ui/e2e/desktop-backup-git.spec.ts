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
    const panel = card.getByLabel('Git snapshot preview');
    const previewButton = card.getByRole('button', { name: 'Preview snapshot', exact: true });
    // A preview is bound to the daemon state it showed: Write refuses a stale
    // one ("Couldn't write the snapshot…") — the product working. The snapshot
    // is daemon-wide, and on this shared e2e daemon other workers seed and
    // rename workspaces at any moment (config/workspaces.json), so a
    // Preview → Write they disturbed is simply redone.
    const writeSnapshot = () => expect(async () => {
      await previewButton.click();
      await card.getByRole('button', { name: 'Write snapshot', exact: true }).click();
      await expect(card.getByRole('button', { name: 'Snapshot written', exact: true })).toBeDisabled({ timeout: 5_000 });
    }).toPass({ timeout: 45_000 });
    const commitSnapshot = async (message: string) => {
      await card.getByLabel('Snapshot commit message').fill(message);
      await card.getByRole('button', { name: 'Commit snapshot', exact: true }).click();
      await expect(card.getByRole('status')).toContainText('Snapshot committed');
      // Only the snapshot was committed, all of it, and staged user work stays staged.
      expect(git('diff', '--cached', '--name-only')).toBe('unrelated.txt');
      expect(git('status', '--porcelain', '--', '.otto-sync')).toBe('');
    };
    await previewButton.click();
    await expect(panel).toContainText('config/workflows.json');
    await writeSnapshot();
    await commitSnapshot('chore: save portable configuration');
    expect(git('show', 'HEAD:.otto-sync/config/workflows.json')).toContain('Git archived workflow');
    expect(readFileSync(join(repo, 'unrelated.txt'), 'utf8')).toBe('Keep this staged user work\n');
    // Right after a commit, a fresh preview has nothing left to write. The
    // snapshot is daemon-wide and other workers add a workspace every second
    // or two here, which a multi-second UI cycle can't outrun. So bring the
    // repository level with the daemon over the API (a sub-second preview →
    // export → commit) and then look from the UI. A snapshot that never
    // settles (a volatile field, unstable ordering) still fails this.
    const gitApi = async (step: string, data: object) => {
      const r = await ctx.post(`${base}/api/v1/state/git/${step}`, { data });
      expect(r.ok(), `${step}: ${r.status()} ${await r.text()}`).toBeTruthy();
      return r.json();
    };
    await expect(async () => {
      const fresh = await gitApi('preview', { repo_path: repo });
      if (fresh.changes.some((c: { action: string }) => c.action !== 'unchanged')) {
        const written = await gitApi('export', { repo_path: repo, preview_token: fresh.token });
        await gitApi('commit', {
          repo_path: repo, expected_head: written.status.head, snapshot_digest: written.snapshot_digest,
          message: 'chore: follow concurrent configuration changes',
        });
      }
      await previewButton.click();
      await expect(panel.locator('strong').first()).toHaveText('0 changed files', { timeout: 3_000 });
    }).toPass({ timeout: 60_000 });
    await expect(panel).toContainText('The snapshot matches this repository.');
    expect(git('diff', '--cached', '--name-only')).toBe('unrelated.txt');
    expect(git('status', '--porcelain', '--', '.otto-sync')).toBe('');
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
