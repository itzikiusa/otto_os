import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace, seedGitRepo } from './seed';

// Git History's side surfaces on a desktop (rc6 side panes): the Branches
// list pane (⋯ › Hide, ⌘B, Show, persisted width and hidden, the old width
// key read once, the one keyboard walk), the Detail pane beside the graph
// (Esc, focus return, ⌥⌘B) and the stash browser's rows.

let workspaceId = '';
let repoId = '';
let headSha = '';
let featureSha = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const r = await seedGitRepo(ctx, base, workspaceId);
  repoId = r.repoId;
  const git = (...a: string[]) => execFileSync('git', ['-C', r.dir, ...a], { encoding: 'utf8' }).trim();
  git('config', 'commit.gpgsign', 'false');
  for (let c = 1; c <= 4; c++) {
    writeFileSync(join(r.dir, `side_${c}.txt`), `change ${c}\n`);
    git('add', '-A');
    git('commit', '-q', '-m', `Side commit ${c}`);
    if (c === 2) {
      git('branch', 'feature/tap-me');
      featureSha = git('rev-parse', 'HEAD');
    }
  }
  git('tag', 'v1.0.0');
  for (const n of [1, 2]) {
    writeFileSync(join(r.dir, `stash_${n}.txt`), `stashed ${n}\n`);
    git('add', '-A');
    git('stash', 'push', '-q', '-m', `Stash number ${n}`);
  }
  headSha = git('rev-parse', 'HEAD');
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((ws) => localStorage.setItem('otto_workspace', ws as string), workspaceId);
});

async function open(page: Page): Promise<void> {
  await page.goto(`/#/git/${repoId}/graph`);
  await expect(page.locator('.refs-panel .ref-row.current')).toBeVisible({ timeout: 25_000 });
}

test('Branches: ⋯ › Hide, Show and ⌘B; width and hidden persist, the old width key is read once', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('otto_git_graph_side_width', '300'));
  await open(page);
  const pane = page.getByTestId('git-refs');
  expect(Math.round((await pane.boundingBox())!.width)).toBe(300);
  expect(await page.evaluate(() => localStorage.getItem('otto.pane.git.refs.w'))).toBe('300');

  await page.getByTestId('git-refs-more').click();
  const items = page.locator('.ctx-menu [role="menuitem"]');
  await expect(items.first()).toHaveText(/Hide Branches/);
  await items.first().click();
  await expect(pane).toBeHidden();
  await page.getByTestId('show-list-pane').click();
  await expect(pane).toBeVisible();

  await page.locator('.graph-panel').click({ position: { x: 5, y: 5 } });
  await page.keyboard.press('ControlOrMeta+b');
  await expect(pane).toBeHidden();
  await page.reload();
  await expect(page.locator('.graph-row').first()).toBeVisible({ timeout: 25_000 });
  await expect(pane).toBeHidden();
  await expect(page.getByRole('button', { name: 'Show Branches' })).toBeVisible();
  await page.keyboard.press('ControlOrMeta+b');
  await expect(pane).toBeVisible();
});

test('Branches: the keyboard walks and folds the tree; Enter jumps the graph and marks the branch', async ({ page }) => {
  await open(page);
  const main = page.locator('.refs-panel .ref-row.current');
  const folder = page.locator('.refs-panel .ref-folder', { hasText: 'feature' });
  const leaf = page.locator('.refs-panel .ref-row', { hasText: 'tap-me' });
  await main.focus();
  await page.keyboard.press('ArrowDown');
  await expect(folder).toBeFocused();
  await page.keyboard.press('ArrowLeft');
  await expect(folder).toHaveAttribute('aria-expanded', 'false');
  await expect(leaf).toHaveCount(0);
  await page.keyboard.press('ArrowRight');
  await expect(folder).toHaveAttribute('aria-expanded', 'true');
  await page.keyboard.press('ArrowDown');
  await expect(leaf).toBeFocused();
  await page.keyboard.press('ArrowLeft');
  await expect(folder).toBeFocused(); // ← climbs to the parent
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  await expect(leaf).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator(`.graph-row[data-sha="${featureSha}"]`)).toHaveClass(/graph-row-selected/);
  await expect(page.getByTestId('git-detail').locator('.detail-subject')).toHaveText('Side commit 2', { timeout: 15_000 });
});

test('Detail pane: beside the graph; Esc closes it and returns focus, ⌥⌘B reopens it', async ({ page }) => {
  await open(page);
  const pane = page.getByTestId('git-detail');
  // A clean tree lands on the checked-out commit.
  await expect(pane.getByRole('heading', { name: 'Commit detail' })).toBeVisible({ timeout: 15_000 });
  await expect(page.getByRole('dialog', { name: 'Commit detail' })).toHaveCount(0); // inline, not a sheet
  const row = page.locator(`.graph-row[data-sha="${featureSha}"] .graph-select`);
  await row.click();
  await expect(pane.locator('.detail-subject')).toHaveText('Side commit 2');
  await pane.getByRole('button', { name: 'File actions' }).first().click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'History' }).click();
  await expect(pane.getByRole('heading', { name: 'File history' })).toBeVisible();
  await pane.getByRole('button', { name: 'Close File history' }).click();
  await expect(pane.locator('.detail-subject')).toHaveText('Side commit 2');
  await pane.getByRole('button', { name: 'File actions' }).first().focus();
  await page.keyboard.press('Escape');
  await expect(pane).toHaveCount(0);
  await expect(row).toBeFocused();
  await page.keyboard.press('Alt+ControlOrMeta+KeyB');
  await expect(page.getByTestId('git-detail')).toBeVisible();
});

test('stash browser: shared rows that walk with the arrow keys', async ({ page }) => {
  await open(page);
  await page.locator('.refs-panel .side-section-head').filter({ hasText: 'Stashes' }).getByRole('button').first().click();
  await page.locator('.stash-row', { hasText: 'Stash number 2' }).click();
  const dialog = page.getByRole('dialog', { name: 'Stashes' });
  const second = dialog.getByRole('button', { name: /Stash number 2/ });
  const first = dialog.getByRole('button', { name: /Stash number 1/ });
  await expect(second).toHaveAttribute('aria-current', 'true');
  await second.focus();
  await page.keyboard.press('ArrowDown');
  await expect(first).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(first).toHaveAttribute('aria-current', 'true');
  await expect(dialog.getByText('stash_1.txt').first()).toBeVisible();
});
