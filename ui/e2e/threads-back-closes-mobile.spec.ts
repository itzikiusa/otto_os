import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// A phone's Back closes the top-most Threads sheet / drawer / menu before it
// leaves the thread (lib/backCloses.ts): each open overlay holds one history
// entry with the same URL. A normal close (✕, a pick) takes its entry back,
// so the thread's own Back target stays one step away.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

const PHONE = /^(iphone-portrait|android-phone)$/;

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let fixtureDir = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  fixtureDir = mkdtempSync(join(tmpdir(), 'otto-e2e-threads-back-'));
  writeFileSync(join(fixtureDir, 'README.md'), '# Back on a phone\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Back Closes WS', root_path: fixtureDir } });
  expect(r.ok(), await r.text()).toBeTruthy();
  wsId = (await r.json()).id as string;
});

test.afterAll(async () => {
  if (ctx && wsId) {
    const rows = (await (await ctx.get(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`)).json()) as { id: string }[];
    for (const s of rows) await ctx.post(`${base}/api/v1/runtime-sessions/${s.id}/close`).catch(() => {});
  }
  await ctx?.dispose();
  if (fixtureDir) rmSync(fixtureDir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
});

async function newThread(title: string): Promise<string> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: fixtureDir, title } });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

const dialog = (page: Page, name: string) => page.getByRole('dialog', { name });

test('Back closes the top-most sheet, drawer and menu, never the thread', async ({ page }, info) => {
  test.skip(!PHONE.test(info.project.name), 'phones');
  test.setTimeout(120_000);
  const a = await newThread('Back target A');
  const b = await newThread('Back target B');
  await page.goto(`/#/threads/${a}`);
  await expect(page.getByTestId(`runtime-thread-${a}`)).toBeVisible({ timeout: 30_000 });
  await page.goto(`/#/threads/${b}`);
  const thread = page.getByTestId(`runtime-thread-${b}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });

  // The inbox sheet.
  await page.getByTestId('threads-toggle-sidebar').click();
  await expect(dialog(page, 'Threads')).toBeVisible();
  await page.goBack();
  await expect(dialog(page, 'Threads')).toHaveCount(0);
  await expect(page).toHaveURL(new RegExp(`#/threads/${b}$`));

  // The details drawer.
  await page.getByTestId('threads-toggle-inspector').click();
  await expect(dialog(page, 'Thread details')).toBeVisible();
  await page.goBack();
  await expect(dialog(page, 'Thread details')).toHaveCount(0);

  // The ⋯ menu, then a sheet one of its items opens (the menu's entry is stepped over).
  await page.getByTestId('threads-thread-menu').click();
  await expect(page.locator('.ctx-menu')).toBeVisible();
  await page.goBack();
  await expect(page.locator('.ctx-menu')).toHaveCount(0);
  await page.getByTestId('threads-thread-menu').click();
  await page.getByRole('menuitem', { name: /Rename/ }).click();
  await expect(dialog(page, 'Rename thread')).toBeVisible();
  await page.goBack();
  await expect(dialog(page, 'Rename thread')).toHaveCount(0);
  await expect(page).toHaveURL(new RegExp(`#/threads/${b}$`));

  // A normal close leaves no entry behind: the next Back is the previous thread.
  await page.getByTestId('threads-toggle-sidebar').click();
  await expect(dialog(page, 'Threads')).toBeVisible();
  await dialog(page, 'Threads').getByRole('button', { name: 'Close' }).first().click();
  await expect(dialog(page, 'Threads')).toHaveCount(0);
  // The close steps its entry off one task later (a pick's navigation goes first).
  await expect.poll(() => page.evaluate(() => (history.state as { ottoSheet?: number } | null)?.ottoSheet ?? null)).toBeNull();
  await page.goBack();
  await expect(page).toHaveURL(new RegExp(`#/threads/${a}$`));
  await expect(page.getByTestId(`runtime-thread-${a}`)).toBeVisible();
});
