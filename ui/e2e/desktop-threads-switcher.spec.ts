import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport } from './helpers';

// ⌘K opens on what needs you (the bar + threadSwitcherSource): in Threads the
// first group is "Threads", in the sidebar's inbox order and words — needs you
// (Approval / Question), Failed, Done, then the rest and the Working fold —
// one status word + project per row, the order frozen while open. Typing
// filters threads by title and project with the commands; "New thread:
// <query>" starts the stage with the query as the draft; ⌘↵ opens beside in
// a split. Elsewhere only "Threads needing you" shows. Real structured
// threads on the isolated daemon: Codex (`ask` = an approval, `hang` = a
// turn that keeps working) and Claude (`slow <s>` = a turn that ends later).

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';
/** Unique per run: repeated runs share one daemon. */
const RUN = Math.random().toString(36).slice(2, 6);

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'switcher-'));
  writeFileSync(join(dir, 'README.md'), '# switcher\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Switcher WS', root_path: dir } });
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
    if (sessionStorage.getItem('switcher-init')) return;
    sessionStorage.setItem('switcher-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_threads_v1');
  }, wsId);
});

async function thread(provider: 'claude' | 'codex', title: string): Promise<string> {
  const created = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider, cwd: dir, title } });
  expect(created.ok(), await created.text()).toBeTruthy();
  return (await created.json()).id as string;
}

/** A prompt through the API: the thread works while another one is in front. */
async function prompt(id: string, text: string): Promise<void> {
  expect((await ctx.post(`${base}/api/v1/runtime-sessions/${id}/prompt`, { data: { text } })).ok()).toBeTruthy();
}

/** Arrow the bar's selection onto a thread row. */
async function select(page: Page, id: string): Promise<void> {
  await expect(threadRow(page, id)).toBeVisible({ timeout: 3_000 });
  for (let i = 0; i < 20 && (await threadRow(page, id).getAttribute('aria-selected')) !== 'true'; i++) await barInput(page).press('ArrowDown');
  await expect(threadRow(page, id)).toHaveAttribute('aria-selected', 'true');
}

/** ⌘K once the app has its keys bound (a fresh load can swallow the first press). */
async function openPalette(page: Page): Promise<void> {
  await expect(async () => {
    await page.keyboard.press('Meta+k');
    await expect(palette(page)).toBeVisible({ timeout: 1_000 });
  }).toPass({ timeout: 15_000 });
}

/** The ⌘K bar's result list (lib/components/FloatingBar.svelte). */
const palette = (page: Page) => page.locator('#fb-listbox');
const barInput = (page: Page) => page.getByRole('combobox', { name: 'Ask Otto or search commands' });
const threadRow = (page: Page, id: string) => palette(page).locator(`[data-cmd="thread:${id}"]`);

test('⌘K in Threads opens on what needs you; typing filters; New thread and beside', async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1600, height: 1000 });
  const recent = await thread('claude', `Sw recent idea ${RUN}`);
  const working = await thread('codex', `Sw working build ${RUN}`);
  const done = await thread('claude', `Sw zsh startup ${RUN}`);
  const asks = await thread('codex', `Sw approval needed ${RUN}`);
  await page.goto(`/#/threads/${recent}`);
  await expect(page.getByTestId(`runtime-thread-${recent}`)).toBeVisible({ timeout: 30_000 });
  // Off screen: one keeps working, one asks for approval, one finishes unread.
  await prompt(working, 'hang');
  await prompt(asks, 'ask');
  await prompt(done, 'hello');
  await expect(page.getByTestId(`thread-status-${done}`)).toHaveText('Done', { timeout: 20_000 });

  await openPalette(page);
  const pal = palette(page);
  await expect(pal).toBeVisible();
  // The "Threads" group comes first, before the commands.
  await expect(pal.locator('.sec').first()).toHaveText('Threads');
  expect(await pal.locator('[role="option"]').first().getAttribute('data-cmd')).toMatch(/^thread:/);
  const order = async (): Promise<string[]> =>
    (await pal.locator('[data-cmd^="thread:"]').evaluateAll((els) => els.map((e) => (e.getAttribute('data-cmd') ?? '').slice(7)))).filter((id) => [asks, done, working, recent].includes(id));
  // The sidebar's inbox rank (needs you, Done), then Working, then the rest
  // (the recent one may sit past the list's 8 rows in a busy daemon).
  await expect.poll(async () => (await order()).slice(0, 3)).toEqual([asks, done, working]);
  const row = (id: string) => threadRow(page, id);
  // The sidebar's words: Approval / Question from one compact request read.
  await expect(row(asks).locator('.opt-meta')).toHaveText('Approval');
  await expect(row(done).locator('.opt-meta')).toHaveText('Done');
  await expect(row(working).locator('.opt-meta')).toHaveText(/^Working/);
  await expect(row(asks)).toContainText(dir.split('/').pop()!);
  await expectFullyInViewport(page, pal, 'palette');
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: process.env.THRC_SHOTS ? `${process.env.THRC_SHOTS}/switcher-desktop-${scheme}.png` : test.info().outputPath(`switcher-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });

  // Typing filters threads by title and project with the commands; the last
  // row starts a new thread with the query as its draft.
  const input = barInput(page);
  await input.fill(`zsh startup ${RUN}`);
  await expect(pal.locator('[data-cmd^="thread:"]:not([data-cmd="thread:new"])')).toHaveCount(1);
  await expect(row(done)).toBeVisible();
  await expect(pal.locator('[data-cmd="thread:new"]')).toContainText(`New thread: zsh startup ${RUN}`);
  // Enter on the thread opens it in the focused pane.
  await select(page, done);
  await input.press('Enter');
  await expect(pal).toHaveCount(0);
  await expect(page.getByTestId(`runtime-thread-${done}`)).toBeVisible();

  await openPalette(page);
  await barInput(page).fill('Write the release notes');
  await palette(page).locator('[data-cmd="thread:new"]').click();
  await expect(page.getByTestId('new-thread-stage')).toBeVisible();
  await expect(page.getByTestId('new-thread-input')).toHaveValue('Write the release notes');

  // ⌘↵ opens beside when a split is open.
  await page.evaluate((h) => { location.hash = h; }, `#/threads/${recent}`);
  await expect(page.getByTestId(`runtime-thread-${recent}`)).toBeVisible({ timeout: 30_000 });
  await page.getByTestId('threads-thread-menu').click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'View' }).click();
  await page.locator('.ctx-submenu').getByRole('menuitemcheckbox', { name: 'Split' }).click();
  await expect(page.getByTestId('threads-canvas')).toHaveAttribute('data-layout', 'split');
  // A freshly mounted pane may take the focus back once (its composer):
  // re-open the bar until the selection holds.
  await expect(async () => {
    await openPalette(page);
    await barInput(page).fill(`working build ${RUN}`, { timeout: 2_000 });
    await select(page, working);
  }).toPass({ timeout: 30_000 });
  await barInput(page).press('Meta+Enter');
  await expect(page.getByTestId(`thread-pane-${working}`)).toBeVisible();
  await expect(page.getByTestId(`thread-pane-${recent}`)).toBeVisible();
});

