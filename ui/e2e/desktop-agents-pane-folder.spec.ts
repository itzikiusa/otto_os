import { test, expect, type APIRequestContext } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { seedCardRepo, stubForge, type CardRepo } from './git-commit-card-fixture';

// M6: the side panel's Git and Files tabs follow the FOCUSED pane — a chat
// thread working in its own folder (a worktree) as much as a terminal — never
// the workspace folder behind it.

test.use({ serviceWorkers: 'block' });
let ctx: APIRequestContext;
let base = '';
let repo: CardRepo;
let root = '';
let wsId = '';
let sid = '';
let threadId = '';

test.beforeAll(async () => {
  repo = await seedCardRepo('e2e-agents-pane-folder');
  root = mkdtempSync(join(tmpdir(), 'otto-e2e-pane-folder-'));
  writeFileSync(join(root, 'only-in-workspace.txt'), 'x\n');
  ({ ctx, base } = await apiCtx());
  const w = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Pane Folder WS', root_path: root } });
  expect(w.ok(), await w.text()).toBeTruthy();
  wsId = (await w.json()).id as string;
  const s = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, { data: { kind: 'agent', provider: 'shell', title: 'Ancelotti', cwd: root, meta: { origin: 'e2e' } } });
  expect(s.ok(), await s.text()).toBeTruthy();
  sid = (await s.json()).id as string;
  const t = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: repo.dir, title: 'Sacchi chat' } });
  expect(t.ok(), await t.text()).toBeTruthy();
  threadId = (await t.json()).id as string;
});

test.afterAll(async () => {
  if (threadId) await ctx.post(`${base}/api/v1/runtime-sessions/${threadId}/close`).catch(() => {});
  await ctx?.dispose();
  if (root) rmSync(root, { recursive: true, force: true });
});

test('Git and Files follow the focused chat pane into its own folder, and back to the terminal’s', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.setViewportSize({ width: 1440, height: 900 });
  await stubForge(page);
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
  }, wsId);
  await page.goto(`/#/agents/${sid}`);
  await expect(page.locator(`[data-pane-key][data-session="${sid}"]`)).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId(`agents-row-${threadId}`)).toBeVisible({ timeout: 20_000 });
  await page.getByTestId(`agents-row-${threadId}`).click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Open beside' }).click();
  const pane = page.getByTestId(`thread-pane-${threadId}`);
  await expect(pane).toBeVisible({ timeout: 20_000 });
  await pane.locator('.tpane-title').click();

  const rail = page.locator('.rstrip');
  await rail.getByRole('button', { name: 'Git', exact: true }).click();
  // The thread's checkout (dirty), not the workspace folder (no repository).
  await expect(page.getByTestId('commit-card')).toBeVisible({ timeout: 20_000 });
  await rail.getByRole('button', { name: 'Files', exact: true }).click();
  const body = page.locator('.rpanel-body');
  await expect(body.getByText('app.txt', { exact: true })).toBeVisible({ timeout: 15_000 });
  await expect(body.getByText('only-in-workspace.txt', { exact: true })).toHaveCount(0);

  // Focus the terminal: its folder again.
  await page.locator(`[data-pane-key][data-session="${sid}"] .xterm`).click();
  await expect(body.getByText('only-in-workspace.txt', { exact: true })).toBeVisible({ timeout: 15_000 });
});
