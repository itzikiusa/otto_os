import { test, expect, type Page, type Route } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// Route fixtures must bypass the mobile service worker.
test.use({ serviceWorkers: 'block' });

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
  for (let n = 0; n < 30; n++) git('branch', `release/long-reference-${n}`);
  writeFileSync(join(dir, 'first.txt'), 'stash contents\n');
  git('stash', 'push', '-m', 'Audit saved changes');
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


const accounts = [{ id: 'account-a', label: 'Account A', base_url: 'https://a.test' }, { id: 'account-b', label: 'Account B', base_url: 'https://b.test' }];
function issue(key: string) {
  return { key, summary: `${key} important issue with a long descriptive title`, issue_type: 'Story', status: 'In progress', project_key: 'AUDIT', project_name: 'Audit project', url: `https://a.test/${key}`, parent_key: null, description: 'Long description. '.repeat(150), assignee: 'Audit user' };
}
async function focusFixtures(page: Page) {
  await page.route('**/api/v1/issue/accounts', r => r.fulfill({ json: accounts }));
  await page.route('**/api/v1/repos/*/prs?*', r => r.fulfill({ json: { items: [pr(1)], has_more: false } }));
  await page.route('**/api/v1/issue/my-work?*', r => r.fulfill({ json: [issue('AUDIT-1'), issue('AUDIT-2')] }));
  await page.route('**/api/v1/issue/account-*/*', r => r.fulfill({ json: issue(r.request().url().split('/').at(-1)!) }));
}
test('Focus account switch ignores a late work response and clears issue details', async ({ page }) => {
  await focusFixtures(page);
  let pending: Route | undefined;
  await page.route('**/api/v1/issue/my-work?*', r => r.request().url().includes('account-a') ? (pending = r, undefined) : r.fulfill({ json: [issue('B-1')] }));
  await openRepo(page, 'focus');
  await expect.poll(() => !!pending).toBe(true);
  await page.getByTitle('Jira account', { exact: true }).selectOption('account-b');
  await expect(page.locator('.fx-issue')).toContainText('B-1');
  await pending!.fulfill({ json: [issue('A-1')] });
  await page.waitForTimeout(150);
  await expect(page.locator('.fx-issue')).toContainText('B-1');
});
test('Focus quick view ignores a late issue and closes when account changes', async ({ page }) => {
  await focusFixtures(page);
  let pending: Route | undefined;
  await page.route('**/api/v1/issue/account-a/AUDIT-1', r => { pending = r; });
  await openRepo(page, 'focus');
  await page.getByTitle('Quick view AUDIT-1', { exact: true }).click();
  await expect.poll(() => !!pending).toBe(true);
  await page.getByTitle('Quick view AUDIT-2', { exact: true }).click();
  await expect(page.locator('.fx-quick-summary')).toContainText('AUDIT-2');
  await pending!.fulfill({ json: issue('AUDIT-1') });
  await page.waitForTimeout(150);
  await expect(page.locator('.fx-quick-summary')).toContainText('AUDIT-2');
  await page.getByTitle('Jira account', { exact: true }).selectOption('account-b');
  await expect(page.locator('.fx-quick')).toHaveCount(0);
});
test('phone Focus quick view traps focus, closes with Escape and restores its trigger', async ({ page }) => {
  await focusFixtures(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await openRepo(page, 'focus');
  const trigger = page.getByTitle('Quick view AUDIT-1', { exact: true });
  await trigger.click();
  const panel = page.locator('.fx-quick');
  await expect.poll(() => panel.evaluate(el => el.contains(document.activeElement))).toBe(true);
  await expect(panel).toHaveAttribute('aria-modal', 'true');
  await page.keyboard.press('?');
  await expect(page.getByRole('dialog')).toHaveCount(1);
  await panel.getByRole('button', { name: 'Close issue quick view' }).focus();
  await page.keyboard.press('Tab');
  await expect.poll(() => panel.evaluate(el => el.contains(document.activeElement))).toBe(true);
  await page.keyboard.press('Escape');
  await expect(panel).toHaveCount(0);
  await expect(trigger).toBeFocused();
});
test('graph multi-ref list is keyboard accessible and returns focus', async ({ page }) => {
  await openRepo(page);
  const expander = page.locator('.ref-expander').first();
  await expect(expander).toHaveAttribute('tabindex', '0');
  await expect(expander).toBeEnabled();
  await expander.focus();
  await page.keyboard.press('Enter');
  const popup = page.locator('.ref-popover');
  await expect(popup).toBeVisible();
  await expect.poll(() => popup.evaluate(el => el.contains(document.activeElement))).toBe(true);
  await expectFullyInViewport(page, popup);
  await page.keyboard.press('Escape');
  await expect(expander).toBeFocused();
});
test('stash, stale worktree and submodule actions are accessible by keyboard and tap', async ({ page }) => {
  await page.route('**/api/v1/repos/*/worktrees', r => r.fulfill({ json: [{ path: '/tmp/stale-audit-tree', sha: firstSha, branch: 'stale-audit', is_main: false, locked: false, prunable: true, dirty: false, dirty_known: true }] }));
  await page.route('**/api/v1/repos/*/submodules', r => r.fulfill({ json: [{ path: 'vendor/audit-submodule', sha: firstSha, url: 'https://example.test/submodule', state: 'uninitialized' }] }));
  await openRepo(page);
  for (const section of ['STASHES', 'WORKTREES', 'SUBMODULES']) await page.locator('.ref-header').filter({ hasText: section }).click();
  for (const name of ['Actions for stash stash@{0}', 'Actions for worktree /tmp/stale-audit-tree', 'Actions for submodule vendor/audit-submodule']) {
    const button = page.getByRole('button', { name, exact: true });
    await expect(button).toBeVisible();
    await button.focus();
    await page.keyboard.press('Enter');
    await expectFullyInViewport(page, page.locator('.ctx-menu'));
    await page.keyboard.press('Escape');
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await page.locator('.mob-sec-head').filter({ hasText: 'Branches' }).click();
  const stashSection = page.locator('.ref-header').filter({ hasText: 'STASHES' });
  if (await stashSection.getAttribute('aria-expanded') === 'false') await stashSection.click();
  await page.getByRole('button', { name: 'Actions for stash stash@{0}', exact: true }).click();
  await expectFullyInViewport(page, page.locator('.ctx-menu'));
});
async function remoteFixtures(page: Page) {
  await page.route('**/api/v1/git/accounts', r => r.fulfill({ json: accounts.map(a => ({ ...a, provider: 'github', namespace: a.id, username: 'audit' })) }));
  await page.goto('/#/git');
  await page.getByRole('button', { name: 'Add repository', exact: true }).click();
}
test('add repository modes support arrows and Home End', async ({ page }) => {
  await remoteFixtures(page);
  await page.getByRole('tab', { name: 'Local folder' }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('tab', { name: 'Browse remote' })).toBeFocused();
  await expect(page.getByRole('tab', { name: 'Browse remote' })).toHaveAttribute('aria-selected', 'true');
  await page.keyboard.press('End');
  await expect(page.getByRole('tab', { name: 'Clone URL' })).toBeFocused();
  await page.keyboard.press('Home');
  await expect(page.getByRole('tab', { name: 'Local folder' })).toBeFocused();
});
test('remote repository results cannot cross account or query boundaries', async ({ page }) => {
  let pending: Route | undefined;
  const remote = (name: string) => [{ name, full_name: `org/${name}`, clone_url: `https://example.test/${name}.git`, private: false }];
  await page.route('**/api/v1/git/accounts/*/remote-repos*', r => r.request().url().includes('account-a') ? (pending = r, undefined) : r.fulfill({ json: remote('B-repository') }));
  await remoteFixtures(page);
  await page.getByRole('tab', { name: 'Browse remote' }).click();
  await expect.poll(() => !!pending).toBe(true);
  await page.getByRole('dialog', { name: 'Add repository' }).getByLabel('Account', { exact: true }).selectOption('account-b');
  await expect(page.locator('.remote-list')).toContainText('B-repository');
  await pending!.fulfill({ json: remote('A-repository') });
  await page.waitForTimeout(150);
  await expect(page.locator('.remote-list')).not.toContainText('A-repository');
  await page.getByLabel('Search repositories').fill('new-query');
  await expect(page.locator('.remote-list')).not.toContainText('B-repository');
});

const longPath = `src/${'long-directory/'.repeat(10)}reviewed-file.ts`;
async function prFixtures(page: Page) {
  await page.route('**/api/v1/repos/*/prs/*', async route => {
    const path = new URL(route.request().url()).pathname;
    if (/\/prs\/\d+$/.test(path)) {
      const number = Number(path.split('/').at(-1));
      await route.fulfill({ json: { ...pr(number), title: `PR ${number}: ${'Long pull request description '.repeat(4)}`, source_branch: `feature/${'long-branch-'.repeat(9)}`, description_md: '## Change\n\n' + 'Readable summary of the change. '.repeat(50), approved_by: [], reviewers: [], mergeable: true, comments: [{ id: 'comment-1', author: 'reviewer', body: 'Discussion text '.repeat(30), path: null, line: null, created_at: '2026-09-25T08:00:00Z', replies: [], resolved: false }] } });
    } else await route.fallback();
  });
  await page.route('**/api/v1/repos/*/prs/*/diff', r => r.fulfill({ json: diff(longPath) }));
  await page.route('**/api/v1/repos/*/prs/*/commits', r => r.fulfill({ json: [{ sha: secondSha, message: 'Long commit summary '.repeat(10), author: 'Audit author', date: '2026-09-25T08:00:00Z' }] }));
  await page.route('**/api/v1/repos/*/prs/*/reviews', r => r.fulfill({ json: [{ id: 'review-1', repo_id: repoId, pr_number: 1, status: 'done', error: null, comments: [{ id: 'finding-1', review_id: 'review-1', path: longPath, line: 1, severity: 'bug', body: 'Preserve the current account when accepting this response. '.repeat(8), state: 'draft', posted: false, created_at: '2026-09-25T08:00:00Z' }], agents: [], created_at: '2026-09-25T08:00:00Z', verdict: 'request_changes', blocker_count: 1 }] }));
  await page.route('**/api/v1/reviews/review-1/findings*', r => r.fulfill({ json: [] }));
  await page.route('**/api/v1/reviews/review-1/merge-readiness', r => r.fulfill({ json: { unresolved_blocker_count: 1, unresolved_total: 1, resolved_count: 0, ci_status: 'success', approvals: 0, mergeable: true } }));
}
test('switching PR while editing cannot carry the old title into the new PR', async ({ page }) => {
  await prFixtures(page);
  await page.goto(`/#/git/${repoId}/pr/1`);
  await page.getByTitle('Edit the title and description on GitHub').click();
  await page.locator('.prd-title-input').fill('Unsaved draft for PR one');
  await page.evaluate(id => { location.hash = `#/git/${id}/pr/2`; }, repoId);
  await page.getByRole('button', { name: 'Keep editing', exact: true }).click();
  await expect(page.locator('.prd-title-input')).toHaveValue('Unsaved draft for PR one');
  await page.evaluate(id => { location.hash = `#/git/${id}/pr/2`; }, repoId);
  await page.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(page.locator('.prd-title')).toContainText('PR 2:');
  await expect(page.locator('.prd-title-input')).toHaveCount(0);
});
for (const scenario of [
  { name: 'native-light-desktop', scheme: 'light', theme: 'native', width: 1440, height: 900, rtl: false },
  { name: 'native-dark-tablet', scheme: 'dark', theme: 'native', width: 820, height: 1180, rtl: false },
  { name: 'warm-light-phone', scheme: 'light', theme: 'warm', width: 390, height: 844, rtl: false },
  { name: 'warm-dark-phone-rtl', scheme: 'dark', theme: 'warm', width: 390, height: 844, rtl: true },
  { name: 'pro-dark-desktop-rtl', scheme: 'dark', theme: 'pro-dark', width: 1440, height: 900, rtl: true },
]) {
  test(`loaded Focus PR and review visual ${scenario.name}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: scenario.width, height: scenario.height });
    await page.addInitScript(({ scheme, theme, rtl }) => { localStorage.setItem('otto_scheme', scheme); localStorage.setItem('otto_theme', theme); if (rtl) document.documentElement.dir = 'rtl'; }, scenario);
    await focusFixtures(page);
    await prFixtures(page);
    await openRepo(page, 'focus');
    if (scenario.rtl) await page.evaluate(() => { document.documentElement.dir = 'rtl'; });
    await page.getByTitle('Quick view AUDIT-1', { exact: true }).click();
    await expect(page.locator('.fx-quick-summary')).toContainText('AUDIT-1');
    await expectNoHorizontalOverflow(page);
    await expectFullyInViewport(page, page.locator('.fx-quick'));
    await page.screenshot({ path: testInfo.outputPath('focus.png'), fullPage: true });
    await page.getByRole('button', { name: 'Close issue quick view' }).click();
    await page.evaluate(id => { location.hash = `#/git/${id}/pr/1`; }, repoId);
    await expect(page.locator('.prd-title')).toContainText('PR 1:');
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: testInfo.outputPath('pr-summary.png'), fullPage: true });
    await page.getByRole('tab', { name: 'Files', exact: true }).click();
    await expect(page.locator('.diff-root')).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: testInfo.outputPath('pr-files.png'), fullPage: true });
    await page.getByRole('tab', { name: 'Review', exact: true }).click();
    await expect(page.locator('.rp-comment')).toBeVisible();
    await page.getByTestId('rp-post-comment').scrollIntoViewIfNeeded();
    await expectFullyInViewport(page, page.getByTestId('rp-post-comment'));
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: testInfo.outputPath('review.png'), fullPage: true });
  });
}

