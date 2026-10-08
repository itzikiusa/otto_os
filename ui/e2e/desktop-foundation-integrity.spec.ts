import { test, expect, type Route } from '@playwright/test';

test.use({ serviceWorkers: 'block' });

test('daemon settings retain a newer port draft while the prior save finishes', async ({ page }) => {
  const settings = { network_listener: { enabled: true, port: 7700 } };
  let pending: Route | undefined;
  await page.route('**/api/v1/settings', route => {
    if (route.request().method() === 'GET') return route.fulfill({ json: settings });
    pending = route;
  });
  await page.goto('/#/settings/daemon');
  const port = page.getByLabel('Port', { exact: true });
  await expect(port).toHaveValue('7700');
  await port.fill('7701');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect.poll(() => !!pending).toBe(true);
  await port.fill('7702');
  expect(pending!.request().postDataJSON().network_listener.port).toBe(7701);
  await pending!.fulfill({ json: { network_listener: { enabled: true, port: 7701 } } });
  await expect(port).toHaveValue('7702');
  await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeEnabled();
  await expect(page.getByRole('status').filter({ hasText: 'Unsaved changes' })).toBeVisible();
});

test('safety posture shows actual exposure and pending restart at desktop and phone widths', async ({ page }) => {
  await page.route('**/api/v1/security-posture', route => route.fulfill({ json: {
    network_listener: true, network_listener_port: 7799, loopback_only: false,
    network_listener_restart_required: true, active_api_tokens: 0,
  } }));
  await page.route('**/api/v1/audit-log*', route => route.fulfill({ json: { entries: [], total: 0 } }));
  await page.goto('/#/settings/trust-safety');
  await expect(page.getByText('Reachable from your network', { exact: false })).toBeVisible();
  await expect(page.getByText('Saved settings differ.', { exact: false })).toBeVisible();
  await expect(page.getByText('Only this Mac can connect', { exact: false })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
  for (const [width, scheme] of [[1440, 'light'], [390, 'dark']] as const) {
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate(async scheme => {
      const path = '/src/lib/stores/ui.svelte.ts';
      const { ui } = await import(path);
      ui.setScheme(scheme);
    }, scheme);
    await expect(page.getByText('Saved settings differ.', { exact: false })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: `/tmp/r01-trust-safety-${width}-${scheme}.png`, fullPage: true });
  }
});
