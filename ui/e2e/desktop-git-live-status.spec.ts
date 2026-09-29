import { test, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Local changes show up live (otto-git watch.rs → `repo_status_changed`).
//
// Before, a file saved in an editor reached the WIP list only on the next
// auto-fetch round (30 s for the active repo). Opening the repo runs one fetch
// immediately, so anything landing within a few seconds of a later edit can
// only have come from the watcher — the budget below is far under 30 s.
// ─────────────────────────────────────────────────────────────────────────────

const BUDGET_MS = 5_000;

test('an on-disk edit, a new file and a CLI `git add` appear without waiting for auto-fetch', async ({ page }) => {
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-live-'));
  const git = (...a: string[]) => execFileSync('git', ['-C', dir, ...a], { stdio: 'ignore' });
  git('init', '-q');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');
  writeFileSync(join(dir, 'a.ts'), 'export const a = 1;\n');
  writeFileSync(join(dir, '.gitignore'), 'target/\n');
  git('add', '.');
  git('commit', '-q', '-m', 'init');
  writeFileSync(join(dir, 'a.ts'), 'export const a = 2;\n');

  const name = `live-${Date.now()}`;
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, { data: { path: dir, name } });
  expect(r.ok()).toBeTruthy();
  await ctx.dispose();

  await openPage(page, 'git');
  await page.locator('.git-tab-new').click();
  const menu = page.locator('.ctx-menu');
  await menu.locator('.ctx-search-input').fill(name);
  await menu.getByRole('menuitem', { name }).first().click();
  const wipRow = page.locator('.wip-row');
  await expect(wipRow).toBeVisible({ timeout: 15_000 });
  await wipRow.click();
  const panel = page.locator('.wip-panel');
  await expect(panel.locator('.wp-count')).toHaveText('1 file changed');
  await panel.locator('.wp-name', { hasText: 'a.ts' }).first().click();
  await expect(panel.locator('.wp-diff')).toContainText('export const a = 2;');
  // Let the watcher arm (the status read that opened the repo arms it).
  await page.waitForTimeout(800);

  // 1. A new file saved in an editor.
  let t0 = Date.now();
  writeFileSync(join(dir, 'b.ts'), 'export const b = 1;\n');
  await expect(panel.locator('.wp-count')).toHaveText('2 files changed', { timeout: BUDGET_MS });
  const newFileMs = Date.now() - t0;

  // 2. Editing the already-modified, open file: the list is unchanged, the
  //    open diff must still follow the content.
  t0 = Date.now();
  writeFileSync(join(dir, 'a.ts'), 'export const a = 3;\n');
  await expect(panel.locator('.wp-diff')).toContainText('export const a = 3;', { timeout: BUDGET_MS });
  const openDiffMs = Date.now() - t0;

  // 3. Staging from a terminal (only `.git/index` changes): b.ts moves to the
  //    staged section — its row's checkbox reads "Unstage".
  t0 = Date.now();
  git('add', 'b.ts');
  await expect(panel.getByRole('checkbox', { name: 'Unstage b.ts' })).toBeVisible({ timeout: BUDGET_MS });
  const cliAddMs = Date.now() - t0;

  // 4. Churn in an ignored directory never changes the list.
  execFileSync('mkdir', ['-p', join(dir, 'target')]);
  writeFileSync(join(dir, 'target', 'out.o'), 'x');
  await page.waitForTimeout(1_500);
  await expect(panel.locator('.wp-count')).toHaveText('2 files changed');

  console.log(`live status: new file ${newFileMs} ms · open diff ${openDiffMs} ms · CLI add ${cliAddMs} ms`);
});
