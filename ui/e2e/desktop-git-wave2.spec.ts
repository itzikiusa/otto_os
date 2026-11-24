import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';

// Git bug hunt 2026-11-16, wave 2 (UI-only regressions):
// - the header toolbar's "Browse stashes…" opened a 0×0 dialog that PageHeader's
//   overflow measurer hid, and pushed Pull into ⋯ with it;
// - the History / Blame drawers ignored Escape.
// One isolated local repo with a stash; no remote, no forge.

let repoId = '';
let workspaceId = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-wave2-'));
  const git = (...args: string[]) => execFileSync('git', ['-C', dir, ...args], { encoding: 'utf8' }).trim();
  git('init', '-q', '-b', 'main');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');
  writeFileSync(join(dir, 'notes.txt'), 'one\n');
  git('add', '.');
  git('commit', '-qm', 'Add notes');
  writeFileSync(join(dir, 'notes.txt'), 'one\ntwo\n');
  git('stash', 'push', '-m', 'Wave two stash');
  const r = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/repos`, { data: { path: dir, name: `wave2-${Date.now()}` } });
  expect(r.ok()).toBeTruthy();
  repoId = (await r.json()).id;
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_git_auto_fetch', JSON.stringify({ enabled: false }));
  }, workspaceId);
});

async function openRepo(page: Page) {
  await page.goto(`/#/git/${repoId}/graph`);
  await expect(page.locator('.rv-tabs')).toBeVisible();
}

test('the header "Browse stashes…" dialog is visible and leaves Pull in place', async ({ page }) => {
  await openRepo(page);
  // Fetch is the header's one visible verb; branch & stash ride its ⋯.
  const pull = page.locator('.git-header .ph-actions').getByRole('button', { name: 'Fetch', exact: true });
  await expect(pull).toBeVisible();
  await page.locator('.git-header .ph-more').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: /^Browse stashes/ }).click();
  const dialog = page.getByRole('dialog', { name: 'Stashes' });
  await expect(dialog).toBeVisible();
  const box = await dialog.boundingBox();
  expect(box && box.width > 200 && box.height > 200).toBeTruthy();
  await expect(dialog.getByRole('option', { name: /Wave two stash/ })).toBeVisible();
  await expect(pull).toBeVisible();
});

test('Escape closes the History drawer', async ({ page }) => {
  await openRepo(page);
  await page.locator('.graph-row').filter({ has: page.getByText('Add notes', { exact: true }) }).first().click();
  await page.getByRole('button', { name: 'File actions', exact: true }).first().click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'History' }).click();
  const drawer = page.locator('.rv-drawer');
  await expect(drawer).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(drawer).toBeHidden();
});
