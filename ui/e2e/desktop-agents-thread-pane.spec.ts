import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// Chat threads as panes beside the terminals (spec A2): two terminals and one
// structured thread (otto-fake-codex: `ask` = a command approval) tile in
// one canvas; the approval is answered in the thread pane; a reload finds all
// three in place; tiled view tiles the thread too; ⌘-click on a thread row
// still goes to Threads; a thread pane offers no pop-out.

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';
const ids: Record<string, string> = {};
let threadId = '';
const SHOTS = process.env.OTTO_E2E_SHOTS;

test.beforeAll(async () => {
  ({ ctx, base } = await apiCtx());
  dir = mkdtempSync(join(tmpdir(), 'otto-e2e-thread-pane-'));
  const w = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Thread Pane WS', root_path: dir } });
  expect(w.ok(), await w.text()).toBeTruthy();
  wsId = (await w.json()).id as string;
  for (const title of ['Baresi', 'Maldini']) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: { kind: 'agent', provider: 'shell', title, cwd: dir, meta: { origin: 'e2e' } },
    });
    expect(r.ok(), await r.text()).toBeTruthy();
    ids[title] = (await r.json()).id as string;
  }
  const t = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: dir, title: 'Costacurta chat' } });
  expect(t.ok(), await t.text()).toBeTruthy();
  threadId = (await t.json()).id as string;
});

test.afterAll(async () => {
  if (threadId) await ctx.post(`${base}/api/v1/runtime-sessions/${threadId}/close`).catch(() => {});
  await ctx?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('thread-pane-init')) return;
    sessionStorage.setItem('thread-pane-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
    localStorage.setItem('otto_view_mode', 'tabs');
  }, wsId);
});

const row = (page: Page, id: string) => page.getByTestId(`agents-row-${id}`);
const paneOf = (page: Page, id: string) => page.locator(`[data-pane-key][data-session="${id}"]`);

