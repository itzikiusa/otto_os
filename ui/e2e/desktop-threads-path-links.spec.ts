import { test, expect, type APIRequestContext } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

// File references in a runtime thread's messages (Markdown.svelte file-refs)
// were dead: the thread sets no conversation context, so a click did nothing.
// They now resolve through lib/pathLinks against the thread's folder. The
// e2e codex stand-in (`otto-fake-codex`) answers `echo: <prompt>`, so the
// prompt itself puts the paths in the agent's reply.

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'otto-e2e-thread-links-'));
  mkdirSync(join(dir, 'docs', 'tour'), { recursive: true });
  writeFileSync(join(dir, 'docs', 'only-here.md'), '# Only here\n');
  writeFileSync(join(dir, 'docs', 'tour', 'scene-1.txt'), 'scene\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Thread Links WS', root_path: dir } });
  expect(r.ok(), await r.text()).toBeTruthy();
  wsId = (await r.json()).id as string;
});

test.afterAll(async () => {
  if (ctx && wsId) {
    const rows = (await (await ctx.get(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`)).json()) as { id: string }[];
    for (const s of rows) await ctx.post(`${base}/api/v1/runtime-sessions/${s.id}/close`).catch(() => {});
  }
  await ctx?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('thread-links-init')) return;
    sessionStorage.setItem('thread-links-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_threads_v1');
  }, wsId);
});

test('file, folder and missing references in a runtime thread resolve against its folder', async ({ page }) => {
  await page.goto('/#/threads');
  const stage = page.getByTestId('new-thread-stage');
  await expect(stage).toBeVisible({ timeout: 30_000 });
  await stage.getByTestId('picker-agent').click();
  await page.getByTestId('model-picker').getByTestId('model-picker-provider-codex').click();
  await page.keyboard.press('Escape');
  await stage.getByTestId('new-thread-input').fill('see `docs/only-here.md`, `docs/tour/` and `gone/nowhere.md`');
  await stage.getByTestId('new-thread-send-btn').click();

  const thread = page.locator('[data-testid^="runtime-thread-"]');
  await expect(thread).toBeVisible({ timeout: 30_000 });
  await expect(thread.getByText(/^echo: see/).first()).toBeVisible({ timeout: 30_000 });
  // The agent's reply (the echo) is the last message carrying the links.
  const link = (name: string) => thread.locator('a.file-ref', { hasText: name }).last();

  // A file opens as a tab in the thread's Files inspector.
  await link('docs/only-here.md').click();
  await expect(page.getByText('# Only here').or(page.getByRole('heading', { name: 'Only here' })).first()).toBeVisible({ timeout: 15_000 });

  // A folder lists in the Files tab's tree (no file tab for it).
  await link('docs/tour/').click();
  await expect(page.getByText('scene-1.txt')).toBeVisible({ timeout: 15_000 });

  // A path that exists nowhere: a "not found" toast, nothing opens.
  await link('gone/nowhere.md').click();
  const toast = page.locator('.toast').filter({ hasText: 'Not found: gone/nowhere.md' });
  await expect(toast).toBeVisible();
  await expect(toast.getByRole('button', { name: 'Copy path' })).toBeVisible();
});
