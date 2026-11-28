import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace, tempDir } from './seed';

// The Git conflict resolver and the Focus quick view on the shared side
// surfaces, driven with real taps on a phone and a tablet: the "Conflicts"
// list pane IS the page on a phone (a file is pushed, "‹ Conflicts" goes
// back with the file still marked), and sits beside the file on a tablet;
// the Focus issue quick view is the page's Detail pane (a sheet here).

// page.route mocks (the Focus fixtures) never fire once sw.js claims the page.
test.use({ serviceWorkers: 'block' });

let repoId = '';
let workspaceId = '';

async function seedTwoConflicts(): Promise<string> {
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  const dir = tempDir('otto-e2e-conflicts2-');
  const git = (...a: string[]) => execFileSync('git', ['-C', dir, ...a], { stdio: 'ignore' });
  const both = (tag: string) => {
    writeFileSync(join(dir, 'a.txt'), `top\n${tag} a\nbottom\n`);
    writeFileSync(join(dir, 'docs', 'b.txt'), `top\n${tag} b\nbottom\n`);
  };
  mkdirSync(join(dir, 'docs'));
  git('init', '-q', '-b', 'main');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');
  both('base');
  git('add', '-A');
  git('commit', '-q', '-m', 'base');
  git('checkout', '-q', '-b', 'feature');
  both('feature');
  git('commit', '-q', '-am', 'feature');
  git('checkout', '-q', 'main');
  both('main');
  git('commit', '-q', '-am', 'main');
  try {
    git('merge', 'feature');
  } catch {
    /* expected: both files conflict */
  }
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, { data: { path: dir, name: `e2e-two-conflicts-${Date.now().toString(36)}` } });
  if (!r.ok()) throw new Error(`repo seed failed: ${r.status()}`);
  const id = ((await r.json()) as { id: string }).id;
  await ctx.dispose();
  return id;
}

async function openResolver(page: Page, repoId: string): Promise<void> {
  await page.goto(`/#/git/${repoId}/graph`);
  const open = page.locator('.merge-banner .btn', { hasText: 'Resolve conflicts' });
  await expect(open).toBeVisible({ timeout: 25_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  // A tablet may open History on the WIP changes as a sheet over the banner:
  // its Conflicts section has the same way in.
  const fromSheet = page.getByRole('button', { name: 'Open resolver' });
  await ((await fromSheet.isVisible()) ? fromSheet : open).click();
  await expect(page.getByTestId('conflict-file')).toHaveCount(2, { timeout: 15_000 });
}

const issue = (key: string) => ({ key, summary: `${key} summary`, issue_type: 'Story', status: 'In progress', project_key: 'SIDE', project_name: 'Side', url: `https://a.test/${key}`, parent_key: null, description: 'Details.', assignee: 'Someone' });
async function focusFixtures(page: Page): Promise<void> {
  await page.route('**/api/v1/issue/accounts', (r) => r.fulfill({ json: [{ id: 'acc', label: 'Jira', base_url: 'https://a.test' }] }));
  await page.route('**/api/v1/git/accounts', (r) => r.fulfill({ json: [] }));
  await page.route('**/api/v1/issue/my-work?*', (r) => r.fulfill({ json: [issue('SIDE-1'), issue('SIDE-2')] }));
  await page.route('**/api/v1/issue/acc/*', (r) => r.fulfill({ json: issue(r.request().url().split('/').at(-1)!) }));
}

test.beforeAll(async () => {
  repoId = await seedTwoConflicts();
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  await ctx.dispose();
});

test.beforeEach(async ({ page }, info) => {
  test.skip(!['iphone-portrait', 'ipad-portrait'].includes(info.project.name), 'phone and tablet portrait');
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_git_auto_fetch', JSON.stringify({ enabled: false }));
  }, workspaceId);
});

const row = (page: Page, name: string) => page.getByTestId('conflict-file').filter({ hasText: name });

test('conflicts: list → file → Back → another file', async ({ page }, info) => {
  await openResolver(page, repoId);
  const path = page.locator('.resolver .pane-path');
  await expect(page.getByTestId('open-side-panel')).toBeVisible();
  if (info.project.name === 'iphone-portrait') {
    // The list is the page: ☰ · Conflicts 2 · ⋯ · Side panel, no file yet.
    await expect(page.getByTestId('open-navigator')).toBeVisible();
    await expect(page.locator('.page-header-bar')).toContainText('Conflicts');
    await expect(page.locator('.resolver .file-detail')).toBeHidden();
    await expect(row(page, 'a.txt')).toContainText('Conflict');
    await row(page, 'a.txt').tap();
    await expect(path).toHaveText('a.txt');
    await expect(row(page, 'a.txt')).toBeHidden();
    const back = page.getByRole('button', { name: 'Back to Conflicts' });
    await back.tap();
    await expect(row(page, 'a.txt').locator('[aria-current="true"]')).toBeVisible();
    await row(page, 'b.txt').tap();
    await expect(path).toHaveText('docs/b.txt');
    await expect(back).toBeVisible();
    await expect(page.getByTestId('open-side-panel')).toBeVisible();
    // Complete / Abort stay under the file too.
    await expect(page.locator('.resolver-foot .btn.primary')).toBeVisible();
  } else {
    // A tablet: the list beside the file, opened on the first one.
    await expect(path).toHaveText('a.txt');
    await row(page, 'b.txt').tap();
    await expect(path).toHaveText('docs/b.txt');
    await expect(row(page, 'b.txt').locator('[aria-current="true"]')).toBeVisible();
    await expect(page.getByTestId('conflict-files')).toContainText('0 of 2 resolved');
  }
});

test('Focus: the issue quick view is the Detail pane (a sheet on a phone), closed with ✕', async ({ page }, info) => {
  await focusFixtures(page);
  await page.goto('/#/git/focus');
  await page.getByTitle('Quick view SIDE-1', { exact: true }).tap();
  // A phone: a full-width sheet; a tablet keeps the lists' 480 px and has room inline.
  const sheet = info.project.name === 'iphone-portrait' ? page.getByRole('dialog', { name: 'SIDE-1' }) : page.locator('aside.detail-pane[aria-label="SIDE-1"]');
  await expect(sheet).toBeVisible();
  await expect(sheet.locator('.fx-quick-summary')).toHaveText('SIDE-1 summary');
  await expect(sheet.getByRole('button', { name: 'Open in Jira' })).toBeVisible();
  await sheet.getByRole('button', { name: 'Close SIDE-1' }).tap();
  await expect(sheet).toHaveCount(0);
  await page.getByTitle('Quick view SIDE-2', { exact: true }).tap();
  await expect(page.getByTestId('fx-quick').locator('.fx-quick-summary')).toHaveText('SIDE-2 summary');
});
