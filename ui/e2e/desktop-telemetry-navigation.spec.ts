import { test, expect, type Page } from '@playwright/test';

// The navigation roll-up for the sidebar's "used lately" list, for real: the
// managed collector writes ui.navigation spans to ClickHouse, the
// navigation_days materialized view counts them per day, module and device
// class, and GET /telemetry/navigation reads them back. Needs the collector:
// run with OTTO_E2E_TELEMETRY=1 (global-setup enables telemetry and waits for it).

async function api(page: Page): Promise<{ base: string; token: string }> {
  return page.evaluate(() => ({ base: localStorage.getItem('otto_base') ?? '', token: localStorage.getItem('otto_token') ?? '' }));
}

async function visit(page: Page, modules: string[]): Promise<void> {
  for (const m of modules) {
    await page.evaluate((id) => (location.hash = '#/' + id), m);
    await expect(page.locator('.shell')).toBeVisible();
    await expect.poll(() => page.evaluate(() => location.hash)).toContain(m);
    // A navigation counts once its page has painted; one cut short by the next
    // is recorded as cancelled and never counted.
    await page.waitForTimeout(2_000); // ui-guards: allow — lets the page paint before the next navigation
  }
}

test('navigations roll up per module and device class and read back', async ({ page, browser }, info) => {
  test.skip(process.env.OTTO_E2E_TELEMETRY !== '1', 'needs the managed collector (OTTO_E2E_TELEMETRY=1)');
  test.setTimeout(360_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/home');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  await visit(page, ['git', 'connections', 'usage', 'git']);

  // The same account from a phone (touch, 390 px): its navigations count as "phone".
  const phoneCtx = await browser.newContext({
    viewport: { width: 390, height: 844 },
    hasTouch: true,
    isMobile: true,
    storageState: info.project.use.storageState as string,
  });
  const phone = await phoneCtx.newPage();
  await phone.goto(new URL('/#/agents', info.project.use.baseURL ?? page.url()).toString());
  await expect(phone.locator('.shell')).toBeVisible({ timeout: 30_000 });
  await visit(phone, ['threads', 'agents']);

  const { base, token } = await api(page);
  let rows: { module: string; device: string; count: number }[] = [];
  // Each page sends its spans every 5 s; the daemon exports every 5 min, and a
  // read pulls that forward (at most every 2 min) — so this waits up to ~4 min.
  await expect
    .poll(
      async () => {
        const r = await page.request.get(`${base}/api/v1/telemetry/navigation?days=30`, { headers: { Authorization: `Bearer ${token}` } });
        rows = r.ok() ? await r.json() : [];
        const has = (m: string, d: string) => rows.some((x) => x.module === m && x.device === d && x.count > 0);
        return has('git', 'desktop') && has('connections', 'desktop') && has('agents', 'phone') && has('threads', 'phone');
      },
      { timeout: 270_000, intervals: [5_000] },
    )
    .toBe(true);
  console.log('[telemetry-navigation] rows', JSON.stringify(rows));
  expect(rows.find((x) => x.module === 'git' && x.device === 'desktop')!.count).toBeGreaterThanOrEqual(2);
  await phoneCtx.close();
});
