import { test, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';

test.use({ serviceWorkers: 'block' });
let repoId = '';
let workspaceId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base, 'PR draft ownership');
  const dir = mkdtempSync(join(tmpdir(), 'otto-pr-draft-ownership-'));
  const git = (...args: string[]) => execFileSync('git', ['-C', dir, ...args]);
  git('init', '-q', '-b', 'main');
  git('config', 'user.email', 'draft@otto.local');
  git('config', 'user.name', 'Draft ownership test');
  git('config', 'commit.gpgsign', 'false');
  writeFileSync(join(dir, 'a.txt'), 'base\n');
  git('add', '.');
  git('commit', '-qm', 'base');
  git('remote', 'add', 'origin', 'https://github.com/otto-test/draft-ownership.git');
  const result = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/repos`, {
    data: { path: dir, name: 'Draft ownership repo' },
  });
  expect(result.ok()).toBeTruthy();
  repoId = (await result.json()).id;
  await ctx.dispose();
});

test('discarding a PR draft before opening another PR clears its edit state', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop browser journey');
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_git_auto_fetch', JSON.stringify({ enabled: false }));
  }, workspaceId);
  await page.route('**/api/v1/repos/*/prs/*', async (route) => {
    const path = new URL(route.request().url()).pathname;
    const match = path.match(/\/prs\/(\d+)$/);
    if (!match) return route.fallback();
    const number = Number(match[1]);
    await route.fulfill({ json: {
      number, title: `PR ${number} original title`, state: 'open', author: 'reviewer',
      source_branch: `feature/${number}`, target_branch: 'main',
      url: `https://github.com/otto-test/draft-ownership/pull/${number}`,
      updated_at: '2026-10-08T00:00:00Z', draft: false, head_sha: `${number}`.repeat(40),
      description_md: `PR ${number} original description`, approved_by: [], reviewers: [],
      mergeable: true, comments: [],
    } });
  });
  await page.goto(`/#/git/${repoId}/pr/1`);
  await expect(page.locator('.prd-title')).toHaveText('PR 1 original title');
  await page.locator('.prd-desc').getByRole('button', { name: 'Edit', exact: true }).click();
  await page.getByRole('textbox', { name: 'Pull request title', exact: true }).fill('Private draft for PR one');
  await page.getByRole('textbox', { name: 'Pull request description', exact: true }).fill('Do not publish this to PR two');
  await page.evaluate((id) => { location.hash = `#/git/${id}/pr/2`; }, repoId);
  const discard = page.getByRole('dialog', { name: 'Discard unsaved changes?' });
  await expect(discard).toBeVisible();
  await discard.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(page.locator('.prd-title')).toHaveText('PR 2 original title');
  await page.screenshot({ path: testInfo.outputPath('pr-two-after-discard.png'), fullPage: true });
  await expect(page.getByRole('textbox', { name: 'Pull request title', exact: true })).toBeHidden();
  await expect(page.getByRole('textbox', { name: 'Pull request description', exact: true })).toBeHidden();
  await page.locator('.prd-desc').getByRole('button', { name: 'Edit', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Pull request title', exact: true })).toHaveValue('PR 2 original title');
  await expect(page.getByRole('textbox', { name: 'Pull request description', exact: true })).toHaveValue('PR 2 original description');
});
