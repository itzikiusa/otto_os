import { test, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

test('unknown locked worktree status is visible and never requests force', async ({ page }) => {
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-status-'));
  const { ctx, base } = await apiCtx();
  try {
    execFileSync('git', ['init', '-q', dir]);
    execFileSync('git', ['-C', dir, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.test', '-c', 'commit.gpgsign=false', 'commit', '--allow-empty', '-qm', 'fixture']);
    const ws = await seedWorkspace(ctx, base);
    const name = 'e2e-unknown-worktree';
    const seeded = await ctx.post(`${base}/api/v1/workspaces/${ws}/repos`, { data: { path: dir, name } });
    expect(seeded.ok()).toBeTruthy();
    const repo = await seeded.json();
    const path = `${dir}-unknown`;
    const rows = [{path,head:'abc',branch:'unknown-branch',detached:false,bare:false,is_main:false,locked:true,lock_reason:'fixture',prunable:false,dirty:false,dirty_known:false}];
    await page.route(`**/api/v1/repos/${repo.id}/worktrees`, route => route.fulfill({json:rows}));
    let removal: {path:string;force:boolean} | undefined;
    await page.route(`**/api/v1/repos/${repo.id}/worktrees/remove`, async route => {
      removal = route.request().postDataJSON();
      await route.fulfill({json:[]});
    });
    await page.addInitScript(id => { localStorage.setItem('otto_workspace', id); localStorage.setItem('otto_firstrun_dismissed','1'); }, ws);
    await openPage(page, 'git');
    await page.locator('.git-tab-new').click();
    await page.locator('.ctx-menu .ctx-search-input').fill(name);
    await page.getByRole('menuitem', {name, exact:true}).click();
    await page.getByRole('button', {name:/WORKTREES/}).click();
    const row = page.locator('.is-worktree').filter({hasText:'unknown-branch'});
    await expect(row.getByTitle('Changes unknown — status check unavailable')).toHaveText('?');
    await row.click({button:'right'});
    await page.getByRole('menuitem', {name:'Remove worktree…',exact:true}).click();
    const dialog = page.getByRole('dialog', {name:'Remove worktree',exact:true});
    await expect(dialog).toContainText('changes could not be checked');
    await dialog.getByRole('button', {name:'Remove',exact:true}).click();
    await expect.poll(() => removal).toEqual({path,force:false});
  } finally {
    await ctx.dispose();
    rmSync(dir, {recursive:true,force:true});
  }
});
