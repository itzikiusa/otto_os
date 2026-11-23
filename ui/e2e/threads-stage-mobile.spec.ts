import { test, expect, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// The new-thread stage and the Threads frame on a phone and an iPad (MOCK
// mode — mock-runtime.ts): the stage names the repo and shows the access chip
// before sending; the toolbar ⋯ has no Split / Grid / View and no "Continue on
// phone" on the phone itself; the details drawer has a one-line project card,
// no key badges and no Maximize. Screenshots (OTTO_E2E_SHOTS) → fix-T-STAGE/.

type Hooks = { seed: (row: Record<string, unknown>) => string };

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_workspace', 'wsp_otto');
    localStorage.removeItem('otto_threads_v1');
    localStorage.removeItem('otto_threads_chrome_v1');
  });
});

async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'fix-T-STAGE'), { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: join(dir, 'fix-T-STAGE', `${name}-${test.info().project.name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

async function boot(page: Page, hash: string): Promise<void> {
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: Hooks }).__mockRuntime?.seed === 'function');
  await page.evaluate((h) => {
    const m = (window as unknown as { __mockRuntime: Hooks }).__mockRuntime;
    m.seed({ id: 'rt_ph', title: 'Fix flaky login test', provider: 'codex', last_active_at: '2026-11-12T10:09:00Z' });
    location.hash = h;
  }, hash);
}

test('phone: the stage shows the repo and the access before sending', async ({ page }, ti) => {
  test.skip(!/^(iphone|android)/.test(ti.project.name), 'phones only');
  await boot(page, '#/threads/new');
  const stage = page.getByTestId('new-thread-stage');
  await expect(stage).toBeVisible({ timeout: 15_000 });
  await expect(stage.getByTestId('new-thread-project')).toHaveText('otto');
  await stage.getByTestId('picker-agent').click();
  await page.getByTestId('model-picker').getByTestId('model-picker-provider-claude').click();
  await page.keyboard.press('Escape');
  await expect(stage.getByTestId('runtime-access')).toBeVisible();
  await expectFullyInViewport(page, stage.getByTestId('runtime-access'), 'access chip');
  await expectNoHorizontalOverflow(page);
  await shot(page, 'stage');
});

test('phone: ⋯ has no layouts or hand-off; the details drawer is compact', async ({ page }, ti) => {
  test.skip(!/^(iphone|android)/.test(ti.project.name), 'phones only');
  await boot(page, '#/threads/rt_ph');
  await expect(page.getByTestId('runtime-thread-rt_ph')).toBeVisible({ timeout: 15_000 });

  await page.getByTestId('threads-thread-menu').click();
  const menu = page.locator('.ctx-menu').first();
  await expectFullyInViewport(page, menu, 'toolbar ⋯ (phone)');
  const names = await menu.locator('[role^="menuitem"]').allInnerTexts();
  // A phone's View holds only the thread prefs (no layouts).
  for (const gone of [/^Split/, /^Grid/, /^Single thread/, /^Continue on phone/, /^Open beside/]) {
    expect(names.some((n) => gone.test(n)), String(gone)).toBe(false);
  }
  expect(names.filter((n) => /terminal/i.test(n))).toHaveLength(1);
  // The menu is on top of the conversation where it is drawn.
  const box = await menu.boundingBox();
  const onTop = await page.evaluate(({ x, y }) => !!document.elementFromPoint(x, y)?.closest('.ctx-menu'), { x: (box?.x ?? 0) + 20, y: (box?.y ?? 0) + 20 });
  expect(onTop).toBe(true);
  await shot(page, 'more-menu');
  await page.keyboard.press('Escape');

  await page.getByTestId('threads-toggle-inspector').click();
  const panel = page.getByTestId('threads-inspector');
  await expect(panel).toBeVisible();
  const card = panel.getByTestId('thread-details-card');
  await expect(card).toBeVisible();
  await expectFullyInViewport(page, card, 'details card (drawer)');
  await expect(panel.getByRole('button', { name: /Maximize panel|Restore panel/ })).toHaveCount(0);
  await expect(panel.locator('.ti-key').first()).toBeHidden();
  await expect(panel.getByTestId('threads-surface-changes')).toContainText('Changes');
  await shot(page, 'details-drawer');
});

test('iPad portrait: the Threads page fits with its thread list reachable', async ({ page }, ti) => {
  test.skip(ti.project.name !== 'ipad-portrait', 'iPad portrait only');
  await boot(page, '#/threads/rt_ph');
  await expect(page.getByTestId('runtime-thread-rt_ph')).toBeVisible({ timeout: 15_000 });
  await expectNoHorizontalOverflow(page);
  await shot(page, 'page');
});
