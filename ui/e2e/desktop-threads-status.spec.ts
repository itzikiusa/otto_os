import { test, expect, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// With several threads running, which one needs you, which works, which
// finished and which broke — at a glance, and one move to the one that needs
// you (round 5, T-ATTENTION):
//   • every sidebar row's dot + label come from the row itself: a pending
//     request is "Needs you" (after a reload too), a failed last turn is
//     "Failed" / "Usage limit", never idle; a parent speaks for its children
//     and its family sorts by its most urgent member;
//   • a folded project header says "N need you";
//   • "Next that needs you" (chip and ⌥⌘N), ↑/↓ + Enter in the list;
//   • a History row opens read-only (no revive) and expanding History never
//     scrolls the page;
//   • Home's waiting row opens THAT thread; a context menu closes on navigation.
// MOCK mode (mock-runtime.ts seeds the rows, newer daemon fields included);
// the last test drives REAL fake-codex turns (`ask`, `crash`) on the daemon.
// Screenshots: OTTO_E2E_SHOTS/fix-T-ATTENTION/ at 1280×800, light and dark.
// ─────────────────────────────────────────────────────────────────────────────

async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'fix-T-ATTENTION'), { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: join(dir, 'fix-T-ATTENTION', `desktop-${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

const T = (min: number): string => new Date(Date.UTC(2026, 10, 16, 9, min)).toISOString();

async function seedMock(page: Page): Promise<void> {
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: { seed?: unknown } }).__mockRuntime?.seed === 'function');
  await page.evaluate((t) => {
    const m = (window as unknown as { __mockRuntime: { seed: (r: Record<string, unknown>) => string } }).__mockRuntime;
    m.seed({ id: 'rt_wait', title: 'Wait for approval', provider: 'codex', status: 'awaiting_permission', last_active_at: t[0] });
    m.seed({ id: 'rt_work', title: 'Busy refactor', status: 'working', last_active_at: t[1] });
    m.seed({ id: 'rt_fail', title: 'Hit the limit', status: 'idle', last_turn_status: 'failed', last_error_class: 'usage_limit', last_active_at: t[2] });
    m.seed({ id: 'rt_done', title: 'All finished', status: 'idle', last_turn_status: 'completed', last_active_at: t[3] });
    m.seed({ id: 'rt_par', title: 'Parent lead', status: 'idle', last_turn_status: 'completed', last_active_at: t[4] });
    m.seed({ id: 'rt_kid1', title: 'Kid asks', provider: 'codex', status: 'awaiting_permission', parent_id: 'rt_par', meta: { child_of: { id: 'rt_par', by: 'person' } }, last_active_at: t[5] });
    m.seed({ id: 'rt_kid2', title: 'Kid broke', provider: 'codex', status: 'idle', last_turn_status: 'failed', parent_id: 'rt_par', meta: { child_of: { id: 'rt_par', by: 'person' } }, last_active_at: t[6] });
    m.seed({ id: 'rt_closed', title: 'Closed old work', status: 'closed', last_active_at: t[7] });
    location.hash = '#/threads/rt_done';
  }, [T(1), T(40), T(30), T(50), T(0), T(2), T(3), T(4)]);
  await expect(page.getByTestId('runtime-thread-rt_done')).toBeVisible({ timeout: 15_000 });
}

test.describe('mock rows', () => {
  test.beforeEach(async ({ page }, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.addInitScript(() => {
      localStorage.setItem('otto_mock', '1');
      localStorage.setItem('otto_workspace', 'wsp_otto');
      localStorage.removeItem('otto_threads_v1');
      localStorage.removeItem('otto_runtime_waiting');
    });
  });

  test('rows say needs you / working / failed / done; families rank by their most urgent member', async ({ page }) => {
    await seedMock(page);
    const side = page.getByTestId('threads-sidebar');
    const state = (id: string) => side.getByTestId(`thread-row-${id}`).first();
    await expect(state('rt_wait')).toHaveAttribute('data-state', 'needs-you');
    await expect(state('rt_work')).toHaveAttribute('data-state', 'working');
    await expect(state('rt_fail')).toHaveAttribute('data-state', 'failed');
    await expect(state('rt_done')).toHaveAttribute('data-state', 'idle');
    // Never colour alone: the dot names the state.
    await expect(state('rt_fail').locator('[data-state="failed"]').first()).toHaveAttribute('aria-label', 'Usage limit');
    await expect(state('rt_wait').locator('.tr-main [role="img"]').first()).toHaveAttribute('aria-label', 'Needs you');
    // The parent (idle itself) speaks for its waiting child and outranks the working thread.
    await expect(state('rt_par')).toHaveAttribute('data-state', 'needs-you');
    await expect(state('rt_kid2')).toHaveAttribute('data-state', 'failed');
    const ids = await side.locator('[data-testid^="thread-row-"]').evaluateAll((els) => els.map((e) => e.getAttribute('data-testid')!.slice('thread-row-'.length)));
    expect(ids.indexOf('rt_par')).toBeLessThan(ids.indexOf('rt_work'));
    expect(ids.indexOf('rt_work')).toBeLessThan(ids.indexOf('rt_fail'));
    expect(ids.indexOf('rt_fail')).toBeLessThan(ids.indexOf('rt_done'));
    // Desktop rows show the provider without growing.
    const h = (await state('rt_wait').locator('.tr-main').boundingBox())!.height;
    expect(h).toBeLessThanOrEqual(32);
    await expect(state('rt_wait').locator('.tr-prov')).toBeVisible();
    await shot(page, 'sidebar-states');

    // A folded project says what hides behind the fold.
    const toggle = side.locator('.th-proj-toggle').first();
    await toggle.click();
    await expect(side.locator('[data-testid^="threads-project-note-"]').first()).toContainText('2 need you');
    await shot(page, 'sidebar-folded');
    await toggle.click();
    await expectNoHorizontalOverflow(page);
  });

  test('one move to the thread that needs you: the chip, ⌥⌘N, and ↓ + Enter in the list', async ({ page }) => {
    await seedMock(page);
    const side = page.getByTestId('threads-sidebar');
    const chip = side.getByTestId('threads-next-needs-you');
    await expect(chip).toBeVisible();
    await expect(chip).toHaveAttribute('title', /⌥⌘N/);
    await chip.click();
    await expect(page).toHaveURL(/#\/threads\/rt_wait$/);
    await page.keyboard.press('Alt+Meta+n');
    await expect(page).toHaveURL(/#\/threads\/rt_kid1$/);
    // ⌥⌘↓ walks to the next thread in sidebar order.
    await page.keyboard.press('Alt+Meta+ArrowDown');
    await expect(page).toHaveURL(/#\/threads\/rt_kid2$/);

    // ↑/↓ move through the list, Enter opens.
    await side.getByTestId('thread-row-rt_wait').locator('.tr-main').focus();
    await page.keyboard.press('ArrowDown');
    await expect(side.getByTestId('thread-row-rt_par').locator('.tr-main')).toBeFocused();
    await page.keyboard.press('End');
    await page.keyboard.press('Home');
    await expect(side.getByTestId('thread-row-rt_wait').locator('.tr-main')).toBeFocused();
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('Enter');
    await expect(page).toHaveURL(/#\/threads\/rt_par$/);
  });

  test('History opens read-only without scrolling the page; a menu closes on navigation; Home opens the waiting thread', async ({ page }) => {
    await seedMock(page);
    const side = page.getByTestId('threads-sidebar');
    const scrolls = () =>
      page.evaluate(() => {
        let sum = document.scrollingElement?.scrollTop ?? 0;
        for (const el of document.querySelectorAll('body *')) if (!el.closest('.th-scroll') && el.scrollTop > 0 && !['auto', 'scroll'].includes(getComputedStyle(el).overflowY)) sum += el.scrollTop;
        return { doc: document.scrollingElement?.scrollTop ?? 0, clipped: sum };
      });
    expect(await scrolls()).toEqual({ doc: 0, clipped: 0 });
    await side.locator('.th-history').click();
    await expect(side.getByTestId('thread-history-rt_closed')).toBeVisible();
    await page.waitForTimeout(300); // ui-guards: allow — proves an absence: the page did not scroll after History opened
    expect(await scrolls()).toEqual({ doc: 0, clipped: 0 });
    await shot(page, 'history-open');

    // Reading a closed thread starts no agent: it opens with its Revive bar.
    await side.getByTestId('thread-history-rt_closed').locator('.tr-main').click();
    const closed = page.getByTestId('runtime-thread-rt_closed');
    await expect(closed).toBeVisible();
    await expect(closed.getByTestId('runtime-closed')).toBeVisible();
    await expect(side.getByTestId('thread-history-rt_closed')).toBeVisible();
    expect(await scrolls()).toEqual({ doc: 0, clipped: 0 });

    // A menu left open never outlives its screen.
    await side.getByTestId('thread-row-rt_work').locator('.tr-main').click({ button: 'right' });
    await expect(page.locator('.ctx-menu')).toBeVisible();
    await page.evaluate(() => (location.hash = '#/home'));
    await expect(page.locator('.ctx-menu')).toHaveCount(0);

    // Home names each waiting thread and opens that one.
    const row = page.locator('button.gc-row[title="Wait for approval"]');
    await expect(row).toBeVisible({ timeout: 15_000 });
    await row.click();
    await expect(page).toHaveURL(/#\/threads\/rt_wait$/);
  });
});

test.describe('real fake-agent turns', () => {
  let fixtureDir = '';
  test.afterAll(() => {
    if (fixtureDir) rmSync(fixtureDir, { recursive: true, force: true });
  });

  test('a pending approval reads "Needs you" after a full reload; a crashed turn is not idle while you watch', async ({ page }, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
    const { ctx, base } = await apiCtx();
    fixtureDir = mkdtempSync(join(tmpdir(), 'otto-e2e-threads-status-'));
    writeFileSync(join(fixtureDir, 'README.md'), '# Status\n');
    const w = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Status WS', root_path: fixtureDir } });
    expect(w.ok(), await w.text()).toBeTruthy();
    const wsId = (await w.json()).id as string;
    const make = async (title: string): Promise<string> => {
      const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: fixtureDir, title } });
      expect(r.ok(), await r.text()).toBeTruthy();
      return (await r.json()).id as string;
    };
    const prompt = async (id: string, text: string) => expect((await ctx.post(`${base}/api/v1/runtime-sessions/${id}/prompt`, { data: { text } })).ok()).toBeTruthy();
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.addInitScript((id) => {
      if (sessionStorage.getItem('threads-status-init')) return;
      sessionStorage.setItem('threads-status-init', '1');
      localStorage.setItem('otto_workspace', id as string);
      localStorage.removeItem('otto_threads_v1');
      localStorage.removeItem('otto_runtime_waiting');
    }, wsId);
    const done = await make('Echo done');
    await prompt(done, 'hello');
    const asks = await make('Asks approval');
    const crash = await make('Crashes');
    await page.goto(`/#/threads/${done}`);
    await expect(page.getByTestId(`runtime-thread-${done}`)).toBeVisible({ timeout: 30_000 });
    const side = page.getByTestId('threads-sidebar');
    await prompt(asks, 'ask');
    await prompt(crash, 'crash');
    await expect(side.getByTestId(`thread-row-${asks}`)).toHaveAttribute('data-state', 'needs-you', { timeout: 30_000 });
    // Live: the turn ended failed while the list watched (any daemon).
    await expect(side.getByTestId(`thread-row-${crash}`)).toHaveAttribute('data-state', /failed|stopped/, { timeout: 30_000 });
    await page.reload();
    await expect(side.getByTestId(`thread-row-${asks}`)).toHaveAttribute('data-state', 'needs-you', { timeout: 30_000 });
    // After a reload only a daemon with `last_turn_status` knows the crash
    // (the installed one before this round shows the row idle).
    await expect(side.getByTestId(`thread-row-${crash}`)).toHaveAttribute('data-state', /failed|stopped|idle/);
    await expect(side.getByTestId('threads-next-needs-you')).toBeVisible();
    await shot(page, 'real-turns');
    for (const id of [done, asks, crash]) await ctx.post(`${base}/api/v1/runtime-sessions/${id}/close`).catch(() => {});
    await ctx.dispose();
  });
});
