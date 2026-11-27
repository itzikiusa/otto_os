import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// The side-surface foundation (design.md §2, §7) on a phone and a tablet, with
// real taps:
//   phone   ONE header row (no shell bar): ☰ → the ONE drawer → a module; the
//           BottomNav "Menu" tab opens the same drawer; Threads is a list page
//           (push): list → thread → ‹ Threads → another thread; the Side panel
//           is one tap away from the header.
//   tablet  ONE header row too (the page's, Side panel last; arrows in the
//           Rail), the Rail by default; Threads' list is a column beside the content.
// Screenshots (OTTO_E2E_SHOTS) → side-foundation/.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';
const ids: string[] = [];

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'otto-e2e-side-'));
  writeFileSync(join(dir, 'README.md'), '# side foundation\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Side Foundation WS', root_path: dir } });
  expect(r.ok(), await r.text()).toBeTruthy();
  wsId = (await r.json()).id as string;
  for (const title of ['Fix flaky login test', 'Refund webhook retries']) {
    const t = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: dir, title } });
    expect(t.ok(), await t.text()).toBeTruthy();
    const id = (await t.json()).id as string;
    ids.push(id);
    // A first turn (the e2e fake agent echoes it), so the thread is a live inbox row.
    await ctx.post(`${base}/api/v1/runtime-sessions/${id}/prompt`, { data: { text: 'hello' } });
  }
});

