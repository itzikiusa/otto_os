import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, realpathSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { daemonCall } from './ide-polyglot-fixtures';
import { apiCtx } from './seed';

// A list INSIDE a page's content (the IDE Git tool window's commit list) is a
// standalone ListPane: its own header, no ⋯ "Hide", and it leaves the page's
// list (the explorer) alone — before, a ListPane there took the explorer's
// ListDetail, so its ⋯ hid the explorer (lib/components/side/ListHost.svelte).

test.use({ serviceWorkers: 'block', viewport: { width: 1440, height: 900 } });

async function boot(page: Page): Promise<void> {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-side-nested-')));
  mkdirSync(join(root, 'notes'));
  writeFileSync(join(root, 'README.md'), '# nested\n');
  const git = (...args: string[]) => execFileSync('git', ['-C', root, ...args], { stdio: 'pipe' });
  git('init', '-q');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'Otto E2E');
  git('config', 'commit.gpgsign', 'false');
  git('add', '-A');
  git('commit', '-q', '-m', 'first commit');
  writeFileSync(join(root, 'notes', 'second.md'), '# second\n');
  git('add', '-A');
  git('commit', '-q', '-m', 'second commit');
  const c = await apiCtx();
  let wsId: string;
  try {
    wsId = (await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', { name: `Side nested ${Date.now()}`, root_path: root })).id;
  } finally {
    await c.ctx.dispose();
  }
  await page.addInitScript(
    ({ wsId, root }) => {
      if (sessionStorage.getItem('__side_nested_init')) return;
      sessionStorage.setItem('__side_nested_init', '1');
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem('otto.ide.dockOpen', '1');
      localStorage.setItem('otto.ide.dockTab', 'git');
      localStorage.setItem('otto.ide.namedProjects.v1', JSON.stringify([{ id: 'proj-nested', name: 'nested', folders: [{ path: root, name: 'nested' }], updatedAt: Date.now() }]));
    },
    { wsId, root },
  );
  await page.goto('/#/ide');
  await expect(page.getByTestId('ide-page')).toBeVisible({ timeout: 20_000 });
}

test('the Git log commit list is its own list pane and leaves the explorer alone', async ({ page }) => {
  test.setTimeout(120_000);
  await boot(page);
  await page.getByTestId('ide-git-tab-log').click();
  const commits = page.locator('aside.side-list[aria-label="Commits"]');
  await expect(commits.locator('.side-pane-head .sph-title')).toHaveText('Commits', { timeout: 20_000 });
  await expect(commits.locator('.side-pane-head .sph-count')).toHaveText('2');
  // No ⋯ of its own (no menu) — and so no "Hide Files" that would hide the explorer.
  await expect(commits.locator('.side-pane-head [aria-haspopup="menu"]')).toHaveCount(0);
  // The keyboard walk is the block's: ↓ moves to the next commit row.
  const rows = commits.getByTestId('ide-git-commit-row');
  await expect(rows).toHaveCount(2);
  await rows.first().focus();
  await page.keyboard.press('ArrowDown');
  await expect(rows.nth(1)).toBeFocused();
  await rows.nth(1).click();
  await expect(page.locator('.gl-head-subject')).toHaveText('first commit');
  // The page's list is still the explorer: its ⋯ starts with "Hide Files".
  await page.getByTestId('ide-project-tree-more').click();
  await expect(page.getByRole('menuitem').first()).toContainText('Hide Files');
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('ide-project-tree')).toBeVisible();
});
