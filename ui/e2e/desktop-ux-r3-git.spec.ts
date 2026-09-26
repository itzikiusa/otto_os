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
const tabRepos: string[] = [];
const repoNames = ['promotions-service', 'bo_common_ui', 'cs3-platform', 'koala-backoffice', 'go_dependencies'];
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
  git('branch', 'origin/collision');
  git('update-ref', 'refs/remotes/origin/collision', firstSha);
  git('remote', 'add', 'origin', 'https://github.com/otto-test/ux-audit.git');
  const response = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/repos`, { data: { path: dir, name: 'UX audit repository' } });
  expect(response.ok()).toBeTruthy();
  repoId = (await response.json()).id;
  for (const name of repoNames) {
    const tabDir = mkdtempSync(join(tmpdir(), 'otto-ux-tab-'));
    execFileSync('git', ['-C', tabDir, 'init', '-q', '-b', 'feature/customer-experience-2026']);
    execFileSync('git', ['-C', tabDir, '-c', 'user.name=UX audit', '-c', 'user.email=ux@otto.local', '-c', 'commit.gpgsign=false', 'commit', '--allow-empty', '-qm', 'Synthetic repository baseline']);
    const result = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/repos`, { data: { path: tabDir, name } });
    tabRepos.push((await result.json()).id);
  }
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

test('five long repo and branch identities use available width at 2048px', async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 2048, height: 900 });
  await page.goto(`/#/git/${tabRepos[0]}/graph`);
  for (const name of repoNames.slice(1)) {
    await page.locator('.git-tab-new').click();
    await page.locator('.ctx-search-input').fill(name);
    await page.getByRole('menuitem', {name, exact: true}).click();
  }
  await expect(page.locator('.git-tab')).toHaveCount(5);
  await expect(page.locator('.git-tab-branch-name').first()).toHaveText('feature/customer-experience-2026');
  await expect(page.locator('.graph-row').first()).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('five-long-tabs.png') });
  const layout = await page.locator('.git-tabs').evaluate(el => ({
    truncated: [...el.querySelectorAll('.git-tab-name, .git-tab-branch-name')].some(e => e.scrollWidth > e.clientWidth + 1),
    spare: el.closest('.ph-row')!.getBoundingClientRect().right - el.querySelector('.git-tab-new')!.getBoundingClientRect().right,
  }));
  expect(layout.truncated && layout.spare > 80, JSON.stringify(layout)).toBe(false);
  await expectFullyInViewport(page, page.locator('.git-tab-new'));
});

test('nested graph reference menu owns Tab and Escape then restores its action trigger', async ({ page }) => {
  await openRepo(page);
  const expander = page.locator('.ref-expander').first();
  await expect(expander).toBeEnabled();
  await expander.click();
  const popup = page.locator('.ref-popover');
  const action = popup.getByRole('button', { name: 'Actions for release/long-reference-0', exact: true });
  await action.focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('.ctx-menu')).toBeVisible();
  await page.keyboard.press('ArrowDown');
  await expect.poll(() => page.locator('.ctx-menu').evaluate(el => el.contains(document.activeElement))).toBe(true);
  await page.keyboard.press('Escape');
  await expect(page.locator('.ctx-menu')).toHaveCount(0);
  await expect(popup).toBeVisible();
  await expect(action).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.locator('.ctx-menu')).toBeVisible();
  await page.keyboard.press('Tab');
  await expect(page.locator('.ctx-menu')).toHaveCount(0);
  await expect(popup).toBeVisible();
  await expect(action).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(popup).toHaveCount(0);
  await expect(expander).toBeFocused();
});

test('PR comment composer is locked during send and survives failed publication', async ({ page }) => {
  await prFixtures(page);
  let pending: Route | undefined;
  await page.route('**/api/v1/repos/*/prs/1/comments', route => { pending = route; });
  await page.goto(`/#/git/${repoId}/pr/1`);
  const composer = page.getByLabel('New comment', { exact: true });
  await composer.fill('Preserve this comment until the server accepts it');
  await page.getByRole('button', { name: 'Post comment', exact: true }).click();
  await expect.poll(() => !!pending).toBe(true);
  await expect(composer).toBeDisabled();
  await pending!.fulfill({ status: 503, json: {code: 'upstream', message: 'Forge temporarily unavailable'} });
  await expect(composer).toBeEnabled();
  await expect(composer).toHaveValue('Preserve this comment until the server accepts it');
  pending = undefined;
  await page.getByRole('button', { name: 'Post comment', exact: true }).click();
  await expect.poll(() => !!pending).toBe(true);
  expect(pending!.request().postDataJSON().body).toBe('Preserve this comment until the server accepts it');
  await pending!.fulfill({json: {}});
  await expect(composer).toHaveValue('');
});

