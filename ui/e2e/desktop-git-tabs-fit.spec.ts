import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// Git page repo tabs: the branch shows inline only while the strip has room
// (measured, not a repo count); otherwise names only, the active tab keeping
// its branch if that fits. The branch is always in the tab's accessible name
// and tooltip. And Focus is a page-level view, not a repo sub-tab.

let workspaceId = '';
const repos: { id: string; name: string; branch: string }[] = [];

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  for (let i = 0; i < 8; i++) {
    const dir = mkdtempSync(join(tmpdir(), 'otto-git-tabs-'));
    const git = (...args: string[]) => execFileSync('git', ['-C', dir, ...args], { encoding: 'utf8' }).trim();
    const branch = `release/5.02.${40 + i}-long-branch-name`;
    git('init', '-q', '-b', branch);
    git('config', 'user.email', 'tabs@otto.local');
    git('config', 'user.name', 'Tabs');
    git('config', 'commit.gpgsign', 'false');
    writeFileSync(join(dir, 'a.txt'), 'a\n');
    git('add', '.');
    git('commit', '-qm', 'init');
    const name = `repo-${i}`;
    const r = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/repos`, { data: { path: dir, name } });
    expect(r.ok()).toBeTruthy();
    repos.push({ id: (await r.json()).id, name, branch });
  }
  await ctx.dispose();
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('tabs-seeded')) return;
    sessionStorage.setItem('tabs-seeded', '1');
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_right_open', '0');
    localStorage.setItem('otto_git_auto_fetch', JSON.stringify({ enabled: false }));
  }, workspaceId);
});

async function openTabs(page: Page, count: number): Promise<void> {
  const ids = repos.slice(0, count).map((r) => r.id);
  await page.addInitScript((state) => localStorage.setItem('otto_git_open_tabs', state), JSON.stringify({ openRepoIds: ids, activeRepoId: ids[0], sub: {} }));
  await openPage(page, 'git');
}
const tab = (page: Page, i: number) => page.getByRole('tab', { name: `${repos[i].name} — ${repos[i].branch}`, exact: true });

const reEscape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

test('few repos: every tab shows its branch inline', async ({ page }) => {
  await page.setViewportSize({ width: 1800, height: 800 });
  await openTabs(page, 2);
  for (const i of [0, 1]) {
    await expect(tab(page, i)).toBeVisible();
    await expect(tab(page, i).locator('.git-tab-branch')).toBeVisible();
    await expect(tab(page, i)).toHaveAttribute('title', new RegExp(`^${reEscape(repos[i].name)} — ${reEscape(repos[i].branch)}`));
  }
  await expect(page.locator('.git-tablist')).toHaveAttribute('data-branches', 'all');
});

test('many repos: names only (active may keep its branch); a wider window brings branches back', async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 800 });
  await openTabs(page, 8);
  const list = page.locator('.git-tablist');
  await expect(list).toHaveAttribute('data-branches', /^(active|none)$/);
  for (let i = 1; i < 8; i++) await expect(tab(page, i).locator('.git-tab-branch')).toBeHidden();
  // Still named by branch for AT + tooltip.
  await expect(tab(page, 3)).toHaveAttribute('aria-label', `${repos[3].name} — ${repos[3].branch}`);
  const mode = await list.getAttribute('data-branches');
  if (mode === 'active') await expect(tab(page, 0).locator('.git-tab-branch')).toBeVisible();
  await page.screenshot({ path: test.info().outputPath('git-tabs-many.png') });
  // Close tabs and widen the window until they fit: branches come back
  // without a reload.
  await page.setViewportSize({ width: 1800, height: 800 });
  // The close button is the tab's sibling, not its child (axe
  // nested-interactive), so it is found in the strip by its own name.
  for (let i = 7; i >= 2; i--) await list.getByRole('button', { name: `Close ${repos[i].name}`, exact: true }).click();
  await expect(list).toHaveAttribute('data-branches', 'all');
  await expect(tab(page, 1).locator('.git-tab-branch')).toBeVisible();
});

test('Focus is a page-level view, not a repo sub-tab', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 800 });
  await openTabs(page, 2);
  await expect(page.locator('.rv-tabs')).toBeVisible();
  await expect(page.locator('.rv-tabs').getByRole('tab', { name: 'Focus' })).toHaveCount(0);
  const focus = page.getByRole('button', { name: 'Focus', exact: true });
  await focus.click();
  await expect(focus).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('.rv-tabs')).toHaveCount(0);
  await expect(page.locator('h1.ph-title')).toContainText('Focus');
  // The repo toolbar is gone in Focus, so the freed room brings branches back.
  await expect(page.locator('.git-tablist')).toHaveAttribute('data-branches', 'all');
  await page.screenshot({ path: test.info().outputPath('git-focus.png') });
  await expect(page.locator('.git-tab[aria-selected="true"]')).toHaveCount(0);
  // A repo tab brings its view back.
  await tab(page, 1).click();
  await expect(focus).toHaveAttribute('aria-pressed', 'false');
  await expect(tab(page, 1)).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('.rv-tabs')).toBeVisible();
  // The Side panel's Git tab has no Focus either.
  await openPage(page, 'home');
  await page.locator('.rstrip').getByRole('button', { name: 'Git', exact: true }).click();
  await expect(page.locator('.rpanel .rv-tabs').getByRole('tab', { name: 'Focus' })).toHaveCount(0);
});
