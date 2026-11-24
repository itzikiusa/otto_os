import { test, expect, devices, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { requireThreadMeta } from './daemonFeatures';

// Pins, History and "seen" live on the thread (docs/contracts/api.md
// PATCH /runtime-sessions/{id} `meta`), so a Mac and a phone on one daemon
// agree — live, with no reload. Real structured threads (fake codex).

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let root = '';
const ids: Record<string, string> = {};

test.describe.configure({ mode: 'serial' });

async function thread(key: string, title: string, prompt: string): Promise<void> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: root, title } });
  expect(r.ok(), await r.text()).toBeTruthy();
  ids[key] = (await r.json()).id as string;
  expect((await ctx.post(`${base}/api/v1/runtime-sessions/${ids[key]}/prompt`, { data: { text: prompt } })).ok()).toBeTruthy();
}

async function idle(id: string): Promise<void> {
  await expect
    .poll(async () => ((await (await ctx.get(`${base}/api/v1/runtime-sessions/${id}`)).json()) as { status: string; last_turn_status?: string }).last_turn_status ?? '', { timeout: 30_000 })
    .toBe('completed');
}

test.beforeAll(async () => {
  ({ ctx, base } = await apiCtx());
  await requireThreadMeta(ctx, base);
  root = mkdtempSync(join(tmpdir(), 'otto-e2e-sync-'));
  writeFileSync(join(root, 'README.md'), '# Sync\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Sync WS', root_path: root } });
  expect(r.ok(), await r.text()).toBeTruthy();
  wsId = (await r.json()).id as string;
  await thread('home', 'Sync home thread', 'hello');
  await thread('pin', 'Sync pin me', 'hello');
  await thread('hist', 'Sync move me', 'hello');
  await thread('done', 'Sync finish me', 'hello');
  for (const id of Object.values(ids)) await idle(id);
});

test.afterAll(async () => {
  for (const id of Object.values(ids)) await ctx.post(`${base}/api/v1/runtime-sessions/${id}/close`).catch(() => {});
  await ctx?.dispose();
  if (root) rmSync(root, { recursive: true, force: true });
});

async function open(page: Page): Promise<void> {
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('sync-init')) return;
    sessionStorage.setItem('sync-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_threads_v1');
  }, wsId);
  await page.goto(`/#/threads/${ids.home}`);
  await expect(page.getByTestId(`runtime-thread-${ids.home}`)).toBeVisible({ timeout: 30_000 });
}

