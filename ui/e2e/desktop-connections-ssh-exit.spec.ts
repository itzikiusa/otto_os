import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// A dead SSH tab used to show "Failed (exit 255)" and nothing else: no reason,
// no Reconnect, and clicking the connection again only re-focused the corpse.
// The host is `.invalid` (never resolves), so `ssh` exits at once; the Test
// probe is fulfilled here so no host is contacted for the diagnosis either.
test('an ended SSH tab says why, is marked, reconnects from the tree and closes without asking', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  const name = `ssh-exit-${Date.now()}`;
  const created = await ctx.post(`${base}/api/v1/workspaces/${ws}/connections`, {
    data: { name, kind: 'ssh', params: { host: 'fixture.invalid' } },
  });
  expect(created.ok()).toBeTruthy();
  const { id } = await created.json();
  await page.route(`**/connections/${id}/test`, (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        ok: false,
        latency_ms: null,
        warn_argv: false,
        message: 'Could not resolve hostname fixture.invalid',
        hint: 'Check the host name, or your VPN / DNS.',
        detail: 'ssh: Could not resolve hostname fixture.invalid: nodename nor servname provided',
      }),
    }),
  );
  let opens = 0;
  page.on('request', (r) => {
    if (r.method() === 'POST' && r.url().endsWith(`/connections/${id}/open`)) opens++;
  });

  await openPage(page, 'connections');
  await page.locator('.conn-row', { hasText: name }).click();
  await expect.poll(() => opens).toBe(1);

  const panel = page.getByTestId('ssh-exit-panel');
  await expect(panel).toBeVisible({ timeout: 20_000 });
  await expect(panel.getByTestId('ssh-exit-message')).toContainText('Could not resolve hostname');
  await expect(panel.getByTestId('ssh-exit-hint')).toContainText('Check the host name');
  await panel.getByText('Details', { exact: true }).click();
  await expect(panel.locator('pre')).toContainText('nodename nor servname');
  await expect(panel.getByTestId('ssh-exit-reconnect')).toBeVisible();
  await expect(page.getByTestId('ssh-tab-ended')).toBeVisible();

  // The connection's row reconnects the ended tab instead of focusing it.
  await page.locator('.conn-row', { hasText: name }).click();
  await expect.poll(() => opens).toBe(2);
  await expect(page.getByTestId('ssh-tab')).toHaveCount(1);

  // Once it has ended again, closing kills nothing that runs: no question.
  await expect(page.getByTestId('ssh-tab-ended')).toBeVisible({ timeout: 20_000 });
  await page.getByRole('button', { name: 'Close terminal tab' }).click();
  await expect(page.getByTestId('ssh-tab')).toHaveCount(0);
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await ctx.dispose();
});
