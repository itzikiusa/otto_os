import { test, expect, type APIRequestContext } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { requireThreadMeta } from './daemonFeatures';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// The Threads inbox on a phone / tablet, with touch (tap, long press):
// the sheet lists rows that say their state in a word, each row is ONE tap
// target that opens the thread and closes the sheet, the Working fold
// toggles with a tap, and a long press reaches Pin — with its Undo notice.
// Real runtime threads through the isolated daemon's fake codex.

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let root = '';
const ids: Record<string, string> = {};

test.describe.configure({ mode: 'serial' });

async function thread(key: string, title: string, prompt: string): Promise<void> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: root, title, access_mode: 'approval-required' } });
  expect(r.ok(), await r.text()).toBeTruthy();
  ids[key] = (await r.json()).id as string;
  expect((await ctx.post(`${base}/api/v1/runtime-sessions/${ids[key]}/prompt`, { data: { text: prompt } })).ok()).toBeTruthy();
}

test.beforeAll(async () => {
  ({ ctx, base } = await apiCtx());
  await requireThreadMeta(ctx, base);
  root = mkdtempSync(join(tmpdir(), 'otto-e2e-inbox-phone-'));
  writeFileSync(join(root, 'README.md'), '# Inbox on a phone\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Inbox phone', root_path: root } });
  expect(r.ok(), await r.text()).toBeTruthy();
  wsId = (await r.json()).id as string;
  await thread('idle', 'Phone inbox idle', 'hello');
  await thread('approval', 'Phone inbox approval', 'ask');
  await thread('working', 'Phone inbox working', 'hang');
});

test.afterAll(async () => {
  for (const id of Object.values(ids)) await ctx.post(`${base}/api/v1/runtime-sessions/${id}/close`).catch(() => {});
  await ctx?.dispose();
  if (root) rmSync(root, { recursive: true, force: true });
});

test('the inbox sheet on touch: words, one tap per row, Working fold, long press › Pin with Undo', async ({ page }, info) => {
  test.skip(!info.project.use.hasTouch, 'touch devices only');
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('inbox-phone')) return;
    sessionStorage.setItem('inbox-phone', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_threads_v1');
  }, wsId);
  await page.goto(`/#/threads/${ids.idle}`);
  await expect(page.getByTestId(`runtime-thread-${ids.idle}`)).toBeVisible({ timeout: 30_000 });
  const side = page.getByTestId('threads-sidebar');
  const drawer = !(await side.isVisible());
  if (drawer) await page.getByTestId('threads-toggle-sidebar').tap();
  await expectFullyInViewport(page, side, 'threads sidebar');
  await expectNoHorizontalOverflow(page);

  const row = (k: string) => side.getByTestId(`thread-row-${ids[k]}`);
  await expect(side.getByTestId(`thread-status-${ids.approval}`)).toHaveText('Approval', { timeout: 30_000 });
  for (const k of ['idle', 'approval']) expect((await row(k).locator('.tr-main').boundingBox())!.height).toBeGreaterThanOrEqual(44);
  await expect(row('approval').locator('.tr-actions')).toBeHidden();

  // Working: a finger-sized fold that toggles with a tap.
  const fold = side.getByTestId('threads-working-toggle');
  expect((await fold.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  await expect(row('working')).toBeVisible({ timeout: 30_000 });
  await fold.tap();
  await expect(row('working')).toHaveCount(0);
  await fold.tap();
  await expect(row('working')).toBeVisible();

  // A long press opens the row's menu: Pin acts at once, Undo takes it back.
  const main = row('idle').locator('.tr-main');
  const box = (await main.boundingBox())!;
  const at = { pointerType: 'touch', isPrimary: true, clientX: box.x + 40, clientY: box.y + 20 };
  await main.dispatchEvent('pointerdown', at);
  const menu = page.locator('.ctx-menu');
  await expect(menu).toBeVisible();
  await main.dispatchEvent('pointerup', at);
  await menu.getByRole('menuitem', { name: 'Pin thread' }).tap();
  await expect(side.getByTestId(`thread-pinned-${ids.idle}`)).toBeVisible();
  await page.locator('.toast', { hasText: 'Pinned' }).locator('.toast-action').tap();
  await expect(side.getByTestId(`thread-pinned-${ids.idle}`)).toHaveCount(0);

  // One tap on a row opens it; a phone's sheet closes on the pick.
  await row('approval').locator('.tr-main').tap();
  await expect(page.getByTestId(`runtime-thread-${ids.approval}`)).toBeVisible();
  if (drawer) await expect(side).toBeHidden();
  await expectNoHorizontalOverflow(page);
  const shots = process.env.OTTO_SHOTS_DIR;
  if (shots && drawer) {
    await page.getByTestId('threads-toggle-sidebar').tap();
    await expectFullyInViewport(page, side, 'threads sidebar');
    for (const scheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme: scheme });
      await page.screenshot({ path: join(shots, `after-${info.project.name}-${scheme}.png`) });
    }
  }
});
