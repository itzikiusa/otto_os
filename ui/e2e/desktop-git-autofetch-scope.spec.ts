import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';

test.use({ serviceWorkers: 'block' });
let workspaceId = '';
let repoId = '';
let repos: unknown[] = [];
const status = { branch: 'main', upstream: null, ahead: 0, behind: 0, changes: [] };

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const dir = mkdtempSync(join(tmpdir(), 'otto-git-fetch-scope-'));
  const git = (...args: string[]) => execFileSync('git', ['-C', dir, ...args], { stdio: 'ignore' });
  git('init', '-q', '-b', 'main');
  git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'fixture@otto.local');
  git('config', 'commit.gpgsign', 'false');
  writeFileSync(join(dir, 'README.md'), 'Isolated auto-fetch fixture\n');
  git('add', '.'); git('commit', '-qm', 'Fixture');
  const response = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/repos`, { data: { path: dir, name: 'Fetch scope fixture' } });
  expect(response.ok()).toBeTruthy(); repoId = (await response.json()).id;
  repos = await (await ctx.get(`${base}/api/v1/git/repos`)).json();
  await ctx.dispose();
});

async function boot(page: Page) {
  await page.addInitScript(({ workspaceId, repoId }) => {
    localStorage.setItem('otto_workspace', workspaceId);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto_git_auto_fetch', JSON.stringify({ enabled: true, intervalSec: 120 }));
    localStorage.setItem('otto_git_open_tabs', JSON.stringify({ openRepoIds: [repoId], activeRepoId: repoId, sub: { [repoId]: 'graph' } }));
  }, { workspaceId, repoId });
  await page.bringToFront();
}

async function navigate(page: Page, hash: string) {
  await page.evaluate(value => { window.location.hash = value; }, hash);
}

test('automatic fetch belongs to Git: inactive boot, leave, and return', async ({ page }, info) => {
  await boot(page);
  let fetches = 0;
  await page.route('**/api/v1/repos/*/fetch', async route => {
    fetches++; await route.fulfill({ json: status });
  });
  await page.goto('/#/agents');
  await expect(page.locator('.shell')).toBeVisible();
  await page.clock.install();
  await page.clock.fastForward(35_000);
  await page.waitForTimeout(300);
  expect(fetches, 'saved Git tabs must not fetch while Agents is active').toBe(0);

  await navigate(page, `#/git/${repoId}/graph`);
  await expect(page.locator('.gitpage')).toBeVisible();
  await expect.poll(() => fetches).toBe(1);
  await navigate(page, '#/agents');
  await expect(page.locator('.gitpage')).not.toBeVisible();
  await page.clock.fastForward(35_000);
  await page.waitForTimeout(300);
  expect(fetches, 'leaving Git must stop the next due fetch').toBe(1);

  await navigate(page, `#/git/${repoId}/graph`);
  await expect(page.locator('.gitpage')).toBeVisible();
  await expect.poll(() => fetches).toBe(2);
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await expect(page.locator('html')).toHaveAttribute('data-scheme', scheme);
    const path = info.outputPath(`git-autofetch-${scheme}.png`);
    await page.screenshot({ path, fullPage: true, animations: 'disabled' });
    await info.attach(`git-autofetch-${scheme}`, { path, contentType: 'image/png' });
  }
});

test('a delayed Git tab bootstrap cannot start automatic fetch after leaving', async ({ page }) => {
  await boot(page);
  let release!: () => void;
  const blocked = new Promise<void>(resolve => { release = resolve; });
  let requested = false;
  let fetches = 0;
  await page.route('**/api/v1/git/repos', async route => {
    requested = true; await blocked; await route.fulfill({ json: repos });
  });
  await page.route('**/api/v1/repos/*/fetch', async route => {
    fetches++; await route.fulfill({ json: status });
  });
  try {
    await page.goto(`/#/git/${repoId}/graph`);
    await expect.poll(() => requested).toBe(true);
    await navigate(page, '#/agents');
    await expect(page.locator('.gitpage')).not.toBeVisible();
    release();
    await page.waitForTimeout(1_000); // ui-guards: allow — absence: no fetch after leaving Git
    expect(fetches).toBe(0);
  } finally { release(); }
});
