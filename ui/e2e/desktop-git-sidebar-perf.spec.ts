import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';
import { domCount, isDesktopProject } from './perf';

// ─────────────────────────────────────────────────────────────────────────────
// PERF GATE — the git sidebar on a repo with 2,000 branches in 40 prefix
// folders (r3-03-02). The row cap used to be per FOLDER, so every folder
// mounted its own 150 rows and all 2,000 branches landed in the DOM (~50k
// nodes) on every graph open and refs refresh. The budget is per SECTION now:
// a DOM count, engine-agnostic, fast (one tiny commit + `update-ref --stdin`).
// ─────────────────────────────────────────────────────────────────────────────

const REPO_NAME = 'e2e-git-sidebar-budget';
const FOLDERS = 40;
const PER_FOLDER = 50;

test.beforeAll(async ({}, testInfo) => {
  test.skip(!isDesktopProject(testInfo.project.name), 'desktop projects only');
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  const dir = join(mkdtempSync(join(tmpdir(), 'otto-e2e-sidebar-')), 'work');
  const git = (...args: string[]) => execFileSync('git', ['-C', dir, ...args], { encoding: 'utf8' }).trim();
  execFileSync('git', ['init', '-q', '-b', 'main', dir]);
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');
  git('commit', '-q', '--allow-empty', '-m', 'root');
  const head = git('rev-parse', 'HEAD');
  const refs: string[] = [];
  for (let f = 0; f < FOLDERS; f++) {
    for (let b = 0; b < PER_FOLDER; b++) refs.push(`create refs/heads/team${f}/feature-${b} ${head}`);
  }
  execFileSync('git', ['-C', dir, 'update-ref', '--stdin'], { input: refs.join('\n') + '\n' });
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, { data: { path: dir, name: REPO_NAME } });
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
  await expect(page.locator('.refs-panel')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('.refs-panel .ref-folder').first()).toBeVisible({ timeout: 30_000 });
}

test('2,000 branches in 40 folders mount one section budget, and Show more grows it', async ({ page }) => {
  await openRepo(page);
  const rows = '.refs-panel .ref-row, .refs-panel .ref-folder';
  const first = await domCount(page, rows);
  // One section budget (150 rows) + the other sections' headers/rows — not
  // 2,000 branch rows (the per-folder cap mounted all of them).
  expect(first, `sidebar rows mounted: ${first}`).toBeLessThanOrEqual(200);
  const more = page.locator('.refs-panel .ref-more-leaves').first();
  await expect(more).toBeVisible();
  await expect(more).toContainText('hidden');
  await more.click();
  await expect.poll(() => domCount(page, rows)).toBeGreaterThan(first + 300);
  expect(await domCount(page, rows)).toBeLessThanOrEqual(800);
});
