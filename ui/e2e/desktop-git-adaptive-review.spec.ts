import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import type { Finding } from '../src/lib/api/types';

// Real isolated app/repository; forge/review/goal responses are fixtures. No agent is launched.
test.use({ serviceWorkers: 'block' });
let workspaceId = '';
let repoId = '';
const now = '2026-11-07T10:00:00Z';
const head = 'a'.repeat(40);
const scope = {
  head_sha: head, summary: 'A permission boundary changed in auth.rs.', source: 'agent assessment · 4s',
  related_modules: ['auth.rs:2 → api.rs:8 caller'], risks: ['Anonymous callers can bypass the permission check.'],
  lenses: [{ name: 'Security', reason: 'The permission check changed.', instructions: 'Trace caller access.' }],
  prior_findings: [{ finding_id: 'previous', disposition: 'fixed', evidence: 'api.rs:8 now passes the caller identity.' }],
  verification_notes: ['not_relevant: unrelated old cleanup issue — unchanged code at old.rs:1'],
};
function review(status = 'done') {
  return { id: 'review-adaptive', repo_id: repoId, pr_number: 1, status, error: status === 'error' ? 'Final verification is incomplete.' : null,
    comments: [{ id: 'finding-real', review_id: 'review-adaptive', path: 'auth.rs', line: 2, severity: 'bug', body: 'New permission bypass. Evidence: auth.rs:2 → api.rs:8.', state: 'draft', posted: false, created_at: now }],
    agents: [{ name: 'Security', provider: 'codex', model: '', status: 'done', note: '1 finding', comment_count: 1, findings: [], lens: 'Security' }], created_at: now, scope_plan: scope };
}
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const dir = mkdtempSync(join(tmpdir(), 'otto-adaptive-review-'));
  const git = (...args: string[]) => execFileSync('git', ['-C', dir, ...args], { stdio: 'ignore' });
  git('init', '-q', '-b', 'main'); git('config', 'user.name', 'E2E'); git('config', 'user.email', 'e2e@example.invalid'); git('config', 'commit.gpgsign', 'false');
  writeFileSync(join(dir, 'auth.rs'), 'permission_check();\n'); git('add', '.'); git('commit', '-qm', 'seed');
  git('remote', 'add', 'origin', 'https://github.com/otto-test/adaptive-review.git');
  const response = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/repos`, { data: { path: dir, name: 'Adaptive review fixture' } });
  expect(response.ok()).toBeTruthy(); repoId = (await response.json()).id;
  await ctx.dispose();
});
async function openReview(page: Page, status = 'done') {
  await page.addInitScript(({ ws, repo }) => {
    localStorage.setItem('otto_workspace', ws);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto_git_auto_fetch', JSON.stringify({ enabled: false }));
    localStorage.setItem(`otto_pr_tab_${repo}_1`, 'review');
  }, { ws: workspaceId, repo: repoId });
  await page.route('**/api/v1/repos/*/prs/1', r => r.fulfill({ json: { number: 1, title: 'Permission change', state: 'open', author: 'test', source_branch: 'feature/auth', target_branch: 'main', url: 'https://github.com/otto-test/adaptive-review/pull/1', updated_at: now, draft: false, head_sha: head, description_md: '', approved_by: [], reviewers: [], mergeable: true, comments: [] } }));
  await page.route('**/api/v1/repos/*/prs/1/reviews', r => r.fulfill({ json: [review(status)] }));
  await page.route('**/api/v1/repos/*/prs/1/diff*', r => r.fulfill({ json: { files: [] } }));
  await page.route('**/api/v1/repos/*/prs/1/commits', r => r.fulfill({ json: [] }));
  const finding: Finding = {
    id: 'finding-real', review_id: 'review-adaptive', workspace_id: workspaceId, repo_id: repoId,
    pr_number: 1, fingerprint: 'permission-check', severity: 'high', category: 'Security',
    path: 'auth.rs', line: 2, line_end: 2, title: 'Permission check can be bypassed',
    body: 'An anonymous request reaches the new action without permission.',
    evidence: 'auth.rs:2 → api.rs:8', agent_reasoning_summary: 'Traced anonymous caller access.',
    suggested_fix: 'Require caller permission before the action.', status: 'open', state: 'open',
    linked_commit: null, linked_test: null, reviewer: 'Security', regressed: false,
    requires_human_approval: false, approval_decision: null, approved_by: null, approved_at: null,
    jira_key: null, jira_url: null, produced_by_agent: null, repo_rule_id: null,
    fix_session_id: null, occurrence_count: 1, created_at: now, updated_at: now,
  };
  await page.route('**/api/v1/reviews/review-adaptive/findings', r => r.fulfill({ json: [finding] }));
  await page.route('**/api/v1/reviews/review-adaptive/merge-readiness', r => r.fulfill({ json: { unresolved_total: 1, unresolved_blocker: 1, total: 1, complete: true } }));
  await page.goto(`/#/git/${repoId}/pr/1`);
  await expect(page.getByRole('tab', { name: 'Review', exact: true })).toBeVisible();
  await page.getByRole('tab', { name: 'Review', exact: true }).click();
}

