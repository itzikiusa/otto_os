import { test, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Commit search on the graph (desktop-browser only).
//
// The search bar above the graph queries the DAEMON (`log --all --grep=`), not
// the rows the graph happens to have loaded — so a match is found whether or not
// it is on screen. This spec seeds 60 commits with ONE distinctive subject,
// focuses the box with ⌘F, and asserts a single hit + the match banner, then
// that clicking the hit selects that commit in the graph.
// ─────────────────────────────────────────────────────────────────────────────

const REPO_NAME = 'e2e-search-repo';
const NEEDLE = 'feat: needle-xyz lands here';
let repoDir = '';
let needleSha = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  repoDir = mkdtempSync(join(tmpdir(), 'otto-e2e-search-'));
  const git = (...a: string[]) => execFileSync('git', ['-C', repoDir, ...a], { stdio: 'ignore' });
  git('init', '-q');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');
  // 60 commits; the needle sits at #55 so it is well inside history but still on
  // the graph's first page (GraphView pages 10 000 rows at a time).
  for (let i = 1; i <= 60; i++) {
    writeFileSync(join(repoDir, `f${i}.txt`), `line ${i}\n`);
    git('add', '.');
    git('commit', '-q', '-m', i === 55 ? NEEDLE : `chore: routine commit ${i}`);
  }
  needleSha = execFileSync('git', ['-C', repoDir, 'rev-list', '-1', '--grep', 'needle-xyz', 'HEAD'], {
    encoding: 'utf8',
  }).trim();
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, {
    data: { path: repoDir, name: REPO_NAME },
  });
  if (!r.ok()) throw new Error(`repo seed failed: ${r.status()} ${await r.text()}`);
  await ctx.dispose();
});

async function openRepo(page: import('@playwright/test').Page): Promise<void> {
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
  await expect(page.locator('.gsb-input')).toBeVisible({ timeout: 15_000 });
}

test('⌘F focuses the search box and a server-side query finds the one match', async ({ page }) => {
  await openRepo(page);

  // ⌘F focuses the box from anywhere on the git page.
  await page.locator('.rv-head').click();
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+f' : 'Control+f');
  await expect(page.locator('.gsb-input')).toBeFocused();

  // The query runs on the daemon (`--grep`), so it matches the commit's subject
  // regardless of which rows the graph has rendered.
  await page.locator('.gsb-input').fill('needle');
  const results = page.locator('.gsb-result');
  await expect(results).toHaveCount(1, { timeout: 15_000 });
  await expect(results.first()).toContainText('needle-xyz');
  await expectFullyInViewport(page, page.locator('.gsb-results'), 'search results');

  const banner = page.locator('.gsb-banner');
  await expect(banner).toContainText('1 match for "needle"');
  await expect(banner.locator('.gsb-link')).toHaveText('Clear');

  // A query that matches nothing says so instead of showing stale hits.
  await page.locator('.gsb-input').fill('no-such-commit-subject');
  await expect(page.locator('.gsb-result')).toHaveCount(0, { timeout: 15_000 });
  await expect(banner).toContainText('0 matches');

  // Clear resets both the box and the banner.
  await banner.locator('.gsb-link').click();
  await expect(page.locator('.gsb-banner')).toHaveCount(0);
  await expect(page.locator('.gsb-input')).toHaveValue('');
});

test('clicking a search result selects that commit in the graph', async ({ page }) => {
  // Cross-WP: the click publishes the commit on `gitBridge`, and GRAPHVIEW's
  // `gitBridge.focus` effect is what selects + reveals it (WP1 of this batch).
  // This test therefore passes on the merged branch, not on WP5's alone.
  await openRepo(page);
  await page.locator('.gsb-input').fill('needle');
  const results = page.locator('.gsb-result');
  await expect(results).toHaveCount(1, { timeout: 15_000 });
  await results.first().click();

  const detailSha = page.locator('.detail-sha');
  await expect(detailSha).toBeVisible({ timeout: 15_000 });
  const short = ((await detailSha.textContent()) ?? '').trim();
  expect(short.length).toBeGreaterThan(3);
  expect(needleSha.startsWith(short)).toBe(true);
});
