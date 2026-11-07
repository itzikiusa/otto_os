import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

test.use({ serviceWorkers: 'block' });

test('review conversations and selected diffs stack in the owning draft and send with attribution', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const workspaceId = await seedWorkspace(ctx, base);
  await ctx.dispose();
  const ts = '2026-11-07T08:00:00Z';
  const sessions = ['review-alpha', 'review-beta'].map((id) => ({
    id, workspace_id: workspaceId, provider: 'claude', title: id, cwd: '/tmp/review-context', status: 'idle',
    access_mode: 'approval-required', created_by: 'root', created_at: ts, last_active_at: ts, last_sequence: 1, parent_id: null, meta: {}, model: null,
  }));
  const repo = { id: 'review-repo', workspace_id: workspaceId, name: 'Review repo', path: '/tmp/review-context', remote_url: 'https://github.com/example/repo.git', provider: 'github', forge: 'github', git_account_id: null, created_at: ts };
  const pr = { number: 1, title: 'Review context', state: 'open', source_branch: 'feature', target_branch: 'main', author: 'Alice', url: 'https://github.com/example/repo/pull/1', updated_at: ts, draft: false };
  const comments = [
    { id: 'one', author: 'Alice', body: 'Check null input', path: 'src/a.ts', line: 9, created_at: ts, resolved: false, replies: [{ id: 'reply', author: 'Bob', body: 'Agreed', path: null, line: null, created_at: ts, resolved: false, replies: [] }] },
    { id: 'two', author: 'Carol', body: 'Keep the default', path: null, line: null, created_at: ts, resolved: false, replies: [] },
  ];
  await page.addInitScript(({ workspaceId }) => {
    localStorage.setItem('otto_workspace', workspaceId);
    localStorage.setItem('otto_threads_v1', JSON.stringify({ focusedSessionId: 'review-alpha', paneIds: ['review-alpha'], rightOpen: true, inspectorTab: 'pr' }));
    localStorage.setItem('otto_threads_chrome_v1', JSON.stringify({ surfaces: ['pr', 'changes'] }));
  }, { workspaceId });
  await page.route(`**/api/v1/workspaces/${workspaceId}/runtime-sessions`, (route) => route.fulfill({ json: sessions }));
  await page.route('**/api/v1/runtime-sessions/review-*/events**', (route) => route.fulfill({ json: { kind: 'snapshot', snapshot_sequence: 1, snapshot: { provider_session: { status: 'ready', cwd: repo.path, model: null, updated_at: ts, last_error: null }, provider_thread: null, turns: [], turn_items: [], runtime_requests: [], plans: [] } } }));
  await page.route('**/api/v1/workspaces/*/repos', (route) => route.fulfill({ json: [repo] }));
  await page.route('**/api/v1/workspaces/*/repos/detect', (route) => route.fulfill({ json: repo }));
  await page.route('**/api/v1/repos/review-repo/status', (route) => route.fulfill({ json: { branch: 'feature', upstream: null, ahead: 0, behind: 0, changes: [{ path: 'src/a.ts', orig_path: null, kind: 'modified' }] } }));
  await page.route('**/api/v1/repos/review-repo/prs?*', (route) => route.fulfill({ json: { items: [pr], page: 1, per_page: 50, has_more: false } }));
  let releaseDetail: (() => void) | undefined;
  let delayDetail = false;
  let detailRequests = 0;
  await page.route('**/api/v1/repos/review-repo/prs/1', async (route) => {
    detailRequests++;
    if (delayDetail) await new Promise<void>((resolve) => { releaseDetail = resolve; });
    await route.fulfill({ json: { ...pr, comments, description_md: '', reviewers: [], approved_by: [], mergeable: true } }).catch(() => {});
  });
  await page.route('**/api/v1/repos/review-repo/diff?*', (route) => route.fulfill({ json: { files: [{ path: 'src/a.ts', old_path: null, is_binary: false, added: 1, deleted: 0, hunks: [{ header: '@@ -8,1 +8,2 @@', lines: [{ origin: 'context', content: 'before', old_line: 8, new_line: 8 }, { origin: 'add', content: 'after', old_line: null, new_line: 9 }] }] }] } }));
  const drafts = new Map<string, { revision: number; draft: unknown }>();
  await page.route('**/api/v1/runtime-sessions/review-*/draft', (route) => {
    const id = route.request().url().split('/').at(-2)!;
    const saved = drafts.get(id) ?? { revision: 0, draft: null };
    if (route.request().method() === 'PUT') {
      const body = route.request().postDataJSON();
      drafts.set(id, { revision: saved.revision + 1, draft: body.draft });
    }
    return route.fulfill({ json: drafts.get(id) ?? saved });
  });
  const sent: { id: string; text: string }[] = [];
  await page.route('**/api/v1/runtime-sessions/review-*/prompt', (route) => {
    sent.push({ id: route.request().url().split('/').at(-2)!, text: route.request().postDataJSON().text });
    return route.fulfill({ json: { sequence: 1 } });
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/threads');
  const alpha = page.getByTestId('runtime-thread-review-alpha');
  const composer = alpha.getByTestId('runtime-composer');
  await composer.fill('My existing question');
  await page.getByTestId('pr-comment-context-one').click();
  await page.getByTestId('pr-comment-context-two').click();
  await page.getByTestId('pr-comment-context-one').click();
  const chips = alpha.getByTestId('runtime-context-chips');
  await expect(chips.locator('li')).toHaveCount(2);
  await expect(composer).toHaveValue('My existing question');
  expect(sent).toHaveLength(0);
  await chips.getByRole('button', { name: 'Remove Carol · General comment' }).click();
  await expect(chips.locator('li')).toHaveCount(1);
  await page.getByTestId('threads-tab-changes').click();
  await page.getByRole('button', { name: 'Add src/a.ts new line 9 to chat', exact: true }).click();
  await page.getByTestId('diff-hunk-context-0').click();
  await page.getByTestId('diff-file-context').click();
  await expect(chips.locator('li')).toHaveCount(4);
  await page.getByTestId('threads-sidebar').getByText('review-beta', { exact: true }).click();
  await expect(page.getByTestId('runtime-thread-review-beta').getByTestId('runtime-context-chips')).toHaveCount(0);
  await page.getByTestId('threads-sidebar').getByText('review-alpha', { exact: true }).click();
  await expect(chips.locator('li')).toHaveCount(4);
  await expect(composer).toHaveValue('My existing question');
  await alpha.getByTestId('runtime-send').click();
  await expect.poll(() => sent.length).toBe(1);
  expect(sent[0].id).toBe('review-alpha');
  for (const text of ['My existing question', 'Alice', 'Bob', pr.url, 'src/a.ts:9', 'Check null input', 'Agreed', '[new 9] +after']) expect(sent[0].text).toContain(text);
  expect(sent[0].text).not.toContain('Carol');
  await expect(chips).toHaveCount(0);

  // A delayed read for A must disappear immediately when switching to B;
  // the old surface cannot provide a chip into B while B's read is pending.
  delayDetail = true;
  await page.getByTestId('threads-tab-pr').click();
  await expect.poll(() => !!releaseDetail).toBe(true);
  const releaseA = releaseDetail!;
  const requestsBefore = detailRequests;
  await page.getByTestId('threads-sidebar').getByText('review-beta', { exact: true }).click();
  await expect.poll(() => detailRequests).toBeGreaterThan(requestsBefore);
  await expect(page.getByTestId('pr-comment-context-one')).toHaveCount(0);
  releaseA();
  await expect(page.getByTestId('pr-comment-context-one')).toHaveCount(0);
  releaseDetail?.();
  await expect(page.getByTestId('pr-comment-context-one')).toBeVisible();
  await expect(page.getByTestId('runtime-thread-review-beta').getByTestId('runtime-context-chips')).toHaveCount(0);
});
