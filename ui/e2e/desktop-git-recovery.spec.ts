import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

async function fixture(page: Page) {
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-recovery-'));
  const git = (...args: string[]) => execFileSync('git', ['-C', dir, ...args], { encoding: 'utf8' }).trim();
  git('init', '-b', 'main');
  git('config', 'user.name', 'E2E'); git('config', 'user.email', 'e2e@example.invalid');
  git('config', 'commit.gpgsign', 'false');
  for (let n = 0; n < 5; n++) {
    writeFileSync(join(dir, `file${n}`), `${n}\n`);
    git('add', '.'); git('commit', '-m', `test: revision ${n}`);
  }
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  const response = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, { data: { path: dir, name: `recovery-${Date.now()}` } });
  expect(response.ok()).toBeTruthy();
  const { id } = await response.json(); await ctx.dispose();
  await openPage(page, `git/${id}/graph`);
  await page.getByRole('button', { name: 'Recovery tools', exact: true }).click();
  await expect(page.locator('.tools .entry').first()).toBeVisible();
  return { git, id: id as string, dir };
}

test('recovery history creates a branch without changing HEAD', async ({ page }) => {
  const { git } = await fixture(page);
  const head = git('rev-parse', 'HEAD');
  await page.locator('.tools .entry').nth(1).getByRole('button', { name: 'Recover…' }).click();
  await page.locator('.cf-input').fill('recovered-by-ui');
  await page.getByRole('button', { name: 'Create branch', exact: true }).click();
  await expect.poll(() => git('branch', '--list', 'recovered-by-ui')).toContain('recovered-by-ui');
  expect(git('rev-parse', 'HEAD')).toBe(head);
  expect(git('rev-parse', 'recovered-by-ui')).toBe(git('rev-parse', 'HEAD~1'));
});

test('interactive plan reorders and squashes real commits', async ({ page }) => {
  const { git } = await fixture(page);
  await page.getByRole('button', { name: 'Interactive rebase', exact: true }).click();
  await page.getByLabel('Onto revision').fill('HEAD~3');
  await page.getByRole('button', { name: 'Preview plan', exact: true }).click();
  await expect(page.locator('.tools select')).toHaveCount(3);
  await page.getByRole('button', { name: 'Move test: revision 3 up', exact: true }).click();
  await page.locator('.tools select').nth(2).selectOption('squash');
  await page.getByRole('button', { name: 'Start rebase…', exact: true }).click();
  await page.getByRole('button', { name: 'Start rebase', exact: true }).click();
  await expect.poll(() => git('rev-list', '--count', 'HEAD')).toBe('4');
  expect(git('log', '--format=%s', '--reverse', 'HEAD~2..HEAD').split('\n')[0]).toBe('test: revision 3');
});

test('bisect resumes after closing tools and returns to original branch', async ({ page }) => {
  const { git } = await fixture(page);
  const head = git('rev-parse', 'HEAD');
  await page.getByRole('button', { name: 'Bisect', exact: true }).click();
  await page.getByLabel('Known good').fill('HEAD~4');
  await page.getByLabel('Known bad').fill('HEAD');
  await page.getByRole('button', { name: 'Start bisect…', exact: true }).click();
  await page.getByRole('button', { name: 'Start', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Works (good)', exact: true })).toBeVisible();
  const candidate = git('rev-parse', 'HEAD'); expect(candidate).not.toBe(head);
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Recovery tools', exact: true }).click();
  await page.getByRole('button', { name: 'Bisect', exact: true }).click();
  await expect(page.locator('.tools code', { hasText: candidate })).toBeVisible();
  await page.getByRole('button', { name: 'End bisect / return to branch', exact: true }).click();
  await expect.poll(() => git('branch', '--show-current')).toBe('main');
  expect(git('rev-parse', 'HEAD')).toBe(head);
});
