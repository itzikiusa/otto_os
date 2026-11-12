// Usage ▸ Remaining (GET /usage/limits): how much of each subscription limit
// is left per provider login, from the files the CLIs already write. A real
// Codex rollout in the isolated daemon's fixture dir drives the loaded state;
// canned responses drive stale / reset / error / empty, which one daemon root
// cannot show at once. The rule under test throughout: an old percent is
// never shown as current.
import { test, expect } from '@playwright/test';
import { apiCtx } from './seed';
import { openPage, expectAccessible } from './helpers';
import { cannedLimits, claudeLimits, remaining, routeLimits, seedCodexLimits } from './usage-remaining-fixture';

test.describe('usage remaining limits (desktop)', () => {
  test.beforeEach(({}, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop-only suite');
  });

  test('a Codex rollout shows what is left per window; Claude says why it has no numbers', async ({ page }) => {
    const { ctx, base } = await apiCtx();
    await seedCodexLimits(ctx, base);

    // The contract: per-window used + left, the reading's time, credits.
    const body = await (await ctx.get(`${base}/api/v1/usage/limits`)).json();
    const codex = body.accounts.find((a: { key: string }) => a.key === 'codex:default');
    expect(codex.provider).toBe('codex');
    expect(codex.plan).toBe('pro');
    expect(codex.windows.map((w: { name: string }) => w.name)).toEqual(['5-hour', 'Weekly']);
    expect(codex.windows[0].used_percent).toBe(13);
    expect(codex.windows[0].left_percent).toBe(87);
    expect(codex.credits.balance).toBe('62500.0');
    expect(Date.parse(codex.taken_at)).toBeLessThan(Date.now());
    // The isolated daemon's Claude fixture dir is a login Otto has no source for.
    const claude = body.accounts.find((a: { provider: string }) => a.provider === 'claude');
    expect(claude.status).toBe('unknown');
    expect(claude.reason).toContain('does not write its plan limits');
    expect(claude.windows).toEqual([]);

    await openPage(page, 'usage');
    const section = remaining(page);
    await expect(section).toBeVisible();
    // At the top of the page body, above the token stat cards.
    await expect(section.getByRole('heading', { name: 'Remaining' })).toBeVisible();

    const row = section.getByTestId('limits-row-codex:default');
    await expect(row).toContainText('Default Codex login');
    await expect(row).toContainText('Pro');
    await expect(row).toContainText(/updated (just now|\d+ min ago)/);
    await expect(row).toContainText('Credits: 62,500');
    const fiveHour = row.getByRole('meter', { name: /5-hour/ });
    await expect(fiveHour).toHaveAttribute('aria-valuenow', '87');
    await expect(row.getByTestId('limits-window-primary')).toContainText(/87% left · resets in 2 h \d+ min/);
    await expect(row.getByTestId('limits-window-secondary')).toContainText(/59% left · resets in 3 d \d+ h/);
    await expect(row.getByTestId('limits-stale')).toHaveCount(0);

    const claudeRow = section.locator('[data-testid^="limits-row-claude:"]').first();
    await expect(claudeRow.getByTestId('limits-unknown')).toBeVisible();
    await expect(claudeRow).toContainText('does not write its plan limits');
    await expect(claudeRow.getByRole('meter')).toHaveCount(0);

    await expectAccessible(page);
  });

  test('a stale reading says how old it is and a reset window shows no percent', async ({ page }) => {
    await routeLimits(page, cannedLimits());
    await openPage(page, 'usage');
    const row = remaining(page).getByTestId('limits-row-codex:account:acc-work');
    await expect(row).toHaveAttribute('data-status', 'stale');
    await expect(row.getByTestId('limits-stale')).toBeVisible();
    await expect(row).toContainText('read 5 h ago');
    // The weekly percent carries its age, never a bare "8% left".
    await expect(row.getByTestId('limits-window-secondary')).toContainText('8% left 5 h ago');
    // The 5-hour window reset after the reading: no percent at all.
    const reset = row.getByTestId('limits-window-primary');
    await expect(reset).toContainText('Reset since the last reading');
    await expect(reset).not.toContainText('%');
    await expect(reset.getByRole('meter')).not.toHaveAttribute('aria-valuenow', /.+/);
  });

  test('a failed first load shows the error inline with Retry', async ({ page }) => {
    await routeLimits(page, 'error');
    await openPage(page, 'usage');
    const section = remaining(page);
    await expect(section.getByTestId('load-error')).toContainText('Couldn’t load remaining limits');
    // Retry re-asks; once the daemon answers, the rows replace the error.
    await page.unroute('**/api/v1/usage/limits');
    await routeLimits(page, cannedLimits());
    await section.getByRole('button', { name: 'Retry' }).click();
    await expect(section.getByTestId('limits-row-codex:account:acc-work')).toBeVisible();
    await expect(section.getByTestId('load-error')).toHaveCount(0);
  });

  test('a Claude login: off by default, confirmed opt-in, then real numbers', async ({ page }) => {
    let state: 'off' | 'on' | 'ok' = 'off';
    const calls: string[] = [];
    await page.route('**/api/v1/usage/limits**', async (route) => {
      const req = route.request();
      const path = new URL(req.url()).pathname;
      calls.push(`${req.method()} ${path}`);
      if (req.method() === 'PUT') {
        expect(req.postDataJSON()).toEqual({ key: 'claude:account:acc-max', enabled: true });
        state = 'on';
      } else if (req.method() === 'POST') {
        expect(req.postDataJSON()).toEqual({ key: 'claude:account:acc-max' });
        state = 'ok';
      }
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(claudeLimits(state)) });
    });
    await openPage(page, 'usage');
    const row = remaining(page).getByTestId('limits-row-claude:account:acc-max');
    // Off by default: the reason, no numbers, and nothing is ever asked.
    await expect(row.getByTestId('limits-unknown')).toBeVisible();
    await expect(row.getByTestId('limits-check-status')).toContainText('Otto never asks Anthropic');
    const sw = row.getByRole('switch', { name: 'Check Max with Anthropic' });
    await expect(sw).toHaveAttribute('aria-checked', 'false');
    expect(calls.filter((c) => !c.startsWith('GET'))).toEqual([]);
    // Turning it on says what runs and where it goes; cancelling changes nothing.
    await sw.click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toContainText('does not read the login');
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    expect(calls.filter((c) => !c.startsWith('GET'))).toEqual([]);
    await sw.click();
    await page.getByRole('dialog').getByRole('button', { name: 'Turn on' }).click();
    // PUT the switch, then one check; the row shows the CLI's numbers.
    await expect(row.getByTestId('limits-window-five_hour')).toContainText('77% left · resets in');
    expect(calls.filter((c) => !c.startsWith('GET'))).toEqual([
      'PUT /api/v1/usage/limits/claude-check',
      'POST /api/v1/usage/limits/claude-check/run',
    ]);
    await expect(row).toContainText('Max');
    await expect(row.getByTestId('limits-check-status')).toContainText(/Checked (just now|1 min ago)/);
    await expect(row.getByRole('button', { name: 'Check now' })).toBeEnabled();
  });

  test('a failed Claude check is a reason, never 0%; a stale one carries its age', async ({ page }) => {
    await routeLimits(page, claudeLimits('failed'));
    await openPage(page, 'usage');
    const row = remaining(page).getByTestId('limits-row-claude:account:acc-max');
    await expect(row).toContainText('Couldn’t check with Anthropic');
    await expect(row.getByRole('meter')).toHaveCount(0);
    await expect(row).not.toContainText('0%');
    await expect(row.getByTestId('limits-check-status')).toContainText('Last check failed');

    await page.unroute('**/api/v1/usage/limits');
    // A stale check is not re-run behind the user's back more than once per
    // visit; the route answers any run with the same stale reading.
    await page.route('**/api/v1/usage/limits**', (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(claudeLimits('stale')) }),
    );
    await page.reload();
    const stale = remaining(page).getByTestId('limits-row-claude:account:acc-max');
    await expect(stale.getByTestId('limits-stale')).toBeVisible();
    await expect(stale.getByTestId('limits-window-five_hour')).toContainText('77% left 2 h ago');
  });

  test('no logins at all is a calm empty state, not an error', async ({ page }) => {
    await routeLimits(page, { accounts: [], generated_at: new Date().toISOString(), stale_after_secs: 1800 });
    await openPage(page, 'usage');
    const section = remaining(page);
    await expect(section.getByText('No provider logins yet')).toBeVisible();
    await expect(section.getByTestId('load-error')).toHaveCount(0);
  });
});
