import { test, expect, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expectFullyInViewport } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Starting a thread and the Threads page frame (desktop-browser, MOCK mode —
// ui/src/lib/api/mock-runtime.ts; workspace wsp_otto, root and registered repo
// /Users/dev/otto):
//   1. the stage names the workspace's repo (not the home folder) and shows the
//      access chip BEFORE the first send; the created thread gets the folder
//      and access that were shown; another live thread in the same checkout
//      says so with "Use a new worktree";
//   2. the toolbar ⋯ on a thread: at most 9 entries, ONE terminal entry,
//      layout in View;
//   3. closing a parent with open children offers "Close only this" /
//      "Close all 3", and Close all closes the children too;
//   4. at 1280 both panels open keep the conversation ~640 px wide.
// Screenshots (when OTTO_E2E_SHOTS is set) go to fix-T-STAGE/.
// ─────────────────────────────────────────────────────────────────────────────

type Hooks = {
  seed: (row: Record<string, unknown>) => string;
  say: (sid: string, texts: string[]) => void;
  creates: Record<string, unknown>[];
};

test.beforeEach(async ({ page }, ti) => {
  test.skip(ti.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_workspace', 'wsp_otto');
    localStorage.removeItem('otto_threads_v1');
    localStorage.removeItem('otto_threads_chrome_v1');
  });
  await page.setViewportSize({ width: 1280, height: 800 });
});