test('a saved edit finishing after navigation cannot close the next PR editor', async ({ page }) => {
  const warnings: string[] = [];
  page.on('console', message => { if (message.text().includes('derived_inert')) warnings.push(message.text()); });
  await prFixtures(page);
  let saving: Route | undefined;
  await page.route('**/api/v1/repos/*/prs/1', async route => {
    if (route.request().method() === 'PATCH') saving = route;
    else await route.fallback();
  });
  await page.goto(`/#/git/${repoId}/pr/1`);
  await page.getByTitle('Edit the title and description on GitHub').click();
  await page.locator('.prd-title-input').fill('New PR one title');
  await page.getByRole('button', { name: 'Save to GitHub', exact: true }).click();
  await expect.poll(() => !!saving).toBe(true);
  await page.evaluate(id => { location.hash = `#/git/${id}/pr/2`; }, repoId);
  await page.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(page.locator('.prd-title')).toContainText('PR 2:');
  await page.getByTitle('Edit the title and description on GitHub').click();
  await page.locator('.prd-title-input').fill('PR two draft stays here');
  await saving!.fulfill({ json: {} });
  await page.waitForTimeout(150);
  await expect(page.locator('.prd-title-input')).toHaveValue('PR two draft stays here');
  expect(warnings).toEqual([]);
});

