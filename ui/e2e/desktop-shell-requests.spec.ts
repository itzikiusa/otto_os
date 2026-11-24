import { test, expect } from '@playwright/test';

// Shell request diet: switching modules re-reads neither /security-posture
// (read once per daemon boot id) nor /meta, and the machine chip polls on a
// 30 s cadence (no event carries the load average).

test('navigating between modules re-reads no shell facts', async ({ page }) => {
  test.setTimeout(90_000);
  const seen: string[] = [];
  page.on('request', (r) => {
    const path = r.url().replace(/^.*\/api\/v1/, '').split('?')[0];
    if (path === '/security-posture' || path === '/machine/status') seen.push(path);
  });
  await page.goto('/#/settings');
  await expect(page.locator('.statusbar')).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => seen.filter((p) => p === '/security-posture').length).toBe(1);
  const start = seen.length;
  const t0 = Date.now();
  for (const m of ['home', 'agents', 'git', 'connections', 'usage', 'home']) {
    await page.evaluate((id) => (location.hash = '#/' + id), m);
    await expect(page.locator('.shell')).toBeVisible();
    await page.waitForTimeout(2_500); // ui-guards: allow — proving no request follows the navigation
  }
  const during = seen.slice(start);
  expect(during.filter((p) => p === '/security-posture')).toEqual([]);
  // 15 s of navigation: at most one machine read (30 s cadence ± jitter).
  expect(Date.now() - t0).toBeLessThan(30_000);
  expect(during.filter((p) => p === '/machine/status').length).toBeLessThanOrEqual(1);
});
