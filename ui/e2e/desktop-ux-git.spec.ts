import { test, expect, type Page, type Route } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// Real isolated repository; only errors, races and forge responses are stubbed.
let repoId = '';
let workspaceId = '';
let firstSha = '';
let secondSha = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const dir = mkdtempSync(join(tmpdir(), 'otto-ux-git-'));
  const git = (...args: string[]) => execFileSync('git', ['-C', dir, ...args], { encoding: 'utf8' }).trim();
  git('init', '-q', '-b', 'main');
  git('config', 'user.email', 'ux@otto.local');
  git('config', 'user.name', 'UX audit');
  git('config', 'commit.gpgsign', 'false');
  writeFileSync(join(dir, 'first.txt'), 'first version\n');
  git('add', '.'); git('commit', '-qm', 'First audit commit');
  firstSha = git('rev-parse', 'HEAD');
  writeFileSync(join(dir, 'second.txt'), 'second version\n');
  git('add', '.'); git('commit', '-qm', 'Second audit commit');
  secondSha = git('rev-parse', 'HEAD');
  git('branch', 'feature/a-long-branch-for-keyboard-actions');
  git('remote', 'add', 'origin', 'https://github.com/otto-test/ux-audit.git');
  const response = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/repos`, { data: { path: dir, name: 'UX audit repository' } });
  expect(response.ok()).toBeTruthy();
  repoId = (await response.json()).id;
  await ctx.dispose();
});
test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto_git_auto_fetch', JSON.stringify({ enabled: false }));
  }, workspaceId);
});
async function openRepo(page: Page, tab = 'graph') {
  await page.goto(`/#/git/${repoId}/${tab}`);
  await expect(page.locator('.rv-tabs')).toBeVisible();
}
function diff(path: string) {
  return { files: [{ path, old_path: null, is_binary: false, hunks: [{ header: '@@ -0,0 +1 @@', lines: [{ origin: 'add', content: path, old_line: null, new_line: 1 }] }] }] };
}
function pr(number: number, state = 'open') {
  return { number, title: `${state} request ${number}`, state, author: 'audit', source_branch: 'feature/a', target_branch: 'main', url: `https://github.com/otto-test/ux-audit/pull/${number}`, updated_at: '2026-09-25T08:00:00Z', draft: false };
}

test('commit detail keeps the newest selection when an earlier diff responds late', async ({ page }) => {
  let pending: Route | undefined;
  await page.route('**/api/v1/repos/*/diff?*', async (route) => {
    if (decodeURIComponent(route.request().url()).includes(firstSha)) pending = route;
    else await route.fulfill({ json: diff('second.txt') });
  });
  await openRepo(page);
  await page.locator(`.graph-row[data-sha="${firstSha}"]`).click();
  await expect.poll(() => !!pending).toBe(true);
  await page.locator(`.graph-row[data-sha="${secondSha}"]`).click();
  await expect(page.locator('.detail-diff')).toContainText('second.txt');
  await pending!.fulfill({ json: diff('first.txt') });
  // Wait for the response to be applied, then verify its old content never replaces B.
  await page.waitForTimeout(100);
  await expect(page.locator('.detail-diff')).not.toContainText('first.txt');
  await expect(page.locator('.detail-subject')).toHaveText('Second audit commit');
});

test('failed commit diff shows inline Retry instead of no changes', async ({ page }) => {
  let fail = true;
  await page.route('**/api/v1/repos/*/diff?*', (route) => fail
    ? route.fulfill({ status: 503, json: { code: 'unavailable', message: 'Diff temporarily unavailable' } })
    : route.fulfill({ json: diff('second.txt') }));
  await openRepo(page);
  await page.locator(`.graph-row[data-sha="${secondSha}"]`).click();
  const panel = page.locator('.detail-diff');
  await expect(panel.getByRole('button', { name: 'Retry' })).toBeVisible();
  await expect(panel).not.toContainText('No file changes.');
  fail = false;
  await panel.getByRole('button', { name: 'Retry' }).click();
  await expect(panel).toContainText('second.txt');
});

