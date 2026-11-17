import { test, expect, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expectNoHorizontalOverflow } from './helpers';

// The thread list on a phone (MOCK mode — mock-runtime.ts seeds the rows):
// every row is one clear tap target at least 44 px tall, two lines — title
// and time, then what the agent is doing ("Needs you", "Working", "Usage
// limit") and which agent — with no per-row icon buttons (pin / beside / ⋯
// live in the long-press menu), and "Next that needs you" sits on top.
// Screenshots: OTTO_E2E_SHOTS/fix-T-ATTENTION/ light and dark.

test.beforeEach(async ({ page }, ti) => {
  test.skip(!/^(iphone|android)/.test(ti.project.name), 'phones only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_workspace', 'wsp_otto');
    localStorage.removeItem('otto_threads_v1');
    localStorage.removeItem('otto_runtime_waiting');
  });
});

async function shot(page: Page, name: string, project: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'fix-T-ATTENTION'), { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: join(dir, 'fix-T-ATTENTION', `${project}-${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

const T = (min: number): string => new Date(Date.UTC(2026, 10, 16, 9, min)).toISOString();

test('phone rows: two lines, ≥ 44 px, the state in words, one tap target; Next that needs you on top', async ({ page }, ti) => {
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: { seed?: unknown } }).__mockRuntime?.seed === 'function');
  await page.evaluate((t) => {
    const m = (window as unknown as { __mockRuntime: { seed: (r: Record<string, unknown>) => string } }).__mockRuntime;
    m.seed({ id: 'rt_wait', title: 'Approve the database migration before it runs', provider: 'codex', status: 'awaiting_permission', last_active_at: t[0] });
    m.seed({ id: 'rt_work', title: 'Refactor the payments webhook', status: 'working', last_active_at: t[1] });
    m.seed({ id: 'rt_fail', title: 'Summarise the release notes', status: 'idle', last_turn_status: 'failed', last_error_class: 'usage_limit', last_active_at: t[2] });
    m.seed({ id: 'rt_done', title: 'Clean the build directory', status: 'idle', last_turn_status: 'completed', last_active_at: t[3] });
    location.hash = '#/threads/rt_done';
  }, [T(1), T(20), T(10), T(30)]);
  await expect(page.getByTestId('runtime-thread-rt_done')).toBeVisible({ timeout: 15_000 });

  await page.getByTestId('threads-toggle-sidebar').click();
  const side = page.getByTestId('threads-sidebar');
  await expect(side).toBeVisible();
  for (const id of ['rt_wait', 'rt_work', 'rt_fail', 'rt_done']) {
    const main = side.getByTestId(`thread-row-${id}`).locator('.tr-main');
    const box = (await main.boundingBox())!;
    expect(box.height, `${id} row height`).toBeGreaterThanOrEqual(43.9);
    await expect(side.getByTestId(`thread-row-${id}`).locator('.tr-actions')).toBeHidden();
    await expect(side.getByTestId(`thread-row-${id}`).locator('.tr-time')).toBeVisible();
  }
  await expect(side.getByTestId('thread-sub-rt_wait')).toHaveText(/^Needs you · /);
  await expect(side.getByTestId('thread-sub-rt_work')).toHaveText(/^Working · /);
  await expect(side.getByTestId('thread-sub-rt_fail')).toHaveText(/^Usage limit · /);
  await expect(side.getByTestId('thread-sub-rt_done')).not.toHaveText(/Idle/);
  // The waiting thread ranks first.
  const first = await side.locator('[data-testid^="thread-row-"]').first().getAttribute('data-testid');
  expect(first).toBe('thread-row-rt_wait');
  await expectNoHorizontalOverflow(page);
  await shot(page, 'list', ti.project.name);

  // One move to it: the chip opens it and closes the drawer.
  const chip = side.getByTestId('threads-next-needs-you');
  expect((await chip.boundingBox())!.height).toBeGreaterThanOrEqual(43.9);
  await chip.click();
  await expect(page).toHaveURL(/#\/threads\/rt_wait$/);
  await expect(side).toBeHidden();
});
