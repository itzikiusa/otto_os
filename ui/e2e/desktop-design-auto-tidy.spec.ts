import { test, expect } from '@playwright/test';
import { apiCtx } from './seed';

// Design Hall auto-tidy (perf2/11 R1) is OPT-IN: Settings → Backup & restore
// shows the storage gauge with what a pass would free, the toggle starts off,
// and turning it on asks first (Cancel leaves it off; confirming persists it).
// Needs a daemon built from this branch (OTTO_E2E_BIN=target/debug/ottod).

test('design auto-tidy is a visible, off-by-default, confirm-first toggle', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser only');
  const { ctx, base } = await apiCtx();
  try {
    await page.goto('/#/settings/backup');
    const card = page.getByRole('region', { name: 'Design Hall storage', exact: true });
    await expect(card).toBeVisible();
    await expect(card.getByTestId('design-storage-bytes')).toBeVisible();
    await expect(card.getByTestId('design-storage-reclaim')).toContainText('old autosave');
    const toggle = card.getByTestId('design-auto-tidy');
    await expect(toggle).not.toBeChecked();

    // Cancel → still off, nothing persisted.
    await toggle.click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toContainText('Deleted autosaves can’t be restored');
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    await expect(toggle).not.toBeChecked();
    expect((await (await ctx.get(`${base}/api/v1/design/admin/storage`)).json()).auto_tidy).toBe(false);

    // Confirm → on and persisted; turning it off needs no confirm.
    await toggle.click();
    await page.getByRole('dialog').getByRole('button', { name: 'Turn on' }).click();
    await expect(toggle).toBeChecked();
    expect((await (await ctx.get(`${base}/api/v1/design/admin/storage`)).json()).auto_tidy).toBe(true);
    await toggle.click();
    await expect(toggle).not.toBeChecked();
    expect((await (await ctx.get(`${base}/api/v1/design/admin/storage`)).json()).auto_tidy).toBe(false);
  } finally {
    await ctx.dispose();
  }
});
