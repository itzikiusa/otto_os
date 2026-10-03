import { test, expect } from '@playwright/test';
import { apiCtx } from './seed';

// Settings → Backup & restore → Database storage → Run history (perf N8).
// Opt-in: "Keep forever" is the default; choosing a window asks first, and the
// write keeps every other customised data_retention field.
test('run history retention is opt-in, confirmed, and keeps other retention fields', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser only');
  const { ctx, base } = await apiCtx();
  const settings = async () => (await (await ctx.get(`${base}/api/v1/settings`)).json()) as Record<string, any>;
  try {
    const put = await ctx.put(`${base}/api/v1/settings`, {
      data: { data_retention: { audit_log_days: 180, run_history_days: 0 } },
    });
    expect(put.ok(), await put.text()).toBeTruthy();
    await page.goto('/#/settings/backup');
    const card = page.getByRole('region', { name: 'Database storage', exact: true });
    const select = card.getByLabel('Run history', { exact: true });
    await expect(select).toHaveValue('0');
    await expect(select.locator('option:checked')).toHaveText('Keep forever (default)');

    // Cancelling the confirm changes nothing.
    await select.selectOption('30');
    const dialog = page.getByRole('dialog', { name: 'Prune old run history' });
    await expect(dialog).toBeVisible();
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    await expect(select).toHaveValue('0');
    expect((await settings()).data_retention.run_history_days).toBe(0);

    // Confirming opts in, keeping the other customised window.
    await select.selectOption('30');
    await page.getByRole('dialog', { name: 'Prune old run history' }).getByRole('button', { name: 'Keep 30 days' }).click();
    await expect(select).toHaveValue('30');
    await expect.poll(async () => (await settings()).data_retention).toMatchObject({ audit_log_days: 180, run_history_days: 30 });

    // Back to forever needs no confirm.
    await select.selectOption('0');
    await expect.poll(async () => (await settings()).data_retention.run_history_days).toBe(0);
  } finally {
    await ctx.put(`${base}/api/v1/settings`, { data: { data_retention: {} } });
    await ctx.dispose();
  }
});
