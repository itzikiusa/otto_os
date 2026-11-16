import { test, expect } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// A council on a phone (MOCK mode — mock-runtime.ts): one seat at a time as
// a normal thread; the header's Related chip lists the other seats and
// switches to one in a tap, the parent chip leads back to seat 1, the ⋯
// council items fit, and nothing scrolls the page sideways.

test.beforeEach(async ({ page }, ti) => {
  test.skip(!/^(iphone|android)/.test(ti.project.name), 'phones only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_workspace', 'wsp_otto');
    localStorage.removeItem('otto_threads_v1');
  });
});

test('a council on a phone: the Related chip switches seats, the parent chip goes back', async ({ page }, info) => {
  test.skip(info.project.name.includes('landscape'), 'portrait phone layout only');
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: { seed?: unknown } }).__mockRuntime?.seed === 'function');
  await page.evaluate(() => {
    const m = (window as unknown as { __mockRuntime: { seed: (r: Record<string, unknown>) => string; say: (sid: string, t: string[]) => void } }).__mockRuntime;
    const meta = (seat: number) => ({ council: { id: 'cnc_phone', seat, role: 'seat' }, read_only: true });
    m.seed({ id: 'rt_p1', title: 'Cache plan · claude', provider: 'claude', last_active_at: '2026-11-12T10:09:00Z', meta: meta(1) });
    m.seed({ id: 'rt_p2', title: 'Cache plan · codex', provider: 'codex', parent_id: 'rt_p1', last_active_at: '2026-11-12T10:08:00Z', meta: meta(2) });
    m.say('rt_p1', ['Seat one says Redis.']);
    m.say('rt_p2', ['Seat two says Memcached.']);
    location.hash = '#/threads/rt_p1';
  });

  await expect(page.getByTestId('runtime-thread-rt_p1')).toContainText('Seat one says Redis.', { timeout: 15_000 });
  const chip = page.getByTestId('threads-related-chip');
  await expect(chip).toHaveText(/1/);
  await chip.click();
  const pop = page.getByTestId('threads-related-popover');
  await expectFullyInViewport(page, pop, 'related popover (phone)');
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('council-phone-light.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: info.outputPath('council-phone-dark.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'light' });

  // One tap: seat 2 as a full thread; its parent chip leads back to seat 1.
  await pop.getByTestId('related-row-rt_p2').locator('.rl-main').click();
  await expect(page.getByTestId('runtime-thread-rt_p2')).toContainText('Seat two says Memcached.');
  const back = page.getByTestId('threads-back-to-parent');
  await expect(back).toContainText('Cache plan · claude');

  // The ⋯ council items are reachable on the phone too.
  await page.getByTestId('threads-thread-menu').click();
  const menu = page.locator('.ctx-menu').first();
  await expect(menu.getByRole('menuitem', { name: 'Ask every seat…' })).toBeVisible();
  await expectFullyInViewport(page, menu, 'thread ⋯ menu (phone)');
  await page.keyboard.press('Escape');

  await back.click();
  await expect(page.getByTestId('runtime-thread-rt_p1')).toBeVisible();
  await expectNoHorizontalOverflow(page);
});

test('a council side by side on a phone: every pane is a full stacked pane', async ({ page }, info) => {
  test.skip(info.project.name.includes('landscape'), 'portrait phone layout only');
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: { seed?: unknown } }).__mockRuntime?.seed === 'function');
  await page.evaluate(() => {
    const m = (window as unknown as { __mockRuntime: { seed: (r: Record<string, unknown>) => string } }).__mockRuntime;
    const meta = (seat: number) => ({ council: { id: 'cnc_grid', seat, role: 'seat' }, read_only: true });
    const seats = ['claude', 'codex', 'gemini', 'grok'];
    seats.forEach((provider, i) =>
      m.seed({
        id: `rt_g${i + 1}`,
        title: `Grid plan · ${provider}`,
        provider,
        ...(i ? { parent_id: 'rt_g1' } : {}),
        last_active_at: `2026-11-12T10:0${9 - i}:00Z`,
        meta: meta(i + 1),
      }),
    );
    location.hash = '#/threads/rt_g1';
  });
  await expect(page.getByTestId('runtime-thread-rt_g1')).toBeVisible({ timeout: 15_000 });
  await page.getByTestId('threads-thread-menu').click();
  await page.locator('.ctx-menu').first().getByRole('menuitem', { name: 'Open council side by side' }).click();
  const canvas = page.getByTestId('threads-canvas');
  await expect(canvas).toHaveAttribute('data-layout', 'grid');
  // The desktop 2×2 rows used to squash panes 1–2 to a header strip.
  for (const id of ['rt_g1', 'rt_g2', 'rt_g3', 'rt_g4']) {
    const pane = canvas.getByTestId(`thread-pane-${id}`);
    await expect(pane).toBeAttached();
    const h = (await pane.boundingBox())?.height ?? 0;
    expect(h, `${id} pane height`).toBeGreaterThanOrEqual(250);
  }
  await expectNoHorizontalOverflow(page);
});
