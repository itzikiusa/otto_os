import { test, expect } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// A council on a phone (MOCK mode — mock-runtime.ts): the seats become a
// segmented control showing one seat at a time, Synthesize stays reachable,
// and nothing scrolls the page sideways.

test.beforeEach(async ({ page }, ti) => {
  test.skip(!/^(iphone|android)/.test(ti.project.name), 'phones only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_workspace', 'wsp_otto');
    localStorage.removeItem('otto_threads_v1');
  });
});

test('a council shows one seat at a time behind a segmented control', async ({ page }, info) => {
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

  const view = page.getByTestId('council-view');
  await expect(view).toBeVisible({ timeout: 15_000 });
  const one = view.getByTestId('council-column-1');
  const two = view.getByTestId('council-column-2');
  await expect(one).toBeVisible();
  await expect(two).toBeHidden();

  await view.getByTestId('council-tab-2').click();
  await expect(two).toBeVisible();
  await expect(one).toBeHidden();
  await expect(two).toContainText('Seat two says Memcached.');

  const synth = view.getByTestId('council-synthesize');
  await expectFullyInViewport(page, synth, 'Synthesize button');
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('council-phone-light.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: info.outputPath('council-phone-dark.png'), animations: 'disabled' });

  await synth.click();
  await expect(view.getByTestId('council-tab-synthesis')).toBeVisible({ timeout: 15_000 });
  await expect(view.getByTestId('council-column-synthesis')).toContainText('Mock synthesis');
  await expectNoHorizontalOverflow(page);
});