test('PR files and review code keep LTR gutters in an RTL interface', async ({ page }) => {
  await prFixtures(page);
  await page.goto(`/#/git/${repoId}/pr/1`);
  await page.evaluate(() => { document.documentElement.dir = 'rtl'; });
  await page.getByRole('tab', { name: 'Files', exact: true }).click();
  await expect(page.locator('.dtable')).toHaveCSS('direction', 'ltr');
  await expect(page.locator('.hunk-header')).toHaveCSS('direction', 'ltr');
  await page.getByRole('tab', { name: 'Review', exact: true }).click();
  await expect(page.locator('.rp-diff-snippet')).toHaveCSS('direction', 'ltr');
});
test('draft review posting previews destination and audience before sending', async ({ page }) => {
  await prFixtures(page);
  let posts = 0;
  await page.route('**/api/v1/pr-review-comments/finding-1/approve', async route => {
    posts++;
    await route.fulfill({ json: { id: 'finding-1', review_id: 'review-1', path: longPath, line: 1, severity: 'bug', body: 'Posted finding', state: 'approved', posted: true, created_at: '2026-09-25T08:00:00Z' } });
  });
  await page.goto(`/#/git/${repoId}/pr/1`);
  await page.getByRole('tab', { name: 'Review', exact: true }).click();
  await page.getByTestId('rp-post-comment').click();
  const dialog = page.getByRole('dialog', { name: 'Post comment to PR #1?' });
  await expect(dialog).toContainText('Where:');
  await expect(dialog).toContainText('What:');
  await expect(dialog).toContainText('Who sees it:');
  expect(posts).toBe(0);
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  expect(posts).toBe(0);
  await page.getByTestId('rp-post-comment').click();
  await dialog.getByRole('button', { name: 'Post to PR', exact: true }).click();
  await expect(page.locator('.rp-comment')).toContainText('posted');
  expect(posts).toBe(1);
});

