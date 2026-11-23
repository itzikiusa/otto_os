import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// A new thread in its own worktree, in one step (NewThreadStage +
// worktreeBranch.ts): choosing "New worktree" only records the intent (no
// modal); the first message names the branch (the repo's "feat/" prefix +
// a slug of the prompt) and makes the worktree, then the thread starts there
// — Claude and Codex alike. "Name the branch…" overrides the name; a failure
// keeps the draft and shows the error inline with Retry; an empty start gets
// a dated name. Real daemon, real git.

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';
const git = (...a: string[]): string => execFileSync('git', ['-c', 'user.email=e2e@otto', '-c', 'user.name=e2e', ...a], { cwd: dir }).toString();

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'wt-repo-'));
  writeFileSync(join(dir, 'README.md'), '# wt\n');
  git('init', '-q', '-b', 'main');
  git('add', '.');
  git('commit', '-qm', 'init');
  git('branch', 'feat/one');
  git('branch', 'feat/two');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Worktree WS', root_path: dir } });
  expect(r.ok(), await r.text()).toBeTruthy();
  wsId = (await r.json()).id as string;
});

test.afterAll(async () => {
  if (ctx && wsId) {
    const rows = (await (await ctx.get(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`)).json()) as { id: string }[];
    for (const s of rows) await ctx.post(`${base}/api/v1/runtime-sessions/${s.id}/close`).catch(() => {});
  }
  await ctx?.dispose();
  if (dir) {
    for (const line of git('worktree', 'list', '--porcelain').split('\n')) {
      const p = line.startsWith('worktree ') ? line.slice(9) : '';
      if (p && p !== dir && !p.endsWith(dir)) rmSync(p, { recursive: true, force: true });
    }
    rmSync(dir, { recursive: true, force: true });
  }
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
});

const worktrees = (): string => git('worktree', 'list');

async function pickProvider(page: Page, p: 'claude' | 'codex'): Promise<void> {
  const stage = page.getByTestId('new-thread-stage');
  await stage.getByTestId('picker-agent').click();
  await page.getByTestId('model-picker').getByTestId(`model-picker-provider-${p}`).click();
  await page.keyboard.press('Escape');
}

async function chooseNewWorktree(page: Page): Promise<void> {
  const stage = page.getByTestId('new-thread-stage');
  await expect(stage.getByTestId('new-thread-checkout')).toContainText('Current checkout', { timeout: 20_000 });
  await stage.getByTestId('new-thread-checkout').click();
  const menu = page.locator('.ctx-menu');
  await expectFullyInViewport(page, menu, 'checkout menu');
  await menu.getByRole('menuitemcheckbox', { name: /New worktree/ }).click();
  // Only the intent: no dialog, the chip says it.
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(stage.getByTestId('new-thread-checkout')).toContainText('New worktree');
  await expect(stage.getByTestId('new-thread-checkout')).toContainText('from main');
}

test('the first message names the branch and makes the worktree; Claude and Codex', async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  for (const [provider, prompt, branch] of [
    ['claude', 'Fix the flaky checkout test', 'feat/fix-flaky-checkout-test'],
    ['codex', 'Add a rate limit to search', 'feat/add-rate-limit-search'],
  ] as const) {
    await page.goto('/#/threads/new');
    const stage = page.getByTestId('new-thread-stage');
    await expect(stage).toBeVisible({ timeout: 30_000 });
    await pickProvider(page, provider);
    await chooseNewWorktree(page);
    if (provider === 'claude') {
      for (const scheme of ['light', 'dark'] as const) {
        await page.emulateMedia({ colorScheme: scheme });
        await page.screenshot({ path: process.env.THRC_SHOTS ? `${process.env.THRC_SHOTS}/worktree-stage-${scheme}.png` : test.info().outputPath(`wt-${scheme}.png`), animations: 'disabled' });
      }
      await page.emulateMedia({ colorScheme: 'light' });
    }
    await stage.getByTestId('new-thread-input').fill(prompt);
    await stage.getByTestId('new-thread-send-btn').click();
    const thread = page.locator('[data-testid^="runtime-thread-"]').first();
    await expect(thread).toBeVisible({ timeout: 30_000 });
    await expect.poll(worktrees, { timeout: 15_000 }).toContain(`[${branch}]`);
    const id = (await thread.getAttribute('data-testid'))!.replace('runtime-thread-', '');
    const row = (await (await ctx.get(`${base}/api/v1/runtime-sessions/${id}`)).json()) as { cwd: string };
    const wtPath = worktrees().split('\n').find((l) => l.includes(`[${branch}]`))!.split(/\s+/)[0];
    expect(row.cwd.replace(/^\/private/, '')).toBe(wtPath.replace(/^\/private/, ''));
    await expect(thread).toContainText(prompt);
  }
});

test('a failed worktree keeps the draft with Retry; Name the branch… fixes it; no prompt gets a dated name', async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/threads/new');
  const stage = page.getByTestId('new-thread-stage');
  await expect(stage).toBeVisible({ timeout: 30_000 });
  await pickProvider(page, 'codex');
  await chooseNewWorktree(page);
  const input = stage.getByTestId('new-thread-input');
  await input.fill('Ship the share link');
  // Name it after a branch that exists: creating it fails.
  await stage.getByTestId('new-thread-checkout').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Name the branch…' }).click();
  const dlg = page.getByRole('dialog');
  await expect(dlg.getByRole('textbox')).toHaveValue('feat/ship-share-link');
  await dlg.getByRole('textbox').fill('feat/one');
  await dlg.getByRole('button', { name: 'Use this name' }).click();
  await expect(stage.getByTestId('new-thread-checkout')).toContainText('feat/one');
  await stage.getByTestId('new-thread-send-btn').click();
  const failed = stage.getByTestId('new-thread-worktree-failed');
  await expect(failed).toContainText('Couldn’t create the worktree');
  await expect(input).toHaveValue('Ship the share link');
  await expect(page.locator('[data-testid^="runtime-thread-"]')).toHaveCount(0);
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: process.env.THRC_SHOTS ? `${process.env.THRC_SHOTS}/worktree-failed-${scheme}.png` : test.info().outputPath(`wt-failed-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
  // Rename, then Retry: it starts there.
  await stage.getByTestId('new-thread-checkout').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Name the branch…' }).click();
  await dlg.getByRole('textbox').fill('feat/share-link');
  await dlg.getByRole('button', { name: 'Use this name' }).click();
  await failed.getByRole('button', { name: 'Retry' }).click();
  await expect(page.locator('[data-testid^="runtime-thread-"]').first()).toBeVisible({ timeout: 30_000 });
  await expect.poll(worktrees, { timeout: 15_000 }).toContain('[feat/share-link]');

  // No prompt (⌘↵ on an empty box): a dated branch.
  await page.goto('/#/threads/new');
  await expect(stage).toBeVisible({ timeout: 30_000 });
  await pickProvider(page, 'codex');
  await chooseNewWorktree(page);
  await input.click();
  await input.press('Meta+Enter');
  await expect.poll(worktrees, { timeout: 15_000 }).toMatch(/\[feat\/thread-\d{4}-\d{2}-\d{2}-\d{4}\]/);
  await expectNoHorizontalOverflow(page);
});
