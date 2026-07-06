import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Git PR-creation enhancements (focused coverage; desktop-browser only):
//   A. The New-PR sheet shows an "Open as draft" toggle that persists per repo
//      (localStorage) across close/reopen.
//   B. The sheet shows a "Reviewers (optional)" chips input; free-text entry
//      adds a removable chip (the provider typeahead is exercised by Rust
//      tests — the isolated daemon has no real forge account).
//   C. Settings → Git Accounts: every account row has a Test button; the
//      stored-token endpoint's verdict renders inline (endpoint stubbed so the
//      flow is deterministic — the endpoint logic itself is Rust-tested).
//   D. A repo whose remote host is NOT a supported forge (e.g. Bitbucket
//      Server) gets an honest empty state on the PR tab naming the host,
//      instead of a silent dead-end.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let githubRepoId = ''; // remote → github.com (recognized forge)
let serverRepoId = ''; // remote → bitbucket-server.corp.example.com (unrecognized)

/** Register a local repo that carries `remoteUrl` as its origin, so the daemon
 *  detects (or fails to detect) the forge exactly like a real checkout. */
async function seedRepoWithRemote(
  ctx: APIRequestContext,
  base: string,
  wsId: string,
  name: string,
  remoteUrl: string,
): Promise<string> {
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-prx-'));
  const git = (...args: string[]) => execFileSync('git', ['-C', dir, ...args], { stdio: 'ignore' });
  git('init', '-q');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');
  writeFileSync(join(dir, 'readme.md'), `# ${name}\n`);
  git('add', '-A');
  git('commit', '-q', '-m', 'init');
  git('remote', 'add', 'origin', remoteUrl);
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, {
    data: { path: dir, name },
  });
  if (!r.ok()) throw new Error(`repo seed failed: ${r.status()} ${await r.text()}`);
  return ((await r.json()) as { id: string }).id;
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  githubRepoId = await seedRepoWithRemote(
    ctx, base, workspaceId, 'e2e-gh-repo', 'https://github.com/e2e-org/fake-repo.git',
  );
  serverRepoId = await seedRepoWithRemote(
    ctx, base, workspaceId, 'e2e-bbs-repo',
    'https://bitbucket-server.corp.example.com/scm/proj/repo.git',
  );
  // A git account so the Settings row (and its Test button) renders.
  const acct = await ctx.post(`${base}/api/v1/git/accounts`, {
    data: { provider: 'github', label: 'e2e test acct', username: 'e2e-user', token: 'ghp_e2e_fake' },
  });
  if (!acct.ok()) throw new Error(`account seed failed: ${acct.status()} ${await acct.text()}`);
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

async function openNewPrSheet(page: Page): Promise<void> {
  await page.goto(`/#/git/${githubRepoId}/prs`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  await page.getByRole('button', { name: 'New PR' }).click();
  await expect(page.getByText('New Pull Request')).toBeVisible();
}

test('create sheet: draft toggle renders and persists per repo', async ({ page }) => {
  await openNewPrSheet(page);
  const toggle = page.locator('.draft-toggle input[type="checkbox"]');
  await expect(toggle).toBeVisible();
  await expect(toggle).not.toBeChecked();

  // Tick it, close the sheet, reopen: the choice must survive (localStorage).
  await toggle.check();
  await page.getByRole('button', { name: 'Cancel' }).click();
  await page.getByRole('button', { name: 'New PR' }).click();
  await expect(page.locator('.draft-toggle input[type="checkbox"]')).toBeChecked();

  // And it survives a full reload too.
  await page.reload();
  await openNewPrSheet(page);
  await expect(page.locator('.draft-toggle input[type="checkbox"]')).toBeChecked();
  // Untick to leave a clean slate for other tests.
  await page.locator('.draft-toggle input[type="checkbox"]').uncheck();
});

test('create sheet: reviewers chips input adds and removes free-text chips', async ({ page }) => {
  await openNewPrSheet(page);
  const chips = page.getByTestId('pr-reviewers');
  await expect(chips).toBeVisible();

  // No provider account is usable in the isolated daemon → the collaborator
  // lookup fails and the input degrades to free text (design: names pass
  // through verbatim).
  const input = chips.locator('input.chips-text');
  await input.fill('octocat');
  await input.press('Enter');
  await expect(chips.locator('.rev-chip')).toHaveText(/octocat/);

  await input.fill('hubot');
  await input.press('Enter');
  await expect(chips.locator('.rev-chip')).toHaveCount(2);

  // Remove a chip via its × button.
  await chips.locator('.rev-chip-x').first().click();
  await expect(chips.locator('.rev-chip')).toHaveCount(1);
  await expect(chips.locator('.rev-chip')).toHaveText(/hubot/);
});

test('unrecognized forge: PR tab renders an honest empty state naming the host', async ({ page }) => {
  await page.goto(`/#/git/${serverRepoId}/prs`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  await expect(
    page.getByText("Pull requests aren't available for bitbucket-server.corp.example.com"),
  ).toBeVisible();
  await expect(page.getByText('Otto supports GitHub, Bitbucket Cloud, and GitLab', { exact: false })).toBeVisible();
  // No "New PR" button on an unsupported forge — the surface is the message.
  await expect(page.getByRole('button', { name: 'New PR' })).toHaveCount(0);
});

test('git accounts: row Test button renders the verdict inline', async ({ page }) => {
  // Stub the stored-token test endpoint so the UI flow is deterministic (the
  // endpoint logic is covered by Rust tests; a real call would hit github.com).
  await page.route('**/api/v1/git/accounts/*/test', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ ok: true, login: 'octo-e2e', scopes: ['repo', 'read:org'] }),
    }),
  );
  await openPage(page, 'settings/git-accounts');
  // beforeAll runs once per parallel worker, so several identical seeded
  // accounts may exist — any one row exercises the flow.
  const row = page.locator('.acct', { hasText: 'e2e test acct' }).first();
  await expect(row).toBeVisible();
  await row.getByRole('button', { name: 'Test' }).click();
  const verdict = row.locator('.test-result');
  await expect(verdict).toHaveText(/ok — authenticated as octo-e2e/);
  await expect(verdict).toHaveText(/repo, read:org/);
  await expect(verdict).toHaveClass(/ok/);
});

test('git accounts: failed test renders the provider error inline (red)', async ({ page }) => {
  await page.route('**/api/v1/git/accounts/*/test', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ ok: false, error: 'github 401: Bad credentials' }),
    }),
  );
  await openPage(page, 'settings/git-accounts');
  // beforeAll runs once per parallel worker, so several identical seeded
  // accounts may exist — any one row exercises the flow.
  const row = page.locator('.acct', { hasText: 'e2e test acct' }).first();
  await row.getByRole('button', { name: 'Test' }).click();
  const verdict = row.locator('.test-result');
  await expect(verdict).toHaveText(/github 401: Bad credentials/);
  await expect(verdict).toHaveClass(/bad/);
});

test('git accounts: add form has a Test connection button gated on a token', async ({ page }) => {
  await openPage(page, 'settings/git-accounts');
  await page.getByRole('button', { name: 'Add Account' }).first().click();
  const testBtn = page.getByRole('button', { name: 'Test connection' });
  await expect(testBtn).toBeVisible();
  // Add mode with no token typed yet → nothing to test.
  await expect(testBtn).toBeDisabled();
  await page.locator('#ga-token').fill('ghp_typed_token');
  await expect(testBtn).toBeEnabled();
});