test('local review history recovers from an error and expands a long finding on phone', async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  let fail = true;
  await page.route('**/api/v1/repos/*/local-reviews', route => fail
    ? route.fulfill({ status: 503, json: { code: 'upstream', message: 'Review history temporarily unavailable' } })
    : route.fulfill({ json: [{ id: 'local-review-1', repo_id: repoId, pr_number: 0, status: 'done', error: null, comments: [{ id: 'local-finding-1', review_id: 'local-review-1', path: longPath, line: 1, severity: 'bug', body: 'A local review finding with useful evidence. '.repeat(12), state: 'draft', posted: false, created_at: '2026-09-25T08:00:00Z' }], agents: [], created_at: '2026-09-25T08:00:00Z' }] }));
  await openRepo(page, 'review');
  await expect(page.locator('.lrp')).toContainText('Review history temporarily unavailable');
  fail = false;
  await page.locator('.lrp').getByRole('button', { name: 'Retry', exact: true }).click();
  await page.getByRole('button', { name: 'Past reviews (1)' }).click();
  await page.locator('.lrp-history-run-header').click();
  await expect(page.locator('.lrp-history-comment')).toContainText('useful evidence');
  await page.locator('.lrp-history-comment').scrollIntoViewIfNeeded();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: testInfo.outputPath('local-review.png'), fullPage: true });
});

