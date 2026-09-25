import { test, expect } from '@playwright/test';

test('subscription picker isolates providers, adds a profile and checks its login', async ({ page }) => {
  const accounts = [
    { id: 'c', provider: 'claude', label: 'Claude only', created_at: '2026-01-01T00:00:00Z' },
    { id: 'a', provider: 'codex', label: 'Work', created_at: '2026-01-01T00:00:00Z' },
  ];
  let statusRequests = 0;
  await page.addInitScript(() => { localStorage.setItem('otto_base', location.origin); localStorage.setItem('otto_token', 'fixture'); });
  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path === '/api/v1/auth/provider-accounts') {
      if (route.request().method() === 'POST') {
        expect(route.request().postDataJSON()).toEqual({ provider: 'codex', label: 'Personal' });
        const created = { id: 'b', provider: 'codex', label: 'Personal', created_at: '2026-01-01T00:00:00Z' };
        accounts.push(created); return route.fulfill({ json: created });
      }
      return route.fulfill({ json: accounts });
    }
    if (path === '/api/v1/auth/provider-accounts/b/status') {
      statusRequests++; return route.fulfill({ json: { signed_in: statusRequests > 1 } });
    }
    return route.fulfill({ status: 404, json: { message: 'unexpected fixture request' } });
  });
  await page.goto('/e2e/fixtures/accounts.html');
  const picker = page.getByRole('combobox', { name: 'codex account' });
  await expect(picker.locator('option')).toHaveText(['Default CLI account', 'Work']);
  await picker.selectOption('a');
  await expect(page.getByLabel('Selected account')).toHaveText('a');
  await page.getByRole('button', { name: 'Add account…', exact: true }).click();
  await page.getByLabel('Account label').fill('Personal');
  await page.getByRole('button', { name: 'Create profile', exact: true }).click();
  await expect(picker).toHaveValue('b');
  await page.getByRole('button', { name: 'Check sign-in' }).click();
  await expect(page.locator('p[role=status]')).toHaveText('Sign-in required');
  await page.getByRole('button', { name: 'Check sign-in' }).click();
  await expect(page.locator('p[role=status]')).toHaveText('Signed in');
  await picker.selectOption('');
  await expect(page.getByLabel('Selected account')).toHaveText('default');
  await expect(page.getByRole('button', { name: 'Check sign-in' })).toHaveCount(0);
  await page.setViewportSize({ width: 390, height: 844 });
  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
});
