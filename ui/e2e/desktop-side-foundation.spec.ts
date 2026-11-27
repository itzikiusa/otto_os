import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

// The side-surface foundation on desktop (design.md §3, §7.3): the Threads
// list pane header (name · count · New · ⋯), "Hide Threads ⌘B" under ⋯ and
// "Show Threads" in the page header while hidden, ⌘B, the thread details as a
// Detail pane (⌥⌘B), the Side panel rail on Threads too, and the arrow-key
// walk in the Navigator. Screenshots (OTTO_E2E_SHOTS) → side-foundation/.

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';
const ids: string[] = [];

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'otto-e2e-side-d-'));
  writeFileSync(join(dir, 'README.md'), '# side foundation\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Side Foundation Desktop', root_path: dir } });
  expect(r.ok(), await r.text()).toBeTruthy();
  wsId = (await r.json()).id as string;
  for (const title of ['Fix flaky login test', 'Refund webhook retries']) {
    const t = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: dir, title } });
    expect(t.ok(), await t.text()).toBeTruthy();
    ids.push((await t.json()).id as string);
  }
});

test.afterAll(async () => {
  for (const id of ids) await ctx?.post(`${base}/api/v1/runtime-sessions/${id}/close`).catch(() => {});
  await ctx?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
  await page.setViewportSize({ width: 1440, height: 900 });
});

async function shot(page: Page, name: string): Promise<void> {
  const out = process.env.OTTO_E2E_SHOTS;
  if (!out) return;
  mkdirSync(join(out, 'side-foundation'), { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    await page.screenshot({ path: join(out, 'side-foundation', `${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

test('Threads list pane: header, Hide under ⋯, Show in the page header, ⌘B; details as a Detail pane', async ({ page }) => {
  test.setTimeout(90_000);
  await page.goto(`/#/threads/${ids[0]}`);
  const list = page.getByTestId('threads-sidebar');
  await expect(list).toBeVisible({ timeout: 30_000 });
  await expect(list.locator('.sph-title')).toHaveText('Threads');
  await expect(list.getByTestId('threads-new-btn')).toBeVisible();
  await expect(page.locator('.rstrip'), 'the Side panel rail is on Threads too').toBeVisible();
  await shot(page, 'desktop-threads');

  await list.getByRole('button', { name: 'Threads actions' }).click();
  await page.getByRole('menuitem', { name: /Hide Threads/ }).click();
  await expect(list).toBeHidden();
  const show = page.getByTestId('show-list-pane');
  await expect(show).toHaveAttribute('aria-label', 'Show Threads');
  await show.click();
  await expect(list).toBeVisible();
  await page.locator('body').click({ position: { x: 700, y: 400 } });
  await page.keyboard.press('ControlOrMeta+b');
  await expect(list).toBeHidden();
  await page.keyboard.press('ControlOrMeta+b');
  await expect(list).toBeVisible();

  // ↓ walks the rows.
  const first = page.getByTestId(`thread-row-${ids[1]}`).locator('.tr-main');
  await first.focus();
  await page.keyboard.press('Home');
  const focusedTestid = () => page.evaluate(() => document.activeElement?.closest('[data-testid^="thread-"]')?.getAttribute('data-testid') ?? null);
  const top = await focusedTestid();
  await page.keyboard.press('End');
  expect(await focusedTestid()).not.toBe(top);

  // The thread details: a Detail pane (info glyph), inline at 1440.
  await page.getByTestId('threads-toggle-inspector').click();
  const insp = page.locator('aside[data-testid="threads-inspector"]');
  await expect(insp).toBeVisible({ timeout: 15_000 });
  await expect(insp.getByRole('button', { name: 'Close Thread details' })).toBeVisible();
  await expect(insp.getByTestId('threads-surface-add')).toBeVisible();
  await shot(page, 'desktop-threads-details');
  await insp.getByRole('button', { name: 'Close Thread details' }).click();
  await expect(insp).toHaveCount(0);
});

test('Navigator: ↑ / ↓ walk the rows, → / ← fold a section', async ({ page }) => {
  await page.goto('/#/home');
  const nav = page.locator('.navigator');
  await expect(nav).toBeVisible({ timeout: 20_000 });
  await shot(page, 'desktop-shell');
  const rows = nav.locator('.nav-scroll .nav-item');
  await rows.first().focus();
  const label = () => page.evaluate(() => (document.activeElement as HTMLElement | null)?.innerText?.trim() ?? '');
  const a = await label();
  await page.keyboard.press('ArrowDown');
  const b = await label();
  expect(b).not.toBe(a);
  await page.keyboard.press('ArrowUp');
  expect(await label()).toBe(a);
  const head = nav.getByTestId('sidebar-group-head-build');
  if (await head.count()) {
    await head.focus();
    await page.keyboard.press('ArrowLeft');
    await expect(head).toHaveAttribute('aria-expanded', 'false');
    await page.keyboard.press('ArrowRight');
    await expect(head).toHaveAttribute('aria-expanded', 'true');
  }
});