test('PR title and description cannot accept edits while their save is pending', async ({ page }) => {
  await prFixtures(page);
  let saving: Route | undefined;
  await page.route('**/api/v1/repos/*/prs/1', async route => {
    if (route.request().method() === 'PATCH') saving = route;
    else await route.fallback();
  });
  await page.goto(`/#/git/${repoId}/pr/1`);
  await page.getByTitle('Edit the title and description on GitHub').click();
  await page.locator('.prd-title-input').fill('Save this title');
  await page.getByRole('button', { name: 'Save to GitHub', exact: true }).click();
  await expect.poll(() => !!saving).toBe(true);
  await expect(page.getByRole('textbox', { name: 'Pull request title', exact: true })).toBeDisabled();
  await expect(page.getByRole('textbox', { name: 'Pull request description' })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Saving…', exact: true })).toBeDisabled();
  await saving!.fulfill({ json: {} });
  await expect(page.locator('.prd-title-input')).toHaveCount(0);
});

test('remote browsing refreshes after reopening and ignores older queries', async ({ page }) => {
  let pending: Route | undefined;
  await page.route('**/api/v1/git/accounts/*/remote-repos*', route => {
    const query = new URL(route.request().url()).searchParams.get('q') ?? 'initial';
    if (query === 'old') { pending = route; return; }
    return route.fulfill({ json: [{ name: `${query}-repository`, full_name: `org/${query}`, clone_url: `https://example.test/${query}.git`, private: false }] });
  });
  await remoteFixtures(page);
  await page.getByRole('tab', { name: 'Browse remote' }).click();
  await expect(page.locator('.remote-list')).toContainText('initial-repository');
  await page.getByLabel('Search repositories').fill('old');
  await expect.poll(() => !!pending).toBe(true);
  await page.getByLabel('Search repositories').fill('new');
  await expect(page.locator('.remote-list')).toContainText('new-repository');
  await pending!.fulfill({ json: [{ name: 'old-repository', full_name: 'org/old', clone_url: 'https://example.test/old.git', private: false }] });
  await page.waitForTimeout(150);
  await expect(page.locator('.remote-list')).not.toContainText('old-repository');
  await page.getByRole('dialog', { name: 'Add repository' }).getByRole('button', { name: 'Cancel', exact: true }).click();
  await page.getByRole('button', { name: 'Add repository', exact: true }).click();
  await expect(page.locator('.remote-list')).toContainText('new-repository');
});

test('many graph references remain reachable in a short RTL viewport', async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1100, height: 500 });
  await openRepo(page);
  await page.evaluate(() => { document.documentElement.dir = 'rtl'; });
  await page.locator('.ref-expander').first().click();
  const popup = page.getByRole('dialog', { name: 'Commit references' });
  await expectFullyInViewport(page, popup);
  await popup.locator('.ref-action').last().focus();
  await expectFullyInViewport(page, popup.locator('.ref-action').last());
  await page.screenshot({ path: testInfo.outputPath('graph-many-refs.png'), fullPage: true });
  await popup.locator('.ref-action').last().click();
  await expectFullyInViewport(page, page.locator('.ctx-menu'));
  await page.keyboard.press('Escape');
  await expect(popup).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(popup).toHaveCount(0);
});

test('Focus quick-view failure explains the error and retries in place', async ({ page }) => {
  await focusFixtures(page);
  let fail = true;
  await page.route('**/api/v1/issue/account-a/AUDIT-1', route => fail
    ? route.fulfill({ status: 503, json: { code: 'upstream', message: 'Jira temporarily unavailable' } })
    : route.fulfill({ json: issue('AUDIT-1') }));
  await openRepo(page, 'focus');
  await page.getByTitle('Quick view AUDIT-1', { exact: true }).click();
  await expect(page.locator('.fx-quick')).toContainText('Jira temporarily unavailable');
  fail = false;
  await page.locator('.fx-quick').getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.locator('.fx-quick-summary')).toContainText('AUDIT-1');
});