test('a pin, History and "seen" made on one device show on the other at once', async ({ page, browser }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser drives both devices');
  await page.setViewportSize({ width: 1440, height: 900 });
  await open(page);
  const mac = page.getByTestId('threads-sidebar');

  const phoneCtx = await browser.newContext({ ...devices['iPhone 14'], storageState: info.project.use.storageState as string, baseURL: info.project.use.baseURL });
  const phone = await phoneCtx.newPage();
  await open(phone);
  await phone.getByTestId('threads-toggle-sidebar').tap();
  const sheet = phone.getByTestId('threads-sidebar');
  await expect(sheet).toBeVisible();

  // Mac pins → the phone's sheet shows it pinned, live.
  await mac.getByTestId(`thread-row-${ids.pin}`).hover();
  await page.getByTestId(`thread-pin-${ids.pin}`).click();
  await expect(mac.getByTestId(`thread-pinned-${ids.pin}`)).toBeVisible();
  await expect(sheet.getByTestId(`thread-pinned-${ids.pin}`)).toBeVisible({ timeout: 15_000 });

  // Undo goes through the same write: unpinned on both.
  await page.locator('.toast', { hasText: 'Pinned' }).locator('.toast-action').click();
  await expect(sheet.getByTestId(`thread-pinned-${ids.pin}`)).toHaveCount(0, { timeout: 15_000 });
  await mac.getByTestId(`thread-row-${ids.pin}`).hover();
  await page.getByTestId(`thread-pin-${ids.pin}`).click();
  await expect(sheet.getByTestId(`thread-pinned-${ids.pin}`)).toBeVisible({ timeout: 15_000 });

  // The phone moves one to History (long press › Move to History) → gone from the Mac's list.
  const main = sheet.getByTestId(`thread-row-${ids.hist}`).locator('.tr-main');
  const box = (await main.boundingBox())!;
  const at = { pointerType: 'touch', isPrimary: true, clientX: box.x + 40, clientY: box.y + 20 };
  await main.dispatchEvent('pointerdown', at);
  await expect(phone.locator('.ctx-menu')).toBeVisible();
  await main.dispatchEvent('pointerup', at);
  await phone.locator('.ctx-menu').getByRole('menuitem', { name: 'Move to History' }).tap();
  await expect(mac.getByTestId('threads-active-list').getByTestId(`thread-row-${ids.hist}`)).toHaveCount(0, { timeout: 15_000 });
  await mac.getByTestId('threads-history-toggle').click();
  await expect(mac.getByTestId(`thread-history-${ids.hist}`)).toBeVisible();

  // A turn ends while neither device has the thread in front: Done on both.
  // The phone opens it → the Mac's Done clears, live.
  await ctx.post(`${base}/api/v1/runtime-sessions/${ids.done}/prompt`, { data: { text: 'hello again' } });
  await expect(mac.getByTestId(`thread-status-${ids.done}`)).toHaveText('Done', { timeout: 30_000 });
  await expect(sheet.getByTestId(`thread-status-${ids.done}`)).toHaveText('Done', { timeout: 30_000 });
  await sheet.getByTestId(`thread-row-${ids.done}`).locator('.tr-main').tap();
  await expect(phone.getByTestId(`runtime-thread-${ids.done}`)).toBeVisible();
  await expect(mac.getByTestId(`thread-status-${ids.done}`)).not.toHaveText('Done', { timeout: 15_000 });

  // A reload (a new device's first look) reads it all from the threads.
  await page.reload();
  await expect(mac.getByTestId(`thread-pinned-${ids.pin}`)).toBeVisible({ timeout: 30_000 });
  await expect(mac.getByTestId('threads-active-list').getByTestId(`thread-row-${ids.hist}`)).toHaveCount(0);
  await expect(mac.getByTestId(`thread-status-${ids.done}`)).not.toHaveText('Done');
  const rows = (await (await ctx.get(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`)).json()) as { id: string; meta?: Record<string, unknown> }[];
  const meta = (k: string) => rows.find((r) => r.id === ids[k])?.meta ?? {};
  expect(meta('pin').thread_pinned).toBe(true);
  expect(meta('hist').thread_history).toBe(true);
  expect(typeof meta('done').thread_seen_at).toBe('string');
  await phoneCtx.close();
});

test('the old local pins move onto the threads once; a refused write rolls back', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser');
  await page.setViewportSize({ width: 1440, height: 900 });
  // This browser still has an old local list (before pins lived on threads).
  await page.addInitScript(([w, home]) => {
    if (sessionStorage.getItem('mig-init')) return;
    sessionStorage.setItem('mig-init', '1');
    localStorage.setItem('otto_workspace', w);
    localStorage.removeItem('otto_threads_meta_migrated');
    localStorage.setItem('otto_threads_v1', JSON.stringify({ pinnedIds: [home, 'gone-thread'], settledIds: [] }));
  }, [wsId, ids.home]);
  await page.goto(`/#/threads/${ids.pin}`);
  const mac = page.getByTestId('threads-sidebar');
  await expect(mac.getByTestId(`thread-pinned-${ids.home}`)).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(async () => ((await (await ctx.get(`${base}/api/v1/runtime-sessions/${ids.home}`)).json()) as { meta?: Record<string, unknown> }).meta?.thread_pinned, { timeout: 15_000 })
    .toBe(true);
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('otto_threads_v1') ?? '{}').pinnedIds ?? null)).toBeNull();

  // The daemon refuses: the pin rolls back with the shared error toast.
  await page.route('**/api/v1/runtime-sessions/*', (route) =>
    route.request().method() === 'PATCH' ? route.fulfill({ status: 403, json: { title: 'Forbidden', detail: 'not yours' } }) : route.fallback(),
  );
  await mac.getByTestId(`thread-pinned-${ids.home}`).hover();
  await mac.getByTestId(`thread-pinned-${ids.home}`).locator('.tr-main').click({ button: 'right' });
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Unpin thread' }).click();
  await expect(page.locator('.toast', { hasText: 'Couldn’t save that change' })).toBeVisible();
  await expect(mac.getByTestId(`thread-pinned-${ids.home}`)).toBeVisible();
});