test('PR comment completion after navigation does not read disposed identity', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', e => errors.push(e.message));
  page.on('console', e => { if (e.text().includes('derived_inert')) errors.push(e.text()); });
  await prFixtures(page);
  let pending: Route | undefined;
  await page.route('**/api/v1/repos/*/prs/1/comments', route => { pending = route; });
  await page.goto(`/#/git/${repoId}/pr/1`);
  await page.getByLabel('New comment', { exact: true }).fill('Submitted comment');
  await page.getByRole('button', { name: 'Post comment', exact: true }).click();
  await expect.poll(() => !!pending).toBe(true);
  await page.evaluate(id => { location.hash = `#/git/${id}/pr/2`; }, repoId);
  await page.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(page.locator('.prd-title')).toContainText('PR 2:');
  await page.getByLabel('New comment', { exact: true }).fill('PR two draft');
  await pending!.fulfill({ json: {} });
  await page.waitForTimeout(150);
  await expect(page.getByLabel('New comment', { exact: true })).toHaveValue('PR two draft');
  expect(errors).toEqual([]);
});

test('failed comment reply stays editable and presents an inline retry error', async ({ page }, testInfo) => {
  await page.setViewportSize({width: 390, height: 844});
  await page.addInitScript(() => { localStorage.setItem('otto_theme', 'warm'); localStorage.setItem('otto_scheme', 'light'); });
  await prFixtures(page);
  let fail = true;
  await page.route('**/api/v1/repos/*/prs/1/comments', route => fail ? route.fulfill({ status: 503, json: {code: 'upstream', message: 'Forge temporarily unavailable'} }) : route.fulfill({json: {}}));
  await page.goto(`/#/git/${repoId}/pr/1`);
  const thread = page.locator('.cmt').first();
  await thread.getByRole('button', { name: 'Reply', exact: true }).click();
  await thread.getByPlaceholder('Reply…').fill('Reply must not disappear');
  await thread.getByRole('button', { name: 'Reply', exact: true }).click();
  await expect(thread.getByRole('alert')).toContainText('Forge temporarily unavailable');
  await expect(thread.getByPlaceholder('Reply…')).toHaveValue('Reply must not disappear');
  await expectNoHorizontalOverflow(page);
  await page.screenshot({path: testInfo.outputPath('reply-error-warm-light-phone.png')});
  await page.evaluate(id => { location.hash = `#/git/${id}/pr/2`; }, repoId);
  await page.getByRole('button', { name: 'Keep editing', exact: true }).click();
  await expect(thread.getByPlaceholder('Reply…')).toHaveValue('Reply must not disappear');
  fail = false;
  await thread.getByRole('button', { name: 'Reply', exact: true }).click();
  await expect(thread.getByPlaceholder('Reply…')).toHaveCount(0);
});

test('request changes composer locks pending content and can retry a failed send', async ({ page }) => {
  await prFixtures(page);
  let pending: Route | undefined;
  await page.route('**/api/v1/repos/*/prs/1/request-changes', route => { pending = route; });
  await page.goto(`/#/git/${repoId}/pr/1`);
  await page.locator('.prd-actions').getByRole('button', { name: 'Request changes', exact: true }).click();
  const panel = page.locator('.prd-request-changes');
  await panel.getByLabel('What needs to change').fill('Please cover the retry failure');
  await panel.getByRole('button', { name: 'Request changes', exact: true }).click();
  await expect.poll(() => !!pending).toBe(true);
  await expect(panel.getByLabel('What needs to change')).toBeDisabled();
  await expect(panel.getByRole('button', { name: 'Cancel', exact: true })).toBeDisabled();
  await pending!.fulfill({ status: 503, json: {code: 'upstream', message: 'Forge temporarily unavailable'} });
  await expect(panel.getByLabel('What needs to change')).toBeEnabled();
  await expect(panel.getByLabel('What needs to change')).toHaveValue('Please cover the retry failure');
  pending = undefined;
  await panel.getByRole('button', { name: 'Request changes', exact: true }).click();
  await expect.poll(() => !!pending).toBe(true);
  expect(pending!.request().postDataJSON().body).toBe('Please cover the retry failure');
  await pending!.fulfill({json: {}});
  await expect(panel).toHaveCount(0);
});

test('local and remote references with the same name retain their own menus', async ({ page }, testInfo) => {
  await openRepo(page);
  await expect(page.locator('.ref-expander').first()).toBeEnabled();
  await page.locator('.ref-expander').first().click();
  const local = page.locator('.ref-popover .ref-action-row').filter({has: page.locator('.kind-local')}).filter({hasText: 'origin/collision'});
  await expect(local).toHaveCount(1);
  await local.getByRole('button', {name: 'Actions for origin/collision', exact: true}).click();
  await expect(page.locator('.ctx-menu')).toContainText('Rename');
  await page.keyboard.press('Escape');
  await page.keyboard.press('Escape');
  // Ref metadata is authoritative even if log decorations are ambiguous.
  await page.route('**/api/v1/repos/*/refs*', async route => {
    const response = await route.fetch();
    const refs = await response.json();
    refs.remote.find((b: {name: string}) => b.name === 'origin/collision').sha = secondSha;
    await route.fulfill({json: refs});
  });
  await page.reload();
  const expander = page.locator('.ref-expander').first();
  await expect(expander).toBeEnabled();
  await expander.click();
  const remote = page.locator('.ref-popover .ref-action-row').filter({has: page.locator('.kind-remote')}).filter({hasText: 'origin/collision'});
  await expect(remote).toHaveCount(1);
  await remote.getByRole('button', {name: 'Actions for origin/collision', exact: true}).click();
  await expect(page.locator('.ctx-menu')).not.toContainText('Rename');
  await expect(page.locator('.ctx-menu')).toContainText('Checkout');
  await page.screenshot({path: testInfo.outputPath('collision-reference-menu.png')});
});