test('PR filter change ignores an in-flight page from the old filter', async ({ page }) => {
  let oldPage: Route | undefined;
  await page.route('**/api/v1/repos/*/prs?*', async (route) => {
    const url = new URL(route.request().url());
    const state = url.searchParams.get('state') ?? 'open';
    if (state === 'open' && url.searchParams.get('page') === '2') oldPage = route;
    else await route.fulfill({ json: { items: [pr(state === 'open' ? 1 : 2, state)], has_more: state === 'open' } });
  });
  await openRepo(page, 'prs');
  await page.getByRole('button', { name: 'Load more' }).click();
  await expect.poll(() => !!oldPage).toBe(true);
  await page.getByRole('button', { name: 'Merged', exact: true }).click();
  await expect(page.locator('.prlist')).toContainText('merged request 2');
  await oldPage!.fulfill({ json: { items: [pr(3)], has_more: false } });
  await page.waitForTimeout(100);
  await expect(page.locator('.prlist')).not.toContainText('open request 3');
});

test('Focus account failure offers Retry instead of account setup', async ({ page }) => {
  let fail = true;
  await page.route('**/api/v1/issue/accounts', (route) => fail
    ? route.fulfill({ status: 503, json: { code: 'unavailable', message: 'Account service unavailable' } })
    : route.fulfill({ json: [] }));
  await page.route('**/api/v1/repos/*/prs?*', (route) => route.fulfill({ json: { items: [], has_more: false } }));
  await openRepo(page, 'focus');
  const section = page.locator('.fx-section').filter({ hasText: 'My Jira work' });
  await expect(section.getByRole('button', { name: 'Retry' })).toBeVisible();
  await expect(section.getByText('Connect a Jira account', { exact: false })).toHaveCount(0);
  fail = false;
  await section.getByRole('button', { name: 'Retry' }).click();
  await expect(section.getByRole('button', { name: 'Add Jira account…' })).toBeVisible();
});

test('graph diff stays LTR inside an RTL UI', async ({ page }) => {
  await openRepo(page);
  await page.evaluate(() => { document.documentElement.dir = 'rtl'; });
  await page.locator(`.graph-row[data-sha="${secondSha}"]`).click();
  await expect(page.locator('.dl-table')).toBeVisible();
  await expect(page.locator('.dl-table')).toHaveCSS('direction', 'ltr');
});

test('branch actions work with keyboard and phone tap', async ({ page }) => {
  await openRepo(page);
  const menu = page.getByRole('button', { name: 'Actions for branch feature/a-long-branch-for-keyboard-actions', exact: true });
  await expect(menu).toBeVisible();
  await menu.focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('.ctx-menu')).toBeVisible();
  await expectFullyInViewport(page, page.locator('.ctx-menu'));
  await page.keyboard.press('Escape');
  await page.setViewportSize({ width: 390, height: 844 });
  await page.locator('.mob-sec-head').filter({ hasText: 'Branches' }).click();
  await menu.click();
  await expectFullyInViewport(page, page.locator('.ctx-menu'));
  await expectNoHorizontalOverflow(page);
});

for (const scenario of [
  { name: 'light-desktop', scheme: 'light', theme: 'native', width: 1440, height: 900, rtl: false },
  { name: 'dark-desktop', scheme: 'dark', theme: 'native', width: 1440, height: 900, rtl: false },
  { name: 'warm-dark-phone', scheme: 'dark', theme: 'warm', width: 390, height: 844, rtl: false },
  { name: 'light-rtl-tablet', scheme: 'light', theme: 'native', width: 820, height: 1180, rtl: true },
]) {
  test(`graph visual review ${scenario.name}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: scenario.width, height: scenario.height });
    await page.addInitScript(({ scheme, theme }) => {
      localStorage.setItem('otto_scheme', scheme);
      localStorage.setItem('otto_theme', theme);
    }, scenario);
    await openRepo(page);
    if (scenario.rtl) await page.evaluate(() => { document.documentElement.dir = 'rtl'; });
    await page.locator(`.graph-row[data-sha="${secondSha}"]`).click();
    await expect(page.locator('.dl-table')).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await expect(page.locator('.dl-table')).toHaveCSS('direction', 'ltr');
    await page.screenshot({ path: testInfo.outputPath(`${scenario.name}.png`), fullPage: true });
  });
}
