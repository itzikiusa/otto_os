import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Git-flow branch rows (desktop-browser only).
//
// 1. "Already in develop / main": origin/HEAD is `main`, but a release lands in
//    `develop` first. The mark used to check only the default branch, so a
//    git-flow repo showed nothing until release day. The row must name every
//    integration branch holding the tip — here `develop`, not `main`.
// 2. Delete stays reachable: the branch menu was capped at 12 items, which
//    pushed Delete behind a "+N more — type to narrow" hint.
// ─────────────────────────────────────────────────────────────────────────────

const REPO_NAME = 'e2e-landed-repo';

function git(dir: string, ...args: string[]): string {
  return execFileSync('git', ['-C', dir, ...args], { encoding: 'utf8' }).trim();
}

function commit(dir: string, file: string, msg: string): void {
  writeFileSync(join(dir, file), `${msg}\n`);
  git(dir, 'add', '-A');
  git(dir, 'commit', '-q', '-m', msg);
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);

  const root = mkdtempSync(join(tmpdir(), 'otto-e2e-landed-'));
  const origin = join(root, 'origin.git');
  const repoDir = join(root, 'work');

  execFileSync('git', ['init', '-q', '--bare', '-b', 'main', origin]);
  execFileSync('git', ['clone', '-q', origin, repoDir]);
  git(repoDir, 'config', 'user.email', 'e2e@otto.local');
  git(repoDir, 'config', 'user.name', 'E2E');
  git(repoDir, 'config', 'commit.gpgsign', 'false');
  git(repoDir, 'checkout', '-q', '-b', 'main');
  commit(repoDir, 'a.txt', 'init');
  git(repoDir, 'checkout', '-q', '-b', 'develop');
  commit(repoDir, 'd.txt', 'develop work');
  git(repoDir, 'checkout', '-q', '-b', 'release/1.0');
  commit(repoDir, 'r.txt', 'release work');
  git(repoDir, 'checkout', '-q', 'develop');
  git(repoDir, 'merge', '-q', '--no-ff', '--no-edit', 'release/1.0');
  git(repoDir, 'push', '-q', '-u', 'origin', 'main', 'develop', 'release/1.0');
  git(repoDir, 'remote', 'set-head', 'origin', 'main');

  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, {
    data: { path: repoDir, name: REPO_NAME },
  });
  if (!r.ok()) throw new Error(`repo seed failed: ${r.status()} ${await r.text()}`);
  await ctx.dispose();
});

async function openRepo(page: Page): Promise<void> {
  await openPage(page, 'git');
  const existingTab = page.locator('.git-tab-name', { hasText: REPO_NAME });
  if (await existingTab.count()) {
    await existingTab.first().click();
  } else {
    await page.locator('.git-tab-new').click();
    const menu = page.locator('.ctx-menu');
    await menu.locator('.ctx-search-input').fill(REPO_NAME);
    await menu.getByRole('menuitem', { name: REPO_NAME }).first().click();
  }
  await expect(page.locator('.refs-panel')).toBeVisible({ timeout: 20_000 });
}

function localRow(page: Page, label: string) {
  return page
    .locator('.ref-row:not(.remote):not(.tag):not(.stash-row)')
    .filter({ has: page.locator('.ref-name', { hasText: new RegExp(`^${label}$`) }) })
    .first();
}

test('git-flow rows name where a branch landed, and Delete stays in the menu', async ({ page }) => {
  await openRepo(page);

  // The release (folder "release" → leaf "1.0") is in develop, not main.
  const release = localRow(page, '1.0');
  await expect(release.locator('.merged-mark')).toHaveText('develop', { timeout: 20_000 });
  await expect(release.locator('.merged-mark')).toHaveAttribute('title', /Already in develop — safe to delete/);
  const remoteRelease = page
    .locator('.ref-row.remote')
    .filter({ has: page.locator('.ref-name', { hasText: /^1\.0$/ }) })
    .first();
  await expect(remoteRelease.locator('.merged-mark')).toHaveText('develop');
  // Integration branches never flag each other (main ⊂ develop is noise).
  await expect(localRow(page, 'main').locator('.merged-mark')).toHaveCount(0);
  await expect(localRow(page, 'develop').locator('.merged-mark')).toHaveCount(0);

  // Every branch action is in the menu — nothing collapsed behind "+N more".
  await release.click({ button: 'right' });
  const menu = page.locator('.ctx-menu');
  await expect(menu).toBeVisible();
  await expect(menu.locator('.ctx-more')).toHaveCount(0);
  await expect(menu.getByRole('menuitem', { name: /^Delete release\/1\.0…$/ })).toBeVisible();
});