test('pending merge completion after dismissing and navigating keeps the new PR stable', async ({ page }) => {
  const errors: string[] = [];
  page.on('console', e => { if (e.text().includes('derived_inert')) errors.push(e.text()); });
  await prFixtures(page);
  await page.route('**/api/v1/repos/*/prs/1/checks', route => route.fulfill({json: {ci: {state: 'success'}, checks: []}}));
  await page.route('**/api/v1/repos/*/prs/1/readiness', route => route.fulfill({json: {approvals: 1, mergeable: true, unpushed: 0, branch_freshness: 'fresh', review: null}}));
  let pending: Route | undefined;
  await page.route('**/api/v1/repos/*/prs/1/merge', route => { pending = route; });
  await page.goto(`/#/git/${repoId}/pr/1`);
  await page.locator('.prd-actions').getByRole('button', {name: 'Merge', exact: true}).click();
  await page.getByRole('dialog').getByRole('button', {name: 'Merge pull request', exact: true}).click();
  await expect.poll(() => !!pending).toBe(true);
  await page.keyboard.press('Escape');
  await page.evaluate(id => { location.hash = `#/git/${id}/pr/2`; }, repoId);
  await expect(page.locator('.prd-title')).toContainText('PR 2:');
  await pending!.fulfill({json: {}});
  await page.waitForTimeout(150);
  await expect(page.locator('.prd-title')).toContainText('PR 2:');
  expect(errors).toEqual([]);
});


test('short repo tabs keep natural width on a wide screen', async ({ page }) => {
  await page.setViewportSize({width: 2048, height: 900});
  await openRepo(page);
  const tab = page.locator('.git-tab').first();
  await expect(tab.locator('.git-tab-branch-name')).toHaveText('main');
  expect((await tab.boundingBox())!.width).toBeLessThan(300);
});

test('phone RTL merge failure preserves strategy and retries the mocked publication', async ({ page }, testInfo) => {
  await page.setViewportSize({width: 390, height: 844});
  await page.addInitScript(() => { localStorage.setItem('otto_theme', 'warm'); localStorage.setItem('otto_scheme', 'dark'); });
  await prFixtures(page);
  await page.route('**/api/v1/repos/*/prs/1/checks', route => route.fulfill({json: {ci: {state: 'success'}, checks: []}}));
  await page.route('**/api/v1/repos/*/prs/1/readiness', route => route.fulfill({json: {approvals: 1, mergeable: true, unpushed: 0, branch_freshness: 'fresh', review: null}}));
  let posts = 0;
  await page.route('**/api/v1/repos/*/prs/1/merge', route => {
    posts++;
    expect(route.request().postDataJSON()).toEqual({strategy: 'squash', delete_source_branch: true});
    return posts === 1 ? route.fulfill({status: 503, json: {code: 'upstream', message: 'Forge temporarily unavailable'}}) : route.fulfill({json: {}});
  });
  await page.goto(`/#/git/${repoId}/pr/1`);
  await page.evaluate(() => { document.documentElement.dir = 'rtl'; });
  await page.locator('.prd-actions').getByRole('button', {name: 'Merge', exact: true}).click();
  const modal = page.getByRole('dialog', {name: 'Merge pull request'});
  await modal.getByLabel('Strategy').selectOption('squash');
  await modal.getByLabel('Delete source branch after merge').check();
  await modal.getByRole('button', {name: 'Merge pull request', exact: true}).click();
  await expect(modal.getByRole('alert')).toContainText('Forge temporarily unavailable');
  await expect(modal.getByLabel('Strategy')).toHaveValue('squash');
  await expectFullyInViewport(page, modal);
  await expectNoHorizontalOverflow(page);
  await page.screenshot({path: testInfo.outputPath('merge-error-warm-dark-phone-rtl.png')});
  await modal.getByRole('button', {name: 'Merge pull request', exact: true}).click();
  await expect(modal).toHaveCount(0);
  expect(posts).toBe(2);
});

test('lane tooltip prefers the checked-out branch when many references share a tip', async ({ page }) => {
  await openRepo(page);
  await expect(page.locator('.ref-expander').first()).toBeEnabled();
  await expect(page.locator(`.graph-row[data-sha="${secondSha}"] .lane-hit title`).last()).toHaveText('main');
});
