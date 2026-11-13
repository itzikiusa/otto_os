import { test, expect, type APIRequestContext } from '@playwright/test';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, openNewSessionSheet } from './helpers';

// ⌘T on Threads starts a thread (a page claim, like the API page's new
// request) instead of the terminal "New session" sheet, and that sheet's one
// quiet line ("Start a thread") goes to the Threads composer (`#/threads/new`).

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';
let thread = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'otto-e2e-new-kind-'));
  const w = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'New Session Kind WS', root_path: dir } });
  expect(w.ok(), await w.text()).toBeTruthy();
  wsId = (await w.json()).id as string;
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: 'Kind Seed Thread', cwd: dir, meta: { origin: 'e2e', surface: 'threads', nested_provider: 'claude' } },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  thread = (await r.json()).id as string;
});

test.afterAll(async () => {
  if (thread && ctx) await ctx.delete(`${base}/api/v1/sessions/${thread}`).catch(() => {});
  await ctx?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('new-kind-init')) return;
    sessionStorage.setItem('new-kind-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_threads_v1');
  }, wsId);
});

const sheet = (page: import('@playwright/test').Page) => page.locator('.sheet[role="dialog"][aria-label="New session"]');

test('⌘T on Threads opens the new-thread stage, not the New session sheet', async ({ page }) => {
  await page.goto(`/#/threads/${thread}`);
  await expect(page.getByTestId('threads-breadcrumb')).toContainText('Kind Seed Thread', { timeout: 30_000 });
  const stage = page.getByTestId('new-thread-stage');
  await expect(stage).toHaveCount(0);

  await page.keyboard.press('Meta+t');
  await expect(stage).toBeVisible({ timeout: 10_000 });
  await expect(page.getByTestId('threads-breadcrumb')).toContainText('New thread');
  await expect(sheet(page)).toHaveCount(0);
  expect(await page.evaluate(() => location.hash)).toMatch(/^#\/threads/);
});

test('the New session sheet says what a terminal is and "Start a thread" goes to the Threads stage', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(page.locator('.navigator')).toBeVisible({ timeout: 30_000 });
  const dialog = await openNewSessionSheet(page);
  const hint = dialog.getByTestId('session-kind-hint');
  await expect(hint).toContainText('terminal');
  await expectFullyInViewport(page, hint);

  await hint.getByRole('button', { name: 'Start a thread' }).click();
  await expect(sheet(page)).toHaveCount(0);
  await expect(page.getByTestId('new-thread-stage')).toBeVisible({ timeout: 15_000 });
  // `threads/new` is a one-shot route: it is replaced by `threads`.
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/threads');
});
