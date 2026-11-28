import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace, tempDir } from './seed';

// Desktop: the Git conflict resolver's "Conflicts" list pane (⋯ › Hide
// Conflicts ⌘B, Show Conflicts in the header, the shared row walk), the
// Focus issue quick view as the page's Detail pane (inline, Esc, ⌥⌘B), and
// the Side panel tabs' shared rows (Files shows the open file as selected).

let repoId = '';
let workspaceId = '';
let root = '';

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
  const list = (await (await ctx.get(`${base}/api/v1/workspaces`)).json()) as { id: string; root_path: string }[];
  root = list.find((w) => w.id === workspaceId)!.root_path;
  writeFileSync(join(root, 'README.md'), '# side\n');
  writeFileSync(join(root, 'NOTES.md'), '# notes\n');
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('side-c-seeded')) return;
    sessionStorage.setItem('side-c-seeded', '1');
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_git_auto_fetch', JSON.stringify({ enabled: false }));
    localStorage.setItem('otto_right_open', '0');
  }, workspaceId);
});

test('conflicts list pane: one header, ⋯ › Hide, Show, ⌘B and the row walk', async ({ page }) => {
  await openResolver(page, repoId);
  const pane = page.getByTestId('conflict-files');
  await expect(pane.locator('.side-pane-head')).toContainText('Conflicts');
  await expect(pane.locator('.sph-count')).toHaveText('2');
  await expect(pane).toContainText('0 of 2 resolved');
  // Opens on the first file; its row is the marked one.
  await expect(page.locator('.resolver .pane-path')).toHaveText('a.txt');
  const rows = pane.locator('[data-side-row]');
  await expect(rows.first()).toHaveAttribute('aria-current', 'true');
  // ↓ walks to the next row; Enter opens it.
  await rows.first().focus();
  await page.keyboard.press('ArrowDown');
  await expect(rows.nth(1)).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.locator('.resolver .pane-path')).toHaveText('docs/b.txt');
  // ⋯ starts with "Hide Conflicts ⌘B"; hidden, "Show Conflicts" sits in the header.
  await page.getByTestId('conflict-files-more').click();
  const first = page.locator('.ctx-menu [role="menuitem"]').first();
  await expect(first).toContainText('Hide Conflicts');
  await first.click();
  await expect(pane).toBeHidden();
  expect(await page.evaluate(() => localStorage.getItem('otto.pane.git.conflicts.hidden'))).toBe('1');
  await page.getByTestId('show-list-pane').click();
  await expect(pane).toBeVisible();
  await page.locator('.resolver .pane-path').click();
  await page.keyboard.press('ControlOrMeta+b');
  await expect(pane).toBeHidden();
  await page.keyboard.press('ControlOrMeta+b');
  await expect(pane).toBeVisible();
});

test('Focus quick view: an inline Detail pane with one action, Esc and ⌥⌘B', async ({ page }) => {
  await focusFixtures(page);
  await page.goto('/#/git/focus');
  const trigger = page.getByTitle('Quick view SIDE-1', { exact: true });
  await trigger.click();
  const pane = page.getByTestId('fx-quick');
  await expect(pane).toBeVisible();
  await expect(page.locator('aside.detail-pane[aria-label="SIDE-1"]')).toBeVisible();
  await expect(pane.locator('.dp-title')).toHaveText('SIDE-1');
  await expect(pane.locator('.fx-quick-summary')).toHaveText('SIDE-1 summary');
  await expect(pane.getByRole('button', { name: 'Open in Jira' })).toBeEnabled();
  // Esc from inside the pane closes it and hands focus back to the row.
  await pane.getByRole('button', { name: 'Open in Jira' }).focus();
  await page.keyboard.press('Escape');
  await expect(pane).toHaveCount(0);
  await expect(trigger).toBeFocused();
  // ⌥⌘B brings the last issue back.
  await page.keyboard.press('ControlOrMeta+Alt+b');
  await expect(page.getByTestId('fx-quick').locator('.dp-title')).toHaveText('SIDE-1');
  await page.getByRole('button', { name: 'Close SIDE-1' }).click();
  await expect(page.getByTestId('fx-quick')).toHaveCount(0);
});

test('Side panel Files tab: the open file is marked with the shared row look', async ({ page }) => {
  await page.goto('/#/home');
  await page.locator('.rstrip').getByRole('button', { name: 'Files', exact: true }).click();
  const panel = page.locator('.rpanel');
  const readme = panel.locator('.tree-item', { hasText: 'README.md' });
  await expect(readme).toBeVisible({ timeout: 15_000 });
  await expect(readme).not.toHaveClass(/selected/);
  await readme.locator('.tree-row').click();
  await expect(panel.locator('.viewer-name-text')).toHaveText('README.md');
  await expect(readme).toHaveClass(/selected/);
  await expect(readme.locator('[role="treeitem"]')).toHaveAttribute('aria-selected', 'true');
  const look = await readme.evaluate((el) => ({ bg: getComputedStyle(el).backgroundColor, weight: getComputedStyle(el.querySelector('.side-row-title')!).fontWeight }));
  expect(look.bg).not.toBe('rgba(0, 0, 0, 0)');
  expect(Number(look.weight)).toBeGreaterThanOrEqual(600);
  // The tree keeps its own keys: ↓ moves to the next row.
  await readme.locator('.tree-row').focus();
  await page.keyboard.press('ArrowUp');
  await expect(panel.locator('.tree-item', { hasText: 'NOTES.md' }).locator('.tree-row')).toBeFocused();
});