async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'fix-T-STAGE'), { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: join(dir, 'fix-T-STAGE', `${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

async function boot(page: Page, rows: Record<string, unknown>[] = []): Promise<void> {
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: Hooks }).__mockRuntime?.seed === 'function');
  await page.evaluate((list) => {
    const m = (window as unknown as { __mockRuntime: Hooks }).__mockRuntime;
    for (const r of list) m.seed(r);
  }, rows);
}

/** A runtime thread's status as the page's store holds it. */
const statusOf = (page: Page, id: string): Promise<string | null> =>
  page.evaluate(async (sid) => {
    const path = '/src/modules/threads/runtime/runtimeThreads.svelte.ts';
    const mod = (await import(/* @vite-ignore */ path)) as { runtimeThreads: { get(id: string): { status: string } | null } };
    return mod.runtimeThreads.get(sid)?.status ?? null;
  }, id);

test('the stage names the repo and shows the access before the first send; the start matches', async ({ page }) => {
  await boot(page, [{ id: 'rt_busy', title: 'Refactor retries', provider: 'codex', status: 'working', cwd: '/Users/dev/otto/src', last_active_at: '2026-11-12T10:09:00Z' }]);
  await page.evaluate(() => { location.hash = '#/threads/new'; });
  const stage = page.getByTestId('new-thread-stage');
  await expect(stage).toBeVisible({ timeout: 15_000 });

  // The workspace's repo, by name, never "Home" / "What should we work on?".
  const project = stage.getByTestId('new-thread-project');
  await expect(project).toHaveText('otto');
  await expect(project).toHaveAttribute('title', /\/Users\/dev\/otto/);
  await expect(page.getByTestId('threads-breadcrumb')).not.toContainText('Home');

  // The access the first turn runs with, before sending: the saved default
  // (Settings › Providers skips permission prompts by default = Full access).
  await stage.getByTestId('picker-agent').click();
  await page.getByTestId('model-picker').getByTestId('model-picker-provider-claude').click();
  await page.keyboard.press('Escape');
  const chip = stage.getByTestId('runtime-access');
  await expect(chip).toContainText('Full access');
  // A repo: no caution.
  await expect(stage.getByTestId('new-thread-access-caution')).toHaveCount(0);

  // Another live thread works in this checkout: one muted line with a way out.
  const busy = stage.getByTestId('new-thread-same-checkout');
  await expect(busy).toContainText('Another thread is working in this checkout');
  await expect(busy.getByRole('button', { name: 'Use a new worktree' })).toBeVisible();
  // One attach control.
  await expect(stage.getByTestId('new-thread-attach')).toHaveCount(1);
  await expect(stage.getByTestId('new-thread-attach-image')).toHaveCount(0);
  await shot(page, 'stage-1280');

  // Pick Supervised and send: the thread starts where and how it said.
  await chip.click();
  await page.locator('.ctx-menu').getByRole('menuitemcheckbox', { name: 'Supervised' }).click();
  await expect(chip).toContainText('Supervised');
  await stage.getByTestId('new-thread-input').fill('Check the cache settings');
  await stage.getByTestId('new-thread-send-btn').click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __mockRuntime: Hooks }).__mockRuntime.creates.length)).toBe(1);
  const body = await page.evaluate(() => (window as unknown as { __mockRuntime: Hooks }).__mockRuntime.creates[0]);
  expect(body).toMatchObject({ provider: 'claude', cwd: '/Users/dev/otto', access_mode: 'approval-required' });
});

test('the toolbar ⋯ holds at most 9 entries and one terminal entry', async ({ page }) => {
  await boot(page, [{ id: 'rt_menu', title: 'Fix invoice rounding', provider: 'claude', last_active_at: '2026-11-12T10:09:00Z' }]);
  await page.evaluate(() => { location.hash = '#/threads/rt_menu'; });
  await expect(page.getByTestId('runtime-thread-rt_menu')).toBeVisible({ timeout: 15_000 });
  await page.getByTestId('threads-thread-menu').click();
  const menu = page.locator('.ctx-menu').first();
  await expectFullyInViewport(page, menu, 'toolbar ⋯');
  const names = await menu.locator('[role^="menuitem"]').allInnerTexts();
  expect(names.length, names.join(' | ')).toBeLessThanOrEqual(9);
  expect(names.filter((n) => /terminal/i.test(n))).toHaveLength(1);
  for (const gone of [/^Pin thread/, /^Open beside/, /^Move to History/, /^Single thread/, /^Split/, /^Grid/]) {
    expect(names.some((n) => gone.test(n)), String(gone)).toBe(false);
  }
  // The exits say what they do, in words a touch screen shows too.
  await expect(menu.getByRole('menuitem', { name: /Close thread…/ })).toContainText('to History');
  await expect(menu.getByRole('menuitem', { name: /Delete thread…/ })).toContainText('for good');
  await menu.getByRole('menuitem', { name: 'View' }).click();
  await expect(page.locator('.ctx-submenu').getByRole('menuitemcheckbox', { name: 'Split' })).toBeVisible();
  await shot(page, 'more-menu-1280');
});

test('closing a parent with open children offers to close them too', async ({ page }) => {
  const kid = (id: string, title: string) => ({ id, title, provider: 'codex', status: 'working', parent_id: 'rt_parent', last_active_at: '2026-11-12T10:08:00Z', meta: { child_of: { id: 'rt_parent', by: 'agent' } } });
  await boot(page, [
    { id: 'rt_parent', title: 'Fix the build', provider: 'claude', last_active_at: '2026-11-12T10:09:00Z' },
    kid('rt_kid1', 'slow 40'),
    kid('rt_kid2', 'check linker'),
  ]);
  await page.evaluate(() => { location.hash = '#/threads/rt_parent'; });
  await expect(page.getByTestId('runtime-thread-rt_parent')).toBeVisible({ timeout: 15_000 });
  await page.getByTestId('threads-thread-menu').click();
  await page.locator('.ctx-menu').first().getByRole('menuitem', { name: /Close thread…/ }).click();
  const dlg = page.getByRole('dialog');
  await expect(dlg).toContainText('2 threads opened from it keep running.');
  await expect(dlg.getByRole('button', { name: 'Close only this' })).toBeVisible();
  await expect(dlg.getByRole('button', { name: 'Close all 3' })).toBeVisible();
  await shot(page, 'close-with-children');
  await dlg.getByRole('button', { name: 'Close all 3' }).click();
  for (const id of ['rt_parent', 'rt_kid1', 'rt_kid2']) await expect.poll(() => statusOf(page, id)).toBe('closed');
});

test('at 1280 both panels open leave the conversation about 640 px', async ({ page }) => {
  await boot(page, [{ id: 'rt_wide', title: 'Long model thread', provider: 'claude', last_active_at: '2026-11-12T10:09:00Z' }]);
  await page.evaluate(() => { location.hash = '#/threads/rt_wide'; });
  await expect(page.getByTestId('runtime-thread-rt_wide')).toBeVisible({ timeout: 15_000 });
  const toggle = page.getByTestId('threads-toggle-inspector');
  if ((await toggle.getAttribute('aria-pressed')) !== 'true') await toggle.click();
  await expect(page.getByTestId('threads-inspector')).toBeVisible();
  await expect(page.getByTestId('threads-sidebar')).toBeVisible();
  const center = await page.locator('.th-center-col').boundingBox();
  expect(center?.width ?? 0).toBeGreaterThanOrEqual(630);
  // The thread-list and right-panel toggles are told apart.
  await expect(page.getByTestId('threads-toggle-sidebar')).toHaveAttribute('title', /thread list/);
  await expect(page.getByTestId('threads-toggle-inspector')).toHaveAttribute('title', /right panel/);
  await expect(page.getByTestId('threads-surface-changes')).toContainText('Changes');
  await shot(page, 'both-panels-1280');
});
