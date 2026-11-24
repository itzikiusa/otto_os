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

test('the New session stage: Chat with no message opens an empty chat pane here (A2, A4)', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(page.getByTestId('agents-inbox')).toBeVisible({ timeout: 30_000 });
  const dialog = await openNewSessionSheet(page);
  const kind = dialog.getByTestId('ns-kind');
  await expectFullyInViewport(page, kind);
  await kind.getByRole('radio', { name: 'Chat' }).click();
  await dialog.locator('.provider-card', { hasText: 'codex' }).locator('.card-main').click();
  await dialog.getByRole('button', { name: 'Start chat' }).click();
  await expect(sheet(page)).toHaveCount(0);
  await expect(page.locator('[data-testid^="thread-pane-"]')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('[data-testid^="thread-pane-"] [data-testid="runtime-composer"]')).toBeVisible({ timeout: 20_000 });
  await expect.poll(() => page.evaluate(() => location.hash)).toMatch(/^#\/agents\//);
});

// Chat starts ONE thread: the batch steppers and the "Starting N sessions"
// line go with Terminal, and the count resets to the one provider.
test('Chat hides the batch steppers and starts one thread', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(page.getByTestId('agents-inbox')).toBeVisible({ timeout: 30_000 });
  const dialog = await openNewSessionSheet(page);
  const codex = dialog.locator('.provider-card', { hasText: 'codex' });
  await codex.locator('.card-main').click();
  await codex.getByRole('button', { name: 'One more codex session' }).click();
  await expect(dialog.locator('.hint.batch')).toContainText('Starting 2 sessions');
  await dialog.getByTestId('ns-kind').getByRole('radio', { name: 'Chat' }).click();
  await expect(dialog.locator('.count-ctl:visible')).toHaveCount(0);
  await expect(dialog.locator('.hint.batch')).toHaveCount(0);
  await dialog.getByTestId('ns-kind').getByRole('radio', { name: 'Terminal' }).click();
  await expect(dialog.locator('.hint.batch')).toHaveCount(0);
  await page.keyboard.press('Escape');
});

// Enter on a − / + steps the count; only ⌘↩ (or Enter on a tile) starts.
test('Enter on a provider\'s + steps the count instead of starting', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(page.getByTestId('agents-inbox')).toBeVisible({ timeout: 30_000 });
  const dialog = await openNewSessionSheet(page);
  const shell = dialog.locator('.provider-card', { hasText: 'shell' });
  await shell.locator('.card-main').click();
  await shell.getByRole('button', { name: 'One more shell session' }).focus();
  await page.keyboard.press('Enter');
  await expect(dialog.locator('.hint.batch')).toContainText('Starting 2 sessions');
  await expect(sheet(page)).toBeVisible();
  await page.keyboard.press('Escape');
});

// A chat that started never asks to discard what it was given (the extra
// folders are a terminal option), and its chips say what Chat still uses.
test('a started chat closes without a discard prompt; Chat shows the chips it uses', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(page.getByTestId('agents-inbox')).toBeVisible({ timeout: 30_000 });
  const dialog = await openNewSessionSheet(page);
  await dialog.getByTestId('ns-options').click();
  await dialog.locator('#ns-title').fill('Kaka');
  await dialog.locator('#ns-extra-dir').fill(dir);
  await dialog.locator('#ns-extra-dir').press('Enter');
  await dialog.getByTestId('ns-options').click();
  await dialog.getByTestId('ns-kind').getByRole('radio', { name: 'Chat' }).click();
  await expect(dialog.locator('.opt-chip')).toHaveText(['Title: Kaka']);
  await dialog.locator('.provider-card', { hasText: 'codex' }).locator('.card-main').click();
  await dialog.getByRole('button', { name: 'Start chat' }).click();
  await expect(sheet(page)).toHaveCount(0, { timeout: 20_000 });
  await expect(page.locator('[data-testid^="thread-pane-"]')).toBeVisible({ timeout: 20_000 });
});