test('outside Threads ⌘K gains "Threads needing you" only when one waits', async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const quiet = await thread('codex', 'Sw quiet');
  await page.goto(`/#/threads/${quiet}`);
  await expect(page.getByTestId(`runtime-thread-${quiet}`)).toBeVisible({ timeout: 30_000 });
  await page.goto('/#/home');
  await openPalette(page);
  const pal = palette(page);
  // Only waiting threads ever show outside Threads.
  await expect(threadRow(page, quiet)).toHaveCount(0);
  await page.keyboard.press('Escape');

  const waits = await thread('codex', 'Sw waits on you');
  await prompt(waits, 'ask');
  await page.goto('/#/home');
  await expect(async () => {
    await openPalette(page);
    await expect(threadRow(page, waits)).toBeVisible({ timeout: 1_000 });
  }).toPass({ timeout: 20_000 });
  await expect(palette(page).locator('.sec').first()).toHaveText('Threads needing you');
  const w = threadRow(page, waits);
  await expect(w.locator('.opt-meta')).toHaveText('Needs you');
  await expect(palette(page).locator('[data-cmd="thread:new"]')).toHaveCount(0);
  await w.click();
  await expect(page.getByTestId(`runtime-thread-${waits}`)).toBeVisible({ timeout: 30_000 });
});

test('each ⌘K open orders by what needs you now, not by the last open', async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const first = await thread('codex', `Sw order first ${RUN}`);
  const later = await thread('codex', `Sw order later ${RUN}`);
  await prompt(first, 'hello');
  await prompt(later, 'hello');
  await page.goto(`/#/threads/${first}`);
  await expect(page.getByTestId(`runtime-thread-${first}`)).toBeVisible({ timeout: 30_000 });
  const mine = async (): Promise<string[]> =>
    (await palette(page).locator('[data-cmd^="thread:"]').evaluateAll((els) => els.map((e) => (e.getAttribute('data-cmd') ?? '').slice(7)))).filter((id) => id === first || id === later);
  await openPalette(page);
  await expect.poll(mine).toHaveLength(2);
  // The one this open lists second starts waiting on the person while the bar is closed.
  const waiter = (await mine())[1];
  await page.keyboard.press('Escape');
  await expect(palette(page)).toHaveCount(0);
  await prompt(waiter, 'ask');
  await expect(page.getByTestId(`thread-status-${waiter}`)).toHaveText('Approval', { timeout: 20_000 });
  await openPalette(page);
  // No keystroke, no other change: the order this open shows must already be right.
  expect((await mine())[0]).toBe(waiter);
});

test('switching threads with the composer focused throws nothing (Chromium blur during teardown)', async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const a = await thread('codex', `Sw teardown a ${RUN}`);
  const b = await thread('codex', `Sw teardown b ${RUN}`);
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
  await page.goto(`/#/threads/${a}`);
  const composer = page.getByTestId(`runtime-thread-${a}`).getByTestId('runtime-composer');
  await expect(composer).toBeVisible({ timeout: 30_000 });
  for (const [from, to] of [[a, b], [b, a]]) {
    await page.getByTestId(`runtime-thread-${from}`).getByTestId('runtime-composer').focus();
    // Switch without moving the focus first (a route change, ⌥⌘↓, a notification).
    await page.evaluate((h) => { location.hash = h; }, `#/threads/${to}`);
    await expect(page.getByTestId(`runtime-thread-${to}`)).toBeVisible({ timeout: 30_000 });
  }
  expect(errors.filter((e) => /state_unsafe_mutation/.test(e))).toEqual([]);
});
