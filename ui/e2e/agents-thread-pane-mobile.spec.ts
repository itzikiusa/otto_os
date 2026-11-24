import { test, expect, type APIRequestContext } from '@playwright/test';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

// Phone (spec A2 + A6): a thread row in the Agents list pushes the full-height
// chat (the Threads conversation + composer); Back returns to the list.

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';
let threadId = '';
const SHOTS = process.env.OTTO_E2E_SHOTS;

test.beforeEach(async ({ page }, info) => {
  test.skip(!['iphone-portrait', 'android-phone'].includes(info.project.name), 'phone projects');
  ({ ctx, base } = await apiCtx());
  dir = mkdtempSync(join(tmpdir(), 'otto-e2e-thread-pane-m-'));
  const w = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Thread Pane Phone', root_path: dir } });
  expect(w.ok(), await w.text()).toBeTruthy();
  wsId = (await w.json()).id as string;
  const t = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: dir, title: 'Nesta chat' } });
  expect(t.ok(), await t.text()).toBeTruthy();
  threadId = (await t.json()).id as string;
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
  }, wsId);
});

test.afterEach(async () => {
  if (threadId) await ctx.post(`${base}/api/v1/runtime-sessions/${threadId}/close`).catch(() => {});
  await ctx?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test('a thread row pushes the full-height chat; Back returns to the list', async ({ page }, info) => {
  await page.goto('/#/agents');
  const row = page.getByTestId(`agents-row-${threadId}`);
  await expect(row).toBeVisible({ timeout: 30_000 });
  await row.tap();
  const pane = page.getByTestId(`thread-pane-${threadId}`);
  await expect(pane).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId('agents-inbox')).toHaveCount(0);
  const thread = pane.getByTestId(`runtime-thread-${threadId}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  await expect(thread.getByTestId('runtime-composer')).toBeVisible();
  // Full height: the pane fills the space under the app bar.
  const vp = page.viewportSize()!;
  expect((await pane.boundingBox())!.height).toBeGreaterThan(vp.height * 0.6);
  const back = page.getByTestId('agents-back-to-list');
  expect((await back.boundingBox())!.height).toBeGreaterThanOrEqual(40);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/agents-thread-pane-${info.project.name}.png` });
  await back.tap();
  await expect(page.getByTestId('agents-inbox')).toBeVisible();
  await row.tap();
  await expect(pane).toBeVisible({ timeout: 30_000 });
  await page.goBack();
  await expect(page.getByTestId('agents-inbox')).toBeVisible();
});

// ‹ returns to the list without a new history entry: the system Back from
// the list then leaves Agents instead of walking back into the session.
test('‹ steps back to the list; the system Back then leaves Agents', async ({ page }) => {
  await page.goto('/#/settings');
  await expect(page.locator('body')).toBeVisible();
  await page.evaluate(() => (location.hash = '#/agents'));
  const row = page.getByTestId(`agents-row-${threadId}`);
  await expect(row).toBeVisible({ timeout: 30_000 });
  await row.tap();
  const pane = page.getByTestId(`thread-pane-${threadId}`);
  await expect(pane).toBeVisible({ timeout: 30_000 });
  await page.getByTestId('agents-back-to-list').tap();
  await expect(page.getByTestId('agents-inbox')).toBeVisible();
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/agents');
  await page.goBack();
  await expect.poll(() => page.evaluate(() => location.hash)).toMatch(/^#\/settings/);
  await expect(pane).toHaveCount(0);
});