test('scope explains exact lenses and keeps excluded claims out of comments', async ({ page }, info) => {
  await openReview(page);
  const panel = page.locator('.review-scope');
  await panel.locator('summary').click();
  await expect(panel).toContainText('The permission check changed.');
  await expect(panel).toContainText('codex');
  await expect(panel).toContainText('previous');
  await expect(panel).toContainText('Claims checked and excluded');
  await expect(panel).toContainText('unrelated old cleanup issue');
  await expect(page.locator('.rp-comment').filter({ hasText: 'unrelated old cleanup issue' })).toHaveCount(0);
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: info.outputPath(`adaptive-review-${scheme}.png`), fullPage: true });
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(panel).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBeTruthy();
  await page.screenshot({ path: info.outputPath('adaptive-review-phone.png'), fullPage: true });
});

test('fix goal uses reviewed id/default target once, retries inline, and opens progress', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const calls: unknown[] = [];
  await page.route('**/api/v1/reviews/*/fix-goal', async r => {
    expect(new URL(r.request().url()).pathname.endsWith('/reviews/review-adaptive/fix-goal')).toBeTruthy();
    calls.push(r.request().postDataJSON());
    if (calls.length === 1) return r.fulfill({ status: 503, json: { code: 'upstream', message: 'Temporary launch failure' } });
    await r.fulfill({ json: { id: 'goal-review-fixture' } });
  });
  await openReview(page);
  await page.getByRole('button', { name: 'Run agent to fix…', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Fix review findings' });
  await expect(dialog.getByRole('spinbutton')).toHaveValue('9.5');
  await dialog.getByRole('spinbutton').fill('11');
  await expect(dialog.getByRole('button', { name: 'Run agent to fix', exact: true })).toBeDisabled();
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await page.getByRole('button', { name: 'Run agent to fix…', exact: true }).click();
  await dialog.getByRole('spinbutton').fill('9.5');
  await dialog.getByRole('button', { name: 'Run agent to fix', exact: true }).click();
  await expect(dialog).toContainText('Temporary launch failure');
  await expect(dialog).toContainText('Couldn’t start fixes');
  expect(calls).toEqual([{ target_score: 9.5 }]);
  await dialog.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page).toHaveURL(/#\/loops\/goal-review-fixture$/);
  expect(calls).toEqual([{ target_score: 9.5 }, { target_score: 9.5 }]);
});

test('incomplete verification cannot launch fixes or show a clean result', async ({ page }) => {
  await openReview(page, 'error');
  await expect(page.getByText('Final verification is incomplete.', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Run agent to fix…', exact: true })).toHaveCount(0);
  await expect(page.getByText('No issues found', { exact: true })).toHaveCount(0);
});
