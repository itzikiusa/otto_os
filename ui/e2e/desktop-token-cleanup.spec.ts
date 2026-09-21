import { test, expect } from '@playwright/test';

test('deleted-session cleanup is explicit, preserves personal/live tokens and retains failures', async ({ page }) => {
  const common = { token_prefix: 'fixture', created_at: '2026-01-01T00:00:00Z', last_seen_at: null, expires_at: '2030-01-01T00:00:00Z' };
  const tokens = [
    { ...common, id: 'personal', label: 'CI token' },
    { ...common, id: 'live', label: 'Live session', session_id: 'l', session_exists: true },
    { ...common, id: 'orphan', label: 'Deleted managed', session_id: 'gone', session_exists: false },
    { ...common, id: 'legacy', label: 'Deleted legacy candidate', legacy_session_id: 'old', session_exists: false },
  ];
  const deleted: string[] = [];
  await page.addInitScript(() => { localStorage.setItem('otto_base', location.origin); localStorage.setItem('otto_token', 'fixture'); });
  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path === '/api/v1/auth/tokens') return route.fulfill({ json: tokens });
    if (route.request().method() === 'DELETE') {
      const id = path.split('/').at(-1)!; deleted.push(id);
      return id === 'legacy' ? route.fulfill({ status: 500, json: { message: 'temporary failure' } }) : route.fulfill({ status: 204 });
    }
    return route.fulfill({ status: 404 });
  });
  await page.goto('/e2e/fixtures/tokens.html');
  await page.getByLabel('Filter tokens').selectOption('orphaned');
  await expect(page.getByText('Deleted managed', { exact: true })).toBeVisible();
  await expect(page.getByText('CI token', { exact: true })).toHaveCount(0);
  expect(deleted).toEqual([]);
  await page.getByRole('button', { name: 'Revoke deleted-session tokens…' }).click();
  await expect(page.getByRole('dialog')).toContainText('Legacy tokens are identified by their labels');
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  expect(deleted).toEqual([]);
  await page.getByRole('button', { name: 'Revoke deleted-session tokens…' }).click();
  await page.getByRole('button', { name: 'Revoke tokens', exact: true }).click();
  await expect.poll(() => deleted).toEqual(['orphan', 'legacy']);
  await expect(page.getByText('Deleted managed', { exact: true })).toHaveCount(0);
  await expect(page.getByText('Deleted legacy candidate', { exact: true })).toBeVisible();
  await page.getByLabel('Filter tokens').selectOption('all');
  await expect(page.getByText('CI token', { exact: true })).toBeVisible();
  await expect(page.getByText('Live session', { exact: true })).toBeVisible();
});
