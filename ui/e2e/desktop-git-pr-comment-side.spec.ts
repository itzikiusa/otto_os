import { test, expect, type Route } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';

// Inline PR comments carry the diff SIDE: a deleted row posts `side: old` with
// its old number, and a thread on the old side renders under the deleted row
// only (keying by number alone drew it under the added row with the same
// number too). Forge responses are stubbed; the repo is a real throwaway one.
test.use({ serviceWorkers: 'block' });

let repoId = '';
let workspaceId = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const dir = mkdtempSync(join(tmpdir(), 'otto-pr-side-'));
  const git = (...args: string[]) => execFileSync('git', ['-C', dir, ...args], { encoding: 'utf8' }).trim();
  git('init', '-q', '-b', 'main');
  git('config', 'user.email', 'side@otto.local');
  git('config', 'user.name', 'Side test');
  git('config', 'commit.gpgsign', 'false');
  writeFileSync(join(dir, 'a.ts'), 'one\n');
  git('add', '.'); git('commit', '-qm', 'init');
  git('remote', 'add', 'origin', 'https://github.com/otto-test/pr-side.git');
  const res = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/repos`, { data: { path: dir, name: 'PR side repo' } });
  expect(res.ok()).toBeTruthy();
  repoId = (await res.json()).id;
  await ctx.dispose();
});
test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_git_auto_fetch', JSON.stringify({ enabled: false }));
  }, workspaceId);
});

const PATH = 'src/a.ts';
const thread = { id: 'c-old', author: 'reviewer', body: 'Why drop this line?', path: PATH, line: 1, side: 'old', outdated: false, created_at: '2026-10-03T08:00:00Z', replies: [], resolved: false };
const outdated = { ...thread, id: 'c-out', body: 'Stale remark', side: 'new', line: 9, outdated: true };

const extra: Record<string, unknown>[] = [];
async function fixtures(page: import('@playwright/test').Page, onPost: (r: Route) => void) {
  await page.route('**/api/v1/repos/*/prs/*', async (route) => {
    const p = new URL(route.request().url()).pathname;
    if (!/\/prs\/\d+$/.test(p)) return route.fallback();
    await route.fulfill({ json: { number: 1, title: 'Side PR', state: 'open', author: 'a', source_branch: 'feature/a', target_branch: 'main', url: 'https://github.com/otto-test/pr-side/pull/1', updated_at: '2026-10-03T08:00:00Z', draft: false, head_sha: 'h1', description_md: '', approved_by: [], reviewers: [], mergeable: true, comments: [thread, outdated, ...extra] } });
  });
  await page.route('**/api/v1/repos/*/prs/*/diff*', (r) => r.fulfill({ json: { files: [{ path: PATH, old_path: null, is_binary: false, hunks: [{ header: '@@ -1 +1 @@', lines: [
    { origin: 'del', content: 'one', old_line: 1, new_line: null },
    { origin: 'add', content: 'uno', old_line: null, new_line: 1 },
  ] }] }] } }));
  await page.route('**/api/v1/repos/*/prs/*/commits', (r) => r.fulfill({ json: [] }));
  await page.route('**/api/v1/repos/*/prs/*/reviews', (r) => r.fulfill({ json: [] }));
  await page.route('**/api/v1/repos/*/prs/1/comments', onPost);
}

test('old-side thread renders once and a deleted-row comment posts side old', async ({ page }) => {
  let posted: Record<string, unknown> | null = null;
  await fixtures(page, (route) => {
    posted = route.request().postDataJSON();
    const c = { id: 'c-new', author: 'me', body: 'Posted', path: PATH, line: 1, side: 'old', outdated: false, created_at: '2026-10-03T09:00:00Z', replies: [], resolved: false };
    extra.push(c);
    void route.fulfill({ json: c });
  });
  await page.goto(`/#/git/${repoId}/pr/1`);
  await page.getByRole('tab', { name: 'Files', exact: true }).click();
  const diff = page.locator('.prd-diff');
  await expect(diff.getByText('Why drop this line?')).toHaveCount(1);
  // The outdated thread is a file-level comment with a chip.
  await expect(diff.locator('.file-comments-block').getByText('Stale remark')).toBeVisible();
  await expect(diff.locator('.outdated-chip')).toContainText('Outdated');

  // Click the deleted row's gutter → composer on the OLD side.
  await diff.locator('.gut.old.commentable', { hasText: /^1$/ }).first().click();
  const box = diff.getByPlaceholder('Comment on old line 1…');
  await box.fill('Posted');
  await diff.getByRole('button', { name: 'Comment', exact: true }).click();
  await expect.poll(() => posted).not.toBeNull();
  expect(posted).toMatchObject({ path: PATH, line: 1, side: 'old', old_line: 1 });
  // Shown under the deleted row once (appended, then reconciled by the quiet reload).
  await expect(diff.getByText('Posted', { exact: true })).toHaveCount(1);
});