test.afterAll(async () => {
  for (const id of ids) await ctx?.post(`${base}/api/v1/runtime-sessions/${id}/close`).catch(() => {});
  await ctx?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
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

test('phone: one header row, ☰ and Menu open the one drawer, a module from it', async ({ page }, info) => {
  test.skip(info.project.name !== 'iphone-portrait', 'iPhone portrait');
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#/home');
  const header = page.locator('.mcenter [data-testid="page-header"]').first();
  await expect(header).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.mtopbar'), 'no second top bar on a page with a PageHeader').toHaveCount(0);
  await expect(header.getByRole('button', { name: 'Open navigator' })).toBeVisible();
  await expect(header.getByRole('button', { name: 'Side panel' })).toBeVisible();
  await expect(header.getByRole('button', { name: 'Go back' }), 'no history arrows on a phone').toHaveCount(0);
  await expectNoHorizontalOverflow(page);
  await shot(page, 'phone-shell');

  await header.getByRole('button', { name: 'Open navigator' }).tap();
  const drawer = page.locator('.drawer[aria-label="Navigator"]');
  await expect(drawer).toBeVisible();
  await expect(drawer.getByTestId('short-nav-extras')).toBeVisible();
  await expect(drawer.locator('.notif-bell, [aria-label^="Notifications"]').first()).toBeVisible();
  const w = (await drawer.boundingBox())!.width;
  expect(w, 'left drawer min(86vw, 320px)').toBeLessThanOrEqual(321);
  await shot(page, 'phone-drawer');
  await drawer.locator('.nav-item', { hasText: 'Connections' }).first().tap();
  await expect(page).toHaveURL(/#\/connections/);
  await expect(drawer).toBeHidden();
  await expect(page).toHaveTitle(/^Connections/);

  // The BottomNav tab is "Menu" and opens the same drawer.
  const menu = page.getByTestId('bottomnav-menu');
  await expect(menu).toHaveAttribute('aria-label', /^Menu/);
  await menu.tap();
  await expect(drawer).toBeVisible();
  await drawer.getByRole('button', { name: 'Close Navigator' }).tap();
  await expect(drawer).toBeHidden();
});

test('phone: Threads is a list page — thread, ‹ Threads, another thread; Side panel one tap away', async ({ page }, info) => {
  test.skip(info.project.name !== 'iphone-portrait', 'iPhone portrait');
  test.setTimeout(90_000);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#/threads');
  const list = page.getByTestId('threads-sidebar');
  await expect(list).toBeVisible({ timeout: 30_000 });
  const header = page.locator('[data-testid="threads-page"] > [data-testid="page-header"]');
  await expect(header.locator('.ph-title')).toHaveText('Threads');
  await expect(header.getByRole('button', { name: 'Open navigator' })).toBeVisible();
  await expect(header.getByTestId('threads-new-btn'), 'the list pane\'s one primary is in the header').toBeVisible();
  await expect(page.locator('.mtopbar')).toHaveCount(0);
  await expect(page.getByTestId('threads-open-navigator')).toHaveCount(0);
  const row = (id: string) => page.getByTestId(`thread-row-${id}`).locator('.tr-main');
  // Other specs' threads share the list: filter to this spec's folder.
  await list.getByTestId('threads-search').fill(dir.split('/').pop()!);
  await expect(row(ids[0])).toBeVisible({ timeout: 20_000 });
  await expectNoHorizontalOverflow(page);
  await shot(page, 'phone-threads-list');

  await row(ids[0]).tap();
  await expect(page).toHaveURL(new RegExp(`#/threads/${ids[0]}`));
  await expect(list).toBeHidden();
  const back = header.getByRole('button', { name: 'Back to Threads' });
  await expect(back).toBeVisible();
  await expect(header.getByRole('button', { name: 'Open navigator' })).toHaveCount(0);
  await shot(page, 'phone-thread');

  await back.tap();
  await expect(list).toBeVisible();
  await expect(row(ids[0]), 'the thread you came from stays marked').toHaveAttribute('aria-current', 'true');
  await row(ids[1]).tap();
  await expect(page).toHaveURL(new RegExp(`#/threads/${ids[1]}`));
  await expect(page.getByTestId(`runtime-thread-${ids[1]}`)).toBeVisible({ timeout: 20_000 });

  await header.getByRole('button', { name: 'Side panel' }).tap();
  const side = page.locator('.drawer[aria-label="Side panel"]');
  await expect(side).toBeVisible();
  expect((await side.boundingBox())!.width, 'a right sheet is full width on a phone').toBeGreaterThanOrEqual(389);
  await shot(page, 'phone-threads-sidepanel');
});

test('tablet: the Rail by default; the Threads list is a column beside the conversation', async ({ page }, info) => {
  test.skip(info.project.name !== 'ipad-portrait', 'iPad portrait');
  test.setTimeout(90_000);
  await page.setViewportSize({ width: 820, height: 1180 });
  await page.goto('/#/home');
  await expect(page.locator('.msidebar.mfolded .rail')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.mtopbar'), 'a tablet has ONE header row too').toHaveCount(0);
  const header = page.locator('.mcenter [data-testid="page-header"]').first();
  await expect(header.getByRole('button', { name: 'Side panel', exact: true })).toBeVisible();
  await expect(header.getByRole('button', { name: 'Open navigator' }), 'the Navigator is a column here').toHaveCount(0);
  await expect(page.locator('.rail').getByRole('button', { name: 'Go back' }), 'history arrows stay in the Rail').toBeVisible();
  await shot(page, 'tablet-shell');
  await page.goto(`/#/threads/${ids[0]}`);
  const list = page.getByTestId('threads-sidebar');
  await expect(list).toBeVisible({ timeout: 30_000 });
  const thread = page.getByTestId(`runtime-thread-${ids[0]}`);
  await expect(thread).toBeVisible({ timeout: 20_000 });
  const lb = (await list.boundingBox())!;
  const tb = (await thread.boundingBox())!;
  expect(lb.x + lb.width).toBeLessThanOrEqual(tb.x + 2);
  expect(tb.width, 'the content keeps 480 px (tablet clamp)').toBeGreaterThanOrEqual(470);
  await expectNoHorizontalOverflow(page);
  await shot(page, 'tablet-threads');
  // The Rail ⇄ Navigator choice is remembered.
  await page.locator('.rail').getByRole('button', { name: 'Expand sidebar' }).click();
  await expect(page.locator('.msidebar:not(.mfolded) .navigator')).toBeVisible();
  await page.reload();
  await expect(page.locator('.msidebar:not(.mfolded) .navigator')).toBeVisible({ timeout: 20_000 });
  await page.locator('.navigator').getByRole('button', { name: 'Collapse sidebar' }).click();
  await expect(page.locator('.msidebar.mfolded .rail')).toBeVisible();
});
