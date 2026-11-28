import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace, seedGitRepo } from './seed';

// Git History's side surfaces on a phone and a tablet (rc6 side panes), with
// real taps: the Branches list pane (the page on a phone, a pushed graph with
// "‹ Branches"), the Detail pane (a sheet: the commit, a file's history) and
// the stash browser (one column that pushes on a phone).

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
  test.skip((page.viewportSize()?.width ?? 0) > 1024, 'phone and tablet layouts');
  await page.addInitScript((ws) => localStorage.setItem('otto_workspace', ws as string), workspaceId);
});

/** A phone (or a phone on its side): the list pushes its content. */
const pushes = (page: Page): boolean => {
  const v = page.viewportSize()!;
  return v.width <= 640 || v.height <= 500;
};

const branch = (page: Page, text: string) => page.locator('.refs-panel .ref-row', { hasText: text });

test('phone: Branches is the page, a branch pushes the graph and Back keeps it marked', async ({ page }) => {
  test.skip(!pushes(page), 'a phone');
  await page.goto(`/#/git/${repoId}/graph`);
  const main = page.locator('.refs-panel .ref-row.current');
  await expect(main).toBeVisible({ timeout: 25_000 });
  // One header row: ☰ · Branches · ⋯ · Side panel; the graph waits behind the list.
  await expect(page.getByTestId('open-navigator')).toBeVisible();
  await expect(page.getByTestId('open-side-panel')).toBeVisible();
  await expect(page.locator('.graph-row').first()).toBeHidden();

  await branch(page, 'tap-me').tap();
  const tip = page.locator(`.graph-row[data-sha="${featureSha}"]`);
  await expect(tip).toHaveClass(/graph-row-selected/, { timeout: 15_000 });
  await expect(tip).toBeInViewport();
  await expect(page.getByRole('dialog', { name: 'Commit detail' })).toHaveCount(0); // the graph, not a sheet
  await expect(page.getByTestId('open-side-panel')).toBeVisible();

  await page.getByRole('button', { name: 'Back to Branches' }).tap();
  await expect(branch(page, 'tap-me')).toHaveAttribute('aria-selected', 'true');
  await main.tap();
  const head = page.locator(`.graph-row[data-sha="${headSha}"]`);
  await expect(head).toBeInViewport({ timeout: 15_000 });

  // The marked row opens its detail: a full-width sheet with one Close.
  await head.locator('.graph-select').tap();
  const sheet = page.getByRole('dialog', { name: 'Commit detail' });
  await expect(sheet).toBeVisible();
  // A file's history takes the same sheet; its Close gives the commit back.
  await sheet.getByRole('button', { name: 'File actions' }).first().tap();
  await page.getByRole('menuitem', { name: 'History' }).tap();
  const history = page.getByRole('dialog', { name: 'File history' });
  await expect(history.getByRole('region', { name: 'File history' })).toBeVisible();
  await history.getByRole('button', { name: 'Close File history' }).tap();
  await expect(sheet.locator('.detail-panel')).toBeVisible();
  await sheet.getByRole('button', { name: 'Close Commit detail' }).tap();
  await expect(sheet).toHaveCount(0);
  await expect(head).toBeVisible();
});

test('phone: the stash browser pushes to a stash and Back returns to the list', async ({ page }) => {
  test.skip(!pushes(page), 'a phone');
  await page.goto(`/#/git/${repoId}/graph`);
  await expect(page.locator('.refs-panel .ref-row.current')).toBeVisible({ timeout: 25_000 });
  await page.locator('.refs-panel .side-section-head').filter({ hasText: 'Stashes' }).getByRole('button').first().tap();
  await page.locator('.stash-row', { hasText: 'Stash number 1' }).tap();
  const dialog = page.getByRole('dialog', { name: 'Stashes' });
  // Opened on the tapped stash: its detail is the page, Back returns to the list.
  await expect(dialog.getByRole('button', { name: 'Apply', exact: true })).toBeVisible({ timeout: 15_000 });
  await expect(dialog.getByRole('button', { name: /Stash number 2/ })).toBeHidden();
  await dialog.getByRole('button', { name: 'Back to Stashes' }).tap();
  const other = dialog.getByRole('button', { name: /Stash number 2/ });
  await expect(other).toBeVisible();
  await expect(dialog.getByRole('button', { name: /Stash number 1/ })).toHaveAttribute('aria-current', 'true');
  await other.tap();
  await expect(dialog.getByRole('button', { name: 'Apply', exact: true })).toBeVisible();
  await expect(dialog.locator('.detail .title')).toHaveText('Stash number 2');
});

test('tablet: Branches beside the graph, a branch marks its tip, a commit opens a sheet', async ({ page }) => {
  test.skip(pushes(page), 'a tablet');
  await page.goto(`/#/git/${repoId}/graph`);
  await expect(page.locator('.graph-row').first()).toBeVisible({ timeout: 25_000 });
  await branch(page, 'tap-me').tap();
  const tip = page.locator(`.graph-row[data-sha="${featureSha}"]`);
  await expect(tip).toHaveClass(/graph-row-selected/, { timeout: 15_000 });
  // No room beside the graph: the detail waits for the tap, then is a sheet.
  await expect(page.getByRole('dialog', { name: 'Commit detail' })).toHaveCount(0);
  await tip.locator('.graph-select').tap();
  const sheet = page.getByRole('dialog', { name: 'Commit detail' });
  await expect(sheet).toBeVisible();
  await expect(sheet.locator('.detail-subject')).toHaveText('Side commit 2');
  await sheet.getByRole('button', { name: 'Close Commit detail' }).tap();
  await expect(sheet).toHaveCount(0);
  await expect(page.getByTestId('open-side-panel')).toBeVisible();
});
