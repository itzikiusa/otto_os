import { test, expect, type APIRequestContext } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// A new thread in its own worktree on a phone, by touch (NewThreadStage):
// the checkout chip's menu records "New worktree" (no dialog), the send
// makes the worktree from the prompt and starts the thread there.

const PROJECTS = ['iphone-portrait', 'ipad-portrait'];

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';
const git = (...a: string[]): string => execFileSync('git', ['-c', 'user.email=e2e@otto', '-c', 'user.name=e2e', ...a], { cwd: dir }).toString();

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'wt-phone-'));
  writeFileSync(join(dir, 'README.md'), '# wt\n');
  git('init', '-q', '-b', 'main');
  git('add', '.');
  git('commit', '-qm', 'init');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Worktree phone', root_path: dir } });
  wsId = (await r.json()).id as string;
});

test.afterAll(async () => {
  if (dir) {
    for (const line of git('worktree', 'list', '--porcelain').split('\n')) {
      const p = line.startsWith('worktree ') ? line.slice(9) : '';
      if (p && !p.endsWith(dir)) rmSync(p, { recursive: true, force: true });
    }
    rmSync(dir, { recursive: true, force: true });
  }
  await ctx?.dispose();
});

test('phone: New worktree by touch, made on send', async ({ page }, info) => {
  test.skip(!PROJECTS.includes(info.project.name), 'phone and tablet projects');
  test.setTimeout(90_000);
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
  await page.goto('/#/threads/new');
  const stage = page.getByTestId('new-thread-stage');
  await expect(stage).toBeVisible({ timeout: 30_000 });
  const chip = stage.getByTestId('new-thread-checkout');
  await expect(chip).toContainText('Current checkout', { timeout: 20_000 });
  expect((await chip.boundingBox())!.height, 'touch target').toBeGreaterThanOrEqual(43.5);
  await chip.tap();
  const menu = page.locator('.ctx-menu');
  await expectFullyInViewport(page, menu, 'checkout menu (phone)');
  await menu.getByRole('menuitemcheckbox', { name: /New worktree/ }).tap();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(chip).toContainText('New worktree');
  await expectNoHorizontalOverflow(page);
  const out = process.env.THRC_SHOTS;
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: out ? `${out}/worktree-${info.project.name}-${scheme}.png` : info.outputPath(`wt-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
  const prompt = `Tidy the readme ${info.project.name.split('-')[0]}`;
  await stage.getByTestId('new-thread-input').tap();
  await stage.getByTestId('new-thread-input').fill(prompt);
  await stage.getByTestId('new-thread-send-btn').tap();
  await expect(page.locator('[data-testid^="runtime-thread-"]').first()).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => git('worktree', 'list'), { timeout: 15_000 }).toContain(`[tidy-readme-${info.project.name.split('-')[0]}]`);
});
