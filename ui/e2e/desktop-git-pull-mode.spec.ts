import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// R6 — pull mode & rebase from the graph (desktop-browser only).
//
// Pull stopped being implicit (a switch never pulls), so the explicit Pull has
// to say WHAT it will do: the toolbar button carries the repo's own
// `pull.rebase` / `pull.ff` setting, the ▾ menu overrides it for one pull, and
// `ff-only` on a diverged branch is an actionable 409 rather than a merge.
//
// NOTE: this spec exercises WP5's routes (`GET /pull-mode`, `POST /rebase`,
// `POST /pull {mode}`). It is written here because it belongs with the rest of
// the switch/pull UI, and it only passes once WP5 has landed — it was NOT run
// on the WP1 branch.
// ─────────────────────────────────────────────────────────────────────────────

const REPO_NAME = 'e2e-pull-mode-repo';
let repoDir = '';

function git(dir: string, ...args: string[]): string {
  return execFileSync('git', ['-C', dir, ...args], { encoding: 'utf8' }).trim();
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);

  const root = mkdtempSync(join(tmpdir(), 'otto-e2e-pullmode-'));
  const origin = join(root, 'origin.git');
  const seed = join(root, 'seed');
  repoDir = join(root, 'work');

  execFileSync('git', ['init', '-q', '--bare', '-b', 'main', origin]);
  execFileSync('git', ['init', '-q', '-b', 'main', seed]);
  git(seed, 'config', 'user.email', 'e2e@otto.local');
  git(seed, 'config', 'user.name', 'E2E');
  git(seed, 'config', 'commit.gpgsign', 'false');
  writeFileSync(join(seed, 'shared.txt'), 'l1\nl2\nl3\n');
  git(seed, 'add', '-A');
  git(seed, 'commit', '-q', '-m', 'init');
  git(seed, 'remote', 'add', 'origin', origin);
  git(seed, 'push', '-q', '-u', 'origin', 'main');

  execFileSync('git', ['clone', '-q', origin, repoDir]);
  git(repoDir, 'config', 'user.email', 'e2e@otto.local');
  git(repoDir, 'config', 'user.name', 'E2E');
  git(repoDir, 'config', 'commit.gpgsign', 'false');
  // The repo's own configured mode — the toolbar must read THIS, not a default.
  git(repoDir, 'config', 'pull.rebase', 'true');

  // Diverge: upstream and local both add a commit on main.
  writeFileSync(join(seed, 'upstream.txt'), 'theirs\n');
  git(seed, 'add', '-A');
  git(seed, 'commit', '-q', '-m', 'upstream work');
  git(seed, 'push', '-q', 'origin', 'main');
  writeFileSync(join(repoDir, 'local.txt'), 'mine\n');
  git(repoDir, 'add', '-A');
  git(repoDir, 'commit', '-q', '-m', 'local work');
  git(repoDir, 'fetch', '-q', 'origin');

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
  await expect(page.locator('.rv-tabs')).toBeVisible();
  await expect(page.locator('.refs-panel')).toBeVisible({ timeout: 20_000 });
}

test('the Pull button names the repo’s mode, and ff-only refuses a diverged branch', async ({
  page,
}) => {
  await openRepo(page);

  // `pull.rebase=true` → the split button's main label says so.
  const pullBtn = page.locator('.toolbar .split .tbtn').first();
  await expect(pullBtn).toContainText('Pull (rebase)', { timeout: 20_000 });

  // The ▾ menu overrides the mode for a single pull: ff-only on a DIVERGED
  // branch is a 409 with an actionable line, and it must not start a merge.
  await page.locator('.toolbar .split .caret').click();
  const menu = page.locator('.ctx-menu');
  await expect(menu).toBeVisible();
  await menu.getByRole('menuitem', { name: 'Pull (fast-forward only)' }).click();

  await expect(
    page.locator('.toast', { hasText: 'Not possible to fast-forward' }),
  ).toBeVisible({ timeout: 20_000 });
  expect(git(repoDir, 'status', '--porcelain')).toBe('');
  expect(git(repoDir, 'log', '-1', '--format=%P').split(/\s+/).filter(Boolean)).toHaveLength(1);
});

test('a conflicting rebase from the branch menu opens the resolver with op "rebase"', async ({
  page,
}) => {
  await openRepo(page);

  // Rebase the current branch onto its diverged upstream from the remote row.
  await page.locator('.ref-row.remote', { hasText: 'main' }).first().click({ button: 'right' });
  const menu = page.locator('.ctx-menu');
  await expect(menu).toBeVisible();
  await menu.getByRole('menuitem', { name: /^Rebase main onto origin\/main/ }).click();

  const confirm = page.locator('.sheet[role="dialog"][aria-label="Rebase"]');
  await expect(confirm).toBeVisible({ timeout: 20_000 });
  await expect(confirm).toContainText('commits will be replayed');
  await confirm.getByRole('button', { name: 'Rebase' }).click();

  await expect(page.locator('.toast', { hasText: 'Rebased' })).toBeVisible({ timeout: 20_000 });
  // Non-conflicting here: the branch is linear afterwards (no merge commit).
  expect(git(repoDir, 'log', '-1', '--format=%P').split(/\s+/).filter(Boolean)).toHaveLength(1);
});