async function shots(page: Page, name: string): Promise<void> {
  if (!SHOTS) return;
  for (const s of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: s });
    await expect(page.locator('html')).toHaveAttribute('data-scheme', s);
    await page.screenshot({ path: `${SHOTS}/${name}-1440-${s}.png`, animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

test('two terminals and a thread tile together; the approval is answered in the pane; a reload keeps all three', async ({ page }) => {
  await page.goto(`/#/agents/${ids.Baresi}`);
  await expect(paneOf(page, ids.Baresi)).toBeVisible({ timeout: 30_000 });
  // ⌘-click opens beside (terminals), the row menu's "Open beside" does it for a thread.
  await row(page, ids.Maldini).click({ modifiers: ['ControlOrMeta'] });
  await expect(paneOf(page, ids.Maldini)).toBeVisible();
  await expect(row(page, threadId)).toBeVisible({ timeout: 20_000 });
  await row(page, threadId).click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Open beside' }).click();

  const pane = page.getByTestId(`thread-pane-${threadId}`);
  await expect(pane).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('[data-pane-key]')).toHaveCount(3);
  await expect(page.locator(`[data-tab-id="${threadId}"]`)).toContainText('Costacurta chat');
  // One header line: status word · title (· folder) · ⋯ · ✕ — no Terminal ⇄ Chat switch.
  await expect(pane.getByTestId('pane-status')).toHaveText(/\S/);
  await expect(pane.locator('.tpane-title')).toHaveText('Costacurta chat');
  await expect(pane.getByTestId('pane-view')).toHaveCount(0);

  // The Threads conversation and composer, right here.
  const thread = pane.getByTestId(`runtime-thread-${threadId}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('ask');
  await composer.press('Enter');
  const dock = thread.getByTestId('runtime-attention');
  await expect(dock).toBeVisible({ timeout: 20_000 });
  await expect(dock).toHaveAttribute('data-kind', 'approval');
  await expect(pane.getByTestId('pane-status')).toHaveText(/Needs you|Approval/);
  // The tab strip says it in a word — no bare status dots on tabs.
  await expect(page.locator(`[data-tab-id="${threadId}"]`)).toContainText('Needs you');
  await expect(page.locator('.tabbar .dot')).toHaveCount(0);
  await shots(page, 'agents-thread-pane-approval');
  await dock.getByTestId('runtime-dock-accept').click();
  await expect(dock).toHaveCount(0, { timeout: 20_000 });
  await expectNoHorizontalOverflow(page);

  // A narrow tile still reads: the composer fits inside its pane.
  const [pb, cb] = [(await pane.boundingBox())!, (await composer.boundingBox())!];
  expect(cb.x).toBeGreaterThanOrEqual(pb.x - 1);
  expect(cb.x + cb.width).toBeLessThanOrEqual(pb.x + pb.width + 1);

  // No pop-out on a thread pane; Open in Threads is there.
  await pane.getByRole('button', { name: /^More…/ }).click();
  await expect(page.getByRole('menuitem', { name: 'Open in Threads' })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'Open in New Window' })).toHaveCount(0);
  // One way to History here (Close thread…): Threads' "Move to History" tidy
  // changed nothing on Agents (M1); the layout presets live with the canvas (M2).
  await expect(page.getByRole('menuitem', { name: 'Close thread…' })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'Move to History' })).toHaveCount(0);
  await expect(page.getByRole('menuitem', { name: /^Use .* layout$/ })).toHaveCount(0);
  await page.keyboard.press('Escape');

  await page.reload();
  await expect(page.getByTestId(`thread-pane-${threadId}`)).toBeVisible({ timeout: 30_000 });
  await expect(paneOf(page, ids.Baresi)).toBeVisible();
  await expect(paneOf(page, ids.Maldini)).toBeVisible();
  await expect(page.locator('[data-pane-key]')).toHaveCount(3);
  await expect(page.getByTestId(`thread-pane-${threadId}`).getByTestId(`runtime-thread-${threadId}`)).toBeVisible({ timeout: 30_000 });
  await shots(page, 'agents-thread-pane-tiled-split');

  // Tiled view tiles the thread with the terminals.
  await page.evaluate(() => localStorage.setItem('otto_view_mode', 'tiled'));
  await page.reload();
  await expect(page.locator('.tile-slot').filter({ has: page.getByTestId(`thread-pane-${threadId}`) })).toBeVisible({ timeout: 30_000 });
  await expect(page.locator(`.tile-slot[data-tile-id="${ids.Baresi}"]`)).toBeVisible();
  await shots(page, 'agents-thread-pane-tiled');
});

test('a plain click on a thread row opens its pane here; ⌘-click goes to Threads', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(row(page, threadId)).toBeVisible({ timeout: 30_000 });
  await row(page, threadId).click();
  await expect(page.getByTestId(`thread-pane-${threadId}`)).toBeVisible({ timeout: 20_000 });
  await expect.poll(() => page.evaluate(() => location.hash)).toBe(`#/agents/${threadId}`);
  await row(page, threadId).click({ modifiers: ['ControlOrMeta'] });
  await expect.poll(() => page.evaluate(() => location.hash)).toBe(`#/threads/${threadId}`);
});

// A focused chat pane is the route (`#/agents/<threadId>`): a reload must not
// call it gone (it was looked up as a terminal session only).
test('a reload with a chat pane focused restores it without a false "gone" toast', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(row(page, threadId)).toBeVisible({ timeout: 30_000 });
  await row(page, threadId).click();
  const pane = page.getByTestId(`thread-pane-${threadId}`);
  await expect(pane).toBeVisible({ timeout: 20_000 });
  await expect.poll(() => page.evaluate(() => location.hash)).toBe(`#/agents/${threadId}`);
  await page.reload();
  await expect(pane).toBeVisible({ timeout: 30_000 });
  // The route's lookup runs at mount, before the pane shows: give a late toast its chance.
  await page.waitForTimeout(1500); // ui-guards: allow — proves an absence (no toast)
  await expect(page.locator('.toast.error')).toHaveCount(0);
});

// ⌘⇧T reopens a closed chat pane (not an older terminal tab).
test('⌘⇧T reopens a closed chat pane', async ({ page }) => {
  await page.goto(`/#/agents/${ids.Baresi}`);
  await expect(paneOf(page, ids.Baresi)).toBeVisible({ timeout: 30_000 });
  await expect(row(page, threadId)).toBeVisible({ timeout: 20_000 });
  await row(page, threadId).click();
  const pane = page.getByTestId(`thread-pane-${threadId}`);
  await expect(pane).toBeVisible({ timeout: 20_000 });
  await pane.locator('.tpane-title').click();
  await page.keyboard.press('ControlOrMeta+w');
  await expect(page.locator(`[data-tab-id="${threadId}"]`)).toHaveCount(0);
  await page.keyboard.press('ControlOrMeta+Shift+t');
  await expect(pane).toBeVisible({ timeout: 20_000 });
  await expect(page.locator(`[data-tab-id="${threadId}"]`)).toHaveCount(1);
  await expect.poll(() => page.evaluate(() => location.hash)).toBe(`#/agents/${threadId}`);
});

// ⋯ › Details reads the session's usage on demand: the open menu shows the
// figure once the read lands, not on the next open.
test('⋯ › Details shows the usage read it just asked for', async ({ page }) => {
  await page.route('**/api/v1/sessions/*/usage', async (r) => {
    await new Promise((f) => setTimeout(f, 400));
    await r.fulfill({ json: { session_id: ids.Baresi, total_tokens: 12_345, cost_usd: 0.42, input_tokens: 10_000, output_tokens: 2_345 } });
  });
  await page.goto(`/#/agents/${ids.Baresi}`);
  const pane = paneOf(page, ids.Baresi);
  await expect(pane).toBeVisible({ timeout: 30_000 });
  await pane.getByRole('button', { name: /^More…/ }).click();
  await page.getByRole('menuitem', { name: 'Details' }).hover();
  await expect(page.getByRole('menuitem', { name: /^Usage: 12/ })).toBeVisible({ timeout: 5_000 });
});

// The right panel heads a focused chat pane with its details card too (A5).
test('a focused chat pane gets the details card in the right panel', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(row(page, threadId)).toBeVisible({ timeout: 30_000 });
  await row(page, threadId).click();
  await expect(page.getByTestId(`thread-pane-${threadId}`)).toBeVisible({ timeout: 20_000 });
  await page.locator('.rstrip').getByRole('button', { name: 'Files', exact: true }).click();
  const card = page.getByTestId('agents-details-card');
  await expect(card.getByTestId('thread-details-card')).toBeVisible({ timeout: 20_000 });
  await expect(card.getByTestId('details-folder')).toHaveAttribute('title', new RegExp(`${dir.split('/').pop()}$`));
  await shots(page, 'agents-thread-pane-details');
});

// A lone chat tab has no strip ✕: its ⋯ closes it.
test('a lone chat pane closes from its ⋯', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(row(page, threadId)).toBeVisible({ timeout: 30_000 });
  await row(page, threadId).click();
  const pane = page.getByTestId(`thread-pane-${threadId}`);
  await expect(pane).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('[data-pane-key]')).toHaveCount(1);
  await pane.getByRole('button', { name: /^More…/ }).click();
  await page.getByRole('menuitem', { name: 'Close pane' }).click();
  await expect(pane).toHaveCount(0);
});
